//! Musical map document and the editor/backend seam. This is project musical
//! state, independent of appearance and of the lifetime of an editor window.
use harmonigraph_core::lattice_map::{LatticeMap, TuningEngine};
use harmonigraph_core::LatticePos;
use serde::{Deserialize, Serialize};

pub const MAP_CAPACITY: usize = 128;
pub const MIDI_LABELS: [&str; 12] =
    ["C", "C♯", "D", "D♯", "E", "F", "F♯", "G", "G♯", "A", "A♯", "B"];

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct MapRecord {
    pub nodes: [[i32; 3]; 12],
    pub position: [i32; 3],
}
impl Default for MapRecord {
    fn default() -> Self {
        LatticeMap::default().into()
    }
}
fn coordinates(p: LatticePos) -> [i32; 3] {
    [p.threes, p.fives, p.sevens]
}
fn position(p: [i32; 3]) -> LatticePos {
    LatticePos::new(p[0], p[1], p[2])
}
impl From<LatticeMap> for MapRecord {
    fn from(map: LatticeMap) -> Self {
        Self { nodes: map.nodes.map(coordinates), position: coordinates(map.position) }
    }
}
impl MapRecord {
    pub fn resolve(&self) -> Option<LatticeMap> {
        let map = LatticeMap { nodes: self.nodes.map(position), position: position(self.position) };
        map.valid().then_some(map)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct NamedMap {
    pub name: String,
    pub geometry: MapRecord,
    pub deleted: bool,
}
impl Default for NamedMap {
    fn default() -> Self {
        Self { name: "C · 3×4".into(), geometry: Default::default(), deleted: false }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct MapDocument {
    /// Slot index is the host identity. Slots are never removed or recycled.
    pub slots: Vec<NamedMap>,
    pub order: Vec<usize>,
}
impl Default for MapDocument {
    fn default() -> Self {
        Self { slots: vec![NamedMap::default()], order: vec![0] }
    }
}
impl MapDocument {
    pub fn map(&self, id: usize) -> Option<LatticeMap> {
        self.slots.get(id).filter(|m| !m.deleted)?.geometry.resolve()
    }
    pub fn capture(&mut self, map: LatticeMap, name: String) -> Option<usize> {
        if self.slots.len() >= MAP_CAPACITY || !map.valid() {
            return None;
        }
        let id = self.slots.len();
        self.slots.push(NamedMap { name, geometry: map.into(), deleted: false });
        self.order.push(id);
        Some(id)
    }
    pub fn names(&self) -> Vec<(usize, String)> {
        // Invalid order entries from a hand-edited file cannot hide or alias slots.
        let mut ids: Vec<_> = self
            .order
            .iter()
            .copied()
            .filter(|&id| id < self.slots.len().min(MAP_CAPACITY))
            .collect();
        ids.extend(0..self.slots.len().min(MAP_CAPACITY));
        let mut seen = [false; MAP_CAPACITY];
        ids.into_iter()
            .filter_map(|id| {
                if std::mem::replace(&mut seen[id], true) || self.slots[id].deleted {
                    None
                } else {
                    Some((id, self.slots[id].name.clone()))
                }
            })
            .collect()
    }
}

#[derive(Clone, Debug, Default)]
pub struct MapEditor {
    pub restore_id: u64,
    pub working: Option<LatticeMap>,
    pub edit_shape: bool,
    pub undo: Vec<LatticeMap>,
}
impl MapEditor {
    pub fn restore(&mut self, id: u64) {
        if self.restore_id != id {
            self.restore_id = id;
            self.working = None;
            self.edit_shape = false;
            self.undo.clear();
        }
    }
    pub fn change(&mut self, map: LatticeMap) {
        if self.working == Some(map) || !map.valid() {
            return;
        }
        if let Some(old) = self.working {
            if self.undo.len() == 64 {
                self.undo.remove(0);
            }
            self.undo.push(old);
        }
        self.working = Some(map);
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MapPlayback {
    pub engine: TuningEngine,
    pub selected: usize,
    pub map: Option<LatticeMap>,
    pub audition: bool,
}

#[derive(Clone, Debug)]
pub struct MapView {
    /// Current host/document intent; pending distinguishes it from audio adoption.
    pub playback: MapPlayback,
    pub pending: bool,
    pub names: Vec<(usize, String)>,
    pub working: Option<LatticeMap>,
    pub edit_shape: bool,
    pub can_undo: bool,
    pub full: bool,
}
impl MapView {
    pub fn editing(&self) -> bool {
        self.playback.engine == TuningEngine::LatticeMap
            && self.edit_shape
            && self.working.is_some()
    }
}
#[derive(Clone, Debug)]
pub enum MapEdit {
    Engine(TuningEngine),
    Select(usize),
    Audition,
    Return,
    EditShape(bool),
    Replace(LatticePos),
    Position(LatticePos),
    Undo,
    Capture(String),
    Rename(usize, String),
    MoveEarlier(usize),
    Delete(usize),
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn saved_identity_survives_rename_order_delete_and_exact_copy_recall() {
        let mut doc = MapDocument::default();
        let mut map = LatticeMap { position: LatticePos::new(50, 0, 0), ..Default::default() };
        map.replace(LatticePos::new(54, 0, 0));
        assert_eq!(doc.capture(map, "Passage".into()), Some(1));
        doc.slots[0].deleted = true;
        doc.slots[1].name = "Renamed".into();
        doc.order.reverse();
        let recalled: MapDocument = ron::from_str(&ron::to_string(&doc).unwrap()).unwrap();
        assert_eq!(recalled.map(1), Some(map));
        assert_eq!(recalled.map(0), None);
        assert_eq!(doc.capture(LatticeMap::default(), "New".into()), Some(2));
        assert_eq!(recalled.names(), vec![(1, "Renamed".into())]);
    }
}
