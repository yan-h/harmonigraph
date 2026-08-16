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
//!   loudness is one colour wherever it is drawn.
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

use crate::{ViewConfig, PITCH_LUT_N, SPECTRAL_RING_MIN_SPAN};

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
    /// No ring at all: the picture is the MIDI one, whole.
    #[default]
    Off,
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
    /// wedge is not empty, it is the ramp's floor colour, and every node in
    /// the window wears one. A stretch of spectrum with nothing in it is a
    /// reading, not a gap.
    Spectrum,
}

impl SpectralReading {
    /// Whether the ring is asked for at all — anything but [`Off`](Self::Off).
    pub fn draws(self) -> bool {
        self != SpectralReading::Off
    }
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
    /// The FREQUENCY scheme's ramp: `SpectrumConfig::spectrogram_gradient`
    /// through [`pitch_ramp_lut`](crate::pitch_ramp_lut), which is the table
    /// the spectrogram's cells, the spectrum curve and the Spiral pane's
    /// segments are already read off. Every audio-lit element on the lattice
    /// indexes it by its own LEVEL, exactly as those do.
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
    /// ramp's floor colour everywhere rather than as an empty annulus: the
    /// ring is a MEASUREMENT of a range, and a range with nothing in it is a
    /// reading, not a gap.
    pub levels: Box<SpectralLevels>,
}

impl SpectralPaint {
    /// No analyzer: nothing audio-lit, no ring, and a ramp nothing indexes.
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

    /// The paint `view` asks for, against the analyzer's `lut`, with no
    /// levels measured into it yet.
    ///
    /// The clamps live here and not in `ViewConfig::sanitize`, for the reason
    /// every other geometry clamp does: the drawing code is reached by more
    /// routes than the persist door — a take replay, the offline renderer's
    /// layout, a standalone harness — and a hand-edited or inverted pair must
    /// still come out as an annulus somebody can see rather than as a ring
    /// that silently is not there while the bar reads out a number.
    pub fn new(view: &ViewConfig, lut: [Vec4; PITCH_LUT_N]) -> SpectralPaint {
        let (inner, outer) = if view.spectral_reading.draws() {
            let inner = clamp_or(view.spectral_ring_inner, 0.0, 0.0, 1.0 - SPECTRAL_RING_MIN_SPAN);
            (
                inner,
                clamp_or(view.spectral_ring_outer, 1.0, inner + SPECTRAL_RING_MIN_SPAN, 1.0),
            )
        } else {
            (0.0, 0.0)
        };
        SpectralPaint {
            lut,
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

    fn ringed(inner: f32, outer: f32, range: f32) -> ViewConfig {
        ViewConfig {
            spectral_reading: SpectralReading::Spectrum,
            spectral_ring_inner: inner,
            spectral_ring_outer: outer,
            spectral_ring_range: range,
            ..ViewConfig::default()
        }
    }

    /// The ring's radii and its range reach the picture drawable, whatever a
    /// hand-edited blob holds — a NaN (which walks through a `clamp`
    /// untouched), an infinity, a pair dialled inside out, a pair off both
    /// ends of the range.
    ///
    /// The alternative to each is a ring that silently is not there, or one
    /// whose wedges span a pitch window of nothing, while the bar reads out a
    /// number.
    #[test]
    fn a_hand_edited_audio_ring_still_draws_an_annulus() {
        for (inner, outer, range) in [
            (f32::NAN, 0.4, 200.0),
            (0.3, f32::NAN, 200.0),
            (f32::INFINITY, f32::NEG_INFINITY, f32::NAN),
            (0.6, 0.2, -80.0),
            (-3.0, 9.0, 1e9),
            (0.98, 0.99, 0.0),
        ] {
            let paint = SpectralPaint::new(&ringed(inner, outer, range), [Vec4::ZERO; PITCH_LUT_N]);
            assert!(
                paint.outer - paint.inner >= SPECTRAL_RING_MIN_SPAN - 1e-6,
                "({inner}, {outer}) reached the picture as ({}, {})",
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

    /// The reading OFF is an empty annulus, which is how the selector reaches
    /// the shader: one thing says whether the ring draws, so the setting and
    /// the picture cannot disagree.
    #[test]
    fn the_ring_off_is_an_empty_annulus() {
        let view = ViewConfig { spectral_reading: SpectralReading::Off, ..ringed(0.3, 0.5, 200.0) };
        let paint = SpectralPaint::new(&view, [Vec4::ZERO; PITCH_LUT_N]);
        assert!(!paint.ring_draws(), "the ring drew with the reading Off");
        assert_eq!((paint.inner, paint.outer), (0.0, 0.0));
        assert!(!SpectralPaint::silent().ring_draws(), "a silent scene drew a ring");
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
        let raw = ringed(0.3, 0.5, 200.0);
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
