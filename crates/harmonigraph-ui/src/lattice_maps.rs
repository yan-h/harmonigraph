//! Musical map document and the editor/backend seam. This is project musical
//! state, independent of appearance and of the lifetime of an editor window.
use harmonigraph_core::lattice_map::{LatticeMap, TuningEngine};
use harmonigraph_core::LatticePos;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

pub const MAP_CAPACITY: usize = 128;
pub const MIDI_LABELS: [&str; 12] =
    ["C", "C♯", "D", "D♯", "E", "F", "F♯", "G", "G♯", "A", "A♯", "B"];

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct MapRecord {
    pub nodes: [[i32; 3]; 12],
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
        Self { nodes: map.nodes.map(coordinates) }
    }
}
impl MapRecord {
    pub fn resolve(&self) -> Option<LatticeMap> {
        let map = LatticeMap { nodes: self.nodes.map(position), position: LatticePos::ORIGIN };
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

/// The visible slots in display order, with their names — see
/// [`MapDocument::names`].
///
/// `Arc<str>` rather than `String`: the editor rebuilds a [`MapView`] every
/// frame it draws the Lattice or Tuning pane, and the names are the only part
/// of it that owns heap. Sharing them makes that rebuild a `Vec` and up to
/// [`MAP_CAPACITY`] refcount bumps instead of that many string copies.
pub type MapNames = Vec<(usize, Arc<str>)>;

/// A process-unique ticket for one state of a [`MapDocument`], minted at
/// construction, at deserialization and at every mutation.
///
/// Not a counter on the document, because a document is also REPLACED whole
/// when saved state loads: a per-document counter would restart at zero there
/// and alias a state a memo had already seen. Minting from one process-wide
/// source leaves "this is not the document my value came from" the only thing
/// the key can say, and makes the load case need no bump site to remember.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Revision(u64);
impl Default for Revision {
    fn default() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct MapDocument {
    /// Slot index is the host identity. Slots are never removed or recycled.
    ///
    /// Public for READING — the plugin's `lattice-map` parameter formats a slot
    /// name through it. Every mutation goes through a method below, because two
    /// derived values are keyed on [`revision`](Self::revision) and a write that
    /// skips the bump is invisible to both: [`names`](Self::names) serves a
    /// stale name in the UI forever, and the audio thread's map bank serves a
    /// stale shape — a capture or a delete that never reaches playback at all.
    pub slots: Vec<NamedMap>,
    pub order: Vec<usize>,
    /// Skipped rather than persisted: a loaded document is a new state, and
    /// `Default` mints it a ticket nothing has seen.
    #[serde(skip)]
    revision: Revision,
}
impl Default for MapDocument {
    fn default() -> Self {
        Self { slots: vec![NamedMap::default()], order: vec![0], revision: Revision::default() }
    }
}
impl MapDocument {
    /// Which state this is, for both values derived from a document:
    /// [`MapEditor::names`]'s memo, and the audio thread's map bank in the
    /// plugin's `AudioMaps::adopt`. Everything either one reads — a slot's
    /// existence, its name, its deleted flag, its GEOMETRY, and `order` —
    /// moves this.
    ///
    /// Geometry is the entry the methods below cover by their shape rather
    /// than by a bump of their own: it reaches a slot only through
    /// [`capture`](Self::capture), and nothing rewrites an existing slot's.
    /// A method that ever does has to move this too, or the bank goes stale
    /// where the names would not — the one direction this key can be wrong in.
    pub fn revision(&self) -> Revision {
        self.revision
    }
    fn touch(&mut self) {
        self.revision = Revision::default();
    }
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
        self.touch();
        Some(id)
    }
    pub fn rename(&mut self, id: usize, name: String) {
        if let Some(slot) = self.slots.get_mut(id) {
            slot.name = name;
            self.touch();
        }
    }
    pub fn delete(&mut self, id: usize) {
        if let Some(slot) = self.slots.get_mut(id) {
            slot.deleted = true;
            self.touch();
        }
    }
    /// Move `id` one place earlier in the display order, rebuilding `order`
    /// from the visible sequence so a hand-edited file's stray entries do not
    /// survive the swap.
    pub fn move_earlier(&mut self, id: usize) {
        let mut order: Vec<_> = self.names().into_iter().map(|(id, _)| id).collect();
        if let Some(index) = order.iter().position(|&item| item == id) {
            if index > 0 {
                order.swap(index, index - 1);
            }
        }
        self.order = order;
        self.touch();
    }
    /// The visible slots in display order. [`MapNames`] says why the names are
    /// shared rather than copied; [`MapEditor::names`] is what the editor
    /// should call, which reaches this only when the document has changed.
    pub fn names(&self) -> MapNames {
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
                    Some((id, Arc::from(self.slots[id].name.as_str())))
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
    /// Memo of [`MapDocument::names`] and the document state it was read from.
    ///
    /// It lives here rather than in the document because the document is
    /// behind an `RwLock` the AUDIO thread `try_read`s: filling a cache on the
    /// draw path would mean taking the WRITE lock every frame and losing that
    /// read. The editor's own `Mutex` is already held by the one caller that
    /// builds a [`MapView`], and the audio thread never reads this field.
    names: Option<(Revision, MapNames)>,
}
impl MapEditor {
    /// `document`'s names, rebuilt only when the document is a state this memo
    /// has not seen. The key carries the document's revision and nothing else:
    /// the rest of a [`MapView`] — selection, offsets, the working copy — is
    /// rebuilt by its caller every frame, because none of it decides this value.
    pub fn names(&mut self, document: &MapDocument) -> MapNames {
        let revision = document.revision();
        match &self.names {
            Some((seen, names)) if *seen == revision => names.clone(),
            _ => {
                let names = document.names();
                self.names = Some((revision, names.clone()));
                names
            }
        }
    }
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MapPlayback {
    pub engine: TuningEngine,
    pub selected: usize,
    pub offset: LatticePos,
    pub map: Option<LatticeMap>,
    pub audition: bool,
}

impl Default for MapPlayback {
    fn default() -> Self {
        Self {
            engine: TuningEngine::default(),
            selected: 0,
            offset: LatticePos::ORIGIN,
            map: None,
            audition: false,
        }
    }
}

/// Fixed host ranges: extension counts represent ten lattice steps each.
pub const OFFSET_LIMIT: i32 = 9;
pub const EXTENSION_STEP: i32 = 10;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MapOffsets {
    pub fine: LatticePos,
    pub extension: LatticePos,
}
impl Default for MapOffsets {
    fn default() -> Self {
        Self { fine: LatticePos::ORIGIN, extension: LatticePos::ORIGIN }
    }
}
impl MapOffsets {
    pub fn total(self) -> LatticePos {
        LatticePos::new(
            self.fine.threes + EXTENSION_STEP * self.extension.threes,
            self.fine.fives + EXTENSION_STEP * self.extension.fives,
            self.fine.sevens + EXTENSION_STEP * self.extension.sevens,
        )
    }
}

#[derive(Clone, Copy, Debug)]
pub enum MapOffsetLane {
    Fine,
    Extension,
}

#[derive(Clone, Debug)]
pub struct MapView {
    /// Current host/document intent; pending distinguishes it from audio adoption.
    pub playback: MapPlayback,
    pub offsets: MapOffsets,
    pub pending: bool,
    /// Shared with the document's memo (see [`MapEditor::names`]), so cloning a
    /// view is refcounts rather than up to [`MAP_CAPACITY`] string copies.
    pub names: MapNames,
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
#[derive(Clone, Copy, Debug)]
pub enum MapAxis {
    Fifths,
    Thirds,
    Sevenths,
}

#[derive(Clone, Debug)]
pub enum MapEdit {
    Engine(TuningEngine),
    Select(usize),
    Audition,
    Return,
    EditShape(bool),
    Replace(LatticePos),
    BeginOffset(MapAxis, MapOffsetLane),
    Offset(MapAxis, MapOffsetLane, i32),
    EndOffset(MapAxis, MapOffsetLane),
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
        map.position = LatticePos::ORIGIN;
        assert_eq!(recalled.map(1), Some(map));
        assert_eq!(recalled.map(0), None);
        assert_eq!(doc.capture(LatticeMap::default(), "New".into()), Some(2));
        assert_eq!(recalled.names(), vec![(1, "Renamed".into())]);
    }

    /// `Arc::ptr_eq` is the instrument: a memo hit hands back the very
    /// allocations the last call did, and a rebuild cannot.
    #[test]
    fn names_are_memoized_and_every_document_mutation_invalidates_them() {
        let mut doc = MapDocument::default();
        assert_eq!(doc.capture(LatticeMap::default(), "Passage".into()), Some(1));
        let mut editor = MapEditor::default();
        let first = editor.names(&doc);
        assert_eq!(first.len(), 2, "the fixture needs a name that survives each mutation");
        let held = editor.names(&doc);
        assert!(Arc::ptr_eq(&held[0].1, &first[0].1), "an unchanged document must not rebuild");

        // Each of the four mutations the editor can make, in turn. Slot 0 is
        // never the one edited, so its Arc is what says the LIST was rebuilt
        // rather than merely that the edited entry changed.
        let mut previous = first;
        // What was done, how, and the names it must leave visible.
        type Mutation<'a> = (&'a str, &'a dyn Fn(&mut MapDocument), &'a [&'a str]);
        let mutations: [Mutation; 4] = [
            ("rename", &|doc| doc.rename(1, "Renamed".into()), &["C · 3×4", "Renamed"]),
            (
                "capture",
                &|doc| {
                    doc.capture(LatticeMap::default(), "Added".into());
                },
                &["C · 3×4", "Renamed", "Added"],
            ),
            ("reorder", &|doc| doc.move_earlier(1), &["Renamed", "C · 3×4", "Added"]),
            ("delete", &|doc| doc.delete(1), &["C · 3×4", "Added"]),
        ];
        for (what, mutate, visible) in mutations {
            mutate(&mut doc);
            let next = editor.names(&doc);
            assert!(
                !Arc::ptr_eq(&next[0].1, &previous[0].1),
                "a {what} must move the revision the memo is keyed on"
            );
            assert_eq!(next.iter().map(|(_, name)| &**name).collect::<Vec<_>>(), visible, "{what}");
            previous = next;
        }
    }

    /// The case a per-document counter gets wrong: a loaded document restarts
    /// such a counter at zero and collides with a memo read from an unrelated
    /// document, which is why the revision is a process-unique ticket.
    #[test]
    fn a_loaded_document_never_answers_from_another_documents_memo() {
        let mut editor = MapEditor::default();
        // A memo taken from a FRESH document — revision zero under a counter.
        assert_eq!(&*editor.names(&MapDocument::default())[0].1, "C · 3×4");
        let mut other = MapDocument::default();
        other.rename(0, "Other".into());
        let loaded: MapDocument = ron::from_str(&ron::to_string(&other).unwrap()).unwrap();
        assert_eq!(&*editor.names(&loaded)[0].1, "Other");
    }
}
