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

    /// [`parse`](Self::parse), also naming what an export says about the
    /// document (#1927 B, #1956 Q2): each key it carries that this build does
    /// not read, such as one a rename retired, and each setting whose value
    /// [`normalize`](Self::normalize) repaired, such as one past a range this
    /// build narrowed.
    ///
    /// A repair is found by comparing the document as read with the document
    /// as normalized, value by value, so a setting needs nothing here to be
    /// named. An enum variant serializes as a bare name the comparison cannot
    /// tell apart, but no `sanitize` repairs one.
    pub fn parse_noting(serialized: &str) -> Result<(Self, LoadNotes), String> {
        let (document, unread) = crate::persist_keys::from_str_noting::<Self>(serialized)
            .map_err(|err| format!("appearance did not parse ({err})"))?;
        let held = document.values()?;
        let document = document.normalize()?;
        let mut repaired = Vec::new();
        repairs("", &held, &document.values()?, &mut repaired);
        Ok((document, LoadNotes { unread, repaired }))
    }

    /// The document as a tree of plain values, for [`repairs`].
    fn values(&self) -> Result<ron::Value, String> {
        ron::from_str(&self.serialize())
            .map_err(|err| format!("appearance did not read back ({err})"))
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

/// What [`AppearanceDocument::parse_noting`] found to say about a document.
#[derive(Debug, Default)]
pub struct LoadNotes {
    /// The path of each key the document carries that this build does not
    /// read.
    pub unread: Vec<String>,
    /// Each setting whose value the load repaired.
    pub repaired: Vec<Repair>,
}

/// One setting a load repaired: its path, as [`LoadNotes::unread`] spells
/// one, and its value as held and as drawn, in RON.
#[derive(Debug)]
pub struct Repair {
    pub path: String,
    pub held: String,
    pub drawn: String,
}

impl LoadNotes {
    /// The line an export writes for each, unread keys first.
    pub fn lines(&self) -> impl Iterator<Item = String> + '_ {
        let unread = self.unread.iter().map(|key| {
            format!(
                "the look holds {key}, which this build does not read; if it was renamed, \
                 its setting draws at its default"
            )
        });
        let repaired = self.repaired.iter().map(|Repair { path, held, drawn }| {
            format!("the look holds {path} at {held}, which this build does not draw; it draws at {drawn}")
        });
        unread.chain(repaired)
    }
}

/// Each path under `path` whose value differs between `held` and `drawn`, two
/// readings of one document and so of one shape: a struct is walked by its
/// keys, a list by its items, and a `Some` passed through as
/// `persist_keys::dotted` passes it. The comparison is exact, so a NaN made a
/// number is a difference and a NaN left one is not.
fn repairs(path: &str, held: &ron::Value, drawn: &ron::Value, out: &mut Vec<Repair>) {
    use ron::Value;
    let at = |segment: &dyn std::fmt::Display| match path {
        "" => segment.to_string(),
        _ => format!("{path}.{segment}"),
    };
    match (held, drawn) {
        (Value::Map(held), Value::Map(drawn)) => {
            for (key, held) in held.iter() {
                if let (Value::String(name), Some(drawn)) = (key, drawn.get(key)) {
                    repairs(&at(name), held, drawn, out);
                }
            }
        }
        (Value::Seq(held), Value::Seq(drawn)) => {
            for (index, (held, drawn)) in held.iter().zip(drawn).enumerate() {
                repairs(&at(&index), held, drawn, out);
            }
        }
        (Value::Option(Some(held)), Value::Option(Some(drawn))) => repairs(path, held, drawn, out),
        _ if held != drawn => {
            let show = |value: &Value| ron::to_string(value).unwrap_or_default();
            out.push(Repair { path: path.to_owned(), held: show(held), drawn: show(drawn) });
        }
        _ => {}
    }
}
