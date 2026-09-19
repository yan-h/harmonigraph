//! [`GlowCurve`] — the shape of the node glow's falloff.

use super::*;

/// The shape of the node glow's falloff through [`ViewConfig::glow_reach`].
/// The centre and the far edge stay fixed at 1 and 0 while one signed exponent
/// bends the curve between them without an interpolation join or an inflection.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
// A curve nested in a persisted view still has its own fallback, so a
// hand-edited blob missing its shape costs that field alone.
#[serde(default)]
pub struct GlowCurve {
    /// Signed exponent, [`GLOW_CURVE_SHAPE_MIN`]..=[`GLOW_CURVE_SHAPE_MAX`].
    /// Zero is linear; positive falls early and negative falls late.
    pub shape: f32,
}

impl GlowCurve {
    /// The glow level `p` of the way from its centre to its edge.
    ///
    /// `level = expm1(k(1 - p)) / expm1(k)`. Positive `k` is a fast
    /// exponential-like decay, negative `k` holds then falls, and zero is the
    /// straight line.
    pub fn sample(self, p: f32) -> f32 {
        let p = if p.is_finite() { p.clamp(0.0, 1.0) } else { 0.0 };
        if p >= 1.0 {
            return 0.0;
        }
        exponential_level(p, self.shape())
    }

    /// The finite, bounded shape consumed by the renderer.
    pub fn shape(self) -> f32 {
        self.sanitized().shape
    }

    /// Repair the value into the finite range the editor can produce.
    pub fn sanitized(mut self) -> Self {
        let fresh = GlowCurve::default();
        self.shape =
            finite_or(self.shape, fresh.shape).clamp(GLOW_CURVE_SHAPE_MIN, GLOW_CURVE_SHAPE_MAX);
        self
    }
}

/// The normalized exponential at `p`. The series is the same one the shader
/// uses where direct subtraction of two almost-equal exponentials loses the
/// bend to float cancellation.
fn exponential_level(p: f32, shape: f32) -> f32 {
    let remaining = 1.0 - p;
    if shape.abs() < 0.05 {
        let shape2 = shape * shape;
        return remaining * (1.0 - shape * p * 0.5 + shape2 * p * (2.0 * p - 1.0) / 12.0);
    }
    (shape * remaining).exp_m1() / shape.exp_m1()
}

impl Default for GlowCurve {
    fn default() -> Self {
        // An early falloff: the light spends most of its level near the node
        // and thins across the gaps, so the fields the wide reach overlaps
        // meet as a haze rather than a plateau. Captured from the DAW on
        // 2026-09-07; the late fall it replaced is a drag the other way.
        GlowCurve { shape: 1.238_095_3 }
    }
}
