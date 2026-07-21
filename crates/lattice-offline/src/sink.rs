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
        let mut command = Command::new(ffmpeg());
        command
            .args(["-hide_banner", "-loglevel", "warning", "-y"])
            .args(["-f", "rawvideo", "-pix_fmt", "rgba"])
            .args(["-s", &format!("{w}x{h}")])
            .args(["-r", &format!("{}", options.fps)])
            .args(["-i", "-"]);
        if let Some(audio) = options.audio {
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

        let child = command.spawn().map_err(|e| {
            format!(
                "could not start {}: {e}\n\
                 Install ffmpeg (brew install ffmpeg), set LATTICE_FFMPEG to its path, \
                 or render to a .png sequence or .rgba stream instead.",
                ffmpeg()
            )
        })?;
        Ok(Sink::Video { child })
    }

    pub fn push(&mut self, frame: &[u8]) -> Result<(), String> {
        match self {
            Sink::Video { child } => {
                let stdin = child.stdin.as_mut().ok_or("ffmpeg stdin closed")?;
                stdin.write_all(frame).map_err(|e| {
                    format!("ffmpeg stopped reading frames ({e}); it likely printed why above")
                })
            }
            Sink::Raw { file } => file.write_all(frame).map_err(|e| e.to_string()),
            Sink::Pngs { dir, stem, index, size } => {
                let path = dir.join(format!("{stem}-{index:05}.png"));
                *index += 1;
                image::save_buffer(&path, frame, size[0], size[1], image::ExtendedColorType::Rgba8)
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

/// The ffmpeg to run. A DAW's process environment often has a minimal
/// PATH, and this tool may be launched from one, so an override exists.
fn ffmpeg() -> String {
    std::env::var("LATTICE_FFMPEG").unwrap_or_else(|_| "ffmpeg".into())
}
