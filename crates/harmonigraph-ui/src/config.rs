//! The display settings the UI persists: the Spectral pane's analyzer
//! configuration and the shared bar ranges it is edited through. Serde-facing
//! — [`SpectrumConfig`] carries a container-level `default`, so a key missing
//! from a blob comes back at the fresh-install value and costs only itself.
//!
//! The video render settings persist alongside these but live in
//! `harmonigraph-take`, because a take carries the frame it was composed at.

use harmonigraph_scene::Gradient;

/// The Spectral pane's analysis window length, picked in the Analyzer
/// settings section: longer windows resolve bass pitch more sharply but
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

/// A look the heatmap's gradient can be set to in one press: four named ramps,
/// each stated as the [`Gradient`] that draws it.
///
/// A starting point and NOT a mode. Nothing stores which one was pressed,
/// because a press writes six numbers a bar can then move, and a name that went
/// on claiming the picture after a drag would be naming a look that is not on
/// screen. What a preset is worth is therefore what it opens on, and each is
/// six numbers rather than a list of color stops for a reason worth stating,
/// since a stop list is the obvious way to write a palette and the one this
/// declines.
///
/// **A stop list interpolates in sRGB BYTES, which is not a perceptual space.**
/// Its grey ramp puts mid-grey at `L*` 54 where an even one puts 50, and its
/// colored ramps wander off the straight line between their own ends by as much
/// as a tenth of the black-to-white distance in Oklab — measured against these
/// four. A heatmap is read for its quiet detail, and an even ramp is what reads
/// that detail honestly, so evenness is the property worth having and a line
/// straight in `L*` is what has it.
///
/// What a preset therefore carries is a ramp's CHARACTER — where its arc
/// starts, how far round it runs, how bright it closes, how much color it holds
/// — and not a curve through hand-picked stops. Anything a stop list could say
/// that six knobs cannot is a wander, and the wander is the part not worth
/// keeping.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpectrogramPreset {
    /// Black to white with no color at all — the classic, and the most neutral
    /// over the roll's own ribbons.
    Mono,
    /// Navy through blue to cyan. The cool ramp.
    Ice,
    /// Violet through teal and green to yellow (viridis-like) — what a fresh
    /// install opens on.
    Aurora,
    /// Indigo through magenta and orange to cream: [`Self::Aurora`]'s warm
    /// counterpart, over the other half of the circle.
    Magma,
}

impl SpectrogramPreset {
    /// Every preset, in the order the row offers them: the neutral one first,
    /// then the three colored ones.
    ///
    /// Built from an exhaustive `match` rather than written out as a literal,
    /// the same way [`SpectralOrientation::ALL`] is and for the same reason: a
    /// fifth look cannot reach the enum without reaching the row.
    pub const ALL: [SpectrogramPreset; 4] = {
        use SpectrogramPreset::*;
        const fn covered(p: SpectrogramPreset) {
            match p {
                Mono | Ice | Aurora | Magma => (),
            }
        }
        covered(Mono);
        [Mono, Ice, Aurora, Magma]
    };

    /// The name the button carries.
    pub fn label(self) -> &'static str {
        match self {
            SpectrogramPreset::Mono => "Mono",
            SpectrogramPreset::Ice => "Ice",
            SpectrogramPreset::Aurora => "Aurora",
            SpectrogramPreset::Magma => "Magma",
        }
    }

    /// What the button's tooltip says: the arc, in the colors it walks
    /// through. Not the six numbers — the bars below say those, and say them
    /// as the reader will next want to drag them.
    pub fn hint(self) -> &'static str {
        match self {
            SpectrogramPreset::Mono => "Black to white; the most neutral over the roll",
            SpectrogramPreset::Ice => "Navy to blue to cyan",
            SpectrogramPreset::Aurora => "Violet to teal to green to yellow",
            SpectrogramPreset::Magma => "Indigo to magenta to orange to cream",
        }
    }

    /// The six numbers the press writes.
    ///
    /// Each is stated as the two ENDS its bars read out — the `L*` and the
    /// chroma share that silence and a full bucket are drawn at — and composed
    /// into the middle-and-signed-ramp pair the gradient stores, so that what is
    /// written here is what the pane says back. Every one of them opens at `L*`
    /// 0, which is the heatmap's own requirement rather than a taste: the region
    /// is laid on a black bed, so silence has to BE black or the plane's edge
    /// shows.
    ///
    /// The chroma pair is where a fraction of the gamut earns its keep. It does
    /// not fall to 0 at the quiet end and does not need to: what the fraction is
    /// OF collapses toward 0 as `L*` approaches either end of its axis, so a
    /// constant share already draws black at the bottom, saturated in the middle
    /// and pale at the top — the shape all three colored ramps were hand-picked
    /// to have. See [`Gradient::chroma`].
    ///
    /// The shares are denominated in the floor every hue can hold rather than in
    /// each hue's own ceiling, which is a tighter axis and sits them high: each
    /// pair is the one holding ITS preset's mean colorfulness where the ceiling
    /// held it at the pair these were hand-dialled as (Ice 0.55..0.90, Aurora
    /// 0.40..0.85, Magma 0.70..0.85). A preset is its look, not its numbers, so
    /// the numbers move when the denominator does.
    pub fn gradient(self) -> Gradient {
        let of = |(l_lo, l_hi): (f32, f32), hue: (f32, f32), (c_lo, c_hi): (f32, f32)| Gradient {
            hue_start: hue.0,
            hue_span: hue.1,
            lightness: (l_lo + l_hi) * 0.5,
            lightness_ramp: l_hi - l_lo,
            chroma: (c_lo + c_hi) * 0.5,
            chroma_ramp: c_hi - c_lo,
        };
        match self {
            // The hue is unreachable at chroma 0 and is written as the one the
            // colored presets are read against anyway, so that a Mono picture
            // dragged off 0 opens on a color rather than on whatever angle 0
            // happened to be spelled with.
            SpectrogramPreset::Mono => of((0.0, 100.0), (269.0, 0.0), (0.0, 0.0)),
            SpectrogramPreset::Ice => of((0.0, 92.0), (269.0, -70.0), (0.635, 0.985)),
            SpectrogramPreset::Aurora => of((0.0, 88.0), (302.0, -193.0), (0.518, 0.968)),
            SpectrogramPreset::Magma => of((0.0, 90.0), (295.0, 136.0), (0.819, 0.969)),
        }
    }
}


/// Everything the Spectral pane's display is configured by, edited in the
/// Display tab's Analyzer section and persisted with the UI state.
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
    ///
    /// This end has a second control, as the Span does: dragging the spectrum
    /// along the depth axis zooms the window about the floor
    /// (`panes::spectral::gestures::drag_zoom`). The floor has only the bar,
    /// there being one depth axis to drag along and the floor being the end
    /// pinned to the baseline.
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
    /// Two numbers rather than one, exactly as the lattice's gutter and gutter
    /// fade are two ([`harmonigraph_scene::ViewConfig::sevens_gutter_soft`]):
    /// tying the fade to the reach makes a wider outline always a blurrier one,
    /// and how far a note stands off its background is a different question
    /// from how sharply it does. They share one CONTROL — the Analyzer section's
    /// Outline bar, a handle at each — because both are distances from the
    /// note's edge, which makes them two points on one axis.
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
    /// The heatmap's color ramp: the same six-knob [`Gradient`] the lattice
    /// colors pitch through, spanning the analyzer's LEVEL instead — its bottom
    /// is what reads as silence and its top what reads as a full bucket, which
    /// is the Spectrum's own Level window and not a second one.
    ///
    /// One type for both, because a heatmap's palette and a pitch ramp are the
    /// same object asked of different axes, and the six knobs say more about a
    /// ramp than a list of four names can: how far round the circle it walks,
    /// how much brightness it spends on level, how much color, and which end
    /// carries each. [`SpectrogramPreset`] is where the four names went — a row
    /// of buttons that WRITE these six, rather than a mode the picture is in.
    ///
    /// Its bottom wants to sit at `L*` 0 and the presets all do, the heatmap
    /// being laid on a black bed (see `spectral_pane`): silence drawn at
    /// anything else puts a visible plane edge where the analyzer's history
    /// runs out. Nothing enforces it — a gradient that lifts the floor is a
    /// legal picture and an occasionally useful one, showing exactly how far
    /// back the history reaches.
    ///
    /// Three more knobs belong here on the obvious reading and are absent on
    /// purpose, each because its neutral position is the one worth looking at.
    /// An overall opacity fades the heatmap out from under the notes, at the
    /// price of the scheme it shares with the curve (see `heatmap_mesh`); a
    /// contrast curve bends the level a gradient is already even in; and a
    /// private dB window lets the same bucket mean two things in one pane. The
    /// window is the Spectrum's Level, always: one range means "loud" is the
    /// same claim in the curve and in the heatmap, which is the whole reason
    /// they share
    /// [`loudness_db`](crate::panes::spectral::axes::loudness_db).
    ///
    /// Their fields are gone from the blob too, and so is the
    /// `spectrogram_color` this replaces. That costs nothing on load — serde
    /// ignores keys it has no field for, which
    /// `a_persist_blob_carrying_a_since_removed_field_still_loads` pins — so a
    /// project saved with a palette name simply loads at the fresh gradient.
    pub spectrogram_gradient: Gradient,
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
        // The outline and its fade, which are ONE control (the Analyzer section's
        // Outline bar) over two numbers, and held to the same bound for the
        // same reason as the lattice's gutter pair
        // ([`ViewConfig::sanitize`](harmonigraph_scene::ViewConfig::sanitize)):
        // the fade is measured back from the reach, so a fade wider than its
        // reach has no place on the axis to draw a handle. It draws as a fade
        // over the whole reach either way — `roll.wgsl` floors it at the
        // note's own edge — so the clamp costs the picture nothing.
        //
        // The finite check is not redundant with the clamp beside it: a NaN
        // reach becomes the MAX of the fade's clamp, and `f32::clamp` asserts
        // `min <= max`, which a NaN fails — taking the editor down as the
        // project opens. The same trap the level pair below names.
        let fresh = SpectrumConfig::default();
        self.roll_outline =
            if self.roll_outline.is_finite() { self.roll_outline } else { fresh.roll_outline }
                .clamp(0.0, ROLL_OUTLINE_MAX);
        self.roll_outline_fade = if self.roll_outline_fade.is_finite() {
            self.roll_outline_fade
        } else {
            fresh.roll_outline_fade
        }
        .clamp(0.0, self.roll_outline);
        // The level pair, against the same threat and for the same reason.
        // `loudness_raw` already refuses a collapsed or inverted window, which
        // is what a `max` can answer; a NaN end it cannot, because NaN loses
        // every comparison and survives to divide the mapping into the NaN
        // geometry egui panics on. Repairing it here rather than there keeps
        // one number in the blob and one on the bar, and there are now two
        // controls writing this pair — the bar and the drag across the
        // spectrum — neither of which can produce either shape itself.
        self.floor_db = if self.floor_db.is_finite() { self.floor_db } else { LEVEL_MIN_DB };
        self.ceiling_db = if self.ceiling_db.is_finite() { self.ceiling_db } else { LEVEL_MAX_DB };
        self.floor_db = self.floor_db.clamp(LEVEL_MIN_DB, LEVEL_MAX_DB - LEVEL_RANGE_MIN_SPAN);
        self.ceiling_db =
            self.ceiling_db.clamp(self.floor_db + LEVEL_RANGE_MIN_SPAN, LEVEL_MAX_DB);
        // And the heatmap's gradient, which `ViewConfig::sanitize` does for the
        // lattice's one door over. Asked of the type rather than restated, so
        // the pane and the file agree about which gradients are legal.
        //
        // What this buys is NOT the picture, and the distinction is the whole
        // reason the line is easy to delete: both bars paint `sanitized()` and
        // read out what they paint, and `with_lut` sanitizes again at the table,
        // so an unrepaired blob already draws and reads out the repaired
        // gradient. What it buys is that the FILE stops holding a pair the
        // picture is not at — otherwise the two disagree indefinitely, since
        // nothing rewrites the field until someone drags that bar. CLAUDE.md is
        // the line: "The value on screen must still be the value the file
        // holds." `a_blob_with_a_nonsense_heatmap_gradient_loads_at_a_drawable_one`
        // is what fails without it.
        self.spectrogram_gradient = self.spectrogram_gradient.sanitized();
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
/// The analyzer's alone, and read by nothing else. Color & light's Color range
/// is the span the color gradient is spread over, and it
/// is bounded by [`COLOR_RANGE_MIN_SPAN`] rather than by this: the reasoning
/// above is about the size of TYPE and says nothing whatever about how tightly
/// a gradient may be aimed.
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

/// Closest the two ends of Color & light's Color range may come — the span the
/// color gradient covers, which is a different bar from the analyzer's window
/// on the same axis. An octave: a gradient aimed tighter than one has notes a
/// semitone apart at opposite ends of the spectrum, and nothing about the
/// analyzer's own floor ([`PITCH_RANGE_MIN_SPAN`], which is about how small
/// type may be drawn) says anything about that.
pub(crate) const COLOR_RANGE_MIN_SPAN: f32 = 12.0;

/// How far the roll's time span may be taken, in seconds. Named because two
/// controls now set it — the Analyzer section's Span bar and the drag across the
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
///
/// Two controls hold to it, as with the Span: the Analyzer section's Level bar and
/// the drag across the spectrum. A gesture clamping to its own idea of how
/// close the pair may come would push the bar past its own end.
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
            // The even ramp, which is the one that reads a heatmap's quiet
            // detail honestly — and now even in `L*` rather than in bytes.
            spectrogram_gradient: SpectrogramPreset::Aurora.gradient(),
        }
    }
}
