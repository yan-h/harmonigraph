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

/// What the Spectral pane's axis gridlines are labeled with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SpectrumLabels {
    /// A gridline at every C, labeled with Bitwig octave numbers.
    Notes,
    /// Gridlines on the analyzer-standard 1-2-5 series (20, 50, 100, ...
    /// 10k, 20k Hz).
    Frequency,
}

/// Which way the Spectral pane's pitch axis runs.
///
/// The pane is written once against an abstract (pitch, depth) plane and
/// mapped onto the screen at draw time, so every element — gridlines,
/// spectrum curve, piano roll — turns together.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SpectralOrientation {
    /// Follow the pane's shape: time runs along the LONG side (so a
    /// scrolling spectrogram gets the room), pitch across the short one.
    /// Reads correctly both under the lattice and beside it, no setting to
    /// change.
    #[default]
    Auto,
    /// "Across": time runs left(now)->right(past) along the pane, pitch
    /// climbs bottom->top, and the spectrum sits on the left, joined to the
    /// spectrogram. (Serialized name kept from when this meant pitch axis.)
    Horizontal,
    /// "Upright": time runs top(now)->bottom(past) down the pane, pitch runs
    /// left->right, and the spectrum sits on top, joined to the spectrogram.
    Vertical,
}

impl SpectralOrientation {
    /// Whether TIME (the spectrogram/roll axis) runs vertically down the
    /// pane, with pitch across it. Resolves [`Auto`](Self::Auto) to the
    /// pane's long side.
    pub(crate) fn is_time_vertical(self, rect: egui::Rect) -> bool {
        match self {
            SpectralOrientation::Auto => rect.height() > rect.width(),
            SpectralOrientation::Horizontal => false,
            SpectralOrientation::Vertical => true,
        }
    }
}

/// What colors a note in the piano roll.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum RollColor {
    /// The lattice's own channel colors, so a note is the same color here
    /// as the node it lit up.
    Channel,
    /// The pitch gradient every channel-9..13 voice uses, applied to all
    /// channels — reads as a single ramp low-to-high.
    Pitch,
    /// One flat accent color: the roll recedes and the lattice leads.
    Accent,
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
/// The plugin cannot render video itself — that is `lattice-offline`, a
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
    /// Path to the `lattice-offline` binary. Empty means the
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
            extra_args: "--size 1920x1080".into(),
            playhead: false,
            frame: RenderFrame::default(),
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
    /// The lattice's share of the frame, `0..1` (the rest is the spectral pane).
    #[serde(default = "default_frame_split")]
    pub split: f32,
    /// Lattice over spectral, rather than side by side.
    #[serde(default)]
    pub stacked: bool,
}

pub(crate) fn default_aspect_w() -> u32 {
    16
}
pub(crate) fn default_aspect_h() -> u32 {
    9
}
pub(crate) fn default_frame_split() -> f32 {
    0.68
}

impl Default for RenderFrame {
    fn default() -> Self {
        RenderFrame { aspect_w: 16, aspect_h: 9, split: 0.68, stacked: false }
    }
}

/// Everything the Spectral pane's display is configured by, edited in the
/// Spectrum settings tab and persisted with the UI state.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SpectrumConfig {
    /// Horizontal (pitch left-to-right) or vertical (pitch bottom-to-top),
    /// or Auto to follow the pane's shape; see [`SpectralOrientation`]. Those
    /// are the only orientations offered — the spectrum always stands up from
    /// its baseline with pitch ascending, so there is nothing to flip.
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
    /// Axis gridline labeling.
    #[serde(default = "default_labels")]
    pub labels: SpectrumLabels,
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
    // The played-note timeline (lattice-core's NoteRoll) drawn over the
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
    /// Points shaved off a released note's tail, so repeated notes at one
    /// pitch stay separate instead of merging into one bar.
    ///
    /// The tail only: a held note still reaches the now-line, and no onset
    /// ever moves off the moment it was played.
    #[serde(default = "default_roll_color")]
    pub roll_color: RollColor,
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
    /// The heatmap's color ramp.
    #[serde(default)]
    pub spectrogram_color: SpectrogramColor,
    /// Overall spectrogram opacity, so it can sit under the notes without
    /// swamping them. (For the heatmap alone, turn the note ribbons off with
    /// `show_roll`.)
    #[serde(default = "default_spectrogram_opacity")]
    pub spectrogram_opacity: f32,
    /// Give the heatmap its own dB window instead of sharing the curve's.
    ///
    /// Off — the default, and what the heatmap always did — means it reads
    /// `floor_db`/`ceiling_db` like the curve. They answer different
    /// questions, though: the curve wants a range that keeps peaks on the
    /// pane, the heatmap one that separates quiet detail from the background,
    /// and those rarely coincide. `serde(default)` is false, so an existing
    /// blob keeps the shared behaviour it was saved with.
    #[serde(default)]
    pub spectrogram_own_range: bool,
    /// The heatmap's own dB window, used only while `spectrogram_own_range`.
    #[serde(default = "default_spectrogram_floor_db")]
    pub spectrogram_floor_db: f32,
    #[serde(default = "default_ceiling_db")]
    pub spectrogram_ceiling_db: f32,
    /// Contrast curve on the heatmap's 0..1 level: 1 is linear, below 1 lifts
    /// quiet detail toward the bright end, above 1 pushes it into the dark.
    ///
    /// Separate from the dB window on purpose. Moving the floor DISCARDS
    /// everything under it; gamma keeps the whole range and only changes how
    /// it is spread, so background hiss can be pushed down without losing the
    /// quiet partials just above it.
    #[serde(default = "default_one")]
    pub spectrogram_gamma: f32,
}

pub(crate) fn default_spectrogram_floor_db() -> f32 {
    -60.0
}

pub(crate) fn default_one() -> f32 {
    1.0
}

pub(crate) fn default_spectrogram_opacity() -> f32 {
    0.85
}

/// Enough of an edge to hold a shape against a bright spectrogram cell,
/// little enough that it doesn't read as a second outline of its own.
pub(crate) fn default_keyline() -> f32 {
    0.3
}

/// The default pitch range is the analyzer's whole axis — the zoom starts
/// showing everything there is.
pub(crate) fn default_low_midi() -> f32 {
    lattice_core::spectrum::SPECTRUM_MIN_MIDI
}

pub(crate) fn default_high_midi() -> f32 {
    lattice_core::spectrum::SPECTRUM_MAX_MIDI
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
            let midi = |octave: i32| lattice_core::notes::octave_start_midi(octave) as f32;
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
/// [`AudioSpectrum::HISTORY_MAX_SECONDS`] is sized to reach back to, so the
/// heatmap can fill the widest span the axis offers.
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

/// A full-scale sine reads as full height, which is what the pane did before
/// the ceiling was adjustable at all.
pub(crate) fn default_ceiling_db() -> f32 {
    LEVEL_MAX_DB
}

pub(crate) fn default_true() -> bool {
    true
}

pub(crate) fn default_labels() -> SpectrumLabels {
    SpectrumLabels::Notes
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

/// A hairline of background between repeats — enough to read two taps as
/// two, little enough that a note's length is still its length.
pub(crate) fn default_roll_color() -> RollColor {
    RollColor::Channel
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
            orientation: SpectralOrientation::Horizontal,
            window: SpectrumWindow::Balanced,
            floor_db: -60.0,
            ceiling_db: default_ceiling_db(),
            smoothing: 0.55,
            tilt: default_tilt(),
            labels: SpectrumLabels::Notes,
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
            roll_color: default_roll_color(),
            note_names: true,
            note_name_scale: default_one(),
            show_spectrogram: true,
            spectrogram_color: SpectrogramColor::default(),
            spectrogram_opacity: default_spectrogram_opacity(),
            spectrogram_own_range: false,
            spectrogram_floor_db: default_spectrogram_floor_db(),
            spectrogram_ceiling_db: default_ceiling_db(),
            spectrogram_gamma: default_one(),
        }
    }
}
