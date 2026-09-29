//! Local pigment on MIDI slices; independent of atmosphere and animation.
use super::*;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum NoteMaterialStyle {
    #[default]
    Smooth,
    QuietPigment,
    BrokenTraces,
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct NoteMaterialConfig {
    pub style: NoteMaterialStyle,
    /// Radial displacement and fringe, 0..=1 (the selected study at 1).
    pub roughness: f32,
    /// Opacity of unplayed octave guides, 0..=1.
    pub quiet_visibility: f32,
    /// Trace half-width relative to the selected study, 0.25..=2.
    pub guide_width: f32,
}
impl Default for NoteMaterialConfig {
    fn default() -> Self {
        Self {
            style: NoteMaterialStyle::Smooth,
            roughness: 1.0,
            quiet_visibility: 1.0,
            guide_width: 1.0,
        }
    }
}
impl NoteMaterialConfig {
    pub fn sanitized(mut self) -> Self {
        let fresh = Self::default();
        self.roughness = finite_or(self.roughness, fresh.roughness).clamp(0.0, 1.0);
        self.quiet_visibility =
            finite_or(self.quiet_visibility, fresh.quiet_visibility).clamp(0.0, 1.0);
        self.guide_width = finite_or(self.guide_width, fresh.guide_width).clamp(0.25, 2.0);
        self
    }
    /// Conservative UV bound: displacement plus pigment bleed, mirrored in WGSL.
    pub fn fringe(self) -> f32 {
        if self.style == NoteMaterialStyle::Smooth {
            0.0
        } else {
            0.3 * self.roughness
        }
    }
}
