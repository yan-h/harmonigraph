//! The settings shared by the live picture, editor saves and recorded takes.

use crate::{RenderConfig, SpectrumConfig};
use harmonigraph_scene::{Camera, ViewConfig};

const APPEARANCE_VERSION: u32 = 1;

/// Owned directly by live state. Only save/capture/export boundaries snapshot
/// this document; drawing and analysis borrow the individual groups.
#[derive(Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct AppearanceDocument {
    version: u32,
    pub camera: Camera,
    pub view: ViewConfig,
    pub spectrum: SpectrumConfig,
    pub render: RenderConfig,
}

impl Default for AppearanceDocument {
    fn default() -> Self {
        Self {
            version: APPEARANCE_VERSION,
            camera: Camera::default(),
            view: ViewConfig::default(),
            spectrum: SpectrumConfig::default(),
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
        Self::parse_noting(serialized).map(|(document, _)| document)
    }

    /// [`parse`](Self::parse), also naming the path of each key the document
    /// carries that this build does not read, such as one a rename retired:
    /// the setting it held draws at its default, which an export says
    /// (#1927 B).
    pub fn parse_noting(serialized: &str) -> Result<(Self, Vec<String>), String> {
        let (document, unread) = crate::persist_keys::from_str_noting::<Self>(serialized)
            .map_err(|err| format!("appearance did not parse ({err})"))?;
        Ok((document.normalize()?, unread))
    }

    /// The line an export writes for a key [`parse_noting`](Self::parse_noting)
    /// named.
    pub fn unread_note(key: &str) -> String {
        format!(
            "the look holds {key}, which this build does not read; if it was renamed, \
             its setting draws at its default"
        )
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
        self.render.sanitize();
        Ok(self)
    }
}
