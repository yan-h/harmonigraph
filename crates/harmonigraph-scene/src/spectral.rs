//! The lattice's AUDIO channel: what the analyzer says, and the one colour
//! scheme everything lit by it is painted in.
//!
//! Two colour schemes reach the picture, and this module is the whole of the
//! second one:
//!
//! - **MIDI** — [`ViewConfig::pitch_gradient`](crate::ViewConfig) spread over
//!   the Color range, read through
//!   [`pitch_lut_color`](crate::pitch_lut_color). A note's disc, its octave
//!   wedges, its melody and bass rings and its ribbon on the piano roll all
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
/// The floor is a fifth of an analyzer bucket (3.125¢). Below it a whole wedge
/// spans less than the grid's own step, so the arc is one bucket stretched
/// across it and the ring says nothing a single number could not.
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
    /// The node bodies and the octave band are lit from the analyzer rather
    /// than from the keys ([`ViewConfig::spectral_light`]), so they take their
    /// colour off [`lut`](Self::lut) at their own level instead of off the
    /// pitch ramp at their own pitch.
    ///
    /// A flag on the paint and not on the view, because it is what the SHADER
    /// has to know: the octave word carries a level either way, and which of
    /// the two schemes that level is painted in is the whole difference.
    pub lit: bool,
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
    /// The analyzer's own reading, bucket by bucket. All zeros where nothing
    /// has been measured, which the ring draws as the ramp's floor colour
    /// everywhere rather than as an empty annulus: the ring is a MEASUREMENT
    /// of a range, and a range with nothing in it is a reading, not a gap.
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
            lit: false,
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
        let (inner, outer) = if view.spectral_ring {
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
            lit: view.spectral_light,
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
            spectral_ring: true,
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

    /// The ring OFF is an empty annulus, which is how the toggle reaches the
    /// shader: one thing says whether the ring draws, so the flag and the
    /// picture cannot disagree.
    #[test]
    fn the_ring_off_is_an_empty_annulus() {
        let view = ViewConfig { spectral_ring: false, ..ringed(0.3, 0.5, 200.0) };
        let paint = SpectralPaint::new(&view, [Vec4::ZERO; PITCH_LUT_N]);
        assert!(!paint.ring_draws(), "the ring drew with the toggle off");
        assert_eq!((paint.inner, paint.outer), (0.0, 0.0));
        assert!(!SpectralPaint::silent().ring_draws(), "a silent scene drew a ring");
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
