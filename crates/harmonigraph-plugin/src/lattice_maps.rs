//! Editor-owned musical document, bounded audio snapshot and sample-indexed
//! next-attack state. No editor window is needed for restore or automation.
use crate::Row;
use harmonigraph_clap::prelude::*;
use harmonigraph_core::configuration::ResolvedConfig;
use harmonigraph_core::lattice_map::{Follow, LatticeMap, TuningEngine};
use harmonigraph_core::LatticePos;
use harmonigraph_ui::lattice_maps::*;
use parking_lot::Mutex;
use std::collections::VecDeque;
use std::sync::Arc;

/// The highest `lattice-map` value: the host numbers the document's slots.
pub const LAST_MAP: i32 = MAP_CAPACITY as i32 - 1;

/// The slot a `lattice-map` value selects, clamped as the host's label is.
fn map_slot(value: i32) -> usize {
    value.clamp(0, LAST_MAP) as usize
}

/// The follow offset as one atomic word: it only ever moves fifths and thirds.
pub fn pack(offset: LatticePos) -> u64 {
    (u64::from(offset.threes as u32) << 32) | u64::from(offset.fives as u32)
}
pub fn unpack(word: u64) -> LatticePos {
    LatticePos::new((word >> 32) as u32 as i32, word as u32 as i32, 0)
}

/// Where following has moved the map, as the Hub last published it. The Hub
/// publishes nothing moved while following is off or a mode change has yet to
/// reach an attack; the parameters mask it as well, because with no process
/// callbacks running the Hub publishes nothing at all.
fn followed(params: &crate::HarmonigraphParams) -> LatticePos {
    let following = TuningEngine::from_index(params.int(Row::TuningEngine))
        == TuningEngine::LatticeMap
        && Follow::from_index(params.int(Row::MapFollow)) != Follow::Off;
    if following {
        unpack(params.map_followed.load(std::sync::atomic::Ordering::Acquire))
    } else {
        LatticePos::ORIGIN
    }
}

/// The host parameter behind one lane: the plugin's one place an axis and a
/// grain name a row.
pub(crate) fn lane_row(axis: MapAxis, grain: MapOffsetLane) -> Row {
    match (axis, grain) {
        (MapAxis::Fifths, MapOffsetLane::Fine) => Row::MapFifths,
        (MapAxis::Thirds, MapOffsetLane::Fine) => Row::MapThirds,
        (MapAxis::Sevenths, MapOffsetLane::Fine) => Row::MapSevenths,
        (MapAxis::Fifths, MapOffsetLane::Extension) => Row::MapFifthsExtension,
        (MapAxis::Thirds, MapOffsetLane::Extension) => Row::MapThirdsExtension,
        (MapAxis::Sevenths, MapOffsetLane::Extension) => Row::MapSeventhsExtension,
    }
}

pub fn offsets(params: &crate::HarmonigraphParams) -> MapOffsets {
    let mut offsets = MapOffsets::default();
    for (axis, grain) in LANES {
        *offsets.lane_mut(axis, grain) = params.int(lane_row(axis, grain));
    }
    offsets
}

pub fn offset(params: &crate::HarmonigraphParams) -> LatticePos {
    offsets(params).total()
}

/// Which map is selected, and where it stands: the selection the lattice
/// marks, which the tape samples beside its notes (#1719). It is what the
/// parameters ask for, which the audio thread adopts at its next attack, so
/// it reproduces what the live lattice showed. The selection rather than the
/// shape: a map edited since a recording marks its notes under the edit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct MapSelection {
    pub engine: TuningEngine,
    pub slot: usize,
    /// The lanes' offset, and where following has moved the map.
    pub position: LatticePos,
}

impl MapSelection {
    /// The selection the parameters ask for, at the lanes' offset alone.
    fn requested(params: &crate::HarmonigraphParams) -> MapSelection {
        MapSelection {
            engine: TuningEngine::from_index(params.int(Row::TuningEngine)),
            slot: map_slot(params.int(Row::LatticeMap)),
            position: offset(params),
        }
    }

    /// Moved on by where following has taken the map.
    fn followed_by(self, followed: LatticePos) -> MapSelection {
        MapSelection { position: self.position + followed, ..self }
    }

    /// The document's map in this slot, standing here.
    fn placed(self, document: &MapDocument) -> Option<LatticeMap> {
        document.map(self.slot).map(|shape| translated(shape, self.position))
    }
}

/// The selection the lattice marks now.
pub(crate) fn selection(params: &crate::HarmonigraphParams) -> MapSelection {
    MapSelection::requested(params).followed_by(followed(params))
}

/// A recorded `selection` as the lattice marks it: the map in that slot as
/// the document holds it now, standing where it stood then.
pub(crate) fn recorded(
    params: &crate::HarmonigraphParams,
    selection: MapSelection,
) -> harmonigraph_ui::tape::RecordedMap {
    let map = params.maps.read(|document| selection.placed(document));
    harmonigraph_ui::tape::RecordedMap { engine: selection.engine, map }
}

fn translated(mut shape: LatticeMap, offset: LatticePos) -> LatticeMap {
    shape.position = offset;
    shape
}

pub fn view(params: &crate::HarmonigraphParams) -> MapView {
    params.maps.read(|document| view_of(params, document))
}

fn view_of(params: &crate::HarmonigraphParams, document: &MapDocument) -> MapView {
    let mut editor = params.map_editor.lock();
    let adopted = *params.map_playback.lock();
    let offsets = offsets(params);
    let requested = MapSelection::requested(params);
    let selected = requested.slot;
    let playback = MapPlayback {
        engine: requested.engine,
        selected,
        offset: requested.position,
        map: requested.placed(document),
        follow: Follow::from_index(params.int(Row::MapFollow)),
    };
    // Intent is compared before following is added: the Hub's own movement is
    // not host state waiting for audio to adopt it.
    let unadopted = (playback != adopted).then_some(playback);
    let followed = followed(params);
    let shown = MapPlayback { map: requested.followed_by(followed).placed(document), ..playback };
    MapView {
        playback: shown,
        offsets,
        followed,
        unadopted,
        names: editor.names(document),
        edit_shape: editor.edit_shape,
        can_undo: playback.map.is_some() && editor.can_undo(selected),
        full: document.is_full(),
    }
}

pub fn edit(params: &crate::HarmonigraphParams, host: &dyn EditorHost, edit: MapEdit) {
    let set = |row: Row, value: i32| {
        host.begin_set_parameter(row.index());
        host.set_parameter(row.index(), value as f32);
        host.end_set_parameter(row.index());
    };
    let selected = map_slot(params.int(Row::LatticeMap));
    // Whether the saved document changed, decided by each arm from what it
    // did rather than by its variant: a no-op edit must not mark the host
    // project modified.
    let document_changed = match edit {
        MapEdit::Engine(mode) => {
            set(Row::TuningEngine, mode.index());
            false
        }
        MapEdit::Select(id) => {
            if id < MAP_CAPACITY {
                set(Row::LatticeMap, id as i32);
            }
            false
        }
        MapEdit::EditShape(on) => {
            params.map_editor.lock().edit_shape = on;
            false
        }
        // The document lock is taken before the editor's, in the order `view`
        // takes them.
        MapEdit::Undo => params.maps.write(|document| {
            let undone = params.map_editor.lock().undo(selected);
            undone.is_some_and(|map| document.reshape(selected, map))
        }),
        // A lane's gesture is the editor's registry's (`editor::gestures`),
        // which the parameter backend opens and closes around this edit.
        MapEdit::BeginOffset(..) | MapEdit::EndOffset(..) => false,
        MapEdit::Offset(axis, lane, value) => {
            host.set_parameter(lane_row(axis, lane).index(), value as f32);
            false
        }
        // From the lanes as they are: an editor set reaches them at once, so
        // a second press before the next callback builds on the first.
        MapEdit::Translate(step, lane) => {
            let from = offsets(params);
            if let Some(to) = from.translated(step, lane) {
                for ((axis, lane, before), (_, _, after)) in
                    from.lanes().into_iter().zip(to.lanes())
                {
                    if before != after {
                        set(lane_row(axis, lane), after);
                    }
                }
            }
            false
        }
        MapEdit::Follow(mode) => {
            set(Row::MapFollow, mode.index());
            false
        }
        MapEdit::Replace(destination) => params.maps.write(|document| {
            let mut editor = params.map_editor.lock();
            let old = document.map(selected).filter(|_| editor.edit_shape);
            let reshaped = old.is_some_and(|old| {
                // The destination was picked against the map as it sounds.
                let mut map = translated(old, offset(params) + followed(params));
                map.replace(destination) && {
                    map.position = LatticePos::ORIGIN;
                    document.reshape(selected, map)
                }
            });
            if let Some(old) = old.filter(|_| reshaped) {
                editor.record(selected, old);
            }
            reshaped
        }),
        // The host is told of the selection out of the door, as of every set.
        MapEdit::Duplicate => {
            let id = params.maps.write(|document| {
                let copy = document
                    .map(selected)
                    .zip(document.name(selected).map(|n| format!("{n} copy")));
                copy.and_then(|(map, name)| document.capture(map, name))
            });
            if let Some(id) = id {
                set(Row::LatticeMap, id as i32);
            }
            id.is_some()
        }
        // Every document mutation goes through a `MapDocument` method, because
        // each one has to move the revision `MapEditor::names` is keyed on.
        MapEdit::Rename(id, name) => params.maps.write(|document| document.rename(id, name)),
        // Deleting the selected map selects its neighbour, as Duplicate selects
        // its copy, rather than leaving new attacks on a map that is gone.
        MapEdit::Delete(id) => {
            let (neighbour, deleted) = params.maps.write(|document| {
                // Only a live map has a neighbour, so a delete that found one
                // went through.
                (document.neighbour(id).filter(|_| id == selected), document.delete(id))
            });
            if let Some(neighbour) = neighbour {
                set(Row::LatticeMap, neighbour as i32);
            }
            deleted
        }
        MapEdit::Move(id, step) => params.maps.write(|document| document.move_by(id, step)),
    };
    if document_changed {
        params.settings.maps_changed();
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AttackState {
    pub engine_revision: u64,
    pub config: ResolvedConfig,
    pub playback: MapPlayback,
}
#[derive(Clone, Copy)]
struct Entry {
    sample: i64,
    state: AttackState,
}
const HISTORY: usize = 8192;

/// The host parameters a callback's map state starts from, read at its
/// configuration boundary.
#[derive(Clone, Copy)]
pub struct Seed {
    pub engine: i32,
    pub map: i32,
    pub offsets: MapOffsets,
    pub follow: i32,
}
impl Seed {
    pub fn read(params: &crate::HarmonigraphParams) -> Self {
        Self {
            engine: params.int(Row::TuningEngine),
            map: params.int(Row::LatticeMap),
            offsets: offsets(params),
            follow: params.int(Row::MapFollow),
        }
    }
}

/// Each slot's geometry, as audio reads it: fixed and `Copy`, never names.
pub(crate) type Bank = [Option<LatticeMap>; MAP_CAPACITY];

fn bank(document: &MapDocument) -> Bank {
    std::array::from_fn(|id| document.map(id))
}

/// The Lattice Map document behind its door (`crate::shared`), and the
/// geometry each write publishes to audio as it ends, wait-free for the
/// reader, as the tuning publishes its configuration (#1938).
pub struct Maps {
    document: crate::shared::Shared<Published>,
    /// Taken once, by the audio owner.
    audio: Mutex<Option<triple_buffer::Output<Bank>>>,
}
struct Published {
    document: MapDocument,
    audio: triple_buffer::Input<Bank>,
}
impl Default for Maps {
    fn default() -> Self {
        let document = MapDocument::default();
        let (audio, output) = triple_buffer::triple_buffer(&bank(&document));
        Self {
            document: crate::shared::Shared::new(Published { document, audio }),
            audio: Mutex::new(Some(output)),
        }
    }
}
impl Maps {
    pub fn read<R>(&self, read: impl FnOnce(&MapDocument) -> R) -> R {
        self.document.read(|published| read(&published.document))
    }
    /// Change the document, then publish its geometry before the door
    /// closes, when the write changed it.
    pub fn write<R>(&self, write: impl FnOnce(&mut MapDocument) -> R) -> R {
        self.document.write(|published| {
            let before = bank(&published.document);
            let written = write(&mut published.document);
            let after = bank(&published.document);
            if after != before {
                published.audio.write(after);
            }
            written
        })
    }
}

pub struct AudioMaps {
    published: Arc<Mutex<MapPlayback>>,
    followed: Arc<std::sync::atomic::AtomicU64>,
    /// The geometry the document last published, brought up to date once a
    /// callback, at [`adopt`](Self::adopt), so a callback reads one bank.
    bank: triple_buffer::Output<Bank>,
    pub playback: MapPlayback,
    history: VecDeque<Entry>,
    /// The current callback's first sample and length.
    start: i64,
    frames: u32,
    /// The host parameters, for their decoding of an automation value.
    table: Arc<Table>,
    engine_revision: u64,
    offsets: MapOffsets,
}
impl AudioMaps {
    pub fn new(params: &crate::HarmonigraphParams) -> Self {
        Self {
            published: params.map_playback.clone(),
            followed: params.map_followed.clone(),
            bank: params.maps.audio.lock().take().expect("one audio owner"),
            playback: MapPlayback::default(),
            history: VecDeque::with_capacity(HISTORY),
            start: 0,
            frames: 0,
            table: params.table.clone(),
            engine_revision: 0,
            offsets: MapOffsets::default(),
        }
    }
    /// The Hub's follow offset, for the editor to draw the map where it sounds.
    pub fn publish_followed(&self, offset: LatticePos) {
        self.followed.store(pack(offset), std::sync::atomic::Ordering::Release);
    }
    pub fn begin(&mut self, boundary: &Boundary) {
        self.start = boundary.steady_time;
        self.frames = boundary.frames;
    }
    pub fn adopt(&mut self, config: ResolvedConfig, seed: Seed) {
        // The geometry the last finished write published: wait-free, no lock.
        self.bank.update();
        self.set_engine(TuningEngine::from_index(seed.engine));
        self.playback.selected = map_slot(seed.map);
        self.offsets = seed.offsets;
        self.playback.offset = self.offsets.total();
        self.set_follow(Follow::from_index(seed.follow));
        self.resolve();
        self.push(self.start, config);
    }
    fn set_engine(&mut self, engine: TuningEngine) {
        if self.playback.engine != engine {
            self.engine_revision = self.engine_revision.saturating_add(1);
            self.playback.engine = engine;
        }
    }
    /// Under Lattice Map a follow mode is a retuning mode for this purpose:
    /// changing it starts the Hub's context, and so its follow offset, over at
    /// the next attack. Under any other engine it decides nothing, and must
    /// not reset Adaptive's context; entering Lattice Map resets it anyway.
    fn set_follow(&mut self, follow: Follow) {
        if self.playback.follow != follow {
            if self.playback.engine == TuningEngine::LatticeMap {
                self.engine_revision = self.engine_revision.saturating_add(1);
            }
            self.playback.follow = follow;
        }
    }
    /// The retuning-mode revision the latest observed state carries, which the
    /// Hub compares against the one its context was built under.
    pub fn engine_revision(&self) -> u64 {
        self.engine_revision
    }
    fn resolve(&mut self) {
        self.playback.map = self.bank.peek_output_buffer()[self.playback.selected]
            .map(|shape| translated(shape, self.playback.offset));
        if let Some(mut published) = self.published.try_lock() {
            *published = self.playback;
        }
    }
    /// An automation event at its own sample, decoded by the parameter table
    /// as the shell decodes the same event into the table: the timeline and
    /// what the host reads back hold one value (#1814).
    pub fn observe(&mut self, event: Input, config: ResolvedConfig) {
        let InputValue::Parameter { id, value, modulation: false } = event.value else { return };
        let Some((row, plain)) = self.table.decode(id, value) else { return };
        let plain = plain as i32;
        match Row::ALL[row] {
            Row::LatticeMap => self.playback.selected = map_slot(plain),
            Row::TuningEngine => self.set_engine(TuningEngine::from_index(plain)),
            Row::MapFollow => self.set_follow(Follow::from_index(plain)),
            row => {
                let lane = LANES.into_iter().find(|&(axis, grain)| lane_row(axis, grain) == row);
                let Some((axis, grain)) = lane else { return };
                *self.offsets.lane_mut(axis, grain) = plain;
                self.playback.offset = self.offsets.total();
            }
        }
        self.resolve();
        self.push(event.sample, config);
    }
    /// The state the latest observation leaves, under `config`: what an attack
    /// the history no longer reaches is assigned from.
    pub fn live(&self, config: ResolvedConfig) -> AttackState {
        AttackState { config, playback: self.playback, engine_revision: self.engine_revision }
    }
    fn push(&mut self, sample: i64, config: ResolvedConfig) {
        let state = self.live(config);
        if let Some(last) = self.history.back_mut() {
            if last.sample == sample {
                last.state = state;
                return;
            }
            if last.state == state {
                return;
            }
        }
        if self.history.len() == HISTORY {
            self.history.pop_front();
        }
        self.history.push_back(Entry { sample, state });
    }
    pub fn at(&self, sample: i64) -> Option<AttackState> {
        // Never extrapolate through a host callback whose automation is unknown.
        if sample >= self.start + i64::from(self.frames) {
            return None;
        }
        let end = self.history.partition_point(|entry| entry.sample <= sample);
        end.checked_sub(1).map(|index| self.history[index].state)
    }
    /// Entries strictly inside `(start, end)`. The history is sample-ordered —
    /// the same invariant `at` binary-searches on — so both ends are found by
    /// `partition_point` rather than by walking up to `HISTORY` entries per
    /// process block on the audio thread. `max` keeps an empty or inverted
    /// window from handing `range` a backwards bound, which panics: a
    /// zero-frame segment asks for `changes(start, start)`, and an entry
    /// sitting exactly on `start` puts `first` one past `last`.
    pub fn changes(&self, start: i64, end: i64) -> impl Iterator<Item = (i64, AttackState)> + '_ {
        let first = self.history.partition_point(|entry| entry.sample <= start);
        let last = self.history.partition_point(|entry| entry.sample < end).max(first);
        self.history.range(first..last).map(|entry| (entry.sample, entry.state))
    }
    pub fn reset(&mut self) {
        self.history.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use harmonigraph_core::configuration::ConfigReducer;
    use harmonigraph_core::lattice_map::TuningEngine;

    /// A history filled to its `HISTORY` cap, one entry per sample. A window
    /// queried in the middle then has thousands of entries on either side of
    /// it, which is where a scan and a pair of `partition_point`s can only
    /// disagree at the window's own two ends; a handful of entries never
    /// reaches that.
    fn filled() -> AudioMaps {
        let params = crate::HarmonigraphParams::default();
        let mut maps = AudioMaps::new(&params);
        let config = ConfigReducer::default().resolved();
        for sample in 0..HISTORY as i64 {
            // Each push differs from the last, so none is coalesced away.
            maps.playback.offset.threes = sample as i32;
            maps.push(sample, config);
        }
        assert_eq!(maps.history.len(), HISTORY);
        maps
    }

    #[test]
    fn changes_is_the_open_interval_at_both_ends_of_a_full_history() {
        let maps = filled();
        // Both ends land exactly on an entry, which is what an off-by-one in
        // either partition_point would admit.
        let window: Vec<_> = maps
            .changes(4000, 4010)
            .map(|(sample, state)| {
                assert_eq!(state.playback.offset.threes, sample as i32);
                sample
            })
            .collect();
        assert_eq!(window, (4001..4010).collect::<Vec<_>>());
        // The two ends of the deque itself, where a clamp would be wrong the
        // other way and drop a real change.
        assert_eq!(maps.changes(-1, 2).map(|(sample, _)| sample).collect::<Vec<_>>(), vec![0, 1]);
        let top = HISTORY as i64 - 1;
        assert_eq!(
            maps.changes(top - 2, top + 5).map(|(sample, _)| sample).collect::<Vec<_>>(),
            vec![top - 1, top]
        );
        // Adjacent ends: the interval is open, so neither entry qualifies.
        assert_eq!(maps.changes(4000, 4001).count(), 0);
    }

    /// Every document mutation reaches the audio bank at the next adoption,
    /// including a capture after the first and a delete that must leave the
    /// other shape intact.
    #[test]
    fn document_mutations_reach_the_audio_bank_without_changing_slot_identity() {
        let params = crate::HarmonigraphParams::default();
        let config = ConfigReducer::default().resolved();
        let mut second = LatticeMap::default();
        assert!(second.replace(LatticePos::new(4, 0, 0)), "slot 1 must differ from the default");
        let mut maps = AudioMaps::new(&params);
        let adopted = |maps: &mut AudioMaps| {
            maps.adopt(config, Seed::read(&params));
            *maps.bank.peek_output_buffer()
        };
        let bank = adopted(&mut maps);
        assert_eq!((bank[0], bank[1]), (Some(LatticeMap::default()), None));

        assert_eq!(params.maps.write(|d| d.capture(second, "Passage".into())), Some(1));
        assert_eq!(adopted(&mut maps)[1], Some(second), "the fixture never reached a second slot");
        params.maps.write(|d| d.rename(1, "Renamed".into()));
        params.maps.write(|d| d.move_by(1, MapMove::Earlier));
        assert!(!maps.bank.updated(), "a rename and a move publish nothing to audio");
        let bank = adopted(&mut maps);
        assert_eq!((bank[0], bank[1]), (Some(LatticeMap::default()), Some(second)));

        params.maps.write(|d| d.delete(0));
        let bank = adopted(&mut maps);
        assert_eq!(bank[0], None, "a delete must reach the audio thread's bank");
        assert_eq!(bank[1], Some(second));
    }

    /// An automation value a hair below a half step selects the slot the
    /// table holds for it, which `f64` rounding of the host value would not
    /// (#1814): the timeline and what the host reads back agree.
    #[test]
    fn the_observer_decodes_automation_as_the_table_does() {
        let params = crate::HarmonigraphParams::default();
        let mut maps = AudioMaps::new(&params);
        let config = ConfigReducer::default().resolved();
        let id = harmonigraph_clap::wrapper::hash_param_id(Row::LatticeMap.id());
        let value: f64 = 2.4999999;
        assert_eq!(value.round(), 2.0);
        assert_eq!(params.table.decode(id, value), Some((Row::LatticeMap.index(), 3.0)));
        let parameter = InputValue::Parameter { id, value, modulation: false };
        maps.observe(Input { sample: 0, value: parameter }, config);
        assert_eq!(maps.playback.selected, 3);
    }

    /// The map's pending label waits on what the parameters request, not on
    /// the map following moves: with following moving it every frame, an
    /// unadopted request still lights the label after the wait (#1814 Q2).
    #[test]
    fn the_map_label_waits_on_the_request_while_following_moves_the_map() {
        use harmonigraph_ui::{PendingLabel, PENDING_AFTER};
        let params = crate::HarmonigraphParams::default();
        params.set(Row::TuningEngine, TuningEngine::LatticeMap.index() as f32);
        params.set(Row::MapFollow, Follow::Thirds.index() as f32);
        params.set(Row::MapFifths, 3.0);
        let start = std::time::Instant::now();
        let mut label = PendingLabel::default();
        let frames = 1 + (PENDING_AFTER.as_millis() / 16) as i32;
        let shown: Vec<bool> = (0..=frames)
            .map(|frame| {
                // The Hub moves the map a third every frame.
                let moved = LatticePos::new(0, frame, 0);
                params.map_followed.store(pack(moved), std::sync::atomic::Ordering::Release);
                let view = view(&params);
                assert_eq!(view.followed, moved, "the fixture moves the map");
                let at = start + std::time::Duration::from_millis(16 * frame as u64);
                label.shown(view.unadopted, at)
            })
            .collect();
        assert!(!shown[0] && shown[frames as usize], "{shown:?}");
    }

    #[test]
    fn changes_over_an_empty_window_yields_nothing() {
        let maps = filled();
        // `record` asks for `changes(start, start)` whenever a segment is zero
        // frames long, and `range` panics on a backwards bound.
        assert_eq!(maps.changes(4000, 4000).count(), 0);
        assert_eq!(maps.changes(4000, 3990).count(), 0);
    }
}
