//! `harmonigraph-offline` — render a recorded take to video, with no DAW, no
//! window, and no realtime.
//!
//! ```text
//! harmonigraph-offline piece.take --audio piece.wav --out piece.mp4 --size 3840x2160
//! ```
//!
//! The visualization is a pure function of its inputs, so a take
//! (recorded once, in the DAW or the standalone harness) can be re-rendered
//! as often as you like, at any frame rate and resolution, with any layout
//! — long after the music stopped and as slowly as the GPU needs.
//!
//! See `docs/offline-rendering.md` for the whole workflow.

mod align;
mod frames;
mod render;
mod replay;
mod sink;
mod wav;

use harmonigraph_ui::{Layout, PRESETS};
use render::Settings;
use replay::Replay;
use sink::{Sink, VideoOptions};

/// Where [`USAGE`] carries the preset names, replaced at print time with
/// [`PRESETS`] so there is only ever one list.
const PRESET_TOKEN: &str = "PRESET_LIST";

const USAGE: &str = "\
harmonigraph-offline — render a recorded take to video

USAGE:
    harmonigraph-offline <take.take> [OPTIONS]

OPTIONS:
    -o, --out <PATH>       Output. .mp4/.mov/.mkv go through ffmpeg;
                           .png writes a numbered sequence; .rgba writes
                           a raw stream.  [default: <take>.mp4]
    -a, --audio <WAV>      Audio to use instead of the take's own recording
                           — a clean bounce in place of a crackly one. It
                           is auto-aligned to the take (see --align), feeds
                           the spectrum, and is muxed into the video.
    -l, --layout <SPEC>    Preset name or path to a .ron layout.
                           Presets: PRESET_LIST
                           [default: side-by-side]
    -s, --size <WxH>       Output pixels. At an aspect other than the one the
                           take was framed at, the picture is recomposed to
                           fit rather than letterboxed, and it says so.
                           [default: the take's own aspect and Resolution]
        --scale <F>        Pixels per point — the UI's zoom. Bigger means
                           chunkier text relative to the frame.
                           [default: sized so the UI reads like the plugin]
        --fps <N>          Frames per second.  [default: 60]
        --start <SEC>      Skip to here, as a song position — take times are
                           the host transport's, so 0 is the song's start, not
                           the recording's.
                           [default: where the recording begins, less --lead]
        --lead <SEC>       Extra empty frame before the recording starts. The
                           render already opens where the take was captured,
                           so this is only for holding on a still frame first.
                           [default: the take's own Lead-in setting]
        --end <SEC>        Stop here.  [default: the take plus its tail]
        --tail <SEC>       Extra time after the last event, for fades and
                           the roll to clear.  [default: 4]
        --crf <N>          x264 quality, lower is better.  [default: 16]
        --ui-state <FILE>  Override the look recorded in the take with a
                           persist blob (see read-plugin-state.py).
        --ffmpeg <PATH>    ffmpeg to run. Normally found automatically, on
                           PATH or in the usual install locations.
        --align <MODE>     How to line a --audio file up with the picture:
                           auto (default) cross-correlates it against the
                           take\'s own recording; off assumes it starts at
                           take zero; a number sets the start by hand.
        --playhead         Lay the whole take\'s spectrogram out at once and
                           sweep a playhead across it, instead of the live
                           scrolling window. Needs audio.
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
            eprintln!("harmonigraph-offline: {message}");
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
    /// `None` means "use the frame the take was composed for" (its RenderFrame),
    /// falling back to a preset.
    layout: Option<String>,
    /// `None` means "size to the take's frame aspect".
    size: Option<[u32; 2]>,
    scale: Option<f32>,
    fps: f64,
    /// `None` means "start a lead-in before the first note" — see
    /// `start_of_render`. An explicit `--start 0` is NOT the same thing, which
    /// is why this is an Option rather than defaulting to zero.
    start: Option<f64>,
    /// `None` means "use the take's own Lead-in setting".
    lead: Option<f64>,
    end: Option<f64>,
    tail: f64,
    crf: u32,
    ui_state: Option<String>,
    ffmpeg: Option<String>,
    align: Align,
    dump_layout: bool,
    playhead: bool,
}

/// How to place a soundtrack on the take's timeline.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Align {
    /// The take's own recording as-is; a replacement cross-correlated
    /// against that recording. The default, and the point of the feature.
    Auto,
    /// Assume the soundtrack starts where the take does (take zero for a
    /// replacement, the recording's own start otherwise).
    Off,
    /// A hand-set start, in seconds of take time.
    Fixed(f64),
}

impl Default for Args {
    fn default() -> Self {
        Args {
            take: None,
            out: None,
            audio: None,
            layout: None,
            size: None,
            scale: None,
            fps: 60.0,
            start: None,
            lead: None,
            end: None,
            tail: 4.0,
            crf: 16,
            ui_state: None,
            ffmpeg: None,
            align: Align::Auto,
            dump_layout: false,
            playhead: false,
        }
    }
}

fn parse_args() -> Result<Option<Args>, String> {
    parse_args_from(std::env::args().skip(1))
}

/// Split from [`parse_args`] so the flag handling can be tested without a
/// process to hang it off.
///
/// A repeated flag keeps the LAST one, which falls out of assigning into
/// `args` as the loop goes and is what a person recalling a command line and
/// appending a new `--size` expects. Worth a test rather than left implicit:
/// the alternative failure is silent, keeping the earlier value while the
/// typed one looks like it took.
fn parse_args_from(raw: impl IntoIterator<Item = String>) -> Result<Option<Args>, String> {
    let mut args = Args::default();
    let mut raw = raw.into_iter();
    while let Some(arg) = raw.next() {
        let mut value = |name: &str| -> Result<String, String> {
            raw.next().ok_or_else(|| format!("{name} needs a value"))
        };
        match arg.as_str() {
            "-h" | "--help" => {
                // The preset names are `harmonigraph_ui::PRESETS`'s to state,
                // spliced in here rather than written out above. A second copy
                // drifts the moment a preset is added, and it drifts against
                // the one message that would correct it: `Layout::load` prints
                // the real list when a name is not a preset, so the help and
                // the error would disagree exactly when someone is trying to
                // learn the names.
                //
                // A token and a `replace` rather than a `format!`, so the help
                // text stays a plain string with no braces to escape.
                print!("{}", USAGE.replace(PRESET_TOKEN, &PRESETS.join(", ")));
                return Ok(None);
            }
            "-o" | "--out" => args.out = Some(value("--out")?),
            "-a" | "--audio" => args.audio = Some(value("--audio")?),
            "-l" | "--layout" => args.layout = Some(value("--layout")?),
            "-s" | "--size" => args.size = Some(parse_size(&value("--size")?)?),
            "--scale" => args.scale = Some(parse_number("--scale", &value("--scale")?)?),
            "--fps" => args.fps = parse_number("--fps", &value("--fps")?)?,
            "--start" => args.start = Some(parse_number("--start", &value("--start")?)?),
            "--lead" => args.lead = Some(parse_number("--lead", &value("--lead")?)?),
            "--end" => args.end = Some(parse_number("--end", &value("--end")?)?),
            "--tail" => args.tail = parse_number("--tail", &value("--tail")?)?,
            "--crf" => args.crf = parse_number::<f64>("--crf", &value("--crf")?)? as u32,
            "--ui-state" => args.ui_state = Some(value("--ui-state")?),
            "--ffmpeg" => args.ffmpeg = Some(value("--ffmpeg")?),
            "--align" => args.align = parse_align(&value("--align")?)?,
            "--playhead" => args.playhead = true,
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

fn parse_align(text: &str) -> Result<Align, String> {
    match text {
        "auto" => Ok(Align::Auto),
        "off" | "none" => Ok(Align::Off),
        other => other
            .parse()
            .map(Align::Fixed)
            .map_err(|_| format!("--align: expected auto, off, or a number, got {other:?}")),
    }
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
///
/// It decides how SHARPLY the type is rasterized and nothing else about its
/// size: every label sizes itself off the pane it is drawn on, in points, and
/// a pane's points are its pixels over this — so the two cancel and a render
/// of the same frame at any resolution carries the same type, larger. That is
/// the point of it, but it used to work the other way about (point sizes were
/// fixed, so this scaled them), and the difference matters if anyone ever
/// reaches for `--scale` to make the text bigger. It will not.
fn default_scale(size: [u32; 2]) -> f32 {
    const REFERENCE_POINTS_ACROSS: f32 = 1280.0;
    (size[0] as f32 / REFERENCE_POINTS_ACROSS).clamp(1.0, 4.0)
}

/// The pixels a render lands on: `--size` if it was given, otherwise the take's
/// own frame at the resolution it was composed for.
///
/// BOTH halves come out of the take, and that is the point — the frame decides
/// the shape and `short_edge` decides how big, and they are the same two things
/// the Video pane's Aspect and Resolution rows set. A plain
/// `harmonigraph-offline take.take` therefore reproduces what was previewed,
/// which is what makes re-rendering a take by hand to change one unrelated flag
/// safe: `--layout stacked` on its own must not also take a 4K take back down to
/// 1080.
///
/// The fallback for a take carrying no blob at all is `RenderConfig::default()`,
/// so the resolution and the aspect fall back together rather than to a constant
/// here that a second definition could drift from.
fn output_size(size: Option<[u32; 2]>, config: &harmonigraph_ui::RenderConfig) -> [u32; 2] {
    size.unwrap_or_else(|| config.frame.pixels(config.short_edge))
}

/// Where the video starts, in take time.
///
/// Take times are the host's TRANSPORT position, so time zero is the song's
/// start and not the record button's. A passage played from a minute into the
/// arrangement was captured from 60-odd seconds, and rendering from zero opens
/// the video with a minute of empty lattice — nothing was recorded there to
/// draw. That is the bug this exists to fix, reported against a take whose
/// recording begins at 5.48s.
///
/// So the anchor is where the RECORDING begins, not where the song does, and
/// not where the first note lands: a take can carry sound without any MIDI at
/// all, and anchoring to notes would send that case back to song zero. The
/// take's first event serves, since arming writes a full parameter snapshot
/// whether or not anything is played; recorded audio contributes its own start
/// too, and the earliest of them wins.
///
/// `lead` then backs off from that point, for a beat of stillness before the
/// music. It is empty frame by construction — nothing was captured before the
/// recording started — which is why it defaults to none and is a choice rather
/// than a correction.
///
/// `--start` overrides, and stays absolute: it answers "where in the song",
/// which is what you want when skipping to a passage, and it is the only way
/// back to opening at song zero (`--start 0`).
fn start_of_render(explicit: Option<f64>, capture_start: Option<f64>, lead: f64) -> f64 {
    match (explicit, capture_start) {
        (Some(start), _) => start,
        // An empty take: no events, no audio, nothing to anchor to.
        (None, None) => 0.0,
        (None, Some(capture)) => (capture - lead.max(0.0)).max(0.0),
    }
}

/// Whether an explicit `--size` composes the frame the take was dialed in at.
///
/// Compares shape, not pixels: `--size` is how you render the same picture
/// larger, and 3840x2160 is the same picture as 1920x1080. A ratio that
/// disagrees is a different composition — the split falls elsewhere and the
/// lattice camera exposes a different amount of the board — so the render
/// stops matching the preview it was framed in. Half a percent of tolerance
/// covers rounding an odd ratio onto even pixels (21:9 at 1080 is 2520x1080,
/// which is 2.333 against 2.334).
fn size_matches_frame(size: [u32; 2], frame: &harmonigraph_ui::RenderFrame) -> bool {
    let asked = size[0] as f64 / size[1].max(1) as f64;
    let framed = frame.aspect_w.max(1) as f64 / frame.aspect_h.max(1) as f64;
    (asked - framed).abs() <= framed * 0.005
}

/// Line a replacement soundtrack up with the take timeline, reporting what it
/// found. Prefers the take\'s own recording as a timing reference; with no
/// recording it correlates the bounce against the MIDI note-ons directly (the
/// notes are already on the take clock). Falls back to take zero, loudly, only
/// when neither can place it.
fn align_replacement(
    recorded: Option<&std::path::Path>,
    soundtrack: Option<&crate::wav::Audio>,
    reference_start: f64,
    midi_onsets: &[(f64, f32)],
    span: f64,
) -> Result<f64, String> {
    let Some(soundtrack) = soundtrack else { return Ok(0.0) };

    // Most robust: the take\'s own recording, stamped to the same clock as the
    // notes. Fall through only if it is missing or too short to lock onto.
    if let Some(recorded) = recorded {
        let reference = crate::wav::read(recorded)?;
        if let Some(found) = crate::align::align(&reference, reference_start, soundtrack) {
            eprintln!(
                "aligned audio to the take\'s recording: soundtrack starts at \
                 {:.3}s (confidence {:.2})",
                found.start, found.confidence,
            );
            if found.confidence < 0.35 {
                eprintln!(
                    "  low confidence — if the sound drifts against the picture, \
                     set the start by hand with --align <seconds>"
                );
            }
            return Ok(found.start);
        }
    }

    // No usable recording: line the bounce up against the MIDI note onsets.
    // Great for clear attacks; soft or legato onsets match weakly, so say so.
    match crate::align::align_to_notes(midi_onsets, span, soundtrack) {
        Some(found) if found.confidence >= 0.25 => {
            eprintln!(
                "aligned audio to the MIDI note onsets: soundtrack starts at \
                 {:.3}s (confidence {:.2})",
                found.start, found.confidence,
            );
            if found.confidence < 0.4 {
                eprintln!(
                    "  low confidence (soft or sparse onsets) — set --align \
                     <seconds> by hand if it drifts"
                );
            }
            Ok(found.start)
        }
        _ => {
            eprintln!(
                "note: no scratch recording, and the MIDI onsets did not match the \
                 audio confidently — assuming it starts at take zero. Set --align \
                 <seconds> if it drifts."
            );
            Ok(0.0)
        }
    }
}

fn run() -> Result<(), String> {
    let Some(args) = parse_args()? else { return Ok(()) };

    if args.dump_layout {
        // Without a take there's no frame to compose, so dump the named preset
        // (or the default) as a starting point for a custom .ron.
        let layout = Layout::load(args.layout.as_deref().unwrap_or("side-by-side"))?;
        let pretty = ron::ser::PrettyConfig::new().depth_limit(4);
        println!("{}", ron::ser::to_string_pretty(&layout, pretty).map_err(|e| e.to_string())?);
        return Ok(());
    }

    let take_path = args.take.ok_or("no take file given (--help for usage)")?;
    let mut take = harmonigraph_take::Take::read(&take_path).map_err(|e| e.to_string())?;
    if let Some(path) = &args.ui_state {
        take.header.ui_state =
            Some(std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?);
    }

    // The frame the take was composed for in the Video pane. The offline
    // render defaults its size and layout to this, so a plain `harmonigraph-offline
    // take.take` reproduces exactly what was previewed; --size / --layout
    // override.
    let render_config = take
        .header
        .ui_state
        .as_deref()
        .and_then(harmonigraph_ui::render_config_from_persist)
        .unwrap_or_default();
    let frame = render_config.frame;
    let layout = match &args.layout {
        Some(spec) => Layout::load(spec)?,
        None => Layout::split(frame.lattice, frame.split),
    };
    let size = output_size(args.size, &render_config);
    // An explicit --size at a different aspect renders a DIFFERENT picture
    // from the one the take was framed in — nothing letterboxes or crops to
    // reconcile them, the layout simply recomposes at the pixels it is given.
    // It stays legal (rendering a take at a new shape on purpose is a real
    // thing to want), but silently is how you end up with a tall composition
    // in a wide video and no idea which setting did it.
    if args.size.is_some() && !size_matches_frame(size, &frame) {
        let [w, h] = size;
        eprintln!(
            "warning: --size {w}x{h} is not the {}:{} this take was framed at — \
             the picture is recomposed to fit, not letterboxed. Drop --size to \
             render the frame as previewed.",
            frame.aspect_w, frame.aspect_h
        );
    }

    if take.truncated {
        eprintln!(
            "warning: {take_path} ends mid-record — the export was interrupted. \
             Rendering the {} events that survived.",
            take.notes.len()
        );
    }

    // The WAV the take recorded for itself, beside the take file, if any.
    let recorded = take
        .header
        .audio_file
        .as_ref()
        .map(|name| {
            std::path::Path::new(&take_path)
                .parent()
                .unwrap_or(std::path::Path::new("."))
                .join(name)
        })
        .filter(|path| path.is_file());

    // The soundtrack: a `--audio` file replaces the take's own recording;
    // otherwise the recording is the soundtrack.
    let is_replacement = args.audio.is_some();
    let audio_path: Option<std::path::PathBuf> = match &args.audio {
        Some(path) => Some(std::path::PathBuf::from(path)),
        None => {
            if take.header.audio_file.is_some() && recorded.is_none() {
                // A take that names an audio file it no longer has beside
                // it is worth saying out loud: the render would otherwise
                // come out silent with no spectrum and no explanation.
                eprintln!(
                    "warning: take names {:?} but it is not beside the take",
                    take.header.audio_file.as_deref().unwrap_or_default()
                );
            }
            recorded.clone()
        }
    };
    let audio = audio_path.as_deref().map(crate::wav::read).transpose()?;

    if args.playhead && audio.is_none() {
        eprintln!(
            "note: --playhead lays out the audio spectrogram, but this render has \
             no audio; falling back to the scrolling view."
        );
    }

    // Where the soundtrack's first sample falls on the take's timeline.
    // The take's own recording is aligned by construction (the header
    // says where it started). A REPLACEMENT — a clean bounce standing in
    // for a crackly recording — is aligned against that recording by
    // cross-correlation, so it inherits the same alignment to the
    // picture. --align overrides either.
    let reference_start = take.header.audio_start.unwrap_or(0.0);
    // Note-on times on the take clock, for aligning a bounce that has no
    // scratch recording to correlate against.
    let midi_onsets: Vec<(f64, f32)> = take
        .notes
        .iter()
        .filter_map(|note| match note.kind {
            harmonigraph_take::NoteKind::On { velocity } => Some((note.t, velocity)),
            _ => None,
        })
        .collect();
    let audio_start = match args.align {
        Align::Fixed(seconds) => seconds,
        Align::Off => {
            if is_replacement {
                0.0
            } else {
                reference_start
            }
        }
        Align::Auto if !is_replacement => reference_start,
        Align::Auto => align_replacement(
            recorded.as_deref(),
            audio.as_ref(),
            reference_start,
            &midi_onsets,
            take.duration(),
        )?,
    };

    // Default end: the last event plus a tail, so releases finish fading
    // and the roll clears instead of the video cutting mid-decay. If
    // there's audio, don't stop before it does.
    let end = args.end.unwrap_or_else(|| {
        let visual = take.duration() + args.tail;
        audio.as_ref().map_or(visual, |a| visual.max(audio_start + a.seconds()))
    });
    let scale = args.scale.unwrap_or_else(|| default_scale(size));
    // The take's own Lead-in unless the command line says otherwise.
    let lead = args.lead.unwrap_or(render_config.lead_in as f64);
    // Where the recording begins: its first event, or the start of its own
    // audio if that came first (an audio part can be captured before anything
    // is played, and a take may carry no notes at all).
    let capture_start = match (take.first_event(), take.header.audio_start) {
        (Some(event), Some(audio)) => Some(event.min(audio)),
        (only, None) => only,
        (None, only) => only,
    };
    let start = start_of_render(args.start, capture_start, lead);
    let settings = Settings {
        layout,
        size,
        pixels_per_point: scale,
        fps: args.fps,
        start,
        end,
        audio_start,
        whole_song_spectrogram: args.playhead,
    };
    if settings.frame_count() == 0 {
        return Err(format!(
            "nothing to render: the start ({start:.2}s) is at or past the end ({end:.2}s)"
        ));
    }

    let out = args.out.unwrap_or_else(|| {
        std::path::Path::new(&take_path).with_extension("mp4").display().to_string()
    });
    let out = std::path::PathBuf::from(out);
    let mut sink = Sink::create(
        &out,
        &VideoOptions {
            size,
            fps: args.fps,
            audio: audio_path.as_deref(),
            crf: args.crf,
            ffmpeg: args.ffmpeg.as_deref(),
            audio_offset: start - audio_start,
        },
    )?;

    let [w, h] = size;
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
        if !sink.push(frame)? {
            // ffmpeg closed the pipe (e.g. -shortest, the soundtrack ending
            // before the visuals). Stop feeding; finish() below reads whether
            // that was a clean finish or a crash from ffmpeg's exit status.
            return Ok(false);
        }
        done += 1;
        // Progress on one rewritten line; renders are long enough that
        // silence reads as a hang.
        //
        // This line and the `-> {total} frames` one above it are also READ, by
        // the plugin's Video pane, which follows this stderr to drive its
        // progress bar (`harmonigraph_record::parse_report`): the count before ` frames` is
        // the part it matches, so keep that shape. Reformatting the rest is
        // free; losing the count leaves the bar empty.
        if done.is_multiple_of(30) || done == total {
            eprint!("\r  {done}/{total} frames ({:.0}%)", 100.0 * done as f64 / total as f64);
        }
        Ok(true)
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
    if matches!(out.extension().and_then(|e| e.to_str()), Some("rgba") | Some("raw")) {
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

    fn frame(aspect_w: u32, aspect_h: u32) -> harmonigraph_ui::RenderFrame {
        harmonigraph_ui::RenderFrame { aspect_w, aspect_h, ..Default::default() }
    }

    #[test]
    fn the_default_size_puts_the_short_edge_at_1080_with_even_dimensions() {
        let default_short_edge = harmonigraph_ui::RenderConfig::default().short_edge;
        assert_eq!(default_short_edge, 1080, "the Resolution control's own default");
        let sz = |w, h| frame(w, h).pixels(default_short_edge);
        assert_eq!(sz(16, 9), [1920, 1080]);
        assert_eq!(sz(9, 16), [1080, 1920]);
        assert_eq!(sz(1, 1), [1080, 1080]);
        // A non-integer ratio still comes out even (ffmpeg requires it).
        let [w, h] = sz(21, 9);
        assert_eq!(h, 1080);
        assert!(w % 2 == 0 && h % 2 == 0, "{w}x{h} not even");
    }

    /// Re-rendering a take by hand honours the Resolution it was composed at.
    ///
    /// The take carries `short_edge` (the Video pane's Resolution row) exactly
    /// as it carries the frame, and the plugin's own auto-render passes it
    /// through as `--size`. A plain command line has only the blob to read it
    /// from — and a renderer that reads the frame but defaults the resolution
    /// takes a 4K take back down to 1080 with no flag saying so.
    #[test]
    fn a_plain_command_line_renders_at_the_resolution_the_take_was_composed_at() {
        let config = harmonigraph_ui::RenderConfig {
            frame: frame(9, 16),
            short_edge: 2160,
            ..Default::default()
        };
        assert_eq!(output_size(None, &config), [2160, 3840]);

        // An explicit --size still wins: it is the override, and the warning
        // about an aspect it does not match is aimed at exactly that case.
        assert_eq!(output_size(Some([1280, 720]), &config), [1280, 720]);

        // A take with no blob at all falls back to the config's own defaults,
        // both halves together.
        let bare = harmonigraph_ui::RenderConfig::default();
        assert_eq!(output_size(None, &bare), [1920, 1080]);
    }

    /// A repeated flag keeps the last, the way every CLI a person types at
    /// behaves — so a second `--size` on the end of a recalled command line
    /// replaces the first rather than erroring or, worse, silently keeping the
    /// earlier one.
    #[test]
    fn a_repeated_size_keeps_the_last_one() {
        let parse = |flags: &[&str]| {
            parse_args_from(flags.iter().map(|s| s.to_string()))
                .expect("flags parse")
                .expect("not --help")
                .size
        };
        assert_eq!(parse(&["--size", "1920x1080", "--size", "3840x2160"]), Some([3840, 2160]));
        // Across other flags, and across the two spellings.
        assert_eq!(
            parse(&["--size", "1080x1920", "--fps", "30", "-s", "2560x1440"]),
            Some([2560, 1440])
        );
        // And with nothing after it, the only one stands.
        assert_eq!(parse(&["--size", "1080x1920", "--fps", "30"]), Some([1080, 1920]));
    }

    /// The render opens where the RECORDING did, wherever in the song that
    /// falls — the bug being that take times are transport positions, so a
    /// passage played from a minute in used to open on a minute of empty
    /// lattice.
    #[test]
    fn the_render_starts_where_the_recording_did() {
        // The take that prompted this: captured from 5.48s of song time.
        assert!((start_of_render(None, Some(5.48), 0.0) - 5.48).abs() < 1e-9);
        // A lead backs off from there, into frame that is empty by
        // construction — nothing was captured before the take began.
        assert!((start_of_render(None, Some(5.48), 2.0) - 3.48).abs() < 1e-9);
        // ...but never past song zero, which has no frames behind it.
        assert_eq!(start_of_render(None, Some(0.2), 1.0), 0.0);
        assert_eq!(start_of_render(None, Some(0.0), 0.0), 0.0);
        // A negative lead would push the start PAST the capture point and clip
        // the opening, so it is treated as none.
        assert!((start_of_render(None, Some(5.48), -2.0) - 5.48).abs() < 1e-9);
        // An empty take: nothing to anchor to.
        assert_eq!(start_of_render(None, None, 1.0), 0.0);
    }

    /// `--start` is absolute song position and outranks the lead — the only
    /// way back to opening at song zero, and what you want when skipping to a
    /// passage rather than trimming the run-up to one.
    #[test]
    fn an_explicit_start_beats_the_lead() {
        assert_eq!(start_of_render(Some(0.0), Some(5.48), 1.0), 0.0);
        assert_eq!(start_of_render(Some(30.0), Some(5.48), 1.0), 30.0);
        // Including when there are no notes to anchor to.
        assert_eq!(start_of_render(Some(12.0), None, 1.0), 12.0);
    }

    /// `--start 0` has to survive parsing as a REQUEST, not as the absence of
    /// one: if it collapsed to the default the flag would silently do the
    /// opposite of what it says, since the default is now nonzero.
    #[test]
    fn start_zero_is_distinguishable_from_no_start() {
        let parse = |flags: &[&str]| {
            parse_args_from(flags.iter().map(|s| s.to_string())).unwrap().unwrap().start
        };
        assert_eq!(parse(&["--start", "0"]), Some(0.0));
        assert_eq!(parse(&["--fps", "30"]), None);
    }

    /// The warning fires on a different SHAPE and stays quiet for a bigger
    /// render of the same one — otherwise every 4K render nags.
    #[test]
    fn a_size_at_the_frames_own_aspect_is_not_a_mismatch() {
        for short in [1080, 1440, 2160] {
            for (w, h) in [(16, 9), (9, 16), (1, 1), (4, 5), (21, 9)] {
                let f = frame(w, h);
                let size = f.pixels(short);
                assert!(size_matches_frame(size, &f), "{w}:{h} at {short} read as a mismatch");
            }
        }
        // The default Options string this replaced, against a portrait frame:
        // the case that shipped a wide video from a tall preview.
        assert!(!size_matches_frame([1920, 1080], &frame(9, 16)));
        assert!(!size_matches_frame([1920, 1080], &frame(1, 1)));
        assert!(!size_matches_frame([1080, 1080], &frame(16, 9)));
    }
}
