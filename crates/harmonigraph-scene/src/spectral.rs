//! The lattice's AUDIO channel: what the analyzer says, and the one colour
//! scheme everything lit by it is painted in.
//!
//! Two colour schemes reach the picture, and this module is the whole of the
//! second one:
//!
//! - **MIDI** — [`ViewConfig::pitch_gradient`](crate::ViewConfig) spread over
//!   the Color range, read through
//!   [`pitch_lut_color`](crate::pitch_lut_color). A note's disc, its octave
//!   wedges, its melody and bass marks and its ribbon on the piano roll all
//!   come off that one table, so a pitch is one colour wherever it is drawn.
//! - **FREQUENCY** — the analyzer's own ramp
//!   (`SpectrumConfig::spectrogram_gradient`) against its own Level window,
//!   read through [`gradient_color`](crate::gradient_color). A bucket of the
//!   spectrum curve, a cell of the spectrogram, a segment of the Spiral pane
//!   and every audio-lit element of the lattice come off THAT one table, so a
//!   loudness is one LIGHT wherever it is drawn — added over whatever ground
//!   the surface under it has.
//!
//! One light rather than one colour, because the two surfaces have different
//! grounds. The analyzer's panes are bedded on BLACK — a spectrogram cell at
//! silence has to be black or the plane's edge shows — so there the light and
//! the colour are the same thing and the picture is the gradient itself. The
//! ring is bedded on the LATTICE, so it reads a copy of that gradient whose
//! silent end is anchored on the node's own ground ([`ring_gradient`]): a ramp
//! opening at black punches a hole through a grey lattice at every node, which
//! is a picture of a gap where the table means silence. That ground is the
//! neutral grey of [`ViewConfig::ring_ground`] — one number the octave band
//! beside it stands on too, so the node's two rings read as empty in exactly
//! one colour, and one bar moves both.
//!
//! The scheme is chosen by what the element MEASURES, never by which pane it
//! is on: the lattice carries both at once (a node's body held by the keys,
//! its audio ring measured from the spectrum), and the two are told apart by
//! their colour as much as by their radius.
//!
//! That the lattice carries both AT ONCE is the shape of the whole audio
//! channel here rather than a happy accident. The MIDI picture — the node
//! bodies, the octave band, the melody and bass marks — is never lit from the
//! analyzer; the measurement gets a ring of its own inside the band, and
//! [`SpectralReading`] picks which of two readings fills it. So neither
//! picture can be mistaken for the other, and neither has to be given up to
//! see the other.
//!
//! Nothing in this crate reads audio, so [`SpectralPaint`] arrives already
//! measured — `harmonigraph-ui`'s `panes::spectral_fold` is what fills it, and
//! a scene derived without that pass carries [`SpectralPaint::silent`], which
//! paints nothing at all.

use glam::Vec4;
use harmonigraph_core::spectrum::{
    BINS_PER_SEMITONE, SPECTRUM_BINS, SPECTRUM_MAX_MIDI, SPECTRUM_MIN_MIDI,
};

use crate::{pitch_ramp_lut, Gradient, ViewConfig, PITCH_LUT_N};

/// The narrowest and widest a wedge of the audio ring may be dialled to span
/// ([`ViewConfig::spectral_ring_range`]), in cents.
///
/// The ceiling is one octave, and it is the setting at which the ring stops
/// being a set of zoomed segments and becomes ONE continuous reading: a wedge
/// stands for exactly the octave it names, so neighbouring wedges meet at the
/// pitch they share and the whole ring is the spectrum bent round the wheel's
/// own pitch map. Past that the wedges would overlap in pitch — two arcs of
/// one node claiming the same frequency — which is a picture that cannot be
/// read as a position.
///
/// The floor is a sixth of an analyzer bucket (3.125¢). Any wedge narrower
/// than a bucket is one bucket stretched across its arc, saying nothing a
/// single number could not — so the floor is not where the picture sharpens,
/// only what keeps a hand-edited zero from collapsing every wedge to its
/// slot's own flat reading.
pub const SPECTRAL_RANGE_MIN: f32 = 0.5;
/// See [`SPECTRAL_RANGE_MIN`].
pub const SPECTRAL_RANGE_MAX: f32 = 1200.0;

/// Buckets the analyzer's pitch grid holds, and how many of them a semitone
/// spans — `harmonigraph_core::spectrum`'s own numbers, named again here
/// because `harmonigraph-render` sizes its uniform row and walks the grid by
/// them and does not depend on that crate. Aliases, not second values.
pub const SPECTRAL_BUCKETS: usize = SPECTRUM_BINS;
/// See [`SPECTRAL_BUCKETS`].
pub const SPECTRAL_BUCKETS_PER_SEMITONE: usize = BINS_PER_SEMITONE;

/// Which reading of the analyzer the audio ring carries — the one control that
/// says what the lattice's spectrum indicator IS.
///
/// The ring is where a measurement of the sound goes, and the MIDI picture —
/// the node bodies, the octave band, the melody and bass marks — is never any
/// of it. So this is a question about the RING and not about the lattice: it
/// picks which of two readings fills the annulus, and the picture around it is
/// unchanged either way.
///
/// It carries no Off, and that is the point of it: whether the ring is drawn is
/// its WIDTH ([`ViewConfig::spectral_ring_width`]), the same off switch every
/// other layer of a node has and in the same place. An Off here would be a
/// second one for this layer alone, and every reader would then have to know
/// which of the two wins.
///
/// The two are one measurement asked at two zooms, which is what makes them a
/// choice rather than a pair of features. Each wedge of the ring names one
/// octave of the node's pitch class, and:
///
/// - [`Fold`](Self::Fold) answers with ONE number for that octave — is this
///   pitch class sounding here — so a wedge is flat, and what the ring says is
///   read across the lattice rather than within a node.
/// - [`Spectrum`](Self::Spectrum) spreads a window of pitch across the wedge —
///   what is sounding NEAR that octave, and how far off it sits — so a wedge
///   reads as a detuning, and what the ring says is read within one node.
///
/// One choice and not two boxes, because they cannot both fill one annulus and
/// a second annulus is not what either is worth: they answer the same question
/// about the same stretch of sound, and which answer is wanted is a decision
/// about how closely the music is being looked at.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum SpectralReading {
    /// One level per wedge, measured at that octave's own pitch: the FOLD — a
    /// Gaussian kernel of [`ViewConfig::spectral_width`](crate::ViewConfig)
    /// cents over a local noise floor, so what survives is energy concentrated
    /// AT the node's pitch rather than energy near it.
    ///
    /// The lattice's native reading. A partial sits at an exact rational
    /// multiple of its fundamental, so folded to pitch class the first sixteen
    /// harmonics occupy six 7-limit nodes and a timbre draws as a
    /// CONSTELLATION anchored at its fundamental — a shape made of many nodes,
    /// which needs each of them to answer with one number per octave for the
    /// shape to be legible at all.
    #[default]
    Fold,
    /// A window of the RAW spectrum across each wedge, spanning
    /// [`ViewConfig::spectral_ring_range`](crate::ViewConfig) cents centred on
    /// that octave's own pitch: a segment of the spiral spectrogram, bent into
    /// the wedge's arc.
    ///
    /// No kernel and no floor, and that is the design rather than an omission:
    /// a wedge shows a window of PITCH, and every smoothing that helps the
    /// fold is a blur across the one axis the window exists to resolve. A
    /// partial dead on the node paints down the middle of its wedge and one a
    /// comma sharp paints to the clockwise side, in the direction pitch rises
    /// everywhere else on the wheel.
    ///
    /// What it costs is stated rather than hidden: with nothing sounding a
    /// wedge is not empty, it is the ramp's silent end — the node's own ground,
    /// which the ring's ramp is anchored on (see [`ring_gradient`]) — and every
    /// node in the window wears one. A stretch of spectrum with nothing in it
    /// is a reading, not a gap.
    Spectrum,
}

/// The analyzer's loudness at every bucket of its pitch grid, quantized to a
/// byte apiece.
///
/// A byte because that is the grain the rest of the analyzer's picture is
/// already drawn at — the spectrogram stores its columns as dB bytes and the
/// octave packing sends a level as one — and because the whole grid then fits
/// in the lattice's uniform buffer beside the tables already there. Over a
/// 60 dB Level window a step is 0.24 dB, well under what a gradient shows.
pub type SpectralLevels = [u8; SPECTRUM_BINS];

/// What the analyzer says and how the lattice paints it — the audio channel,
/// whole.
///
/// One struct rather than a scatter of fields because the parts are only ever
/// right TOGETHER: a ramp with no levels behind it paints silence in the
/// analyzer's colours, and levels with no ramp paint a measurement in the
/// pitch ramp's, which is the exact confusion the two schemes exist to
/// prevent. [`silent`](Self::silent) is the one state where none of it is
/// read.
pub struct SpectralPaint {
    /// The FREQUENCY scheme's ramp — `SpectrumConfig::spectrogram_gradient`
    /// through [`pitch_ramp_lut`], the same gradient
    /// the spectrogram's cells, the spectrum curve and the Spiral pane's
    /// segments are read off — bedded on the LATTICE rather than on their
    /// black plane. Every audio-lit element on the lattice indexes it by its
    /// own LEVEL, exactly as those do; what differs is the ground each entry
    /// sits on, which [`ring_gradient`] anchors the ramp to before
    /// [`new`](Self::new) bakes the table.
    pub lut: [Vec4; PITCH_LUT_N],
    /// Each wedge of the ring is ONE reading taken at its own octave's pitch
    /// ([`SpectralReading::Fold`]) rather than a window of pitch spread across
    /// it ([`SpectralReading::Spectrum`]).
    ///
    /// A flag on the paint and not the reading itself, because it is the whole
    /// of what the SHADER has to know: [`levels`](Self::levels) carries a grid
    /// either way — folded here, raw there — and where in the wedge that grid
    /// is sampled is the entire difference between the two pictures.
    pub folded: bool,
    /// The audio ring's inner and outer radius in quad UV units, already
    /// clamped to a drawable span. **Both 0 when the ring is off** — an empty
    /// annulus is the one thing that says the ring is not drawn, so the toggle
    /// reaches the picture as geometry and nothing downstream needs the flag
    /// as well.
    pub inner: f32,
    pub outer: f32,
    /// How many cents of the spectrum one wedge of the ring spans, centred on
    /// that wedge's own octave ([`ViewConfig::spectral_ring_range`]). Already
    /// clamped into [`SPECTRAL_RANGE_MIN`]..=[`SPECTRAL_RANGE_MAX`].
    pub range: f32,
    /// The reading the ring paints, bucket by bucket of the analyzer's own
    /// pitch grid — the raw spectrum or the fold over it, whichever
    /// [`folded`](Self::folded) says, both already through the analyzer's Level
    /// window.
    ///
    /// ONE grid for the whole lattice, which is what makes the ring cost the
    /// same whatever the extents are: a node's wedges are a window onto this
    /// table, and which part of it each of them reads is the shader's
    /// arithmetic off the node's own pitch class.
    ///
    /// All zeros where nothing has been measured, which the ring draws as the
    /// ramp's floor — the bed it is anchored on — everywhere rather than as an
    /// empty annulus: the ring is a MEASUREMENT of a range, and a range with
    /// nothing in it is a reading, not a gap.
    pub levels: Box<SpectralLevels>,
}

impl SpectralPaint {
    /// No analyzer: no ring, nothing measured into the grid it reads, and a
    /// ramp nothing indexes.
    ///
    /// Zeros and not a bedded ramp, unlike [`new`](Self::new)'s table, because
    /// the annulus here is empty and the shader never reaches it — building a
    /// table nothing samples would only make a scene with no analyzer look
    /// like one whose ring is at silence.
    ///
    /// What [`derive_scene`](crate::derive_scene) answers, and the right
    /// answer for every shell that draws a lattice without opening an
    /// analyzer.
    pub fn silent() -> SpectralPaint {
        SpectralPaint {
            lut: [Vec4::ZERO; PITCH_LUT_N],
            folded: false,
            inner: 0.0,
            outer: 0.0,
            range: SPECTRAL_RANGE_MAX,
            levels: Box::new([0; SPECTRUM_BINS]),
        }
    }

    /// The paint `view` asks for, its table baked from the analyzer's
    /// `gradient` anchored on the node's own ground, with no levels measured
    /// into it yet.
    ///
    /// The GRADIENT and not the analyzer's baked table, because the anchoring
    /// is a move of the ramp's two ENDS ([`ring_gradient`]) rather than a blend
    /// over the entries it produced: the knobs are what carries it, so this is
    /// the one place they are read for the ring and the one place its table is
    /// built. The ground the ends are moved onto comes off `view`, the same
    /// field the octave band's own ground does.
    ///
    /// Where the ring sits is [`ViewConfig::rings`]'s answer rather than a
    /// second reading of the same bars, because the ring's inner edge is a sum
    /// over the layers INSIDE it: the core's radius, this ring's own gap, and
    /// whether either is dialled to nothing. That sum belongs in one place, and
    /// its clamps are there rather than in `ViewConfig::sanitize` for the
    /// reason every other geometry clamp is — the drawing code is reached by
    /// more routes than the persist door (a take replay, the offline
    /// renderer's layout, a standalone harness), and a hand-edited view must
    /// still come out as a node somebody can see.
    pub fn new(view: &ViewConfig, gradient: Gradient) -> SpectralPaint {
        let (inner, outer) = view.rings().audio;
        SpectralPaint {
            lut: pitch_ramp_lut(ring_gradient(gradient, view.ring_ground_lightness())),
            folded: view.spectral_reading == SpectralReading::Fold,
            inner,
            outer,
            range: clamp_or(
                view.spectral_ring_range,
                SPECTRAL_RANGE_MAX,
                SPECTRAL_RANGE_MIN,
                SPECTRAL_RANGE_MAX,
            ),
            levels: Box::new([0; SPECTRUM_BINS]),
        }
    }

    /// Whether the ring draws at all: an annulus with something in it.
    pub fn ring_draws(&self) -> bool {
        self.outer > self.inner
    }
}

/// The analyzer's gradient as the RING reads it: the same arc, with its silent
/// end moved onto the ground the ring is drawn on — `ground` as an `L*`, and no
/// chroma at all.
///
/// The FREQUENCY scheme's invariant is that a loudness is one LIGHT wherever it
/// is drawn, over whatever ground that surface has — not one colour, which is
/// only the same thing where the ground is black. The analyzer's own panes are
/// that case: the spectrogram's plane IS black (silence there has to be black
/// or the plane's edge shows), so the gradient reaches them untouched and their
/// picture is the gradient itself. The ring's ground is the LATTICE, so its
/// copy of the ramp is re-anchored to stand on it — a ramp opening at black
/// punches a hole through a grey lattice at every node in the window, which is
/// a picture of a gap where the table means silence.
///
/// A move of the LIGHTNESS and not a blend over the baked table, because the
/// lightness is the one axis the ground is a statement about. Screening the
/// analyzer's colours over the ground instead lifts every channel by
/// `ground * (1 - c)`, which washes the middle of the ramp toward the ground's
/// own grey and never quite lets go at the top. At any ground the re-anchor is
/// the closer of the two to the analyzer's mid colours, and it is EXACT at the
/// loud end where a screen is not: measured over the well grey — the darkest
/// rung of the chrome's ladder, and so the one a screen flatters most — the two
/// miss the analyzer's own colours by 10/7/0 and 13/11/10 per channel at `t`
/// 0.25/0.5/1.0. The hue knob is untouched at every level, so a loud wedge and
/// a loud spectrogram cell are one colour rather than two that nearly agree.
///
/// **Both ends of the CHROMA ramp move too, and that is what makes the ground a
/// ground**: the silent end is pinned to chroma 0, so what a quiet wedge draws
/// is the neutral grey of [`ViewConfig::ring_ground`] and not the analyzer
/// gradient's own dark hue at that brightness. The MIDI ring beside it stands
/// on that same grey, and two rings a gap apart whose empty state differed by a
/// tint would read as two measurements in different units when the only
/// difference is which one is empty. The cost is real and is stated rather than
/// hidden: the ramp now opens neutral, so a wedge in the low mids carries less
/// colour than the spectrogram cell of the same loudness — most at chroma ramp
/// 0, where the analyzer's own table is flat in colour and the ring's is not.
/// The loud end is untouched at every setting, which is where the two pictures
/// are actually compared.
///
/// A PIN and not a floor, on both axes. The ground is where the ring starts
/// rather than merely what it must not sink under: a gradient whose own dark
/// end sits above it would otherwise draw a silent wedge in a colour that is
/// not the ground, and the two rings would agree only for some settings of a
/// gradient that knows nothing about either of them.
///
/// Each pair is stored as a middle plus a SIGNED ramp, so the ends are
/// recomposed rather than set, and `t` = 0 is the bright end wherever a ramp
/// runs downward — the pin lands on whichever end `t` = 0 carries, since that
/// is the end a silent wedge draws.
pub fn ring_gradient(gradient: Gradient, ground: f32) -> Gradient {
    // Sanitized first, so the ends are read off the gradient that will actually
    // be drawn — and so a gradient already standing on this ground comes back
    // as the very key `pitch_ramp_lut` would memoize it under.
    let gradient = gradient.sanitized();
    let ground = ground.clamp(0.0, 100.0);
    // Recomposed off the LOUD end: it is the end that must not move, and for a
    // top at or above the ground the subtraction is exact in f32 (Sterbenz), so
    // `lightness ± ramp/2` lands the `t` = 1 end back on exactly the lightness
    // it had and spends the rounding at the end being moved anyway.
    let top = gradient.lightness + gradient.lightness_ramp * 0.5;
    let lightness = (ground + top) * 0.5;
    let top_chroma = gradient.chroma + gradient.chroma_ramp * 0.5;
    let chroma = top_chroma * 0.5;
    Gradient {
        lightness,
        lightness_ramp: (top - lightness) * 2.0,
        chroma,
        // The full ramp its own middle leaves room for, which is exactly what a
        // silent end of 0 asks for — so this reaches `sanitized` at its bound
        // rather than past it, and comes back unflattened.
        chroma_ramp: top_chroma,
        ..gradient
    }
}

/// `value` held inside `low..=high`, or `fallback` where it is not a number at
/// all. `clamp` hands a NaN straight back — every comparison against one is
/// false — so a blob's NaN would otherwise walk through as a radius.
fn clamp_or(value: f32, fallback: f32, low: f32, high: f32) -> f32 {
    if value.is_finite() {
        value.clamp(low, high)
    } else {
        fallback
    }
}

/// The MIDI pitch bucket `bucket` of the analyzer's grid is centred on.
///
/// The grid is absolute log pitch at [`BINS_PER_SEMITONE`] buckets a semitone,
/// and a bucket stands for its own CENTRE rather than its low edge — the half
/// step is what keeps a partial's pitch and the bucket that carries it
/// agreeing to better than a bucket, which at 3.125¢ is the whole resolution
/// the ring has to place a partial with.
pub fn bucket_pitch(bucket: usize) -> f32 {
    SPECTRUM_MIN_MIDI + (bucket as f32 + 0.5) / BINS_PER_SEMITONE as f32
}

/// The pitch axis the analyzer reaches, as the ring reads it: 20 Hz to 20 kHz.
pub const SPECTRAL_AXIS: (f32, f32) = (SPECTRUM_MIN_MIDI, SPECTRUM_MAX_MIDI);

#[cfg(test)]
mod tests {
    use super::*;

    fn ringed(width: f32, range: f32) -> ViewConfig {
        ViewConfig {
            spectral_reading: SpectralReading::Spectrum,
            spectral_ring_width: width,
            spectral_ring_range: range,
            ..ViewConfig::default()
        }
    }

    /// The ring's width and its range reach the picture drawable, whatever a
    /// hand-edited blob holds — a NaN (which walks through a `clamp`
    /// untouched), an infinity, a negative width, a value off either end of
    /// its bar.
    ///
    /// A non-finite width is the one case that answers with NO ring, and
    /// deliberately: 0 is the width bar's own off position, so reading a NaN
    /// as it draws the picture some setting could have asked for. Every finite
    /// width above zero has to come out as an annulus somebody can see — the
    /// alternative is a ring that silently is not there while the bar reads out
    /// a number.
    #[test]
    fn a_hand_edited_audio_ring_still_draws_an_annulus() {
        for (width, range) in [
            (0.2, f32::NAN),
            (0.3, 200.0),
            (9.0, 1e9),
            (0.05, -80.0),
            (f32::INFINITY, 0.0),
            (f32::NAN, 200.0),
            (-3.0, 200.0),
        ] {
            let paint = SpectralPaint::new(&ringed(width, range), Gradient::default());
            assert_eq!(
                paint.ring_draws(),
                width.is_finite() && width > 0.0,
                "a width of {width} reached the picture as ({}, {})",
                paint.inner,
                paint.outer,
            );
            assert!(paint.outer <= 1.0, "the ring reaches past the node at {}", paint.outer);
            assert!(
                (SPECTRAL_RANGE_MIN..=SPECTRAL_RANGE_MAX).contains(&paint.range),
                "a range of {range} reached the picture as {}",
                paint.range,
            );
        }
    }

    /// A width of 0 is an empty annulus, which is how the ring's one off switch
    /// reaches the shader: the setting IS the geometry, so the two cannot
    /// disagree, and the reading beside it says nothing about whether there is
    /// a ring.
    #[test]
    fn a_ring_of_no_width_is_an_empty_annulus() {
        for reading in [SpectralReading::Fold, SpectralReading::Spectrum] {
            let view = ViewConfig { spectral_reading: reading, ..ringed(0.0, 200.0) };
            let paint = SpectralPaint::new(&view, Gradient::default());
            assert!(!paint.ring_draws(), "{reading:?} drew a ring with no width");
            assert_eq!((paint.inner, paint.outer), (0.0, 0.0));
        }
        assert!(!SpectralPaint::silent().ring_draws(), "a silent scene drew a ring");
    }

    /// The ring sits where the STACK puts it: a gap out from the core, and
    /// moved by every layer inside it rather than by a radius of its own.
    ///
    /// The whole of what the width bars buy, checked at the one place a second
    /// copy of the sum would drift — the audio channel is built from
    /// `ViewConfig` on its own, without the scene the octave band comes out of.
    #[test]
    fn the_ring_sits_a_gap_out_from_the_core() {
        let view = ringed(0.25, 200.0);
        let paint = SpectralPaint::new(&view, Gradient::default());
        assert_eq!(paint.inner, view.core_radius + view.ring_gap);
        assert_eq!(paint.outer, paint.inner + 0.25);

        // The core off, and the ring reaches the node's center: no layer to
        // stand off, so no gap is spent on one.
        let bare = SpectralPaint::new(
            &ViewConfig { core_radius: 0.0, ..view.clone() },
            Gradient::default(),
        );
        assert_eq!((bare.inner, bare.outer), (0.0, 0.25));
    }

    /// BOTH readings fill the same annulus, at the same radii, and are told
    /// apart by nothing but where in a wedge the grid is sampled.
    ///
    /// The whole shape of the selector, stated where the flag is set: a
    /// version that gave the fold its own geometry — a different ring, or the
    /// band relit — would pass every other test here and quietly be two
    /// features again rather than one control over one indicator.
    #[test]
    fn the_two_readings_share_one_annulus() {
        let raw = ringed(0.2, 200.0);
        let fold = ViewConfig { spectral_reading: SpectralReading::Fold, ..raw };
        let g = Gradient::default();
        let (fold, raw) = (SpectralPaint::new(&fold, g), SpectralPaint::new(&raw, g));
        assert!(fold.ring_draws() && raw.ring_draws(), "a reading drew no ring");
        assert_eq!(
            (fold.inner, fold.outer),
            (raw.inner, raw.outer),
            "the two readings drew at different radii",
        );
        assert!(fold.folded, "the fold is not read at its wedges' own pitches");
        assert!(!raw.folded, "the raw spectrum lost its window across the wedge");
    }

    /// The shape every analyzer preset has: a ramp whose bottom is BLACK,
    /// because the spectrogram's plane is black and silence there has to be
    /// black. Carries chroma at both ends, which is what makes the ring's
    /// silent end a question rather than a shade of grey by default.
    fn analyzers() -> Gradient {
        Gradient {
            hue_start: 302.0,
            hue_span: -188.0,
            lightness: 50.0,
            lightness_ramp: 100.0,
            chroma: 0.615,
            chroma_ramp: 0.77,
        }
    }

    /// The `L*` a drawn entry carries, off the crate's own curve — the units
    /// the ramp is authored in, and the only ones a claim about the ground
    /// means anything in.
    fn lightness(c: Vec4) -> f32 {
        crate::color::lightness_of_encoded(f64::from(c.x), f64::from(c.y), f64::from(c.z)) as f32
    }

    /// A silent wedge draws the GROUND: the neutral grey
    /// [`ViewConfig::ring_ground`] names, to the byte.
    ///
    /// The whole claim of the pin, and it is stated against the octave band's
    /// own ground rather than against a number written here — the two rings sit
    /// a gap apart on one node and the ask is that their empty state is one
    /// colour, so what this holds is the two ends of that, not a shade.
    ///
    /// A silent wedge draws it deliberately — a reading, not a gap — so most of
    /// the ring is this entry most of the time. Unanchored (the analyzer's own
    /// table, whose bottom is black for the spectrogram's black plane) it is a
    /// hole punched through the lattice at every node in the window.
    #[test]
    fn a_silent_wedge_is_the_grey_the_octave_band_stands_on() {
        for ground in [0.0, 8.8, 20.0, 55.0, 100.0] {
            let view = ViewConfig { ring_ground: ground, ..ringed(0.3, 200.0) };
            let silent = SpectralPaint::new(&view, analyzers()).lut[0];
            let band = crate::grey_of_lightness(view.ring_ground_lightness());
            let step = (silent.truncate() - band.truncate()).abs().max_element();
            assert!(
                step * 255.0 < 0.5,
                "at Ground {ground} the audio ring's silence is {silent:?} \
                 and the octave band's is {band:?} — over half a byte apart",
            );
            assert_eq!(silent.w, 1.0, "the ring's ramp lost its alpha");
        }
    }

    /// The fresh Ground is the skin's own `surface_faint` rung, to under a fifth
    /// of an `L*`.
    ///
    /// The one thing tying the default to the chrome now that the ring reads a
    /// number instead of the skin: with the tie in code the two moved together
    /// and could not be dialled apart, and with no tie at all retuning the
    /// chrome's ladder would leave the lattice standing on the rung the ladder
    /// used to have. This is the tie a bar can still be dragged off.
    #[test]
    fn the_fresh_ground_is_the_skins_faint_surface() {
        let rung = lightness(crate::skin::surface_faint_color());
        let fresh = ViewConfig::default().ring_ground;
        assert!(
            (fresh - rung).abs() < 0.2,
            "the fresh Ground is L* {fresh}, the skin's faint surface L* {rung}",
        );
    }

    /// A gradient whose own dark end ALREADY sits above the ground is still
    /// moved onto it.
    ///
    /// A pin and not a floor, and this is the difference. The ground is where
    /// the ring starts rather than what it must not sink under: left to open
    /// bright, a silent wedge would draw a colour that is not the ground, and
    /// the two rings would agree only for some settings of a gradient that
    /// knows about neither of them. A floor would pass every other test here.
    #[test]
    fn a_ramp_opening_above_the_ground_is_still_moved_onto_it() {
        let high = Gradient { lightness: 60.0, lightness_ramp: 20.0, ..analyzers() };
        let ground = ViewConfig::default().ring_ground;
        assert!(
            lightness(pitch_ramp_lut(high)[0]) > ground,
            "the test's own gradient does not open above the ground",
        );
        let opened = lightness(pitch_ramp_lut(ring_gradient(high, ground))[0]);
        assert!(
            (opened - ground).abs() < 0.2,
            "a ramp opening bright was left at L* {opened} rather than the ground's {ground}",
        );
    }

    /// The top of the ring's ramp is the analyzer's own colour EXACTLY, so the
    /// ring and the spectrogram read as ONE measurement where a reading is
    /// actually being looked at.
    ///
    /// Where the re-anchor converges is what makes it a re-anchoring rather
    /// than a second colour scheme: it moves the bottom of the lightness range
    /// and leaves the top where it stands, so the deviation runs to nothing at
    /// the loud end. A screen over the bed never quite lets go — it moves every
    /// entry by `bed * (1 - c)` per channel, which is only zero at white — and
    /// a lerp toward the bed would miss by more the brighter the reading got.
    #[test]
    fn a_loud_reading_is_the_analyzers_own_colour_exactly() {
        let analyzer = pitch_ramp_lut(analyzers());
        let paint = SpectralPaint::new(&ringed(0.3, 200.0), analyzers());
        assert_eq!(
            paint.lut[PITCH_LUT_N - 1],
            analyzer[PITCH_LUT_N - 1],
            "the ramp's top is no longer the analyzer's own colour",
        );
        assert_ne!(
            paint.lut[0], analyzer[0],
            "the ring's floor is the analyzer's black, so nothing was anchored at all",
        );
    }

    /// A bucket stands for its own centre, and the grid covers the axis the
    /// analyzer claims with no bucket falling outside it.
    #[test]
    fn the_grid_is_centred_on_the_axis_it_spans() {
        assert!(bucket_pitch(0) > SPECTRAL_AXIS.0, "the first bucket sits below the axis");
        assert!(
            bucket_pitch(SPECTRUM_BINS - 1) > SPECTRAL_AXIS.1 - 1.0,
            "the grid stops {} short of the top of the axis",
            SPECTRAL_AXIS.1 - bucket_pitch(SPECTRUM_BINS - 1),
        );
        // One semitone is exactly BINS_PER_SEMITONE buckets apart, which is
        // what makes a cents offset a bucket offset at any pitch.
        let step = bucket_pitch(BINS_PER_SEMITONE) - bucket_pitch(0);
        assert!((step - 1.0).abs() < 1e-4, "a semitone spans {step} of the grid, not 1");
    }
}
