//! The Video pane's settings: what a render is composed and triggered by.
//!
//! They ride in a take only inside the [`AppearanceDocument`]
//! this crate serializes into its header, so they live beside it rather than in
//! the take format. The offline renderer reads the composition fields
//! back out of that document, so a re-render reproduces the framing a take was
//! dialed in at rather than whatever the editor is set to now. They are
//! serde-facing: each round-trips through a saved project's UI blob and through
//! that document.
//!
//! The output size stays outside `RenderFrame` — see
//! [`RenderConfig::short_edge`].
//!
//! The recorder needs none of these types: the GUI hands it the plain values
//! [`RenderTrigger::ends_at_rewind`] and [`RenderConfig::stop_at_bar`] compute,
//! so `harmonigraph-record` links no editor. The export queue's vocabulary
//! (`RenderProgress`, `ExportJob`) stays in `harmonigraph-take` for that reason.

use harmonigraph_scene::repair;

/// What counts as "the take is done", and so when a video gets rendered.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum RenderTrigger {
    /// When you switch Record take off. Predictable, and works no matter how
    /// the transport behaves.
    OnDisarm,
    /// Once the transport has stayed stopped for 0.4 s after recording
    /// something — so a play-through, or an audio export, produces a video
    /// with nothing further to click. Recording disarms itself at the same
    /// moment. A jump while playing, back or forward, records on: the video
    /// shows a jump where you jumped.
    ///
    /// Falls back gracefully: if a host stops calling `process` while the
    /// transport rolls, no stillness is counted and the take records on until
    /// you disarm it.
    #[default]
    OnTransportStop,
    /// When the arranger loop first repeats: exactly one loop is recorded, the
    /// take ends at the loop's end, and that loop renders — no catching the
    /// stop by hand. Meant for looped recording, where a manual stop is always
    /// a beat or two off.
    ///
    /// Detected by the transport wrapping, so **looping must be enabled**.
    /// Hosts don't reliably tell a plugin where the loop markers are (Bitwig
    /// doesn't flag its loop as active, so nih-plug's loop range is `None`), so
    /// with looping off there is nothing to wrap on and it waits for you to
    /// disarm, like [`OnDisarm`](Self::OnDisarm).
    AtLoopEnd,
    /// When the transport plays THROUGH [`stop_bar`](RenderConfig::stop_bar):
    /// the take ends at that bar and renders, with nothing to click.
    ///
    /// For ending at a chosen point rather than wherever the transport stops.
    /// The crossing is decided on the audio thread, off a position the host
    /// reports whether it is playing or not.
    ///
    /// A backward jump records on, as under [`OnDisarm`](Self::OnDisarm), so
    /// a scrub back and a replay renders every attempt, end to end, up to the
    /// one that reached the bar.
    AtBar,
}

impl RenderTrigger {
    /// Whether a backward jump, after forward motion, ends the take: only the
    /// loop repeating under [`AtLoopEnd`](Self::AtLoopEnd). Every other jump
    /// records on.
    pub fn ends_at_rewind(self) -> bool {
        match self {
            RenderTrigger::AtLoopEnd => true,
            RenderTrigger::OnDisarm | RenderTrigger::OnTransportStop | RenderTrigger::AtBar => {
                false
            }
        }
    }
}

#[cfg(test)]
mod trigger_tests {
    use super::RenderTrigger;

    /// Which triggers hand the take's end to the audio thread. Written out one
    /// by one rather than as `!= OnDisarm`, so adding a variant has to decide.
    #[test]
    fn only_loop_end_ends_at_a_rewind() {
        assert!(!RenderTrigger::OnDisarm.ends_at_rewind());
        assert!(!RenderTrigger::OnTransportStop.ends_at_rewind());
        assert!(RenderTrigger::AtLoopEnd.ends_at_rewind());
        assert!(!RenderTrigger::AtBar.ends_at_rewind());
    }

    /// The bar only reaches the audio thread under the one trigger that means
    /// it. Any other choice must publish `None`, or a stop bar left over in a
    /// saved project would end takes recorded under `OnDisarm`.
    #[test]
    fn only_at_bar_publishes_a_stop_bar() {
        let mut config = super::RenderConfig { stop_bar: 65.0, ..Default::default() };
        for trigger in
            [RenderTrigger::OnDisarm, RenderTrigger::OnTransportStop, RenderTrigger::AtLoopEnd]
        {
            config.trigger = trigger;
            assert_eq!(config.stop_at_bar(), None, "{trigger:?} does not stop at a bar");
        }
        config.trigger = RenderTrigger::AtBar;
        assert_eq!(config.stop_at_bar(), Some(64.0), "and it converts to a 0-based bar");
    }

    /// A stop bar or tail out of its field's range, or not a number at all, is repaired
    /// on load rather than carried.
    ///
    /// NaN is the one that has to be named separately: it is not merely out of
    /// range, it compares false against every bar, so a blob carrying one would
    /// leave the trigger selected and silently unable to fire.
    #[test]
    fn a_hand_edited_stop_bar_or_tail_is_repaired_on_load() {
        for (given, want) in
            [(f64::NAN, 65.0), (f64::INFINITY, 65.0), (0.0, 2.0), (1e9, 100_000.0), (33.5, 33.5)]
        {
            let mut config = super::RenderConfig { stop_bar: given, ..Default::default() };
            config.sanitize();
            assert_eq!(config.stop_bar, want, "stop_bar {given}");
        }
        for (given, want) in [(f64::NAN, 0.0), (-1.0, 0.0), (1e9, 60.0), (0.5, 0.5)] {
            let mut config = super::RenderConfig { tail: given, ..Default::default() };
            config.sanitize();
            assert_eq!(config.tail, want, "tail {given}");
        }
    }
}

/// Which spectrogram a render bakes: the Video pane's Video history row.
///
/// Both modes scroll the same history; the choice decides its visible span.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SpectrogramRender {
    /// The live scrolling window, spanning the Analyzer's History duration —
    /// exactly what the preview shows.
    Scrolling,
    /// The live scrolling window spanning the whole render instead, so the last
    /// frame reaches back to the first. Only a render knows its own length, so
    /// the preview keeps the dialled span; the offline renderer applies this,
    /// held to the History duration bar's ends.
    WholeVideo,
}

/// Which pitch range a render draws the Spectral pane at: the Video pane's
/// Frequency range row.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum PitchRangeRender {
    /// The Analyzer's Frequency range, exactly as previewed.
    Dialled,
    /// What the piece fills, anywhere on the axis and whatever the Analyzer's
    /// Frequency range: the pitches its soundtrack lights for half a second or
    /// more, and every note it plays (see `pitch_fit`). Only a render has heard
    /// the whole piece, so the preview keeps the dialled range, and the offline
    /// renderer applies this.
    Fit,
}

/// Which H.264 encoder a video export runs through ffmpeg.
///
/// The picture is the same either way; what differs is how long the export
/// waits on the encoder and what the file costs. At 2560x1440@60 an 82.6 s
/// video took 40-42 s end to end through x264 and 30.1 s through VideoToolbox on
/// an M5 Pro, for a file 4% smaller; a denser take's was a quarter larger.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum VideoEncoder {
    /// libx264 at the `--crf` quality, tuned for the spectrogram's grain.
    X264,
    /// The machine's hardware H.264 encoder (`h264_videotoolbox`) at a constant
    /// quality. About a third faster than x264 at 1440p on a fraction of the
    /// processor, for a file up to a quarter larger and no grain tuning.
    Hardware,
}

/// How a finished take gets turned into a video, edited in the Video
/// pane (its trigger and stop bar beside Record take, the rest in its Render
/// section) and persisted with the UI state.
///
/// The plugin cannot render video itself — that is `harmonigraph-offline`, a
/// separate binary with a headless GPU device and an ffmpeg pipe, and
/// nothing about it belongs inside a real-time audio plugin. What the
/// plugin can do is *run* it, the moment a take is complete.
/// Container-level `default`, so a key missing from a blob loads with the
/// value a fresh install gets — `impl Default`'s, field by field. Per-field
/// `default = "..."` fns said the same thing one field at a time and had to
/// be kept in step with `impl Default` by hand; a pair that drifted meant a
/// blob omitting that key loaded as a config nobody chose. Every persisted
/// struct in the tree is built this way now, and none of them keeps a second
/// set of values for what a blob was SAVED with — see CLAUDE.md's compat
/// section, which makes that an invariant rather than a coincidence.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct RenderConfig {
    /// What "finishes" means; see [`RenderTrigger`].
    pub trigger: RenderTrigger,
    /// The bar [`AtBar`](RenderTrigger::AtBar) ends the take at, counted the
    /// way a host's arranger counts: **bar 1 is the song's start**, so 65 here
    /// is the bar labelled 65 in Bitwig. Ignored under every other trigger.
    ///
    /// Fractional, because the field it is edited in is one number and half a
    /// bar is a legitimate place to cut; nothing rounds it.
    pub stop_bar: f64,
    /// Which spectrogram the render bakes; see [`SpectrogramRender`]. Read by
    /// the offline renderer from the take.
    pub spectrogram: SpectrogramRender,
    /// Which pitch range the render draws; see [`PitchRangeRender`]. A
    /// composition choice like Video history: a recorded look keeps its own.
    pub pitch_range: PitchRangeRender,
    /// The composed video frame — aspect ratio and the lattice/spectral split.
    /// Edited and previewed in the Video pane; the offline renderer reads it
    /// to compose the same picture.
    pub frame: RenderFrame,
    /// The render's short edge in pixels; with [`frame`](Self::frame)'s aspect
    /// this is the whole output size (see [`RenderFrame::pixels`]). The Video
    /// pane's Output size.
    ///
    /// Kept beside `RenderFrame` rather than in it: the frame is the
    /// composition, and this only scales it, so one framing renders the same
    /// picture at any size. It rides in a take like the frame does, so a plain
    /// re-render keeps it, and an export renders at it: only a hand-run
    /// `harmonigraph-offline --size` overrides it.
    pub short_edge: u32,
    /// Seconds the video runs past the take's last event, so releases finish
    /// fading and the roll clears. The Video pane's Tail.
    ///
    /// Never cuts the soundtrack short: the render still runs to the end of the
    /// take's audio. So 0 ends an audio export at its range end rather than a
    /// few silent seconds past it.
    ///
    /// A render choice like [`short_edge`](Self::short_edge): a queued export's
    /// appearance supplies it, and the offline renderer's `--tail` overrides it.
    pub tail: f64,
    /// Which H.264 encoder the export runs; see [`VideoEncoder`]. The Video
    /// pane's Encoder. A render choice like the tail: a queued export takes the
    /// current one, and the offline renderer's `--encoder` overrides it.
    pub encoder: VideoEncoder,
    /// Also write a small AV1 copy beside the video, for keeping once the full
    /// video has been uploaded. The Video pane's Archive copy. A render choice
    /// like the encoder: a queued export takes the current one.
    ///
    /// Encoded from the finished video, so it carries the video's soundtrack
    /// and timing as they are rather than deriving them a second time; see
    /// `harmonigraph-offline`'s `sink::archive`.
    pub archive: bool,
}

impl Default for RenderConfig {
    fn default() -> Self {
        RenderConfig {
            // Transport stop, the trigger that also ends a take during an
            // arrangement export (see docs/intent/video-export.md). The
            // offline renderer reads the same default for a take that cannot
            // say how it ended.
            trigger: RenderTrigger::default(),
            // Off-trigger by default, so this only ever matters once `AtBar`
            // is chosen. 65 rather than 1: a stop bar equal to the song start
            // can never be crossed from below, so the take would simply never
            // end and the trigger would look broken on first use.
            stop_bar: 65.0,
            spectrogram: SpectrogramRender::WholeVideo,
            // Fit, so an export drops the bands a piece leaves dark without a
            // range dialled for it (Yan, #1976).
            pitch_range: PitchRangeRender::Fit,
            frame: RenderFrame::default(),
            // 1440 on the short edge — 2560x1440 at the default 16:9 frame,
            // as captured from the DAW on 2026-10-07.
            short_edge: 1440,
            tail: 0.0,
            // VideoToolbox: the same picture in under half the export time at
            // 1440p, for a larger file (see [`VideoEncoder`]).
            encoder: VideoEncoder::Hardware,
            // Off: the copy roughly doubles an export's wait.
            archive: false,
        }
    }
}

impl RenderConfig {
    /// Fit a deserialized render config to what its own controls can
    /// produce, on the same footing as `ViewConfig::sanitize` in
    /// `harmonigraph-scene` and `SpectrumConfig::sanitize` here: a bar cannot hand
    /// back a nonsense value, a hand-edited blob can, and this is a video
    /// dial rather than a real-time one, so a bad number here silently
    /// composes a frame nobody would have set rather than merely misdrawing
    /// one.
    pub fn sanitize(&mut self) {
        self.frame.sanitize();
        // A NaN read out of a hand-edited blob would compare false against
        // every bar and quietly disable the trigger; clamping to the range the
        // field can produce keeps the number on screen the number that fires.
        let fresh = RenderConfig::default();
        self.stop_bar = repair(self.stop_bar, fresh.stop_bar, STOP_BAR_RANGE.0..=STOP_BAR_RANGE.1);
        // The same for the tail: a NaN would reach the render's end and make
        // its frame count meaningless.
        self.tail = repair(self.tail, fresh.tail, TAIL_RANGE.0..=TAIL_RANGE.1);
    }

    /// Take from `pane` the fields that say how big the video is, how long it
    /// runs on and what encodes it: Output size, Tail, Encoder and Archive copy. These are
    /// the Video pane's whichever look is drawn, where every other field says
    /// how a take was composed and recorded, and a recorded look keeps its own.
    ///
    /// `pane` is destructured exhaustively, so a new field has to be classified
    /// here as one or the other before this compiles.
    pub fn take_output_from(&mut self, pane: &RenderConfig) {
        let RenderConfig {
            trigger: _,
            stop_bar: _,
            spectrogram: _,
            pitch_range: _,
            frame: _,
            short_edge,
            tail,
            encoder,
            archive,
        } = pane;
        self.short_edge = *short_edge;
        self.tail = *tail;
        self.encoder = *encoder;
        self.archive = *archive;
    }

    /// The bar to end the take at, or `None` when the trigger is not
    /// [`AtBar`](RenderTrigger::AtBar).
    ///
    /// The trigger and the number are separate fields so that switching away
    /// and back keeps the bar you dialed in, which means every reader has to
    /// ask both. Asking here once is what stops one of them forgetting.
    ///
    /// **This is where the bar changes base**, and it is the whole reason the
    /// conversion lives in one function. [`stop_bar`](Self::stop_bar) is the
    /// arranger's 1-based bar because that is the number on screen in the host;
    /// the transport position it is compared against counts the song's first
    /// bar as zero. Passing the field through unconverted stops a bar LATE —
    /// and if the bar asked for was the arrangement's last, the comparison
    /// point is never reached at all and the take never ends, which is the
    /// failure this trigger exists to remove.
    pub fn stop_at_bar(&self) -> Option<f64> {
        (self.trigger == RenderTrigger::AtBar).then_some(self.stop_bar - 1.0)
    }
}

/// What [`RenderConfig::stop_bar`]'s field can produce, and so what
/// [`RenderConfig::sanitize`] holds a blob to. The top end is about nine hours
/// at 120 bpm in 4/4 — past any piece, and short of where an f64 bar count
/// stops resolving a fraction of a bar.
///
/// The bottom end is 2 rather than 1 because a take ends where it plays THROUGH
/// the bar, and nothing is before the song's start to play through bar 1 from.
/// Allowing it would put a settable, plausible-looking value on the dial that
/// provably never fires.
pub const STOP_BAR_RANGE: (f64, f64) = (2.0, 100_000.0);

/// What [`RenderConfig::tail`]'s field can produce, in seconds. A minute is
/// past any release or reverb worth waiting for.
pub const TAIL_RANGE: (f64, f64) = (0.0, 60.0);

/// Which side of the video frame the lattice takes; the Spectral pane takes
/// whatever is left.
///
/// Named for where the LATTICE lands rather than for the axis the frame is cut
/// on, because the placement is what you are choosing — "side by side" says
/// which axis and leaves which pane goes where to a convention, and there is no
/// name at all for the mirror of it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum LatticeSide {
    /// Lattice left, Spectral pane right.
    #[default]
    Left,
    /// Lattice right, Spectral pane left.
    Right,
    /// Lattice above, Spectral pane below.
    Top,
    /// Lattice below, Spectral pane above.
    Bottom,
}

impl LatticeSide {
    /// Every side, for the settings row and the layout tests.
    ///
    /// Built from an exhaustive `match` rather than written out as a literal,
    /// so the list cannot fall behind the enum — the same guard
    /// `SpectralOrientation::ALL` in `harmonigraph-ui` uses, for the same reason.
    pub const ALL: [LatticeSide; 4] = {
        use LatticeSide::*;
        // Exhaustive, and the compiler checks it. The arm is `()` because what
        // is wanted is the coverage error, not the value.
        const fn covered(side: LatticeSide) {
            match side {
                Left | Right | Top | Bottom => (),
            }
        }
        covered(Left);
        [Left, Right, Top, Bottom]
    };

    /// Whether the lattice's share of the frame is a HEIGHT — the frame cut
    /// across rather than down.
    ///
    /// An exhaustive `match` rather than a `matches!`: a `matches!` answers
    /// `false` for a variant nobody has thought about yet, so a fifth side
    /// would silently be measured as a width instead of failing to build.
    pub fn sizes_by_height(self) -> bool {
        match self {
            LatticeSide::Top | LatticeSide::Bottom => true,
            LatticeSide::Left | LatticeSide::Right => false,
        }
    }
}

/// The video frame the Video pane composes: an aspect ratio plus the
/// lattice/spectral split. Aspect is size-agnostic (the render's resolution is
/// chosen separately); the split feeds `Layout::split` in `harmonigraph-ui`,
/// so the plugin's live preview and the offline renderer build the identical
/// frame.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct RenderFrame {
    /// Frame aspect numerator (e.g. 16 of 16:9). Drives the preview letterbox
    /// and the render's default resolution.
    pub aspect_w: u32,
    pub aspect_h: u32,
    /// The lattice's share of the frame, `0..1` (the rest is the spectral
    /// pane) — a width beside the Spectral pane, a height above or below it.
    /// The lattice's share whichever side it takes, so this number means one
    /// thing as [`lattice`](Self::lattice) changes under it.
    pub split: f32,
    /// Where the lattice sits; see [`LatticeSide`].
    pub lattice: LatticeSide,
}

impl RenderFrame {
    /// Smallest lattice share offered by the preview divider and composed layout.
    pub const SPLIT_MIN: f32 = 0.05;
    /// Largest lattice share; the spectral pane retains the same minimum share.
    pub const SPLIT_MAX: f32 = 0.95;

    /// Output pixels for this aspect with its SHORT edge at `short_edge`: 16:9
    /// at 1080 is 1920x1080, 9:16 at 1080 is 1080x1920.
    ///
    /// The short edge is the dimension people name a format by ("1080p", "4K"
    /// notwithstanding), and it is the one that keeps a portrait render as
    /// tall as a landscape one is wide instead of a fifth the pixels.
    /// Both dimensions come out even, which ffmpeg's yuv420p requires.
    ///
    /// The single definition of aspect-to-pixels: the Video pane and the
    /// Exports list display it, and `harmonigraph-offline` renders at it from
    /// the appearance it is given unless a hand-run `--size` overrides it.
    /// Every caller agreeing by construction
    /// is the point — a second copy is how the preview and the render come to
    /// disagree about the shape of the picture.
    pub fn pixels(&self, short_edge: u32) -> [u32; 2] {
        let (w, h) = (self.aspect_w.max(1) as f64, self.aspect_h.max(1) as f64);
        let short = short_edge.max(2) as f64;
        let even = |x: f64| ((x.round() as u32).max(2)) & !1;
        if w >= h {
            [even(short * w / h), even(short)]
        } else {
            [even(short), even(short * h / w)]
        }
    }

    /// Fit a deserialized frame to what its own controls can produce.
    ///
    /// `aspect_w`/`aspect_h` need no repair: they are u32 (no NaN to carry)
    /// and only ever set from the Video pane's fixed presets, and every
    /// reader of the pair already floors each side at 1 (see
    /// [`pixels`](Self::pixels)'s own `.max(1)`), so a hand-edited 0 costs
    /// nothing here that isn't already caught where it is used.
    ///
    /// `split` is different: the Video pane's preview divider holds it to
    /// [`SPLIT_MIN`](Self::SPLIT_MIN)..=[`SPLIT_MAX`](Self::SPLIT_MAX),
    /// and `Layout::split` clamps into that same range — which cannot itself
    /// panic (the bounds are constants, not a second field), but does not repair
    /// a NaN either, `clamp` losing every
    /// comparison against one. A NaN split would then reach `Layout::resolve`
    /// as a rect with no finite side — not a crash, but a frame with a torn
    /// composition and nothing in the picture to say why.
    pub fn sanitize(&mut self) {
        self.split =
            repair(self.split, RenderFrame::default().split, Self::SPLIT_MIN..=Self::SPLIT_MAX);
    }
}

impl Default for RenderFrame {
    fn default() -> Self {
        RenderFrame {
            aspect_w: 16,
            aspect_h: 9,
            // Over a third of the frame to the lattice, on the right, the
            // rest to the spectral pane — as captured from the DAW on
            // 2026-10-07. The two are not competing for the same job: the
            // lattice reads at whatever size it is given (it is a handful of
            // nodes, and the camera frames them), while the spectrogram's width
            // IS its time axis, so width buys it seconds on screen.
            split: 0.363_348_75,
            lattice: LatticeSide::Right,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retired_export_fields_are_ignored_and_not_saved_again() {
        let config: RenderConfig = ron::from_str(
            r#"(record_audio:true,auto_render:false,audio_path:"old.wav",audio_offset:"12.5",
                renderer_path:"/renderer",short_edge:2160,trigger:AtBar,stop_bar:17.0)"#,
        )
        .unwrap();
        assert_eq!(config.short_edge, 2160);
        assert_eq!(config.trigger, RenderTrigger::AtBar);
        assert_eq!(config.stop_bar, 17.0);
        let saved = ron::to_string(&config).unwrap();
        for removed in
            ["record_audio", "auto_render", "audio_path", "audio_offset", "renderer_path"]
        {
            assert!(!saved.contains(removed), "retired field saved again: {removed}");
        }
    }
}
