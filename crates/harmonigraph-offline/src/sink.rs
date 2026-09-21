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
    Video { child: Child, writer: Writer, encoded: Encoded, frames: u64 },
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
    /// Frames on their way to the encoder; dropped to close the queue.
    frames: Option<std::sync::mpsc::SyncSender<Vec<u8>>>,
    /// Written buffers coming back to be refilled. Unbounded, and it cannot
    /// grow past the buffers in circulation: nothing but `push` creates one.
    spare: std::sync::mpsc::Receiver<Vec<u8>>,
    thread: Option<std::thread::JoinHandle<Result<(), String>>>,
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
                    Err(e) => return Err(format!("writing a frame to ffmpeg failed: {e}")),
                }
            }
            Ok(())
        });
        Writer { frames: Some(frames), spare, thread: Some(thread) }
    }

    /// Hand one frame over and take a buffer back.
    ///
    /// The frame is MOVED into the queue and a written one comes back in its
    /// place, so the same handful of buffers go round for a whole render and
    /// the pixels are copied once, out of the mapped readback. `try_recv`
    /// rather than `recv`: waiting for a buffer to come back would be waiting
    /// for the encoder, which is what [`QUEUE_DEPTH`] exists not to do. Its
    /// empty answer mints one more buffer, which is how the circulation
    /// reaches its steady size over the first few frames.
    fn push(&mut self, frame: Vec<u8>) -> Result<Vec<u8>, String> {
        let frames = self.frames.as_ref().ok_or("the frame writer has stopped")?;
        match frames.send(frame) {
            Ok(()) => Ok(self.spare.try_recv().unwrap_or_default()),
            Err(_) => {
                self.collect()?;
                Err("the frame writer stopped before accepting all frames".into())
            }
        }
    }

    /// Stop the writer and take its verdict, closing ffmpeg's stdin with it —
    /// the thread owns the pipe, so this is also what lets ffmpeg reach EOF
    /// and exit.
    fn collect(&mut self) -> Result<(), String> {
        self.frames = None;
        match self.thread.take() {
            Some(thread) => {
                thread.join().unwrap_or_else(|_| Err("the frame writer panicked".into()))
            }
            // Already collected, by an earlier `push` that found the channel
            // shut: the verdict was returned then.
            None => Ok(()),
        }
    }
}

/// The silence ffmpeg's native AAC encoder puts before the first real sample:
/// 1024 samples at the 48 kHz it is asked to encode at.
///
/// A decoder learns to drop these only from an edit list, and YouTube asks for
/// a file without one — so the soundtrack starts this much EARLIER in the input
/// instead, and the priming lands where the input's first 21 ms would have
/// been. Measured with a click against a one-frame flash: 21.3 ms late
/// without this, sample-exact with it, at 24, 30 and 60 fps. Specific to `-c:a
/// aac`; another encoder primes by its own amount (Apple's by 2112).
const AAC_PRIMING_SECONDS: f64 = 1024.0 / 48_000.0;

/// How far into the soundtrack the video's first frame falls: the `-ss`
/// [`video_args`] seeks the audio input by, priming included. Negative means
/// the soundtrack starts after the picture and is delayed instead of seeked.
fn soundtrack_seek(audio_offset: f64) -> f64 {
    audio_offset + AAC_PRIMING_SECONDS
}

/// ffmpeg's own count of the frames it has encoded, read off its `-progress`
/// report.
///
/// Not the count handed over, which is all [`Writer`] knows. Between the two
/// sits ffmpeg's backlog — its queues and x264's lookahead — and it is not
/// small: on a 2560x1440 render of 5320 frames the last frame was handed over
/// 26 s before ffmpeg exited, with 15% of the file still to be written. A
/// progress bar driven by frames handed over sat full through all of it.
pub struct Encoded {
    /// Each count ffmpeg reports, in order. Closed when ffmpeg closes its
    /// stdout, which it does on exit.
    counts: std::sync::mpsc::Receiver<u64>,
    latest: u64,
}

impl Encoded {
    fn spawn(stdout: std::process::ChildStdout) -> Encoded {
        let (report, counts) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            use std::io::BufRead;
            // Read to the end whether or not anyone is still listening: a
            // report nobody drains is a pipe that fills and stalls the encoder.
            for line in std::io::BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Some(frames) =
                    line.strip_prefix("frame=").and_then(|n| n.trim().parse().ok())
                {
                    let _ = report.send(frames);
                }
            }
        });
        Encoded { counts, latest: 0 }
    }

    /// The newest count, without waiting for one.
    fn latest(&mut self) -> u64 {
        self.latest = self.counts.try_iter().last().unwrap_or(self.latest);
        self.latest
    }
}

/// How a video sink should be set up.
pub struct VideoOptions<'a> {
    pub size: [u32; 2],
    pub fps: f64,
    /// Authoritative number of video frames, including the visual tail.
    pub frames: u64,
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

impl VideoOptions<'_> {
    /// Samples available after reserving AAC priming within the video duration.
    fn audio_samples(&self) -> u64 {
        ((self.frames as f64 / self.fps * 48_000.0).floor() as u64).saturating_sub(1024)
    }
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
        if options.audio.is_some() && options.audio_samples() == 0 {
            eprintln!(
                "note: video duration is at or below AAC priming (21.333 ms); exporting video only"
            );
        }
        let ffmpeg = find_ffmpeg(options.ffmpeg)?;
        let mut command = Command::new(&ffmpeg);
        command.args(video_args(options, path)).stdin(Stdio::piped()).stdout(Stdio::piped());

        let mut child =
            command.spawn().map_err(|e| format!("could not start {}: {e}", ffmpeg.display()))?;
        // The writer thread OWNS the pipe from here. Nothing else may hold a
        // handle on it: ffmpeg reaches EOF when the last one drops, so a
        // second copy left in `child` would keep the encoder waiting for
        // frames through `finish`.
        let stdin = child.stdin.take().ok_or("ffmpeg stdin closed")?;
        let stdout = child.stdout.take().ok_or("ffmpeg stdout closed")?;
        Ok(Sink::Video {
            child,
            writer: Writer::spawn(stdin),
            encoded: Encoded::spawn(stdout),
            frames: options.frames,
        })
    }

    /// Frames the encoder has finished, for the sink that has one. `None` for
    /// the other two, which write a frame as it is pushed: there a frame handed
    /// over is a frame done.
    pub fn encoded(&mut self) -> Option<u64> {
        match self {
            Sink::Video { encoded, .. } => Some(encoded.latest()),
            Sink::Pngs { .. } | Sink::Raw { .. } => None,
        }
    }

    /// Feed one frame and take a buffer back for the next one.
    /// Every write failure, including a broken pipe, fails the export.
    ///
    /// `frame` is TAKEN rather than borrowed, and the buffer that comes back
    /// is the one to draw into next. The renderer would otherwise allocate a
    /// frame and this would copy it into a recycled one, which at 1440p is
    /// 14.7 MB allocated and 14.7 MB memcpy'd per frame for a picture that is
    /// already a copy out of the mapped readback. The two sinks with no thread
    /// behind them hand the same buffer straight back, having finished with it
    /// by the time they return.
    pub fn push(&mut self, frame: Vec<u8>) -> Result<Vec<u8>, String> {
        match self {
            Sink::Video { writer, .. } => writer.push(frame),
            Sink::Raw { file } => file.write_all(&frame).map(|()| frame).map_err(|e| e.to_string()),
            Sink::Pngs { dir, stem, index, size } => {
                let path = dir.join(format!("{stem}-{index:05}.png"));
                *index += 1;
                image::save_buffer(&path, &frame, size[0], size[1], image::ExtendedColorType::Rgba8)
                    .map(|()| frame)
                    .map_err(|e| format!("{}: {e}", path.display()))
            }
        }
    }

    /// Close the sink and wait for the encoder, handing `progress` each count
    /// of encoded frames on the way — the encoder's backlog is the part of a
    /// render that goes on after the last frame is drawn. Consumes self so a
    /// half-written video can't be mistaken for a finished one.
    pub fn finish(self, mut progress: impl FnMut(u64)) -> Result<(), String> {
        match self {
            Sink::Video { mut child, mut writer, mut encoded, frames } => {
                // Close the queue without waiting on it: the writer drains
                // what is queued — part of the video — and then drops the pipe,
                // which is ffmpeg's EOF. ffmpeg closing its stdout on exit is
                // what ends the reports.
                writer.frames = None;
                for count in encoded.counts.iter() {
                    encoded.latest = count;
                    progress(count);
                }
                let written = writer.collect();
                let status = child.wait().map_err(|e| format!("waiting for ffmpeg: {e}"))?;
                // The exit status first, when it says something went wrong.
                // A write failing and ffmpeg dying are usually one event seen
                // from both ends, and ffmpeg's end is the one that says what
                // happened — a broken pipe here only says the far end is gone.
                if !status.success() {
                    return Err(match written {
                        Err(error) => format!("ffmpeg exited with {status}; {error}"),
                        Ok(()) => format!("ffmpeg exited with {status}"),
                    });
                }
                // Whereas a write that failed against an encoder that exited
                // CLEANLY is its own thing, and silently truncates the video.
                written?;
                if encoded.latest != frames {
                    return Err(format!(
                        "ffmpeg encoded {} frames; expected {frames}",
                        encoded.latest
                    ));
                }
                Ok(())
            }
            Sink::Raw { mut file } => file.flush().map_err(|e| e.to_string()),
            Sink::Pngs { .. } => Ok(()),
        }
    }
}

/// Every argument ffmpeg is started with, bar the program itself.
///
/// Split out of [`Sink::video`] so that what it builds can be read back
/// without running an encoder. Constructed straight into a `Command`, the
/// audio branches below were observable from no test at all — the sink's own
/// tests pass `audio: None` — and the tail the `apad` here pays back went
/// missing through exactly that gap (#985).
///
/// Paths arrive as UTF-8 and stay it: the output path comes from argv and the
/// audio file from a take header beside it, both `String`s upstream.
fn video_args(options: &VideoOptions, path: &std::path::Path) -> Vec<String> {
    let [w, h] = options.size;
    let mut args: Vec<String> = Vec::new();
    args.extend(["-hide_banner", "-loglevel", "warning", "-y"].map(String::from));
    // `key=value` lines on stdout, twice a second: see [`Encoded`].
    args.extend(["-progress", "pipe:1"].map(String::from));
    args.extend(["-f", "rawvideo", "-pix_fmt", "rgba"].map(String::from));
    args.extend(["-s".to_string(), format!("{w}x{h}")]);
    args.extend(["-r".to_string(), format!("{}", options.fps)]);
    args.extend(["-i", "-"].map(String::from));
    // Line the soundtrack up with frame 0, and pull it earlier by the AAC
    // encoder's priming, which a decoder only knows to skip through an
    // edit list this file no longer carries (see the muxer flags below).
    // Forward is a seek into the input, so it goes BEFORE the `-i` it
    // applies to; backward is real silence, because
    // `-itsoffset` delays by an edit list too — ignored, it put a render
    // that opens before the bounce 510 ms out of sync.
    let shift = soundtrack_seek(options.audio_offset);
    let audio = options.audio.filter(|_| options.audio_samples() > 0);
    if let Some(audio) = audio {
        if shift > 0.0 {
            args.extend(["-ss".to_string(), format!("{shift:.6}")]);
        }
        args.extend(["-i".to_string(), audio.to_string_lossy().into_owned()]);
    }
    // YouTube's recommended upload encoding, setting by setting
    // (support.google.com/youtube/answer/1722171): High profile, two
    // B-frames, a closed GOP of half the frame rate (x264 closes its GOPs
    // by default), 4:2:0, BT.709.
    let gop = ((options.fps / 2.0).round() as u32).max(1);
    args.extend(["-c:v", "libx264", "-preset", "slow", "-profile:v", "high"].map(String::from));
    args.extend(["-crf".to_string(), options.crf.to_string()]);
    // The spectrogram's noise floor is grain to an encoder, and half a
    // second of GOP puts a keyframe in front of it twice a second.
    // Each re-draws the grain afresh, which reads as a flicker; grain
    // tuning narrows how differently I, P and B frames draw it. On a
    // 720p60 take at the default CRF, the keyframe jump fell from 0.77
    // to 0.59 grey levels for 5.0 -> 7.4 Mbps — where YouTube puts
    // 720p60.
    args.extend(["-tune", "grain"].map(String::from));
    args.extend(["-bf".to_string(), "2".to_string(), "-g".to_string(), gop.to_string()]);
    // Converted AND tagged. ffmpeg's default conversion is BT.601 and
    // writes no tag, which YouTube reads as BT.709 — every saturated
    // colour shifted. A tag without the conversion is the same shift
    // stated as correct. The tags go on the FRAMES: ffmpeg 7 takes an
    // encoder's colour from what the filters hand it, and silently
    // drops `-color_primaries`/`-color_trc` given as output options.
    args.push("-vf".to_string());
    args.push(
        "scale=out_color_matrix=bt709:out_range=tv,format=yuv420p,\
         setparams=color_primaries=bt709:color_trc=bt709:colorspace=bt709:range=tv"
            .to_string(),
    );
    // Frames arrive at exactly `fps` because the replay steps time
    // itself, so the output rate is simply the input rate. No
    // -vsync/-fps_mode: there is nothing to reconcile, and the two
    // spellings of that flag disagree across ffmpeg versions.
    args.extend(["-r".to_string(), format!("{}", options.fps)]);
    if audio.is_some() {
        let mut filters = Vec::new();
        if shift < 0.0 {
            filters.push(format!("adelay={:.3}:all=1", -shift * 1000.0));
        }
        // With no edit list, AAC priming remains part of the output duration.
        // Reserve its 1024 samples, then pad/trim the resampled input to the
        // remaining video span. Full-duration input would make the container
        // 21.3 ms longer than the picture; `-shortest` would discard its tail.
        let samples = options.audio_samples();
        filters.extend([
            "aresample=48000".to_string(),
            "apad".to_string(),
            format!("atrim=end_sample={samples}"),
        ]);
        args.extend(["-af".to_string(), filters.join(",")]);
        args.extend(["-c:a", "aac", "-b:a", "384k", "-ar", "48000"].map(String::from));
    }
    // YouTube asks for the index up front and no edit lists. Signed
    // composition offsets are what let the B-frames' reorder delay go
    // without one: the first frame is presented at 0 rather than an edit
    // later, which would put the audio ahead by two frames' worth.
    args.extend(
        ["-movflags", "+faststart+negative_cts_offsets", "-use_editlist", "0"].map(String::from),
    );
    args.push(path.to_string_lossy().into_owned());
    args
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
                frames: 64,
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
        let dir = fake_ffmpeg(
            "in-order",
            "cat > \"$(dirname \"$0\")/frames.rgba\"\nprintf 'frame=64\\n'",
        );
        let mut sink = video_to(&dir);
        // Distinguishable per frame, so the check is about ORDER and not just
        // about the byte count.
        let frames: Vec<Vec<u8>> = (0..64u8).map(|k| vec![k; 4 * 2 * 4]).collect();
        // Fed the way the renderer feeds it: the frame goes in by move and the
        // buffer that comes back is the one refilled for the next frame. That
        // is what makes "truncated — a buffer recycled while it is still being
        // written" reachable here; handing a fresh `Vec` over each time would
        // check the queue and never the recycling.
        let mut buffer = Vec::new();
        for frame in &frames {
            buffer.clear();
            buffer.extend_from_slice(frame);
            buffer = sink.push(buffer).expect("the fake encoder is reading");
        }
        sink.finish(|_| {}).expect("a clean finish");

        let written = std::fs::read(dir.join("frames.rgba")).expect("the encoder's input");
        assert_eq!(
            written,
            frames.concat(),
            "the {} frames handed over are not what arrived",
            frames.len(),
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A closed pipe is a failure even when ffmpeg exits zero; the payload
    /// must exceed the kernel pipe capacity to actually reach the write error.
    #[test]
    #[cfg(unix)]
    fn an_encoder_that_closes_the_pipe_early_fails_and_is_reaped() {
        for exit in [0, 7] {
            let dir = fake_ffmpeg(&format!("early-stop-{exit}"), &format!("exit {exit}"));
            let mut sink = video_to(&dir);
            let mut failure = None;
            for _ in 0..64 {
                if let Err(error) = sink.push(vec![0; 1024 * 1024]) {
                    failure = Some(error);
                    break;
                }
            }
            assert!(failure.unwrap().contains("writing a frame to ffmpeg failed"));
            let error = sink.finish(|_| {}).unwrap_err();
            if exit == 0 {
                assert!(error.contains("expected 64"), "{error}");
            } else {
                assert!(error.contains("ffmpeg exited"), "{error}");
            }
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    #[test]
    #[cfg(unix)]
    fn a_write_failure_while_draining_is_reported() {
        let dir = fake_ffmpeg("drain-failure", "exit 0");
        let mut sink = video_to(&dir);
        sink.push(vec![0; 1024 * 1024]).expect("the first frame fits the queue");
        let error = sink.finish(|_| {}).unwrap_err();
        assert!(error.contains("writing a frame to ffmpeg failed"), "{error}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[cfg(unix)]
    fn a_successful_exit_with_too_few_encoded_frames_fails() {
        let dir = fake_ffmpeg("short-count", "cat > /dev/null\nprintf 'frame=63\\n'");
        let mut sink = video_to(&dir);
        for _ in 0..64 {
            sink.push(vec![0; 32]).unwrap();
        }
        let error = sink.finish(|_| {}).unwrap_err();
        assert!(error.contains("encoded 63 frames; expected 64"), "{error}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The encoder's backlog is reported, not waited out in silence: counts
    /// ffmpeg gives after the last frame is handed over reach `finish`'s
    /// caller, up to the last one.
    ///
    /// The stand-in reports only once its input has ended, so every count
    /// here arrives during `finish` — which is where a real encoder spends
    /// the tail of a render, with the last frame drawn and the video not yet
    /// written.
    #[test]
    #[cfg(unix)]
    fn finish_reports_the_encoders_count_until_it_is_done() {
        let dir = fake_ffmpeg(
            "backlog",
            "cat > /dev/null\nprintf 'frame=40\\nfps=12.0\\nprogress=continue\\nframe=64\\nprogress=end\\n'",
        );
        let mut sink = video_to(&dir);
        let mut buffer = Vec::new();
        for _ in 0..64 {
            buffer.clear();
            buffer.resize(4 * 2 * 4, 0);
            buffer = sink.push(buffer).expect("the fake encoder is reading");
        }
        let mut reported = Vec::new();
        sink.finish(|frames| reported.push(frames)).expect("a clean finish");
        assert_eq!(reported, [40, 64]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Use the actual encoder and muxer: inspecting arguments cannot prove
    /// that AAC priming, empty inputs and the container obey the frame plan.
    fn real_ffmpeg() -> Option<std::path::PathBuf> {
        let ffmpeg = find_ffmpeg(None).ok()?;
        if Command::new(ffmpeg.with_file_name("ffprobe")).arg("-version").output().is_err() {
            eprintln!("skipping real encoder tests: ffprobe unavailable");
            return None;
        }
        Some(ffmpeg)
    }

    fn probe(ffmpeg: &std::path::Path, path: &std::path::Path) -> String {
        let output = Command::new(ffmpeg.with_file_name("ffprobe"))
            .args([
                "-v",
                "error",
                "-show_entries",
                "stream=codec_type,nb_frames,duration:format=duration",
                "-of",
                "flat",
            ])
            .arg(path)
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        String::from_utf8(output.stdout).unwrap()
    }

    fn write_wav(path: &std::path::Path, rate: u32, seconds: f64, click: Option<f64>) {
        let mut samples = vec![0.0; (seconds * f64::from(rate)) as usize];
        if let Some(click) = click {
            samples[(click * f64::from(rate)).round() as usize] = 0.8;
        }
        let mut writer = harmonigraph_take::WavWriter::create(path, rate as f32, 1).unwrap();
        writer.write(&samples).unwrap();
        writer.finish().unwrap();
    }

    fn encode(options: &VideoOptions, path: &std::path::Path, flash: Option<u64>) {
        let mut sink = Sink::create(path, options).unwrap();
        let mut buffer = Vec::new();
        for frame in 0..options.frames {
            buffer.resize((options.size[0] * options.size[1] * 4) as usize, 0);
            buffer.fill(if Some(frame) == flash { 255 } else { 0 });
            buffer = sink.push(buffer).unwrap();
        }
        sink.finish(|_| {}).unwrap();
    }

    #[test]
    fn real_encoder_preserves_the_plan_for_every_soundtrack_extent() {
        let Some(ffmpeg) = real_ffmpeg() else { return };
        let dir =
            std::env::temp_dir().join(format!("harmonigraph-real-duration-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // N/fps = 9.05 rather than a whole second. The priming-band seek is
        // still inside the WAV before compensation, but beyond EOF after it.
        for (name, seconds, offset, frames, fps) in [
            ("none", None, 0.0, 181, 20.0),
            ("short", Some(0.25), 0.0, 181, 20.0),
            ("long", Some(12.0), 0.0, 181, 20.0),
            ("empty", Some(0.0), 0.0, 181, 20.0),
            ("exhausted", Some(0.25), 3.0, 181, 20.0),
            ("priming-band", Some(0.25), 0.24, 181, 20.0),
            ("delay", Some(0.25), -0.5, 181, 20.0),
            ("seek", Some(1.0), 0.25, 181, 20.0),
            ("fractional-sample", Some(2.0), 0.0, 61, 59.94),
        ] {
            let wav = dir.join(format!("{name}.wav"));
            if let Some(seconds) = seconds {
                write_wav(&wav, 44_100, seconds, None);
            }
            let out = dir.join(format!("{name}.mp4"));
            let options = VideoOptions {
                size: [16, 16],
                fps,
                frames,
                audio: seconds.map(|_| wav.as_path()),
                crf: 20,
                ffmpeg: ffmpeg.to_str(),
                audio_offset: offset,
            };
            encode(&options, &out, None);
            let info = probe(&ffmpeg, &out);
            assert!(
                info.contains(&format!("streams.stream.0.nb_frames=\"{frames}\"")),
                "{name}: {info}"
            );
            assert_eq!(info.contains("codec_type=\"audio\""), seconds.is_some(), "{name}: {info}");
            let planned = frames as f64 / fps;
            for line in info.lines().filter(|line| line.contains(".duration=")) {
                let duration: f64 =
                    line.split('=').nth(1).unwrap().trim_matches('"').parse().unwrap();
                assert!(
                    (planned - 1.0 / 48_000.0 - 1e-6..=planned + 1e-6).contains(&duration),
                    "{name}: {info}"
                );
            }
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn real_encoder_omits_audio_only_when_the_clip_cannot_hold_aac_priming() {
        let Some(ffmpeg) = real_ffmpeg() else { return };
        let dir =
            std::env::temp_dir().join(format!("harmonigraph-real-tiny-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let wav = dir.join("audio.wav");
        write_wav(&wav, 48_000, 1.0, None);
        for fps in [24.0, 30.0, 46.875, 60.0] {
            let out = dir.join(format!("{fps}.mp4"));
            let options = VideoOptions {
                size: [16, 16],
                fps,
                frames: 1,
                audio: Some(&wav),
                crf: 20,
                ffmpeg: ffmpeg.to_str(),
                audio_offset: 0.0,
            };
            encode(&options, &out, None);
            let info = probe(&ffmpeg, &out);
            assert!(info.contains("streams.stream.0.nb_frames=\"1\""), "{fps}: {info}");
            assert_eq!(info.contains("codec_type=\"audio\""), fps < 46.875, "{fps}: {info}");
            for line in info.lines().filter(|line| line.contains(".duration=")) {
                let duration: f64 =
                    line.split('=').nth(1).unwrap().trim_matches('"').parse().unwrap();
                assert!((duration - 1.0 / fps).abs() <= 1.0 / 48_000.0 + 1e-6, "{fps}: {info}");
            }
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn real_encoder_keeps_the_click_on_the_flash_after_seek_and_delay() {
        let Some(ffmpeg) = real_ffmpeg() else { return };
        let dir =
            std::env::temp_dir().join(format!("harmonigraph-real-sync-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for rate in [44_100, 48_000] {
            for fps in [24u32, 30, 60] {
                for offset in [-0.25, 0.0, 0.25] {
                    let wav = dir.join("click.wav");
                    write_wav(&wav, rate, 1.25, Some(0.5 + offset));
                    let out = dir.join("click.mp4");
                    let options = VideoOptions {
                        size: [16, 16],
                        fps: f64::from(fps),
                        frames: u64::from(fps),
                        audio: Some(&wav),
                        crf: 20,
                        ffmpeg: ffmpeg.to_str(),
                        audio_offset: offset,
                    };
                    encode(&options, &out, Some(u64::from(fps / 2)));
                    let decoded = Command::new(&ffmpeg)
                        .args(["-v", "error", "-i"])
                        .arg(&out)
                        .args(["-map", "0:a:0", "-f", "f32le", "-ac", "1", "-"])
                        .output()
                        .unwrap();
                    assert!(decoded.status.success());
                    let peak = decoded
                        .stdout
                        .chunks_exact(4)
                        .map(|bytes| f32::from_le_bytes(bytes.try_into().unwrap()).abs())
                        .enumerate()
                        .max_by(|a, b| a.1.total_cmp(&b.1))
                        .unwrap()
                        .0;
                    assert_eq!(peak, 24_000, "{rate} Hz / {fps} fps / offset {offset}");
                    let decoded = Command::new(&ffmpeg)
                        .args(["-v", "error", "-i"])
                        .arg(&out)
                        .args(["-map", "0:v:0", "-f", "rawvideo", "-pix_fmt", "gray", "-"])
                        .output()
                        .unwrap();
                    assert!(decoded.status.success());
                    let flash = decoded
                        .stdout
                        .chunks_exact(16 * 16)
                        .enumerate()
                        .max_by_key(|(_, frame)| frame.iter().map(|p| u32::from(*p)).sum::<u32>())
                        .unwrap()
                        .0;
                    assert_eq!(flash, fps as usize / 2);
                }
            }
        }
        std::fs::remove_dir_all(dir).unwrap();
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
