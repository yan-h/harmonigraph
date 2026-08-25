//! Where the rendered frames go.
//!
//! Three sinks, chosen by the output path's extension:
//!
//! - a **video file** — frames are piped raw to `ffmpeg`, which does the
//!   encoding and muxes the bounced audio in. No encoder crate enters
//!   this workspace to make a video; that is the whole reason for the
//!   subprocess.
//! - a **PNG sequence** (`out/%05d.png`) — for checking one frame, or for
//!   handing stills to something else.
//! - a **raw stream** (`.rgba`) — the escape hatch when ffmpeg isn't
//!   there; pipe it in later with the geometry printed at the end.

use std::io::Write;
use std::process::{Child, Command, Stdio};

pub enum Sink {
    Video { child: Child, writer: Writer },
    Pngs { dir: std::path::PathBuf, stem: String, index: u32, size: [u32; 2] },
    Raw { file: std::fs::File },
}

/// Frames waiting to be written, at most. Two, plus the one the writer is
/// pushing into the pipe and the one the renderer is filling: four buffers in
/// flight, which is 33 MB at 1080p and 133 MB at 4K.
///
/// Deep enough is all this has to be. The queue exists to cover the JITTER
/// between the two halves — a frame that renders fast arriving while the
/// encoder is still on a slow one — not to bank work: over a whole render the
/// halves run at whatever the slower one manages, and a deeper queue only
/// holds more megabytes at that same rate. Bounded by FRAMES rather than by
/// bytes for the same reason a 4K frame is worth four of a 1080p one to hold.
const QUEUE_DEPTH: usize = 2;

/// The thread ffmpeg's stdin is written from, and the buffers going round it.
///
/// The pipe is why this exists. A 1080p frame is 8.3 MB and the macOS pipe
/// buffer is 64 KB, so a blocking `write_all` onto ffmpeg's stdin does not
/// hand a frame over — it waits for x264 to consume essentially the whole of
/// it, with the GPU idle for the duration.
///
/// Measured on a 20 s take, 1439 frames at 1920x1080@60 on eight cores, with
/// the encoder's output byte-identical either way:
///
/// | | wall | of which the renderer spends blocked handing a frame over |
/// |---|---|---|
/// | writing the pipe from the render thread | 12.55 s | 5.25 s |
/// | writing it from here | 10.92 s | 3.10 s |
///
/// So 1.15x, and NOT the 1.6x that "the halves cost about the same, so
/// overlapping them halves the wall" predicts — worth knowing before anyone
/// spends a session chasing the rest. The render half alone, with the encoder
/// stubbed out, is 6.59 s. The two halves do now overlap, but they overlap
/// onto the same eight cores, and x264 at `-preset slow` is most of what runs
/// on them: what is left of the 3.10 s is the encoder being genuinely slower
/// than the renderer over stretches, which is backpressure rather than a
/// stall. A deeper queue defers that; it does not remove it. See
/// [`QUEUE_DEPTH`].
///
/// Buffers are recycled round the two channels rather than allocated per
/// frame, which is the difference between moving the cost off the render
/// thread and moving it into the allocator.
pub struct Writer {
    /// Frames on their way to the encoder. `None` once the writer has stopped
    /// and been collected, which is what makes a second `push` after an early
    /// stop a no-op rather than a panic on a dead channel.
    frames: Option<std::sync::mpsc::SyncSender<Vec<u8>>>,
    /// Written buffers coming back to be refilled. Unbounded, and it cannot
    /// grow past the buffers in circulation: nothing but `push` creates one.
    spare: std::sync::mpsc::Receiver<Vec<u8>>,
    /// `Ok(true)` — everything handed over was written; `Ok(false)` — the
    /// encoder closed the pipe and wants no more (see [`Sink::push`]);
    /// `Err` — the write itself failed.
    thread: Option<std::thread::JoinHandle<Result<bool, String>>>,
}

impl Writer {
    /// Start writing `stdin` from its own thread.
    fn spawn(stdin: std::process::ChildStdin) -> Writer {
        let (frames, incoming) = std::sync::mpsc::sync_channel::<Vec<u8>>(QUEUE_DEPTH);
        let (used, spare) = std::sync::mpsc::channel::<Vec<u8>>();
        let thread = std::thread::spawn(move || {
            let mut stdin = stdin;
            while let Ok(frame) = incoming.recv() {
                match stdin.write_all(&frame) {
                    // The buffer goes back to be refilled. A failed send here
                    // means the render side is gone, which the recv above is
                    // about to report anyway.
                    Ok(()) => {
                        let _ = used.send(frame);
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => return Ok(false),
                    Err(e) => return Err(format!("writing a frame to ffmpeg failed: {e}")),
                }
            }
            Ok(true)
        });
        Writer { frames: Some(frames), spare, thread: Some(thread) }
    }

    /// Hand one frame over, with [`Sink::push`]'s answer.
    ///
    /// The answer is about the frames BEFORE this one: an early stop is
    /// something the writer discovers a frame or three later, so the renderer
    /// draws that many past the point ffmpeg stopped listening and they are
    /// dropped. Which costs a few frames of a render that is ending anyway,
    /// and is what buys the overlap the rest of the time.
    fn push(&mut self, frame: &[u8]) -> Result<bool, String> {
        let Some(frames) = self.frames.as_ref() else {
            return Ok(false);
        };
        let mut buffer = self.spare.try_recv().unwrap_or_default();
        buffer.clear();
        buffer.extend_from_slice(frame);
        if frames.send(buffer).is_ok() {
            return Ok(true);
        }
        // The channel is closed only because the writer returned, and it
        // returns only on an early stop or a failure. Which one is its verdict.
        self.collect()
    }

    /// Stop the writer and take its verdict, closing ffmpeg's stdin with it —
    /// the thread owns the pipe, so this is also what lets ffmpeg reach EOF
    /// and exit.
    fn collect(&mut self) -> Result<bool, String> {
        self.frames = None;
        match self.thread.take() {
            Some(thread) => {
                thread.join().unwrap_or_else(|_| Err("the frame writer panicked".into()))
            }
            // Already collected, by an earlier `push` that found the channel
            // shut: the verdict was returned then.
            None => Ok(false),
        }
    }
}

/// How a video sink should be set up.
pub struct VideoOptions<'a> {
    pub size: [u32; 2],
    pub fps: f64,
    /// The bounced audio to mux in, if any.
    pub audio: Option<&'a std::path::Path>,
    /// x264 constant-rate-factor: lower is better and bigger.
    pub crf: u32,
    /// Explicit ffmpeg path (`--ffmpeg`), overriding the search.
    pub ffmpeg: Option<&'a str>,
    /// Where in the audio file the video's first frame falls, in
    /// seconds. Positive seeks into the audio; negative delays it.
    pub audio_offset: f64,
}

impl Sink {
    /// Pick a sink from the output path.
    pub fn create(path: &std::path::Path, options: &VideoOptions) -> Result<Sink, String> {
        let extension =
            path.extension().and_then(|e| e.to_str()).unwrap_or_default().to_ascii_lowercase();
        match extension.as_str() {
            "png" => {
                let dir = path.parent().unwrap_or(std::path::Path::new(".")).to_path_buf();
                std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
                let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("frame").to_string();
                Ok(Sink::Pngs { dir, stem, index: 0, size: options.size })
            }
            "rgba" | "raw" => {
                let file =
                    std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
                Ok(Sink::Raw { file })
            }
            _ => Sink::video(path, options),
        }
    }

    fn video(path: &std::path::Path, options: &VideoOptions) -> Result<Sink, String> {
        let [w, h] = options.size;
        // yuv420p — the pixel format anything will play — needs even
        // dimensions. Caught here rather than 900 frames later, when
        // ffmpeg finally fails at mux time.
        if !w.is_multiple_of(2) || !h.is_multiple_of(2) {
            return Err(format!("video output needs even dimensions for yuv420p; {w}x{h} is odd"));
        }
        let ffmpeg = find_ffmpeg(options.ffmpeg)?;
        let mut command = Command::new(&ffmpeg);
        command
            .args(["-hide_banner", "-loglevel", "warning", "-y"])
            .args(["-f", "rawvideo", "-pix_fmt", "rgba"])
            .args(["-s", &format!("{w}x{h}")])
            .args(["-r", &format!("{}", options.fps)])
            .args(["-i", "-"]);
        if let Some(audio) = options.audio {
            // Line the soundtrack up with frame 0. Seeking forward and
            // delaying are different flags, and they must go BEFORE the
            // input they apply to.
            if options.audio_offset > 0.001 {
                command.args(["-ss", &format!("{:.6}", options.audio_offset)]);
            } else if options.audio_offset < -0.001 {
                command.args(["-itsoffset", &format!("{:.6}", -options.audio_offset)]);
            }
            command.arg("-i").arg(audio);
        }
        command
            .args(["-c:v", "libx264", "-preset", "slow"])
            .args(["-crf", &options.crf.to_string()])
            .args(["-pix_fmt", "yuv420p"])
            // Frames arrive at exactly `fps` because the replay steps time
            // itself, so the output rate is simply the input rate. No
            // -vsync/-fps_mode: there is nothing to reconcile, and the two
            // spellings of that flag disagree across ffmpeg versions.
            .args(["-r", &format!("{}", options.fps)]);
        if options.audio.is_some() {
            // The visual tail usually outlives the bounce (or the other
            // way round); end on whichever runs out first.
            command.args(["-c:a", "aac", "-b:a", "320k", "-shortest"]);
        }
        command.arg(path).stdin(Stdio::piped());

        let mut child =
            command.spawn().map_err(|e| format!("could not start {}: {e}", ffmpeg.display()))?;
        // The writer thread OWNS the pipe from here. Nothing else may hold a
        // handle on it: ffmpeg reaches EOF when the last one drops, so a
        // second copy left in `child` would keep the encoder waiting for
        // frames through `finish`.
        let stdin = child.stdin.take().ok_or("ffmpeg stdin closed")?;
        Ok(Sink::Video { child, writer: Writer::spawn(stdin) })
    }

    /// Feed one frame. `Ok(true)` means keep going; `Ok(false)` means the
    /// encoder has closed the pipe and wants no more frames — a clean early
    /// stop, NOT a failure. ffmpeg does exactly this under `-shortest` when the
    /// soundtrack ends before the visuals (a one-loop take: audio is the loop,
    /// the picture keeps fading out past it). Whether that early stop was
    /// success or a crash is ffmpeg's call, read from its exit status in
    /// [`Self::finish`]; the caller just stops feeding on `Ok(false)`.
    ///
    /// The video sink answers about the frames before this one — see
    /// [`Writer::push`]. `frame` is copied rather than taken, the renderer
    /// handing over a view of a buffer it owns; the copy is into a buffer the
    /// writer hands back, so it costs a memcpy and no allocation.
    pub fn push(&mut self, frame: &[u8]) -> Result<bool, String> {
        match self {
            Sink::Video { writer, .. } => writer.push(frame),
            Sink::Raw { file } => file.write_all(frame).map(|()| true).map_err(|e| e.to_string()),
            Sink::Pngs { dir, stem, index, size } => {
                let path = dir.join(format!("{stem}-{index:05}.png"));
                *index += 1;
                image::save_buffer(&path, frame, size[0], size[1], image::ExtendedColorType::Rgba8)
                    .map(|()| true)
                    .map_err(|e| format!("{}: {e}", path.display()))
            }
        }
    }

    /// Close the sink and wait for the encoder. Consumes self so a
    /// half-written video can't be mistaken for a finished one.
    pub fn finish(self) -> Result<(), String> {
        match self {
            Sink::Video { mut child, mut writer } => {
                // Drain first: the queued frames are part of the video, and
                // the thread holds the pipe ffmpeg is waiting for EOF on.
                let written = writer.collect();
                let status = child.wait().map_err(|e| format!("waiting for ffmpeg: {e}"))?;
                // The exit status first, when it says something went wrong.
                // A write failing and ffmpeg dying are usually one event seen
                // from both ends, and ffmpeg's end is the one that says what
                // happened — a broken pipe here only says the far end is gone.
                if !status.success() {
                    return Err(format!("ffmpeg exited with {status}"));
                }
                // Whereas a write that failed against an encoder that exited
                // CLEANLY is its own thing, and silently truncates the video.
                written.map(|_| ())
            }
            Sink::Raw { mut file } => file.flush().map_err(|e| e.to_string()),
            Sink::Pngs { .. } => Ok(()),
        }
    }
}

/// Where ffmpeg is installed, when it isn't simply on `PATH`.
///
/// This exists because of how this tool actually gets run. Launched from
/// a shell, `ffmpeg` resolves fine. Launched by the *plugin*, it inherits
/// the DAW's environment — and a macOS app started from Finder gets a
/// minimal `PATH` of `/usr/bin:/bin:/usr/sbin:/sbin`, which contains no
/// Homebrew. Searching these by hand turns "render failed, install
/// ffmpeg" (on a machine that has ffmpeg) into a render that works.
const FFMPEG_LOCATIONS: [&str; 4] = [
    "/opt/homebrew/bin/ffmpeg", // Homebrew, Apple Silicon
    "/usr/local/bin/ffmpeg",    // Homebrew, Intel
    "/opt/local/bin/ffmpeg",    // MacPorts
    "/usr/bin/ffmpeg",          // system / Linux
];

/// Resolve ffmpeg: an explicit choice, then `LATTICE_FFMPEG`, then
/// `PATH`, then the conventional install locations. The error names
/// everything tried, because "not found" with no list is unactionable.
fn find_ffmpeg(explicit: Option<&str>) -> Result<std::path::PathBuf, String> {
    let runnable = |path: &std::path::Path| path.is_file();

    if let Some(explicit) = explicit.map(str::trim).filter(|s| !s.is_empty()) {
        let path = std::path::PathBuf::from(explicit);
        return if runnable(&path) {
            Ok(path)
        } else {
            Err(format!("--ffmpeg {explicit:?} is not a file"))
        };
    }
    if let Ok(from_env) = std::env::var("LATTICE_FFMPEG") {
        let path = std::path::PathBuf::from(&from_env);
        return if runnable(&path) {
            Ok(path)
        } else {
            Err(format!("LATTICE_FFMPEG={from_env:?} is not a file"))
        };
    }
    let searched: Vec<std::path::PathBuf> = std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).map(|dir| dir.join("ffmpeg")).collect())
        .unwrap_or_default();
    for candidate in searched.iter().cloned().chain(FFMPEG_LOCATIONS.iter().map(Into::into)) {
        if runnable(&candidate) {
            return Ok(candidate);
        }
    }
    Err(format!(
        "ffmpeg not found. Looked on PATH ({} entr{}) and in {}.\n\
         Install it (brew install ffmpeg), pass --ffmpeg /path/to/ffmpeg, set \
         LATTICE_FFMPEG, or render to a .png sequence or .rgba stream instead.",
        searched.len(),
        if searched.len() == 1 { "y" } else { "ies" },
        FFMPEG_LOCATIONS.join(", "),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stand-in for ffmpeg: a script that ignores the encoder arguments and
    /// does `body` instead. Returns the directory it lives in, so the caller
    /// can name files beside it and remove the lot.
    ///
    /// A subprocess rather than a fake trait because the pipe IS the thing
    /// under test — the queue exists because a real 64 KB kernel pipe blocks a
    /// writer, and an in-process stand-in for it would have neither the
    /// blocking nor the broken-pipe end that the two tests below turn on.
    fn fake_ffmpeg(name: &str, body: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("harmonigraph-sink-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir");
        let script = dir.join("ffmpeg.sh");
        std::fs::write(&script, format!("#!/bin/sh\n{body}\n")).expect("write the script");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
                .expect("chmod");
        }
        dir
    }

    fn video_to(dir: &std::path::Path) -> Sink {
        Sink::create(
            &dir.join("out.mp4"),
            &VideoOptions {
                // Even, or `video` rejects the size before spawning anything.
                size: [4, 2],
                fps: 30.0,
                audio: None,
                crf: 20,
                ffmpeg: Some(dir.join("ffmpeg.sh").to_str().expect("utf-8 path")),
                audio_offset: 0.0,
            },
        )
        .expect("the fake encoder should start")
    }

    /// Every frame reaches the encoder, whole and in order.
    ///
    /// Which is what a thread between the two can break and a blocking write
    /// cannot: frames handed over asynchronously can be dropped at the end
    /// (the queue not drained before the pipe closes), truncated (a buffer
    /// recycled while it is still being written) or reordered. The video is
    /// silently wrong in all three cases — it still plays.
    #[test]
    #[cfg(unix)]
    fn every_frame_handed_over_reaches_the_encoder_in_order() {
        let dir = fake_ffmpeg("in-order", "cat > \"$(dirname \"$0\")/frames.rgba\"");
        let mut sink = video_to(&dir);
        // Distinguishable per frame, so the check is about ORDER and not just
        // about the byte count.
        let frames: Vec<Vec<u8>> = (0..64u8).map(|k| vec![k; 4 * 2 * 4]).collect();
        for frame in &frames {
            assert_eq!(sink.push(frame), Ok(true), "the fake encoder is reading");
        }
        sink.finish().expect("a clean finish");

        let written = std::fs::read(dir.join("frames.rgba")).expect("the encoder's input");
        assert_eq!(
            written,
            frames.concat(),
            "the {} frames handed over are not what arrived",
            frames.len(),
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An encoder that stops listening is a clean early stop, not a failure —
    /// `-shortest` with the soundtrack ending before the picture does exactly
    /// this — and it still reads as one through a queue, where the news
    /// arrives some frames after the event.
    #[test]
    #[cfg(unix)]
    fn an_encoder_that_closes_the_pipe_early_stops_the_render_without_failing_it() {
        // Exits at once, having read nothing: every write after that is a
        // broken pipe.
        let dir = fake_ffmpeg("early-stop", "exit 0");
        let mut sink = video_to(&dir);
        // Well past the 64 KB a pipe will hold without a reader, so this
        // cannot end by running out of frames instead.
        let frame = vec![0u8; 64 * 1024];
        let mut fed = 0;
        while sink.push(&frame).expect("an early stop is not an error") {
            fed += 1;
            assert!(fed < 4096, "the encoder is gone and the sink went on accepting frames");
        }
        sink.finish().expect("an encoder that exited cleanly is a clean finish");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_explicit_ffmpeg_that_does_not_exist_is_reported_not_ignored() {
        let err = find_ffmpeg(Some("/definitely/not/here/ffmpeg")).unwrap_err();
        assert!(err.contains("not a file"), "{err}");
    }

    /// A blank field (the plugin's Options box, left empty) must fall
    /// through to the search rather than being treated as a path.
    #[test]
    fn a_blank_explicit_path_falls_through_to_the_search() {
        // Whatever the machine has, this must not complain about "" .
        match find_ffmpeg(Some("   ")) {
            Ok(_) => {}
            Err(err) => assert!(!err.contains("not a file"), "{err}"),
        }
    }

    /// The conventional locations are the whole point: a plugin inherits
    /// the DAW's minimal PATH, so the list has to cover Homebrew.
    #[test]
    fn the_search_list_covers_both_homebrew_prefixes() {
        assert!(FFMPEG_LOCATIONS.contains(&"/opt/homebrew/bin/ffmpeg"));
        assert!(FFMPEG_LOCATIONS.contains(&"/usr/local/bin/ffmpeg"));
    }
}
