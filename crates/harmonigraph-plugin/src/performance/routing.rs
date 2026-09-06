//! Saved pairing is configuration, never runtime authorization. Restoring two
//! identical UUIDs intentionally preserves their ambiguity in the registry.
use super::clock::Calibration;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedUuid(pub [u8; 16]);
impl Default for SavedUuid {
    fn default() -> Self {
        let mut bytes = [0; 16];
        getrandom::fill(&mut bytes).expect("off-audio session UUID entropy");
        bytes[6] = (bytes[6] & 15) | 0x40;
        bytes[8] = (bytes[8] & 63) | 0x80;
        Self(bytes)
    }
}
impl std::fmt::Display for SavedUuid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (i, byte) in self.0.iter().enumerate() {
            if matches!(i, 4 | 6 | 8 | 10) {
                f.write_str("-")?;
            }
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HubSetup {
    pub uuid: SavedUuid,
    pub calibration: Calibration,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SourceSetup {
    pub selected: Option<SavedUuid>,
    pub calibration: Calibration,
}
