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
    Video { child: Child },
    Pngs { dir: std::path::PathBuf, stem: String, index: u32, size: [u32; 2] },
    Raw { file: std::fs::File },
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
        let extension = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        match extension.as_str() {
            "png" => {
                let dir = path.parent().unwrap_or(std::path::Path::new(".")).to_path_buf();
                std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
                let stem = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("frame")
                    .to_string();
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
            return Err(format!(
                "video output needs even dimensions for yuv420p; {w}x{h} is odd"
            ));
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

        let child = command
            .spawn()
            .map_err(|e| format!("could not start {}: {e}", ffmpeg.display()))?;
        Ok(Sink::Video { child })
    }

    /// Feed one frame. `Ok(true)` means keep going; `Ok(false)` means the
    /// encoder has closed the pipe and wants no more frames — a clean early
    /// stop, NOT a failure. ffmpeg does exactly this under `-shortest` when the
    /// soundtrack ends before the visuals (a one-loop take: audio is the loop,
    /// the picture keeps fading out past it). Whether that early stop was
    /// success or a crash is ffmpeg's call, read from its exit status in
    /// [`Self::finish`]; the caller just stops feeding on `Ok(false)`.
    pub fn push(&mut self, frame: &[u8]) -> Result<bool, String> {
        match self {
            Sink::Video { child } => {
                let stdin = child.stdin.as_mut().ok_or("ffmpeg stdin closed")?;
                match stdin.write_all(frame) {
                    Ok(()) => Ok(true),
                    Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => Ok(false),
                    Err(e) => Err(format!("writing a frame to ffmpeg failed: {e}")),
                }
            }
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
            Sink::Video { mut child } => {
                drop(child.stdin.take());
                let status = child.wait().map_err(|e| format!("waiting for ffmpeg: {e}"))?;
                if status.success() {
                    Ok(())
                } else {
                    Err(format!("ffmpeg exited with {status}"))
                }
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
