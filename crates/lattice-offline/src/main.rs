//! `lattice-offline` — render a recorded take to video, with no DAW, no
//! window, and no realtime.
//!
//! ```text
//! lattice-offline piece.take --audio piece.wav --out piece.mp4 --size 3840x2160
//! ```
//!
//! The visualization is a pure function of its inputs, so a take
//! (recorded once, in the DAW or the standalone harness) can be re-rendered
//! as often as you like, at any frame rate and resolution, with any layout
//! — long after the music stopped and as slowly as the GPU needs.
//!
//! See `docs/offline-rendering.md` for the whole workflow.

mod frames;
mod layout;
mod render;
mod replay;
mod sink;
mod wav;

use layout::Layout;
use render::Settings;
use replay::Replay;
use sink::{Sink, VideoOptions};

const USAGE: &str = "\
lattice-offline — render a recorded take to video

USAGE:
    lattice-offline <take.take> [OPTIONS]

OPTIONS:
    -o, --out <PATH>       Output. .mp4/.mov/.mkv go through ffmpeg;
                           .png writes a numbered sequence; .rgba writes
                           a raw stream.  [default: <take>.mp4]
    -a, --audio <WAV>      Bounced audio: feeds the spectrum analyzer and
                           is muxed into the video.
    -l, --layout <SPEC>    Preset name or path to a .ron layout.
                           Presets: side-by-side, stacked, lattice, spectral
                           [default: side-by-side]
    -s, --size <WxH>       Output pixels.  [default: 1920x1080]
        --scale <F>        Pixels per point — the UI's zoom. Bigger means
                           chunkier text relative to the frame.
                           [default: sized so the UI reads like the plugin]
        --fps <N>          Frames per second.  [default: 60]
        --start <SEC>      Skip to here.  [default: 0]
        --end <SEC>        Stop here.  [default: the take plus its tail]
        --tail <SEC>       Extra time after the last event, for fades and
                           the roll to clear.  [default: 4]
        --crf <N>          x264 quality, lower is better.  [default: 16]
        --ui-state <FILE>  Override the look recorded in the take with a
                           persist blob (see read-plugin-state.py).
        --ffmpeg <PATH>    ffmpeg to run. Normally found automatically, on
                           PATH or in the usual install locations.
        --dump-layout      Print the resolved layout as .ron and exit —
                           the starting point for a custom one.
    -h, --help             Show this.

ENVIRONMENT:
    LATTICE_FFMPEG         ffmpeg to run, if --ffmpeg is not given.
";

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("lattice-offline: {message}");
            std::process::ExitCode::FAILURE
        }
    }
}

/// Hand-rolled rather than a CLI crate: a dozen flags do not justify a
/// dependency in a workspace that documents every one it takes.
struct Args {
    take: Option<String>,
    out: Option<String>,
    audio: Option<String>,
    layout: String,
    size: [u32; 2],
    scale: Option<f32>,
    fps: f64,
    start: f64,
    end: Option<f64>,
    tail: f64,
    crf: u32,
    ui_state: Option<String>,
    ffmpeg: Option<String>,
    dump_layout: bool,
}

impl Default for Args {
    fn default() -> Self {
        Args {
            take: None,
            out: None,
            audio: None,
            layout: "side-by-side".into(),
            size: [1920, 1080],
            scale: None,
            fps: 60.0,
            start: 0.0,
            end: None,
            tail: 4.0,
            crf: 16,
            ui_state: None,
            ffmpeg: None,
            dump_layout: false,
        }
    }
}

fn parse_args() -> Result<Option<Args>, String> {
    let mut args = Args::default();
    let mut raw = std::env::args().skip(1);
    while let Some(arg) = raw.next() {
        let mut value = |name: &str| -> Result<String, String> {
            raw.next().ok_or_else(|| format!("{name} needs a value"))
        };
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(None);
            }
            "-o" | "--out" => args.out = Some(value("--out")?),
            "-a" | "--audio" => args.audio = Some(value("--audio")?),
            "-l" | "--layout" => args.layout = value("--layout")?,
            "-s" | "--size" => args.size = parse_size(&value("--size")?)?,
            "--scale" => args.scale = Some(parse_number("--scale", &value("--scale")?)?),
            "--fps" => args.fps = parse_number("--fps", &value("--fps")?)?,
            "--start" => args.start = parse_number("--start", &value("--start")?)?,
            "--end" => args.end = Some(parse_number("--end", &value("--end")?)?),
            "--tail" => args.tail = parse_number("--tail", &value("--tail")?)?,
            "--crf" => args.crf = parse_number::<f64>("--crf", &value("--crf")?)? as u32,
            "--ui-state" => args.ui_state = Some(value("--ui-state")?),
            "--ffmpeg" => args.ffmpeg = Some(value("--ffmpeg")?),
            "--dump-layout" => args.dump_layout = true,
            other if other.starts_with('-') => {
                return Err(format!("unknown option {other:?} (--help for the list)"))
            }
            other => args.take = Some(other.to_string()),
        }
    }
    Ok(Some(args))
}

fn parse_number<T: std::str::FromStr>(name: &str, text: &str) -> Result<T, String> {
    text.parse().map_err(|_| format!("{name}: {text:?} is not a number"))
}

fn parse_size(text: &str) -> Result<[u32; 2], String> {
    let (w, h) = text
        .split_once(['x', 'X', '*'])
        .ok_or_else(|| format!("--size: expected WxH, got {text:?}"))?;
    Ok([parse_number("--size", w)?, parse_number("--size", h)?])
}

/// Pixels per point when the caller didn't say.
///
/// Point sizes are the UI's type and padding scale, so this is really
/// "how big is the UI relative to the frame". The plugin's own window is
/// 1000x700 points, so matching that density means giving the render
/// about the same number of points across — hence dividing the output
/// width by a reference width rather than by anything about the display.
fn default_scale(size: [u32; 2]) -> f32 {
    const REFERENCE_POINTS_ACROSS: f32 = 1280.0;
    (size[0] as f32 / REFERENCE_POINTS_ACROSS).clamp(1.0, 4.0)
}

fn run() -> Result<(), String> {
    let Some(args) = parse_args()? else { return Ok(()) };

    let layout = Layout::load(&args.layout)?;
    if args.dump_layout {
        let pretty = ron::ser::PrettyConfig::new().depth_limit(4);
        println!(
            "{}",
            ron::ser::to_string_pretty(&layout, pretty).map_err(|e| e.to_string())?
        );
        return Ok(());
    }

    let take_path = args.take.ok_or("no take file given (--help for usage)")?;
    let mut take = lattice_take::Take::read(&take_path).map_err(|e| e.to_string())?;
    if let Some(path) = &args.ui_state {
        take.header.ui_state =
            Some(std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?);
    }

    if take.truncated {
        eprintln!(
            "warning: {take_path} ends mid-record — the export was interrupted. \
             Rendering the {} events that survived.",
            take.notes.len()
        );
    }

    let audio = args.audio.as_deref().map(wav::read).transpose()?;

    // Default end: the last event plus a tail, so releases finish fading
    // and the roll clears instead of the video cutting mid-decay. If
    // there's audio, don't stop before it does.
    let end = args.end.unwrap_or_else(|| {
        let visual = take.duration() + args.tail;
        audio.as_ref().map_or(visual, |a| visual.max(a.seconds()))
    });
    let scale = args.scale.unwrap_or_else(|| default_scale(args.size));
    let settings = Settings {
        layout,
        size: args.size,
        pixels_per_point: scale,
        fps: args.fps,
        start: args.start,
        end,
    };
    if settings.frame_count() == 0 {
        return Err(format!(
            "nothing to render: --start {} is at or past the end ({end:.2}s)",
            args.start
        ));
    }

    let out = args.out.unwrap_or_else(|| {
        std::path::Path::new(&take_path).with_extension("mp4").display().to_string()
    });
    let out = std::path::PathBuf::from(out);
    let mut sink = Sink::create(
        &out,
        &VideoOptions {
            size: args.size,
            fps: args.fps,
            audio: args.audio.as_ref().map(std::path::Path::new),
            crf: args.crf,
            ffmpeg: args.ffmpeg.as_deref(),
        },
    )?;

    let [w, h] = args.size;
    let total = settings.frame_count();
    eprintln!(
        "{take_path}: {:.1}s of events -> {total} frames at {} fps, {w}x{h} @ {scale:.2}x -> {}",
        take.duration(),
        args.fps,
        out.display(),
    );

    let mut replay = Replay::new(take);
    let mut done = 0u64;
    let rendered = render::render(&mut replay, audio.as_ref(), &settings, |frame| {
        sink.push(frame)?;
        done += 1;
        // Progress on one rewritten line; renders are long enough that
        // silence reads as a hang.
        if done.is_multiple_of(30) || done == total {
            eprint!("\r  {done}/{total} frames ({:.0}%)", 100.0 * done as f64 / total as f64);
        }
        Ok(())
    })?;
    eprintln!();
    sink.finish()?;

    // Cutting a take short is a legitimate thing to ask for, and also
    // exactly what a mistyped --end looks like. Say which happened.
    if !replay.is_spent() {
        eprintln!(
            "note: stopped at {end:.2}s with events still to come — \
             raise --end (or drop it to render the whole take)"
        );
    }
    eprintln!("done: {rendered} frames -> {}", out.display());
    if matches!(
        out.extension().and_then(|e| e.to_str()),
        Some("rgba") | Some("raw")
    ) {
        eprintln!(
            "  encode with: ffmpeg -f rawvideo -pix_fmt rgba -s {w}x{h} -r {} -i {} out.mp4",
            args.fps,
            out.display()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_accepts_the_forms_people_actually_type() {
        assert_eq!(parse_size("1920x1080").unwrap(), [1920, 1080]);
        assert_eq!(parse_size("3840X2160").unwrap(), [3840, 2160]);
        assert!(parse_size("1920").is_err());
        assert!(parse_size("widexhigh").is_err());
    }

    /// The default zoom has to keep the UI's proportions across output
    /// sizes — a 4K render with 1080p-sized text would look like a
    /// screenshot of a much bigger window.
    #[test]
    fn default_scale_keeps_the_ui_the_same_relative_size() {
        let points_across = |size: [u32; 2]| size[0] as f32 / default_scale(size);
        assert!((points_across([1920, 1080]) - points_across([3840, 2160])).abs() < 1.0);
        // Small outputs don't go below 1:1, which would render sub-pixel text.
        assert_eq!(default_scale([640, 360]), 1.0);
    }
}
