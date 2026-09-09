//! The settings shared by the live picture, editor saves and recorded takes.

use crate::{panes::spiral::SpiralView, RenderConfig, SpectrumConfig};
use harmonigraph_scene::{Camera, ViewConfig};

const APPEARANCE_VERSION: u32 = 1;

/// Owned directly by live state. Only save/capture/export boundaries snapshot
/// this document; drawing and analysis borrow the individual groups.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct AppearanceDocument {
    version: u32,
    pub camera: Camera,
    pub view: ViewConfig,
    pub spectrum: SpectrumConfig,
    pub spiral: SpiralView,
    pub render: RenderConfig,
}

impl Default for AppearanceDocument {
    fn default() -> Self {
        Self {
            version: APPEARANCE_VERSION,
            camera: Camera::default(),
            view: ViewConfig::default(),
            spectrum: SpectrumConfig::default(),
            spiral: SpiralView::default(),
            render: RenderConfig::default(),
        }
    }
}

impl AppearanceDocument {
    pub fn serialize(&self) -> String {
        ron::to_string(self).expect("appearance settings serialize to RON")
    }

    /// One parse and normalization at the export boundary, before choosing the
    /// output configuration or initializing the picture.
    pub fn parse(serialized: &str) -> Result<Self, String> {
        ron::from_str::<Self>(serialized)
            .map_err(|err| format!("appearance did not parse ({err})"))?
            .normalize()
    }

    /// Also used on the typed document nested inside an editor save.
    pub fn normalize(mut self) -> Result<Self, String> {
        if self.version != APPEARANCE_VERSION {
            return Err(format!(
                "appearance version {} is unsupported (expected {APPEARANCE_VERSION})",
                self.version
            ));
        }
        self.camera.sanitize();
        self.view.sanitize();
        self.spectrum.sanitize();
        self.spiral.sanitize();
        self.render.sanitize();
        Ok(self)
    }
}
