//! The display settings the UI persists: the Spectral pane's analyzer
//! configuration and the shared bar ranges it is edited through. Serde-facing
//! — [`SpectrumConfig`] carries a container-level `default`, so a key missing
//! from a blob comes back at the fresh-install value and costs only itself.
//!
//! The video render settings persist alongside these but live in
//! `harmonigraph-take`, because a take carries the frame it was composed at.

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
/// mapped onto the screen at draw time, so every element — axis markings,
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
    #[default]
    Left,
    /// Time runs right(now)->left(past); pitch climbs bottom->top, and the
    /// spectrum sits on the right.
    Right,
    /// Time runs top(now)->bottom(past) down the pane, pitch runs left->right,
    /// and the spectrum sits on top, joined to the spectrogram.
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
    Magma,
}


/// Everything the Spectral pane's display is configured by, edited in the
/// Spectrum settings tab and persisted with the UI state.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct SpectrumConfig {
    /// Which side of the pane the now-line sits on, and so which way time
    /// runs; see [`SpectralOrientation`]. Four sides and no more — the pitch
    /// axis reads the conventional way in each, so there is nothing left to
    /// flip.
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
    /// One of the fields the struct's container-level `default` earns its
    /// keep on: a blob missing the key loads with the slope a fresh install
    /// gets, where a bare `f32` default would hand it raw-power 0.
    pub tilt: f32,
    /// Overall size of the pane's own markings — the frequency labels along the
    /// axis and the pitch readout that follows the pointer — as a multiple of
    /// their built-in sizes.
    ///
    /// Fixed against the zoom, unlike the note names: a marking says what the
    /// axis is, and the axis is the one thing on the pane that does not change
    /// size when the range is zoomed.
    ///
    /// A saved view loads at 1 and so draws its markings TWICE the size it was
    /// saved at, the built-in size having been rebased by 2 (see
    /// `panes::spectral::axes::MARKING_PT`). Deliberate, and the same call as the
    /// lattice's: 10pt was the wrong number rather than one of several, and a
    /// blob that kept it would be preserving a mistake.
    pub marking_scale: f32,
    /// Strength of the light edge drawn along the spectrum's profile, 0 = none.
    /// The profile's alone — a note's edge is the outline the roll's own pair of
    /// settings draws (see [`roll_outline`](Self::roll_outline)). See
    /// `panes::spectral::roll::keyline`.
    pub keyline: f32,
    /// Displayed pitch range, as (fractional) MIDI note numbers. The
    /// analyzer always covers `SPECTRUM_MIN_MIDI..=SPECTRUM_MAX_MIDI`
    /// (~16 Hz to ~16.7 kHz); this only zooms the view.
    ///
    /// MIDI rather than Hz because the axis is linear in MIDI note, which
    /// makes this both the number the pane wants and — since a semitone is a
    /// constant frequency RATIO — a logarithmic frequency scale. The control
    /// drags it linearly and reads it out in Hz.
    pub low_midi: f32,
    pub high_midi: f32,

    // ---- Piano roll -------------------------------------------------
    // The played-note timeline (harmonigraph-core's NoteRoll) drawn over the
    // same pitch axis, occupying the far end of the depth axis. Time runs
    // away from the spectrum: a note leaving the roll's near edge meets
    // the spectrum peak it is making.
    /// Draw the incoming MIDI's history at all.
    pub show_roll: bool,
    /// Share of the pane's depth given to the roll (the rest is the
    /// spectrum). 0 hides it; 1 gives the whole pane to the roll. Set by
    /// dragging the divider in the Spectral pane itself
    /// (`panes::spectral::gestures::drag_split`) — there is no bar for it.
    pub roll_fraction: f32,
    /// Seconds of history the roll's depth spans.
    pub roll_seconds: f32,
    /// Note ribbon width, in semitones of the pitch axis. This IS the note's
    /// painted width — a note is a solid rectangle of its own color, with
    /// nothing straddling its boundary.
    pub roll_thickness: f32,
    /// How far the dark outline around a note reaches past its edge, in POINTS,
    /// 0 = no outline. It wraps every side, so a note is one bounded object
    /// against the spectrogram rather than a ribbon with edged flanks.
    ///
    /// In points rather than semitones, unlike the ribbon it wraps: an edge is
    /// there to be seen at all zooms, and one measured in semitones would thin
    /// out as the pitch range opened — exactly where a picture full of notes
    /// needs its edges most. What that costs is at the wide end, where the
    /// ribbon floors at `MIN_RIBBON_PX` and a wide outline reaches over the
    /// neighbouring semitone.
    pub roll_outline: f32,
    /// How much of that reach the outline spends fading out, in points: 0 is a
    /// hard edge, and at or past the reach it fades over the whole of it.
    ///
    /// Two settings rather than one, exactly as the lattice's gutter and gutter
    /// fade are two ([`harmonigraph_scene::ViewConfig::sevens_gutter_soft`]):
    /// tying the fade to the reach makes a wider outline always a blurrier one,
    /// and how far a note stands off its background is a different question
    /// from how sharply it does.
    pub roll_outline_fade: f32,
    /// Write each note's name over its ribbon, at the moment it was struck —
    /// see [`panes::spectral::names`](crate::panes::spectral::names).
    pub note_names: bool,
    /// Overall size of those names, as a multiple of their built-in size.
    ///
    /// Rides on top of the pitch zoom, which already grows a name as the range
    /// narrows so that it keeps its footing on the ribbon it is written on —
    /// see `panes::spectral::axes::name_zoom`. This says how big it is at the zoom
    /// you are at.
    ///
    /// A saved view loads at 1 and so draws its names 1.3 times the size it
    /// was saved at, for the reason [`marking_scale`](Self::marking_scale)
    /// gives.
    pub note_name_scale: f32,

    // ---- Spectrogram ------------------------------------------------
    // A frequency-vs-time heatmap of the analyzed audio, drawn in the
    // roll's depth region on the roll's own time axis — so each column of
    // spectral energy lines up with the notes that made it.
    /// Draw the spectrogram heatmap (over the roll's time window). A blob
    /// missing the key loads with it ON, as a fresh install has it, rather
    /// than the `false` a bare `bool` default would mean; one that really did
    /// turn it off carries `false` and still round-trips.
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
    /// whole reason they share
    /// [`loudness_db`](crate::panes::spectral::axes::loudness_db).
    ///
    /// Their fields are gone from the blob too. That costs nothing on load —
    /// serde ignores keys it has no field for, which
    /// `a_persist_blob_carrying_a_since_removed_field_still_loads` pins — so a
    /// project saved with an opacity simply loads without one.
    pub spectrogram_color: SpectrogramColor,
}

impl SpectrumConfig {
    /// Fit a deserialized config to the axes and ranges its controls can
    /// actually produce.
    ///
    /// The pitch pair can be off the axis from more than one direction: a blob
    /// written while the axis ran 16 Hz to 16.7 kHz carries its old ends, and
    /// a hand-edited one can say anything. A range past the axis draws a band
    /// with no buckets behind it; an inverted one divides by zero in
    /// PitchScale.
    pub(crate) fn sanitize(&mut self) {
        use harmonigraph_core::spectrum::{SPECTRUM_MAX_MIDI, SPECTRUM_MIN_MIDI};
        let (floor, ceil) = (SPECTRUM_MIN_MIDI, SPECTRUM_MAX_MIDI);
        // The clamps below do not catch a non-finite end: NaN is its own
        // answer to every comparison, so it survives its own clamp and then
        // becomes the MIN of the next one, where `f32::clamp`'s
        // `assert!(min <= max)` takes the editor down as the project opens.
        // A NaN at the other end does not panic — it is the `self` of its
        // clamp rather than the bound — and is worse for it: the range stays
        // NaN into `PitchScale` and the analyzer draws nothing, silently.
        // The design range is the pair certain to be drawable, which is the
        // answer `sane_scale` gives the text scales below against the same
        // threat, from the same place: a hand-edited blob, or a float that
        // came back corrupt.
        self.low_midi = if self.low_midi.is_finite() { self.low_midi } else { floor };
        self.high_midi = if self.high_midi.is_finite() { self.high_midi } else { ceil };
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
/// proportion to the zoom (see `panes::spectral::axes::name_zoom`), so how far the
/// range may be closed IS how large a name can get: the whole axis is ten
/// octaves, so a two-octave floor puts a name at five times its dialled size
/// and no more. It was one octave, from when the range was a pair of octave
/// numbers and nothing downstream cared how tight it got.
///
/// A range saved narrower than this widens to it on load
/// ([`SpectrumConfig::sanitize`]), takes included — so a video rendered
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

/// How far a note's outline may be taken, and how much of it may be fade — one
/// number for both bars, since a fade past the reach it softens does nothing.
/// Four points is a heavy surround at the ribbon widths this pane is used at:
/// past it the outlines of two neighbouring semitones meet at any zoom worth
/// reading, and the picture is outline with ribbons in it.
pub(crate) const ROLL_OUTLINE_MAX: f32 = 4.0;

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
const DEFAULT_CEILING_DB: f32 = -20.0;

/// The tilt settings offered, per analyzer convention (-1.5 dB/oct
/// increments; see [`SpectrumConfig::tilt`]).
pub const TILT_STEPS: [f32; 5] = [0.0, -1.5, -3.0, -4.5, -6.0];

impl Default for SpectrumConfig {
    fn default() -> Self {
        SpectrumConfig {
            orientation: SpectralOrientation::Left,
            window: SpectrumWindow::Balanced,
            floor_db: -60.0,
            ceiling_db: DEFAULT_CEILING_DB,
            smoothing: 0.55,
            // The slope that flattens typical musical material — what the
            // analyzer is looked at through nearly all the time, so it is
            // where it starts. Raw power (0) buries everything above a
            // couple of kHz.
            tilt: -4.5,
            marking_scale: 1.0,
            // Enough of an edge to hold the profile against a bright
            // spectrogram cell, little enough that it doesn't read as a second
            // curve of its own.
            keyline: 0.3,
            // The pitch range starts as the analyzer's whole axis — the zoom
            // opens showing everything there is.
            low_midi: harmonigraph_core::spectrum::SPECTRUM_MIN_MIDI,
            high_midi: harmonigraph_core::spectrum::SPECTRUM_MAX_MIDI,
            show_roll: true,
            roll_fraction: 0.55,
            roll_seconds: 12.0,
            // Thin: a note is a line through the spectrogram at its own
            // pitch, not a slab over it. At 0.3 semitones a semitone of pitch
            // axis still separates two neighbouring keys, which is what makes
            // the roll readable when the pitch range is zoomed out over the
            // whole spectrum.
            roll_thickness: 0.3,
            // Two points of outline with most of it fading: enough of a dark
            // surround to separate a note from whatever the spectrogram is
            // doing behind it, soft enough that it reads as the note standing
            // off the picture rather than as a second shape drawn around it.
            roll_outline: 2.0,
            roll_outline_fade: 1.5,
            note_names: true,
            note_name_scale: 1.0,
            show_spectrogram: true,
            spectrogram_color: SpectrogramColor::default(),
        }
    }
}
