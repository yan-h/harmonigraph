//! How a take becomes a video: what the render is composed and triggered by,
//! and how far one has got.
//!
//! The SETTINGS live here rather than in the UI that edits them because they
//! are take PAYLOAD. `RenderFrame` is the composition a take was framed at,
//! and the offline renderer reads it out of the take so a re-render reproduces
//! the framing it was dialed in at rather than whatever the editor happens to
//! be set to now. `playhead` is read the same way.
//!
//! Resolution is the deliberate exception and stays outside `RenderFrame` —
//! see [`RenderConfig::short_edge`].
//!
//! They are serde-facing: each round-trips through a saved project's UI blob
//! and through the `ui_state` a take carries.
//!
//! [`RenderProgress`] is the one member that is NEITHER — no serde, and it
//! never enters a take. It is counted off the renderer subprocess's stdout and
//! lives only as long as that process. It sits beside the settings because
//! `harmonigraph-record` drives that subprocess and reports it back, and must
//! not link the editor to do so: settings in, progress out is the whole of
//! what the recorder needs from the Video pane. Moving it back to the UI on
//! the grounds that it is not payload would put the GUI stack back on the
//! record path, which is what #176 removed.

/// What counts as "the take is done", and so when a video gets rendered.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum RenderTrigger {
    /// When you switch Record take off. Predictable, and works no matter how
    /// the transport behaves.
    #[default]
    OnDisarm,
    /// As soon as the transport stops after recording something — so a
    /// play-through, or an audio export, produces a video with nothing
    /// further to click. Recording disarms itself at the same moment.
    ///
    /// Or when the transport goes BACKWARDS, whichever comes first — see
    /// [`ends_at_rewind`](Self::ends_at_rewind). A host puts the playhead back
    /// when an export finishes, and it does so within the few frames the stop is
    /// counted over, so the rewind is the earlier and the surer of the two
    /// signals. The cost is that rewinding mid-take ends it here.
    ///
    /// Falls back gracefully: if a host stops calling `process` the
    /// instant a render finishes AND leaves the playhead where it stopped,
    /// neither signal arrives and the take simply waits for you to disarm it.
    OnTransportStop,
    /// When the arranger loop first repeats: exactly one loop is recorded, the
    /// take ends at the loop's end, and that pass renders — no catching the
    /// stop by hand. Meant for looped recording, where a manual stop is always
    /// a beat or two off.
    ///
    /// Detected by the transport wrapping, so **looping must be enabled**.
    /// Hosts don't reliably tell a plugin where the loop markers are (Bitwig
    /// doesn't flag its loop as active, so nih-plug's loop range is `None`), so
    /// with looping off there is nothing to wrap on and it waits for you to
    /// disarm, like [`OnDisarm`](Self::OnDisarm).
    AtLoopEnd,
}

impl RenderTrigger {
    /// Whether the transport going BACKWARDS ends the take rather than splitting
    /// it into another pass.
    ///
    /// [`OnDisarm`](Self::OnDisarm) is the one trigger that has to survive a
    /// looping transport, so it keeps splitting. The other two want a single
    /// file, and a backward jump is where it ends: the loop repeating for
    /// [`AtLoopEnd`](Self::AtLoopEnd), and the host putting the playhead back
    /// for [`OnTransportStop`](Self::OnTransportStop) — which is what finishing
    /// an audio export looks like from inside the plugin, and arrives before a
    /// stop counted in GUI frames can be sure of it.
    pub fn ends_at_rewind(self) -> bool {
        match self {
            RenderTrigger::OnDisarm => false,
            RenderTrigger::OnTransportStop | RenderTrigger::AtLoopEnd => true,
        }
    }
}

#[cfg(test)]
mod trigger_tests {
    use super::RenderTrigger;

    /// Which triggers hand the take's end to the audio thread. Written out one
    /// by one rather than as `!= OnDisarm`, so adding a variant has to decide.
    #[test]
    fn only_the_trigger_that_survives_a_loop_keeps_splitting() {
        assert!(!RenderTrigger::OnDisarm.ends_at_rewind());
        assert!(RenderTrigger::OnTransportStop.ends_at_rewind());
        assert!(RenderTrigger::AtLoopEnd.ends_at_rewind());
    }
}

/// How a finished take gets turned into a video, edited in the Video
/// pane's Record section and persisted with the UI state.
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
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct RenderConfig {
    /// Record the plugin's audio input into the take. Dormant, and nothing
    /// reads it: the recorder captures audio unconditionally and every render
    /// uses the take's own recording as soundtrack and spectrum, aligned to
    /// the picture by construction — see `RenderRequest::build` in
    /// `harmonigraph-record`, which passes neither `--audio` nor `--align`.
    /// (Named rather than linked: that crate depends on this one, not the
    /// other way round.)
    ///
    /// Kept, with `auto_render`, `audio_path` and `audio_offset`, so the
    /// bounced-audio drop-in the four belong to can be revived without
    /// re-deciding its shape. NOT kept for the blob: serde ignores a key it
    /// has no field for, so removing one costs nothing at load — which is
    /// what `a_persist_blob_carrying_a_since_removed_field_still_loads`
    /// holds, and what five removed spectrogram keys already relied on.
    pub record_audio: bool,
    /// Run the renderer as soon as a take finishes.
    pub auto_render: bool,
    /// What "finishes" means; see [`RenderTrigger`].
    pub trigger: RenderTrigger,
    /// Path to the `harmonigraph-offline` binary. Empty means the
    /// conventional install location, which `update-plugin.sh` writes to.
    pub renderer_path: String,
    /// Bounced audio to pass as `--audio`: it feeds the spectrum curve
    /// and is muxed into the video. Empty renders silent, with no
    /// spectrum — the roll and the lattice are unaffected.
    pub audio_path: String,
    /// Take-time (seconds) where the bounce starts — empty means auto-align to
    /// the MIDI onsets, a number passes `--align`. A string so "empty = auto"
    /// reads naturally and it matches the other free-text fields.
    pub audio_offset: String,
    /// Whole-song playhead spectrogram: lay the take out at once and sweep a
    /// playhead through it, instead of the live scrolling window. Read by the
    /// offline renderer from the take; `--playhead` on the command line also
    /// turns it on. Needs audio.
    pub playhead: bool,
    /// The composed video frame — aspect ratio and the lattice/spectral split.
    /// Edited and previewed in the Video pane; the offline renderer reads it
    /// to compose the same picture.
    pub frame: RenderFrame,
    /// The render's short edge in pixels; with [`frame`](Self::frame)'s aspect
    /// this is the whole output size (see [`RenderFrame::pixels`]).
    ///
    /// Deliberately NOT part of `RenderFrame`: the frame is a composition, and
    /// it rides inside a take so a re-render reproduces the framing it was
    /// dialed in at. Resolution is a per-export choice — draft at 1080, final
    /// at 2160, same picture — so it stays out here where changing it cannot
    /// mean the take was framed differently.
    pub short_edge: u32,
}

impl Default for RenderConfig {
    fn default() -> Self {
        RenderConfig {
            record_audio: false,
            auto_render: false,
            trigger: RenderTrigger::OnDisarm,
            renderer_path: String::new(),
            audio_path: String::new(),
            audio_offset: String::new(),
            playhead: false,
            frame: RenderFrame::default(),
            // 1080 on the short edge — 1920x1080 at the default 16:9 frame,
            // and the resolution every host and site takes without
            // transcoding.
            short_edge: 1080,
        }
    }
}

impl RenderConfig {
    /// Fit a deserialized render config to what its own controls can
    /// produce, on the same footing as `ViewConfig::sanitize` and
    /// `SpectrumConfig::sanitize` in `harmonigraph-ui`: a bar cannot hand
    /// back a nonsense value, a hand-edited blob can, and this is a video
    /// dial rather than a real-time one, so a bad number here silently
    /// composes a frame nobody would have set rather than merely misdrawing
    /// one.
    pub fn sanitize(&mut self) {
        self.frame.sanitize();
    }
}

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
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
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
    /// Output pixels for this aspect with its SHORT edge at `short_edge`: 16:9
    /// at 1080 is 1920x1080, 9:16 at 1080 is 1080x1920.
    ///
    /// The short edge is the dimension people name a format by ("1080p", "4K"
    /// notwithstanding), and it is the one that keeps a portrait render as
    /// tall as a landscape one is wide instead of a fifth the pixels.
    /// Both dimensions come out even, which ffmpeg's yuv420p requires.
    ///
    /// The single definition of aspect-to-pixels: the Video pane displays it,
    /// the plugin passes it as `--size`, and `harmonigraph-offline` falls back
    /// to it when no `--size` is given. Three callers agreeing by construction
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
    /// `split` is different: the Video pane's own bar holds it to
    /// `0.05..=0.95`, and `Layout::split` clamps into that same literal
    /// range — which cannot itself panic (the bounds are constants, not a
    /// second field), but does not repair a NaN either, `clamp` losing every
    /// comparison against one. A NaN split would then reach `Layout::resolve`
    /// as a rect with no finite side — not a crash, but a frame with a torn
    /// composition and nothing in the picture to say why.
    pub fn sanitize(&mut self) {
        self.split = if self.split.is_finite() {
            self.split.clamp(0.05, 0.95)
        } else {
            RenderFrame::default().split
        };
    }
}

impl Default for RenderFrame {
    fn default() -> Self {
        RenderFrame {
            aspect_w: 16,
            aspect_h: 9,
            // A fifth of the frame to the lattice, the rest to the spectral
            // pane. The two are not competing for the same job: the lattice
            // reads at whatever size it is given (it is a handful of nodes,
            // and the camera frames them), while the spectrogram's width IS
            // its time axis, so width buys it seconds on screen.
            split: 0.20,
            lattice: LatticeSide::Left,
        }
    }
}

/// How far a video render running in the background has got.
///
/// Frames rather than a fraction, because frames are what the renderer counts
/// and "3400/5400" says something a filled bar cannot: how long is left, at
/// whatever rate you have been watching it go.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RenderProgress {
    /// Frames written so far.
    pub done: u64,
    /// Frames the render is aiming for — 0 until the renderer has said, which
    /// is a moment into the run (it has a take to read and an encoder to
    /// start first).
    pub total: u64,
}

impl RenderProgress {
    /// The share done, in `0..=1`, or `None` while the total is unknown —
    /// which is not the same as zero, and must not draw as it.
    pub fn fraction(self) -> Option<f32> {
        (self.total > 0).then(|| (self.done as f32 / self.total as f32).clamp(0.0, 1.0))
    }
}
