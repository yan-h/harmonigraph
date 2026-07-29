//! The display settings the UI persists: the Spectral pane's analyzer
//! configuration, the video render settings, and the shared bar ranges they
//! are edited through. Serde-facing — every `default_*` here names what a
//! blob saved before its field existed loads as.

/// The Spectral pane's analysis window length, picked in the Spectrum
/// settings tab: longer windows resolve bass pitch more sharply but
/// respond more slowly (the tradeoff is physics, not implementation).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SpectrumWindow {
    /// 4096 samples (~85 ms at 48 kHz).
    Fast,
    /// 8192 samples (~171 ms): the default balance.
    Balanced,
    /// 16384 samples (~341 ms).
    Precise,
}

impl SpectrumWindow {
    pub fn samples(self) -> usize {
        match self {
            SpectrumWindow::Fast => 4096,
            SpectrumWindow::Balanced => 8192,
            SpectrumWindow::Precise => 16384,
        }
    }
}

/// Which way the Spectral pane runs, named for the side the NOW-line is on —
/// which is the spectrum's own edge, the one the roll's notes arrive at and
/// the heatmap's newest column sits against. Time runs away from it into the
/// pane, and pitch across.
///
/// The pane is written once against an abstract (pitch, depth) plane and
/// mapped onto the screen at draw time, so every element — gridlines,
/// spectrum curve, piano roll — turns together.
///
/// Pitch reads the conventional way in each pair rather than mirroring with
/// time: low at the bottom whenever time is horizontal, low at the left
/// whenever it is vertical. So [`Right`](Self::Right) and
/// [`Bottom`](Self::Bottom) are their partners flipped along TIME alone, and
/// an ascending line still ascends in all four.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SpectralOrientation {
    /// Time runs left(now)->right(past) along the pane, pitch climbs
    /// bottom->top, and the spectrum sits on the left, joined to the
    /// spectrogram.
    ///
    /// The aliases are what this variant was called across two renamings. It
    /// was `Horizontal`, from when the name meant the PITCH axis; and it takes
    /// `Auto` — a fourth setting that resolved to whichever layout gave the
    /// scrolling spectrogram the pane's long side — because a blob naming a
    /// variant that no longer exists does not just lose its orientation, the
    /// parse fails and drops the WHOLE persist, layout and camera with it.
    #[default]
    #[serde(alias = "Horizontal", alias = "Auto")]
    Left,
    /// Time runs right(now)->left(past); pitch climbs bottom->top, and the
    /// spectrum sits on the right.
    Right,
    /// Time runs top(now)->bottom(past) down the pane, pitch runs left->right,
    /// and the spectrum sits on top, joined to the spectrogram.
    #[serde(alias = "Vertical")]
    Top,
    /// Time runs bottom(now)->top(past); pitch runs left->right, and the
    /// spectrum sits along the bottom.
    Bottom,
}

impl SpectralOrientation {
    /// Every orientation, for the settings row and the axis tests.
    ///
    /// Built from an exhaustive `match` rather than written out as a literal, so
    /// the list cannot fall behind the enum: a fifth variant fails to compile
    /// here until it is added, which is what makes the tests that sweep this a
    /// guarantee about the enum rather than about four names someone typed.
    pub(crate) const ALL: [SpectralOrientation; 4] = {
        use SpectralOrientation::*;
        // Exhaustive, and the compiler checks it. The arms are `()` because
        // what is wanted is the coverage error, not the value — a const fn
        // cannot build the array itself.
        const fn covered(o: SpectralOrientation) {
            match o {
                Left | Right | Top | Bottom => (),
            }
        }
        covered(Left);
        [Left, Right, Top, Bottom]
    };

    /// Whether TIME (the spectrogram/roll axis) runs vertically down the pane,
    /// with pitch across it.
    pub(crate) fn is_time_vertical(self) -> bool {
        match self {
            SpectralOrientation::Top | SpectralOrientation::Bottom => true,
            SpectralOrientation::Left | SpectralOrientation::Right => false,
        }
    }

    /// Whether time runs BACKWARD along its screen axis — leftward or upward,
    /// against the direction screen coordinates grow.
    ///
    /// Spelled as an exhaustive `match` rather than a `matches!`, like
    /// [`is_time_vertical`](Self::is_time_vertical): a `matches!` answers
    /// `false` for a variant nobody has thought about yet, so a fifth
    /// orientation would silently draw as [`Left`](Self::Left) instead of
    /// failing to build.
    pub(crate) fn is_time_reversed(self) -> bool {
        match self {
            SpectralOrientation::Right | SpectralOrientation::Bottom => true,
            SpectralOrientation::Left | SpectralOrientation::Top => false,
        }
    }
}

/// The color ramp a spectrogram cell's intensity maps through. A set of
/// looks to pick from — the spectrogram is a heatmap, and the palette is
/// most of its character. Intensity always runs dark (quiet) to bright
/// (loud); these differ only in the hues it passes through.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SpectrogramColor {
    /// Grayscale: black to white. The classic, and the most neutral over
    /// the roll's own colors.
    Mono,
    /// Black → navy → blue → cyan → white. The cool ramp.
    Ice,
    /// Black → violet → teal → green → yellow. A perceptually even ramp
    /// (viridis-like) where every step reads as an equal change — the
    /// default, because an even ramp is the one that reads a heatmap's
    /// quiet detail honestly.
    #[default]
    Aurora,
    /// Black → indigo → magenta → orange → cream. Warmer than
    /// [`Self::Aurora`] and evenly stepped like it.
    ///
    /// The aliases absorb palettes that used to exist: Heat (the familiar
    /// black-red-orange-yellow-white spectrogram, dropped for spending most
    /// of its range in the reds — Magma is that warmth evenly stepped),
    /// Pitch, which tinted each cell with the lattice's own low-to-high
    /// pitch color, and Paper, an inverted ramp for light backgrounds.
    /// Without them a blob still naming one wouldn't just lose its palette:
    /// the parse would fail and drop the WHOLE persist, layout and camera
    /// with it.
    #[serde(alias = "Heat", alias = "Pitch", alias = "Paper")]
    Magma,
}

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
    /// Falls back gracefully: if a host stops calling `process` the
    /// instant a render finishes, the stop is never observed and the take
    /// simply waits for you to disarm it, as before.
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

/// How a finished take gets turned into a video, edited in the Video
/// pane's Record section and persisted with the UI state.
///
/// The plugin cannot render video itself — that is `harmonigraph-offline`, a
/// separate binary with a headless GPU device and an ffmpeg pipe, and
/// nothing about it belongs inside a real-time audio plugin. What the
/// plugin can do is *run* it, the moment a take is complete.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct RenderConfig {
    /// Record the plugin's audio input into the take (see
    /// `SharedState::take_audio`, which mirrors this at runtime).
    #[serde(default)]
    pub record_audio: bool,
    /// Run the renderer as soon as a take finishes.
    #[serde(default)]
    pub auto_render: bool,
    /// What "finishes" means; see [`RenderTrigger`].
    #[serde(default)]
    pub trigger: RenderTrigger,
    /// Path to the `harmonigraph-offline` binary. Empty means the
    /// conventional install location, which `update-plugin.sh` writes to.
    #[serde(default)]
    pub renderer_path: String,
    /// Bounced audio to pass as `--audio`: it feeds the spectrum curve
    /// and is muxed into the video. Empty renders silent, with no
    /// spectrum — the roll and the lattice are unaffected.
    #[serde(default)]
    pub audio_path: String,
    /// Take-time (seconds) where the bounce starts — empty means auto-align to
    /// the MIDI onsets, a number passes `--align`. A string so "empty = auto"
    /// reads naturally and it matches the other free-text fields.
    #[serde(default)]
    pub audio_offset: String,
    /// Extra flags, split on whitespace (no shell quoting):
    /// `--size 3840x2160 --layout side-by-side`.
    #[serde(default)]
    pub extra_args: String,
    /// Whole-song playhead spectrogram: lay the take out at once and sweep a
    /// playhead through it, instead of the live scrolling window. Read by the
    /// offline renderer from the take; `--playhead` on the command line also
    /// turns it on. Needs audio.
    #[serde(default)]
    pub playhead: bool,
    /// The composed video frame — aspect ratio and the lattice/spectral split.
    /// Edited and previewed in the Video pane; the offline renderer reads it
    /// to compose the same picture.
    #[serde(default)]
    pub frame: RenderFrame,
    /// The render's short edge in pixels; with [`frame`](Self::frame)'s aspect
    /// this is the whole output size (see [`RenderFrame::pixels`]).
    ///
    /// Deliberately NOT part of `RenderFrame`: the frame is a composition, and
    /// it rides inside a take so a re-render reproduces the framing it was
    /// dialed in at. Resolution is a per-export choice — draft at 1080, final
    /// at 2160, same picture — so it stays out here where changing it cannot
    /// mean the take was framed differently.
    #[serde(default = "default_short_edge")]
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
            // Empty: the size now comes from Aspect x Resolution, which the
            // plugin passes as `--size` itself. A default that named a
            // resolution here silently outranked the Aspect row above it —
            // a 9:16 frame rendered 1920x1080 and only the preview was tall.
            extra_args: String::new(),
            playhead: false,
            frame: RenderFrame::default(),
            short_edge: default_short_edge(),
        }
    }
}

/// 1080 on the short edge — 1920x1080 at the default 16:9 frame, and the
/// resolution every host and site takes without transcoding.
pub(crate) fn default_short_edge() -> u32 {
    1080
}

impl RenderConfig {
    /// Fold older blob layouts onto the current fields; call after
    /// deserializing a persisted config.
    ///
    /// Blobs written before the Resolution control carry the size in
    /// `extra_args` as a literal `--size WxH` — for most of them the old
    /// default, `--size 1920x1080`, which was applied whatever the Aspect row
    /// said. Lifting its short edge into [`short_edge`](Self::short_edge) and
    /// dropping the flag is what makes a saved project pick up the aspect it
    /// always displayed: the frame decides the shape, the lifted number
    /// decides only how big.
    ///
    /// It lifts EVERY `--size`, not only the retired default, and the cost of
    /// that is worth naming: a `--size` deliberately typed at an aspect the
    /// frame does not have — a landscape render of a portrait composition —
    /// comes back after a reload as the frame's own shape at that size. The
    /// blob cannot tell that apart from the bug (both are a `--size` fighting
    /// the aspect), so one of the two has to lose, and leaving the bug in
    /// place would mean every project that never touched the field keeps
    /// rendering the wrong shape. The deliberate case is re-typed in a
    /// second, is warned about on every render, and is the rarer by far.
    pub fn migrate_legacy(&mut self) {
        self.frame.migrate_legacy();
        if let Some(short) = take_size_flag(&mut self.extra_args) {
            self.short_edge = short;
        }
    }
}

/// Remove a `--size WxH` (or `-s WxH`) pair from a whitespace-split flag
/// string, returning the size's short edge.
///
/// Only the LAST one decides, because that is what the renderer's own parser
/// does with a repeated flag — so a migration reading a different size than
/// the render used would be a migration that changes the picture.
fn take_size_flag(args: &mut String) -> Option<u32> {
    let mut short = None;
    // `kept` borrows `args`, so it has to become an owned string before the
    // assignment below can touch `args` again.
    let kept = {
        let tokens: Vec<&str> = args.split_whitespace().collect();
        let mut kept: Vec<&str> = Vec::with_capacity(tokens.len());
        let mut i = 0;
        while i < tokens.len() {
            let is_size = matches!(tokens[i], "--size" | "-s");
            match (is_size, tokens.get(i + 1).and_then(|v| parse_wxh(v))) {
                (true, Some((w, h))) => {
                    short = Some(w.min(h));
                    i += 2;
                }
                _ => {
                    kept.push(tokens[i]);
                    i += 1;
                }
            }
        }
        kept.join(" ")
    };
    // A `--size` with an unparseable value is left in place: the renderer
    // should reject it and say so, rather than this quietly eating it.
    if short.is_some() {
        *args = kept;
    }
    short
}

/// `WxH` as the renderer's `--size` accepts it. A zero edge is treated as
/// unparseable rather than lifted: it would leave the control on a size no
/// button shows, and the renderer is the right place to reject it.
fn parse_wxh(text: &str) -> Option<(u32, u32)> {
    let (w, h) = text.split_once(['x', 'X', '*'])?;
    let (w, h) = (w.parse().ok()?, h.parse().ok()?);
    (w > 0 && h > 0).then_some((w, h))
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
    /// [`SpectralOrientation::ALL`] uses, for the same reason.
    pub(crate) const ALL: [LatticeSide; 4] = {
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
/// chosen separately); the split feeds [`Layout::split`](crate::Layout::split),
/// so the plugin's live preview and the offline renderer build the identical
/// frame.
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub struct RenderFrame {
    /// Frame aspect numerator (e.g. 16 of 16:9). Drives the preview letterbox
    /// and the render's default resolution.
    #[serde(default = "default_aspect_w")]
    pub aspect_w: u32,
    #[serde(default = "default_aspect_h")]
    pub aspect_h: u32,
    /// The lattice's share of the frame, `0..1` (the rest is the spectral
    /// pane) — a width beside the Spectral pane, a height above or below it.
    /// The lattice's share whichever side it takes, so this number means one
    /// thing as [`lattice`](Self::lattice) changes under it.
    #[serde(default = "default_frame_split")]
    pub split: f32,
    /// Where the lattice sits; see [`LatticeSide`].
    #[serde(default)]
    pub lattice: LatticeSide,
    /// Load-only shim: blobs written before the four sides — and the
    /// `ui_state` carried inside takes recorded then — say `stacked: bool`
    /// here, the two-way choice this replaced. Folded into
    /// [`lattice`](Self::lattice) by
    /// [`migrate_legacy`](Self::migrate_legacy) and never written back.
    #[serde(default, skip_serializing, rename = "stacked")]
    pub legacy_stacked: bool,
}

impl RenderFrame {
    /// Fold `stacked: bool` onto [`lattice`](Self::lattice); call after
    /// deserializing a persisted frame.
    ///
    /// `true` meant lattice above the Spectral pane and `false` meant lattice
    /// to its left, so only the stacked half has anything to say — and a blob
    /// new enough to name a side carries no `stacked` at all, which is what
    /// keeps this from overwriting one.
    pub fn migrate_legacy(&mut self) {
        if std::mem::take(&mut self.legacy_stacked) {
            self.lattice = LatticeSide::Top;
        }
    }

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
}

pub(crate) fn default_aspect_w() -> u32 {
    16
}
pub(crate) fn default_aspect_h() -> u32 {
    9
}
/// A fifth of the frame to the lattice, the rest to the spectral pane. The
/// two are not competing for the same job: the lattice reads at whatever size
/// it is given (it is a handful of nodes, and the camera frames them), while
/// the spectrogram's width IS its time axis, so width buys it seconds on
/// screen. Kept in step with `RenderFrame::default`'s `split` — the serde
/// default and the struct default answering differently would mean a blob
/// that omits the field loads as a frame nobody chose.
pub(crate) fn default_frame_split() -> f32 {
    0.20
}

impl Default for RenderFrame {
    fn default() -> Self {
        RenderFrame {
            aspect_w: 16,
            aspect_h: 9,
            split: default_frame_split(),
            lattice: LatticeSide::Left,
            legacy_stacked: false,
        }
    }
}

/// Everything the Spectral pane's display is configured by, edited in the
/// Spectrum settings tab and persisted with the UI state.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SpectrumConfig {
    /// Which side of the pane the now-line sits on, and so which way time
    /// runs; see [`SpectralOrientation`]. Four sides and no more — the pitch
    /// axis reads the conventional way in each, so there is nothing left to
    /// flip.
    #[serde(default)]
    pub orientation: SpectralOrientation,
    pub window: SpectrumWindow,
    /// Bottom of the dB height scale: what reads as silence. A full-scale
    /// sine sits at 0 dB.
    pub floor_db: f32,
    /// Top of the dB height scale: what reads as full height (and as the
    /// brightest spectrogram cell). 0 dB — a full-scale sine — is where it
    /// starts and the loudest it goes; pulling it down lifts quiet material
    /// into the whole picture instead of the bottom of it.
    ///
    /// The pair is one control, like the pitch range: the window on the
    /// spectrum's dynamics, movable at either end.
    #[serde(default = "default_ceiling_db")]
    pub ceiling_db: f32,
    /// Display inertia: 0 = every refresh lands instantly, 0.9 = slow.
    pub smoothing: f32,
    /// Spectral tilt, in the convention analyzers use: the reference
    /// slope in dB/octave that displays as FLAT, one of [`TILT_STEPS`]
    /// (0, -1.5 .. -6). 0 draws raw power; -3 makes pink noise read
    /// flat; -4.5 flattens typical musical material. The display lifts
    /// treble by the magnitude, pivoting at 1 kHz — there is no
    /// bass-emphasizing direction, matching convention.
    ///
    /// `default_tilt`, not `default`, so a blob saved before the field
    /// existed loads with the slope a fresh install gets rather than the
    /// raw-power 0 a bare f32 default would hand it.
    #[serde(default = "default_tilt")]
    pub tilt: f32,
    /// Overall size of the pane's own markings — the gridline labels above and
    /// the pitch readout that follows the pointer — as a multiple of their
    /// built-in sizes.
    ///
    /// Fixed against the zoom, unlike the note names: a marking says what the
    /// axis is, and the axis is the one thing on the pane that does not change
    /// size when the range is zoomed.
    ///
    /// A saved view loads at 1 and so draws its markings TWICE the size it was
    /// saved at, the built-in size having been rebased by 2 (see
    /// `panes::spectral::MARKING_PT`). Deliberate, and the same call as the
    /// lattice's: 10pt was the wrong number rather than one of several, and a
    /// blob that kept it would be preserving a mistake.
    #[serde(default = "default_one")]
    pub marking_scale: f32,
    /// Strength of the light edge drawn along the spectrum's profile and
    /// around each note ribbon, 0 = none. On a roll note it is the whole of
    /// the rim — how bright the keyline is, and whether it is drawn at all.
    /// See `panes::roll::keyline`.
    #[serde(default = "default_keyline")]
    pub keyline: f32,
    /// Displayed pitch range, as (fractional) MIDI note numbers. The
    /// analyzer always covers `SPECTRUM_MIN_MIDI..=SPECTRUM_MAX_MIDI`
    /// (~16 Hz to ~16.7 kHz); this only zooms the view.
    ///
    /// MIDI rather than Hz because the axis is linear in MIDI note, which
    /// makes this both the number the pane wants and — since a semitone is a
    /// constant frequency RATIO — a logarithmic frequency scale. The control
    /// drags it linearly and reads it out in Hz.
    #[serde(default = "default_low_midi")]
    pub low_midi: f32,
    #[serde(default = "default_high_midi")]
    pub high_midi: f32,
    /// Migration only. The range used to be a pair of Bitwig octave numbers,
    /// so it could only land on C boundaries; `migrate_legacy` folds an older
    /// blob's pair into `low_midi`/`high_midi` and nothing writes them again.
    ///
    /// A sentinel rather than `Option`, because the old blobs wrote a bare
    /// `low_octave: 1` and RON only reads that into an `Option` if it is
    /// spelled `Some(1)` — the field would never populate, and the failed
    /// parse would take the whole persist down with it.
    #[serde(default = "no_legacy_octave", skip_serializing, alias = "low_octave")]
    pub(crate) legacy_low_octave: i32,
    #[serde(default = "no_legacy_octave", skip_serializing, alias = "high_octave")]
    pub(crate) legacy_high_octave: i32,

    // ---- Piano roll -------------------------------------------------
    // The played-note timeline (harmonigraph-core's NoteRoll) drawn over the
    // same pitch axis, occupying the far end of the depth axis. Time runs
    // away from the spectrum: a note leaving the roll's near edge meets
    // the spectrum peak it is making.
    /// Draw the incoming MIDI's history at all.
    #[serde(default = "default_true")]
    pub show_roll: bool,
    /// Share of the pane's depth given to the roll (the rest is the
    /// spectrum). 0 hides it; 1 gives the whole pane to the roll. Set by
    /// dragging the divider in the Spectral pane itself
    /// (`panes::spectral::drag_split`) — there is no bar for it.
    #[serde(default = "default_roll_fraction")]
    pub roll_fraction: f32,
    /// Seconds of history the roll's depth spans.
    #[serde(default = "default_roll_seconds")]
    pub roll_seconds: f32,
    /// Note ribbon width, in semitones of the pitch axis. This IS the note's
    /// painted width — a note is a solid rectangle of its own color, with
    /// nothing straddling its boundary.
    #[serde(default = "default_roll_thickness")]
    pub roll_thickness: f32,
    /// Write each note's name over its ribbon, at the moment it was struck —
    /// see [`panes::names`](crate::panes::names). `default_true`, not `default`, or a state blob
    /// saved before this field existed would load with them off, contradicting
    /// the struct's own default, which is what a fresh install gets.
    #[serde(default = "default_true")]
    pub note_names: bool,
    /// Overall size of those names, as a multiple of their built-in size.
    ///
    /// Rides on top of the pitch zoom, which already grows a name as the range
    /// narrows so that it keeps its footing on the ribbon it is written on —
    /// see `panes::spectral::name_zoom`. This says how big it is at the zoom
    /// you are at.
    ///
    /// A saved view loads at 1 and so draws its names 1.3 times the size it
    /// was saved at, for the reason [`marking_scale`](Self::marking_scale)
    /// gives.
    #[serde(default = "default_one")]
    pub note_name_scale: f32,

    // ---- Spectrogram ------------------------------------------------
    // A frequency-vs-time heatmap of the analyzed audio, drawn in the
    // roll's depth region on the roll's own time axis — so each column of
    // spectral energy lines up with the notes that made it.
    /// Draw the spectrogram heatmap (over the roll's time window).
    /// `default_true`, not `default`, or every state blob saved before this
    /// field existed loads with the spectrogram off — contradicting the
    /// struct's own default, which is what a fresh install gets. A blob that
    /// really did turn it off carries `false` and still round-trips.
    #[serde(default = "default_true")]
    pub show_spectrogram: bool,
    /// The heatmap's color ramp — the only thing about it left to choose.
    ///
    /// Three more knobs belong here on the obvious reading and are absent on
    /// purpose, each because its neutral position is the one worth looking at.
    /// An overall opacity fades the heatmap out from under the notes, at the
    /// price of the scheme it shares with the curve (see `heatmap_mesh`); a
    /// contrast curve bends the level a palette is already chosen to spread
    /// evenly; and a private dB window lets the same bucket mean two things in
    /// one pane. The window is the Spectrum's Level, always: one range means
    /// "loud" is the same claim in the curve and in the heatmap, which is the
    /// whole reason they share [`loudness_db`].
    ///
    /// Their fields are gone from the blob too. That costs nothing on load —
    /// serde ignores keys it has no field for, which
    /// `a_persist_blob_carrying_a_since_removed_field_still_loads` pins — so a
    /// project saved with an opacity simply loads without one.
    #[serde(default)]
    pub spectrogram_color: SpectrogramColor,
}

pub(crate) fn default_one() -> f32 {
    1.0
}

/// Enough of an edge to hold a shape against a bright spectrogram cell,
/// little enough that it doesn't read as a second outline of its own.
pub(crate) fn default_keyline() -> f32 {
    0.3
}

/// The default pitch range is the analyzer's whole axis — the zoom starts
/// showing everything there is.
pub(crate) fn default_low_midi() -> f32 {
    harmonigraph_core::spectrum::SPECTRUM_MIN_MIDI
}

pub(crate) fn default_high_midi() -> f32 {
    harmonigraph_core::spectrum::SPECTRUM_MAX_MIDI
}

/// "This blob had no octave-numbered range", out of the domain the old
/// control could produce (-1..=9).
pub(crate) fn no_legacy_octave() -> i32 {
    i32::MIN
}

impl SpectrumConfig {
    /// Fold an older blob's octave-numbered pitch range into the continuous
    /// one. A pre-Hz blob carries no `low_midi`, so serde would hand it the
    /// full-axis default and silently throw away the zoom the user had set.
    pub(crate) fn migrate_legacy(&mut self) {
        let (low, high) = (self.legacy_low_octave, self.legacy_high_octave);
        (self.legacy_low_octave, self.legacy_high_octave) =
            (no_legacy_octave(), no_legacy_octave());
        if low != no_legacy_octave() && high != no_legacy_octave() {
            let midi = |octave: i32| harmonigraph_core::notes::octave_start_midi(octave) as f32;
            self.low_midi = midi(low);
            self.high_midi = midi(high);
        }
        // Then fit whatever came out to the axis the analyzer actually covers.
        // Every source of this pair can be off it: an octave pair reaches C-1
        // and C9, a blob written while the axis ran 16 Hz to 16.7 kHz carries
        // its old ends, and a hand-edited one can say anything. A range past
        // the axis draws a band with no buckets behind it; an inverted one
        // divides by zero in PitchScale.
        let (floor, ceil) = (default_low_midi(), default_high_midi());
        self.low_midi = self.low_midi.clamp(floor, ceil - PITCH_RANGE_MIN_SPAN);
        self.high_midi = self.high_midi.clamp(self.low_midi + PITCH_RANGE_MIN_SPAN, ceil);
        // And the same treatment for the two text scales, for the same reason
        // and against the same threat: their bars cannot produce a nonsense
        // value, a hand-edited blob can, and these multiply a FONT SIZE. A
        // non-finite one reaches egui as a glyph with no image — every marking
        // and every name silently gone, which reads as a broken plugin rather
        // than as a bad number — and a huge one asks its rasterizer for a
        // glyph wider than the texture atlas can hold.
        self.marking_scale = sane_scale(self.marking_scale);
        self.note_name_scale = sane_scale(self.note_name_scale);
    }
}

/// Closest the two ends of the ANALYZER's pitch range may come.
///
/// Two octaves, and it is the note names that set it. They scale in
/// proportion to the zoom (see `panes::spectral::name_zoom`), so how far the
/// range may be closed IS how large a name can get: the whole axis is ten
/// octaves, so a two-octave floor puts a name at five times its dialled size
/// and no more. It was one octave, from when the range was a pair of octave
/// numbers and nothing downstream cared how tight it got.
///
/// A range saved narrower than this widens to it on load
/// ([`SpectrumConfig::migrate_legacy`]), takes included — so a video rendered
/// from an old take renders at the wider range too.
///
/// Named for the analyzer alone, and read by nothing else. The Nodes tab's
/// colour range is a span of pitch as well and used to borrow this; it has
/// [`COLOR_RANGE_MIN_SPAN`] of its own now, because the reasoning above is
/// about the size of TYPE and says nothing whatever about how tightly a
/// gradient may be aimed.
pub(crate) const PITCH_RANGE_MIN_SPAN: f32 = 24.0;

/// A persisted text scale, fit to the range its bar offers.
///
/// Anything outside it — a hand-edited blob, a NaN out of a corrupt float —
/// becomes the size a fresh install draws at, which is the one value that is
/// certainly meant to be legible. See [`SCALE_BAR_RANGE`].
pub(crate) fn sane_scale(scale: f32) -> f32 {
    if scale.is_finite() {
        scale.clamp(*SCALE_BAR_RANGE.start(), *SCALE_BAR_RANGE.end())
    } else {
        1.0
    }
}

/// What every text-size bar offers, and so what a persisted scale is fit to.
/// One range for the three of them: they are the same control over three kinds
/// of text, and a reader comparing two of them should not have to check
/// whether they mean the same thing by 2.
pub const SCALE_BAR_RANGE: std::ops::RangeInclusive<f32> = 0.3..=3.0;

/// Closest the two ends of the Nodes tab's colour range may come: an octave,
/// which is where it sat while it shared [`PITCH_RANGE_MIN_SPAN`] and had no
/// reason to move when that one did.
pub(crate) const COLOR_RANGE_MIN_SPAN: f32 = 12.0;

/// How far the roll's time span may be taken, in seconds. Named because two
/// controls now set it — the Analyzer tab's Span bar and the drag across the
/// picture — and a gesture that clamped to its own idea of the limits would
/// push the bar past its own ends. The maximum is also what
/// [`AudioSpectrum::HISTORY_MAX_SECONDS`](crate::AudioSpectrum::HISTORY_MAX_SECONDS)
/// is sized to reach back to, so the heatmap can fill the widest span the axis
/// offers.
pub(crate) const ROLL_SECONDS_MIN: f32 = 1.0;
pub(crate) const ROLL_SECONDS_MAX: f32 = 600.0;

/// The level range's domain, in dB. The top is a full-scale sine, the
/// loudest thing a bucket can hold; the bottom is well under any noise
/// floor worth looking at.
pub(crate) const LEVEL_MIN_DB: f32 = -100.0;
pub(crate) const LEVEL_MAX_DB: f32 = 0.0;

/// Closest the two ends of the level range may come. A window narrower than
/// this is all edge and no picture — and, unclamped, a collapsed one divides
/// by zero in `loudness` and paints the NaN geometry egui panics on.
pub(crate) const LEVEL_RANGE_MIN_SPAN: f32 = 12.0;

/// Where the curve's window ends, and it is NOT full scale, because nothing
/// musical gets near full scale in one bucket.
///
/// Measured: a chord of six partials mixed to peak at -12 dBFS reads -23.8 dB
/// in its loudest bucket once the default -4.5 dB/oct tilt has taken its cut,
/// because a chord splits its power across its partials and one bucket only
/// ever holds a share of it. Against a full-scale ceiling that is 0.60 of the
/// pane, so the top two fifths of an analyzer are empty in normal use.
///
/// -20 puts the same chord at 0.90 and a quiet passage where the curve can
/// still be read. A full-scale sine now runs off the top, which is the right
/// trade: the pane is read against material, not against a test tone.
///
/// The bar still offers [`LEVEL_MAX_DB`], so 0 is one drag away.
pub(crate) fn default_ceiling_db() -> f32 {
    -20.0
}

pub(crate) fn default_true() -> bool {
    true
}

pub(crate) fn default_roll_fraction() -> f32 {
    0.55
}

pub(crate) fn default_roll_seconds() -> f32 {
    12.0
}

/// Thin: a note is a line through the spectrogram at its own pitch, not a
/// slab over it. At 0.3 semitones a semitone of pitch axis still separates
/// two neighbouring keys, which is what makes the roll readable when the
/// pitch range is zoomed out over the whole spectrum.
pub(crate) fn default_roll_thickness() -> f32 {
    0.3
}

/// The tilt settings offered, per analyzer convention (-1.5 dB/oct
/// increments; see [`SpectrumConfig::tilt`]).
pub const TILT_STEPS: [f32; 5] = [0.0, -1.5, -3.0, -4.5, -6.0];

/// The slope that flattens typical musical material — what the analyzer is
/// looked at through nearly all the time, so it is where it starts. Raw
/// power (0) buries everything above a couple of kHz.
pub(crate) fn default_tilt() -> f32 {
    -4.5
}

impl Default for SpectrumConfig {
    fn default() -> Self {
        SpectrumConfig {
            orientation: SpectralOrientation::Left,
            window: SpectrumWindow::Balanced,
            floor_db: -60.0,
            ceiling_db: default_ceiling_db(),
            smoothing: 0.55,
            tilt: default_tilt(),
            marking_scale: default_one(),
            keyline: default_keyline(),
            low_midi: default_low_midi(),
            high_midi: default_high_midi(),
            legacy_low_octave: no_legacy_octave(),
            legacy_high_octave: no_legacy_octave(),
            show_roll: true,
            roll_fraction: default_roll_fraction(),
            roll_seconds: default_roll_seconds(),
            roll_thickness: default_roll_thickness(),
            note_names: true,
            note_name_scale: default_one(),
            show_spectrogram: true,
            spectrogram_color: SpectrogramColor::default(),
        }
    }
}
