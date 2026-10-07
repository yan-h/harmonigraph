//! Musical map document and the editor/backend seam. This is project musical
//! state, independent of appearance and of the lifetime of an editor window.
use harmonigraph_core::lattice_map::{Follow, LatticeMap, TuningEngine};
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
/// The editor rebuilds a [`MapView`] every frame it draws the Lattice or Tuning
/// pane. Sharing the immutable list makes each view a single refcount bump;
/// names are copied only when the document revision changes.
pub type MapNames = Arc<[(usize, String)]>;

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
    /// Every mutation goes through a method below, because two
    /// derived values are keyed on [`revision`](Self::revision) and a write that
    /// skips the bump is invisible to both: [`names`](Self::names) serves a
    /// stale name in the UI forever, and the audio thread's map bank serves a
    /// stale shape — a capture or a delete that never reaches playback at all.
    slots: Vec<NamedMap>,
    order: Vec<usize>,
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
    /// Geometry reaches a slot only through [`capture`](Self::capture) and
    /// [`reshape`](Self::reshape), and both move this: a write that skipped it
    /// would leave the bank stale where the names would not — the one
    /// direction this key can be wrong in.
    pub fn revision(&self) -> Revision {
        self.revision
    }
    fn touch(&mut self) {
        self.revision = Revision::default();
    }
    pub fn map(&self, id: usize) -> Option<LatticeMap> {
        self.slots.get(id).filter(|m| !m.deleted)?.geometry.resolve()
    }
    pub fn name(&self, id: usize) -> Option<&str> {
        self.slots.get(id).filter(|m| !m.deleted).map(|m| m.name.as_str())
    }
    /// Deleted slots still consume their stable host identities.
    pub fn is_full(&self) -> bool {
        self.slots.len() >= MAP_CAPACITY
    }
    pub fn capture(&mut self, map: LatticeMap, name: String) -> Option<usize> {
        if self.is_full() || !map.valid() {
            return None;
        }
        let id = self.slots.len();
        self.slots.push(NamedMap { name, geometry: map.into(), deleted: false });
        self.order.push(id);
        self.touch();
        Some(id)
    }
    /// Rewrite a live slot's shape in place, keeping its identity, so host
    /// automation that selects `id` plays the new shape from the next attack.
    /// Refuses a deleted, unused or invalid slot rather than reviving it.
    pub fn reshape(&mut self, id: usize, map: LatticeMap) -> bool {
        let Some(slot) = self.slots.get_mut(id).filter(|m| !m.deleted) else { return false };
        if !map.valid() {
            return false;
        }
        slot.geometry = map.into();
        self.touch();
        true
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
        let mut order: Vec<_> = self.names().iter().map(|(id, _)| *id).collect();
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
                    Some((id, self.slots[id].name.clone()))
                }
            })
            .collect()
    }
}

#[derive(Clone, Debug, Default)]
pub struct MapEditor {
    pub restore_id: u64,
    pub edit_shape: bool,
    /// Each shape edit's slot and the shape it replaced, newest last. Undo
    /// takes only the selected slot's entries, so it never rewrites a map
    /// that is not on screen.
    undo: Vec<(usize, LatticeMap)>,
    /// Memo of [`MapDocument::names`] and the document state it was read from.
    ///
    /// It lives here rather than in the document because the document is
    /// behind an `RwLock` the AUDIO thread `try_read`s: filling a cache on the
    /// draw path would mean taking the WRITE lock every frame and losing that
    /// read. The editor's own `Mutex` is already held by the one caller that
    /// builds a [`MapView`], and the audio thread never reads this field.
    names: Option<(Revision, MapNames)>,
    /// The offsets the last arrow press read and the ones it sent. A host
    /// applies a set at its next process call, so a second press before then
    /// still reads the first one's starting lanes; keyed on those lanes, it
    /// builds on what was sent instead, and any other change to the lanes —
    /// the press landing, or automation — retires the entry.
    pub translated: Option<(MapOffsets, MapOffsets)>,
}
impl MapEditor {
    /// `document`'s names, rebuilt only when the document is a state this memo
    /// has not seen. The key carries the document's revision and nothing else:
    /// the rest of a [`MapView`] — selection, offsets, edit mode — is
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
    /// A restored project is a different document: undo entries name its
    /// slots by identity and would write old shapes into the new ones.
    pub fn restore(&mut self, id: u64) {
        if self.restore_id != id {
            self.restore_id = id;
            self.edit_shape = false;
            self.undo.clear();
        }
    }
    pub fn record(&mut self, id: usize, replaced: LatticeMap) {
        if self.undo.len() == 64 {
            self.undo.remove(0);
        }
        self.undo.push((id, replaced));
    }
    pub fn can_undo(&self, id: usize) -> bool {
        self.undo.iter().any(|&(slot, _)| slot == id)
    }
    /// Remove and return `id`'s newest replaced shape.
    pub fn undo(&mut self, id: usize) -> Option<LatticeMap> {
        let index = self.undo.iter().rposition(|&(slot, _)| slot == id)?;
        Some(self.undo.remove(index).1)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MapPlayback {
    pub engine: TuningEngine,
    pub selected: usize,
    pub offset: LatticePos,
    pub map: Option<LatticeMap>,
    pub follow: Follow,
}

impl Default for MapPlayback {
    fn default() -> Self {
        Self {
            engine: TuningEngine::default(),
            selected: 0,
            offset: LatticePos::ORIGIN,
            map: None,
            follow: Follow::Off,
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

    /// Every lane with the value it holds, in one fixed order.
    pub fn lanes(self) -> [(MapAxis, MapOffsetLane, i32); 6] {
        let (f, e) = (self.fine, self.extension);
        [
            (MapAxis::Fifths, MapOffsetLane::Fine, f.threes),
            (MapAxis::Thirds, MapOffsetLane::Fine, f.fives),
            (MapAxis::Sevenths, MapOffsetLane::Fine, f.sevens),
            (MapAxis::Fifths, MapOffsetLane::Extension, e.threes),
            (MapAxis::Thirds, MapOffsetLane::Extension, e.fives),
            (MapAxis::Sevenths, MapOffsetLane::Extension, e.sevens),
        ]
    }

    /// The lanes after `step` units of `lane` on each axis, or `None` when any
    /// axis would leave its range. A Fine step past ±9 carries into Coarse, so
    /// every total from −99 to +99 stays one arrow press from its neighbours.
    pub fn translated(self, step: LatticePos, lane: MapOffsetLane) -> Option<Self> {
        let axis = |fine: i32, extension: i32, by: i32| {
            let (mut fine, mut extension) = match lane {
                MapOffsetLane::Fine => (fine + by, extension),
                MapOffsetLane::Extension => (fine, extension + by),
            };
            while fine > OFFSET_LIMIT {
                fine -= EXTENSION_STEP;
                extension += 1;
            }
            while fine < -OFFSET_LIMIT {
                fine += EXTENSION_STEP;
                extension -= 1;
            }
            (extension.abs() <= OFFSET_LIMIT).then_some((fine, extension))
        };
        let (f3, e3) = axis(self.fine.threes, self.extension.threes, step.threes)?;
        let (f5, e5) = axis(self.fine.fives, self.extension.fives, step.fives)?;
        let (f7, e7) = axis(self.fine.sevens, self.extension.sevens, step.sevens)?;
        Some(Self { fine: LatticePos::new(f3, f5, f7), extension: LatticePos::new(e3, e5, e7) })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapOffsetLane {
    Fine,
    Extension,
}

#[derive(Clone, Debug)]
pub struct MapView {
    /// Current host/document intent; pending distinguishes it from audio adoption.
    pub playback: MapPlayback,
    pub offsets: MapOffsets,
    /// Where following has moved the map beyond `offsets`, as the Hub last
    /// left it. Already included in `playback.map`, which is what sounds.
    pub followed: LatticePos,
    pub pending: bool,
    /// Shared with the editor's memo (see [`MapEditor::names`]).
    pub names: MapNames,
    pub edit_shape: bool,
    pub can_undo: bool,
    pub full: bool,
}
impl MapView {
    /// Lattice clicks edit the selected saved map, so there must be one.
    pub fn editing(&self) -> bool {
        self.playback.engine == TuningEngine::LatticeMap
            && self.edit_shape
            && self.playback.map.is_some()
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
    EditShape(bool),
    Replace(LatticePos),
    BeginOffset(MapAxis, MapOffsetLane),
    Offset(MapAxis, MapOffsetLane, i32),
    EndOffset(MapAxis, MapOffsetLane),
    /// Move the map from the lattice by whole lane steps (see
    /// [`MapOffsets::translated`]), each changed lane as one host gesture.
    Translate(LatticePos, MapOffsetLane),
    Follow(Follow),
    Undo,
    /// Copy the selected shape into a new slot and select it.
    Duplicate,
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
        doc.delete(0);
        assert!(!doc.reshape(0, LatticeMap::default()), "a tombstone is never revived");
        doc.rename(1, "Renamed".into());
        doc.move_earlier(1);
        let saved = ron::to_string(&doc).unwrap();
        assert!(saved.starts_with("(slots:"));
        assert!(saved.contains("order:[1]"));
        let recalled: MapDocument = ron::from_str(&saved).unwrap();
        map.position = LatticePos::ORIGIN;
        assert_eq!(recalled.map(1), Some(map));
        assert_eq!(recalled.map(0), None);
        assert_eq!(doc.capture(LatticeMap::default(), "New".into()), Some(2));
        assert_eq!(&*recalled.names(), &[(1, "Renamed".into())]);
        assert_eq!(recalled.name(0), None);
        assert_eq!(recalled.name(1), Some("Renamed"));
        assert_eq!(recalled.name(2), None);
        assert!(!recalled.is_full());
        for id in 3..MAP_CAPACITY {
            assert_eq!(doc.capture(LatticeMap::default(), "More".into()), Some(id));
        }
        assert!(doc.is_full());
        assert_eq!(doc.capture(LatticeMap::default(), "Full".into()), None);
    }

    /// `Arc::ptr_eq` is the instrument: a memo hit hands back the very
    /// list the last call did, and a rebuild cannot.
    #[test]
    fn names_are_memoized_and_every_document_mutation_invalidates_them() {
        let mut doc = MapDocument::default();
        assert_eq!(doc.capture(LatticeMap::default(), "Passage".into()), Some(1));
        let mut editor = MapEditor::default();
        let first = editor.names(&doc);
        assert_eq!(first.len(), 2, "the fixture needs a name that survives each mutation");
        let held = editor.names(&doc);
        assert!(Arc::ptr_eq(&held, &first), "an unchanged document must not rebuild");

        // Each of the five mutations the editor can make, in turn.
        let mut previous = first;
        // What was done, how, and the names it must leave visible.
        type Mutation<'a> = (&'a str, &'a dyn Fn(&mut MapDocument), &'a [&'a str]);
        let mutations: [Mutation; 5] = [
            ("rename", &|doc| doc.rename(1, "Renamed".into()), &["C · 3×4", "Renamed"]),
            (
                "reshape",
                &|doc| {
                    let mut map = LatticeMap::default();
                    assert!(map.replace(LatticePos::new(4, 0, 0)));
                    assert!(doc.reshape(1, map));
                    assert_eq!(doc.map(1), Some(map));
                },
                &["C · 3×4", "Renamed"],
            ),
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
                !Arc::ptr_eq(&next, &previous),
                "a {what} must move the revision the memo is keyed on"
            );
            assert_eq!(next.iter().map(|(_, name)| &**name).collect::<Vec<_>>(), visible, "{what}");
            assert!(Arc::ptr_eq(&next, &editor.names(&doc)), "{what} must memoize its snapshot");
            previous = next;
        }
        assert_eq!(&*held, &[(0, "C · 3×4".into()), (1, "Passage".into())]);
    }

    /// The case a per-document counter gets wrong: a loaded document restarts
    /// such a counter at zero and collides with a memo read from an unrelated
    /// document, which is why the revision is a process-unique ticket.
    #[test]
    fn a_loaded_document_never_answers_from_another_documents_memo() {
        let mut editor = MapEditor::default();
        // A memo taken from a FRESH document — revision zero under a counter.
        let first = editor.names(&MapDocument::default());
        assert_eq!(&first[0].1, "C · 3×4");
        let mut other = MapDocument::default();
        other.rename(0, "Other".into());
        let loaded: MapDocument = ron::from_str(&ron::to_string(&other).unwrap()).unwrap();
        let next = editor.names(&loaded);
        assert!(!Arc::ptr_eq(&first, &next));
        assert_eq!(&next[0].1, "Other");
        assert!(Arc::ptr_eq(&next, &editor.names(&loaded)));
    }

    #[test]
    fn a_fine_step_carries_into_coarse_and_stops_at_the_ends() {
        let at = |fine, extension| MapOffsets {
            fine: LatticePos::new(fine, 0, 0),
            extension: LatticePos::new(extension, 0, 0),
        };
        let up = LatticePos::new(1, 0, 0);
        let down = LatticePos::new(-1, 0, 0);
        assert_eq!(at(3, 0).translated(up, MapOffsetLane::Fine), Some(at(4, 0)));
        assert_eq!(at(9, 2).translated(up, MapOffsetLane::Fine), Some(at(0, 3)));
        assert_eq!(at(-9, 0).translated(down, MapOffsetLane::Fine), Some(at(0, -1)));
        // A carry keeps the total, whatever the lanes held before.
        let carried = at(9, 2).translated(up, MapOffsetLane::Fine).unwrap();
        assert_eq!(carried.total().threes, at(9, 2).total().threes + 1);
        // Coarse moves alone and leaves fine automation where it was.
        assert_eq!(at(-3, 0).translated(up, MapOffsetLane::Extension), Some(at(-3, 1)));
        assert_eq!(at(9, 9).translated(up, MapOffsetLane::Fine), None, "+99 is the end");
        assert_eq!(at(0, -9).translated(down, MapOffsetLane::Extension), None);
    }
}
