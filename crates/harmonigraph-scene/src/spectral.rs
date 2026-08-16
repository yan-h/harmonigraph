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
//! beds. The analyzer's panes are bedded on BLACK — a spectrogram cell at
//! silence has to be black or the plane's edge shows — so there the light and
//! the colour are the same thing and the picture is the gradient itself. The
//! ring is bedded on the LATTICE, so it reads the same gradient screened onto
//! the lattice's ground ([`SpectralPaint::new`]): a black floor would punch a
//! hole through a grey lattice at every node, which is a picture of a gap
//! where the table means silence. The bed is the skin's WELL grey and not its
//! panel grey, because a ring bedded on the panel would vanish into it, and an
//! invisible ring is what a ring dialled to no width IS — the off switch
//! ([`ViewConfig::spectral_ring_width`]); recessed, a silent ring is a groove
//! that is plainly still a reading.
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

use crate::{ViewConfig, PITCH_LUT_N};

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
    /// wedge is not empty, it is the ramp's floor — the lattice's own well
    /// grey, the ramp bedded there (see [`SpectralPaint::new`]) — and every
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
    /// through [`pitch_ramp_lut`](crate::pitch_ramp_lut), the same gradient
    /// the spectrogram's cells, the spectrum curve and the Spiral pane's
    /// segments are read off — bedded on the LATTICE rather than on their
    /// black plane. Every audio-lit element on the lattice indexes it by its
    /// own LEVEL, exactly as those do; what differs is the ground each entry
    /// is added over, which [`new`](Self::new) screens in.
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
    /// ramp's floor — the lattice's own bed — everywhere rather than as an
    /// empty annulus: the ring is a MEASUREMENT of a range, and a range with
    /// nothing in it is a reading, not a gap.
    pub levels: Box<SpectralLevels>,
}

impl SpectralPaint {
    /// No analyzer: no ring, nothing measured into the grid it reads, and a
    /// ramp nothing indexes.
    ///
    /// Zeros and not a bed, unlike [`new`](Self::new)'s table, because the
    /// annulus here is empty and the shader never reaches the ramp — bedding
    /// a table nothing samples would only make a scene with no analyzer look
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

    /// The paint `view` asks for, against the analyzer's `lut` re-bedded onto
    /// the lattice's own ground, with no levels measured into it yet.
    ///
    /// The re-bed is [`rebed`], and this is the one place it happens: `lut`
    /// arrives as the analyzer's gradient, which opens at black because the
    /// spectrogram's bed IS black, and the ring's bed is the lattice.
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
    pub fn new(view: &ViewConfig, lut: [Vec4; PITCH_LUT_N]) -> SpectralPaint {
        let (inner, outer) = view.rings().audio;
        SpectralPaint {
            lut: rebed(lut, crate::skin::well_color()),
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

/// The analyzer's ramp screened over `bed`: `c' = bed + c * (1 - bed)` per
/// channel, alpha untouched.
///
/// The FREQUENCY scheme's invariant is that a loudness is one LIGHT wherever
/// it is drawn, ADDED over whatever ground that surface has — not one colour,
/// which is only the same thing where the ground is black. The analyzer's own
/// panes are that case: the spectrogram's plane is black (silence has to BE
/// black or the plane's edge shows), so this is the identity there and their
/// picture is the gradient itself. The ring's ground is the lattice, so its
/// copy of the table is the same light over the lattice's own bed.
///
/// A screen and not a lerp, because the two ends have to hold. At level 0 the
/// entry lands exactly on `bed` — the ring's silence sits ON the surface
/// instead of a black hole punched through it — and at the bright end the
/// deviation from the analyzer's colour is `bed * (1 - c)`, which for the well
/// grey is under 6% at any level and nothing at white. So the ring and the
/// spectrogram still read as one measurement wherever both are on screen.
///
/// sRGB-ENCODED throughout, straight off `skin::ground_color`, because that is
/// the space the shader composites in (see [`crate::skin`]) — a blend in
/// linear light would agree with nothing else in the picture.
fn rebed(lut: [Vec4; PITCH_LUT_N], bed: Vec4) -> [Vec4; PITCH_LUT_N] {
    lut.map(|c| {
        Vec4::new(
            bed.x + c.x * (1.0 - bed.x),
            bed.y + c.y * (1.0 - bed.y),
            bed.z + c.z * (1.0 - bed.z),
            c.w,
        )
    })
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
            let paint = SpectralPaint::new(&ringed(width, range), [Vec4::ZERO; PITCH_LUT_N]);
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
            let paint = SpectralPaint::new(&view, [Vec4::ZERO; PITCH_LUT_N]);
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
        let paint = SpectralPaint::new(&view, [Vec4::ZERO; PITCH_LUT_N]);
        assert_eq!(paint.inner, view.core_radius + view.ring_gap);
        assert_eq!(paint.outer, paint.inner + 0.25);

        // The core off, and the ring reaches the node's center: no layer to
        // stand off, so no gap is spent on one.
        let bare = SpectralPaint::new(
            &ViewConfig { core_radius: 0.0, ..view.clone() },
            [Vec4::ZERO; PITCH_LUT_N],
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
        let lut = [Vec4::ZERO; PITCH_LUT_N];
        let (fold, raw) = (SpectralPaint::new(&fold, lut), SpectralPaint::new(&raw, lut));
        assert!(fold.ring_draws() && raw.ring_draws(), "a reading drew no ring");
        assert_eq!(
            (fold.inner, fold.outer),
            (raw.inner, raw.outer),
            "the two readings drew at different radii",
        );
        assert!(fold.folded, "the fold is not read at its wedges' own pitches");
        assert!(!raw.folded, "the raw spectrum lost its window across the wedge");
    }

    /// A ramp opening at black comes out of [`SpectralPaint::new`] opening at
    /// the lattice's well grey, exactly.
    ///
    /// Every analyzer preset opens at black, because the spectrogram's plane
    /// is black and silence there has to BE black. The ring's plane is the
    /// lattice, and a silent wedge draws the floor deliberately — a reading,
    /// not a gap — so most of the ring is this entry most of the time. Handed
    /// through unbedded it is a black hole punched through the lattice at
    /// every node in the window.
    #[test]
    fn the_rings_silence_sits_on_the_lattice_rather_than_under_it() {
        let mut lut = [Vec4::new(0.4, 0.5, 0.6, 1.0); PITCH_LUT_N];
        lut[0] = Vec4::new(0.0, 0.0, 0.0, 1.0);
        let paint = SpectralPaint::new(&ringed(0.3, 200.0), lut);
        let bed = crate::skin::well_color();
        let floor = paint.lut[0];
        assert!(
            (floor.truncate() - bed.truncate()).length() < 1e-6,
            "a silent wedge draws {floor:?}, not the lattice's own bed {bed:?}",
        );
        assert_eq!(floor.w, 1.0, "the re-bed took the entry's alpha with it");
    }

    /// A bright reading is the analyzer's own colour to within the bed, so the
    /// ring and the spectrogram still read as ONE measurement wherever both
    /// are on screen.
    ///
    /// The bound is what makes the re-bed a re-bedding rather than a second
    /// colour scheme: a screen moves an entry by `bed * (1 - c)` per channel,
    /// which is the whole bed at black and nothing at white. A lerp toward the
    /// bed, or a bed anywhere near the ramp's bright end, would fail here
    /// while still looking plausible at silence.
    #[test]
    fn a_bright_reading_still_matches_the_analyzers_own_colour() {
        let dark = Vec4::new(0.05, 0.02, 0.08, 1.0);
        let mid = Vec4::new(0.5, 0.35, 0.2, 1.0);
        let mut lut = [mid; PITCH_LUT_N];
        lut[0] = dark;
        lut[PITCH_LUT_N - 1] = Vec4::ONE;
        let paint = SpectralPaint::new(&ringed(0.3, 200.0), lut);
        let bed = crate::skin::well_color();
        // White is the one entry the bed cannot move at all, which is what
        // makes the bound tightest exactly where the ring is brightest.
        assert_eq!(
            paint.lut[PITCH_LUT_N - 1],
            Vec4::ONE,
            "the ramp's top is no longer the analyzer's white",
        );
        for (name, bedded, raw) in [("dark", paint.lut[0], dark), ("mid", paint.lut[7], mid)] {
            for ch in 0..3 {
                let delta = (bedded[ch] - raw[ch]).abs();
                let allowed = bed[ch] * (1.0 - raw[ch]);
                assert!(
                    delta <= allowed + 1e-6,
                    "the {name} entry's channel {ch} moved by {delta}, past the bed's {allowed}",
                );
            }
        }
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
