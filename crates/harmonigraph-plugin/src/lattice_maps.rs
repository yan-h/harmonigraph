//! Editor-owned musical document, bounded audio snapshot and sample-indexed
//! next-attack state. No editor window is needed for restore or automation.
use harmonigraph_core::configuration::ResolvedConfig;
use harmonigraph_core::lattice_map::COORDINATE_LIMIT;
use harmonigraph_core::lattice_map::{LatticeMap, TuningEngine};
use harmonigraph_core::LatticePos;
use harmonigraph_ui::lattice_maps::*;
use nice_plug::prelude::*;
use nice_plug::wrapper::clap::configuration::{ConfigurationBoundary, InputValue, OwnedInput};
use parking_lot::{Mutex, RwLock};
use std::collections::VecDeque;
use std::sync::Arc;

fn engine(value: i32) -> TuningEngine {
    match value {
        0 => TuningEngine::Off,
        2 => TuningEngine::LatticeMap,
        _ => TuningEngine::Adaptive,
    }
}

pub fn offset(params: &crate::HarmonigraphParams) -> LatticePos {
    LatticePos::new(
        params.map_fifths.unmodulated_plain_value(),
        params.map_thirds.unmodulated_plain_value(),
        params.map_sevenths.unmodulated_plain_value(),
    )
}

fn translated(mut shape: LatticeMap, offset: LatticePos) -> LatticeMap {
    shape.position = offset;
    shape
}

pub fn view(params: &crate::HarmonigraphParams) -> MapView {
    let document = params.maps.read();
    let mut editor = params.map_editor.lock();
    if let Some(mailbox) = params.configuration.get() {
        editor.restore(mailbox.accepted_restore.load(std::sync::atomic::Ordering::Acquire));
    }
    let adopted = *params.map_playback.lock();
    let selected = params.map.value().clamp(0, 127) as usize;
    let offset = offset(params);
    let playback = MapPlayback {
        engine: engine(params.tuning_engine.value()),
        selected,
        offset,
        map: editor
            .working
            .or_else(|| document.map(selected))
            .map(|shape| translated(shape, offset)),
        audition: editor.working.is_some(),
    };
    MapView {
        playback,
        pending: playback != adopted,
        names: editor.names(&document),
        working: editor.working,
        edit_shape: editor.edit_shape,
        can_undo: !editor.undo.is_empty(),
        full: document.slots.len() >= MAP_CAPACITY,
    }
}

pub fn edit(params: &crate::HarmonigraphParams, setter: &ParamSetter<'_>, edit: MapEdit) {
    let set = |param: &IntParam, value| {
        setter.begin_set_parameter(param);
        setter.set_parameter(param, value);
        setter.end_set_parameter(param);
    };
    let axis_param = |axis| match axis {
        MapAxis::Fifths => &params.map_fifths,
        MapAxis::Thirds => &params.map_thirds,
        MapAxis::Sevenths => &params.map_sevenths,
    };
    let document_edit = matches!(
        &edit,
        MapEdit::Capture(_) | MapEdit::Rename(..) | MapEdit::MoveEarlier(_) | MapEdit::Delete(_)
    );
    match edit {
        MapEdit::Engine(mode) => set(
            &params.tuning_engine,
            match mode {
                TuningEngine::Off => 0,
                TuningEngine::Adaptive => 1,
                TuningEngine::LatticeMap => 2,
            },
        ),
        MapEdit::Select(id) if id < MAP_CAPACITY => set(&params.map, id as i32),
        MapEdit::Select(_) => {}
        MapEdit::Audition => {
            let map = params.maps.read().map(params.map.value() as usize).unwrap_or_default();
            let mut editor = params.map_editor.lock();
            if editor.working.is_none() {
                editor.working = Some(map);
                editor.undo.clear();
            }
        }
        MapEdit::Return => *params.map_editor.lock() = MapEditor::default(),
        MapEdit::EditShape(on) => params.map_editor.lock().edit_shape = on,
        MapEdit::Undo => {
            let mut editor = params.map_editor.lock();
            if let Some(map) = editor.undo.pop() {
                editor.working = Some(map);
            }
        }
        MapEdit::BeginOffset(axis) => setter.begin_set_parameter(axis_param(axis)),
        MapEdit::Offset(axis, value) => setter.set_parameter(axis_param(axis), value),
        MapEdit::EndOffset(axis) => setter.end_set_parameter(axis_param(axis)),
        MapEdit::Replace(destination) => {
            let mut editor = params.map_editor.lock();
            if editor.edit_shape {
                if let Some(mut map) = editor.working {
                    map.position = offset(params);
                    map.replace(destination);
                    map.position = LatticePos::ORIGIN;
                    editor.change(map);
                }
            }
        }
        MapEdit::Capture(name) => {
            let map = params.map_editor.lock().working;
            if let Some(map) = map {
                params.maps.write().capture(map, name);
            }
        }
        // Every document mutation goes through a `MapDocument` method, because
        // each one has to move the revision `MapEditor::names` is keyed on.
        MapEdit::Rename(id, name) => params.maps.write().rename(id, name),
        MapEdit::Delete(id) => params.maps.write().delete(id),
        MapEdit::MoveEarlier(id) => params.maps.write().move_earlier(id),
    }
    if document_edit {
        if let Some(mailbox) = params.configuration.get() {
            mailbox.dirty.store(true, std::sync::atomic::Ordering::Release);
        }
        if let Some(session) = params.session.get() {
            session.request_main();
        }
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

pub struct AudioMaps {
    document: Arc<RwLock<MapDocument>>,
    editor: Arc<Mutex<MapEditor>>,
    published: Arc<Mutex<MapPlayback>>,
    bank: [Option<LatticeMap>; MAP_CAPACITY],
    working: Option<LatticeMap>,
    pub playback: MapPlayback,
    history: VecDeque<Entry>,
    pub boundary: ConfigurationBoundary,
    adopted: bool,
    engine_revision: u64,
    restore_id: u64,
    seed_mode: i32,
    seed_map: i32,
    seed_offset: LatticePos,
}
impl AudioMaps {
    pub fn new(params: &crate::HarmonigraphParams) -> Self {
        Self {
            document: params.maps.clone(),
            editor: params.map_editor.clone(),
            published: params.map_playback.clone(),
            bank: [None; MAP_CAPACITY],
            working: None,
            playback: MapPlayback::default(),
            history: VecDeque::with_capacity(HISTORY),
            boundary: ConfigurationBoundary {
                steady_time: 0,
                frames: 0,
                sample_rate: 44100.0,
                transport_seconds: None,
                playing: false,
            },
            adopted: false,
            engine_revision: 0,
            restore_id: 0,
            seed_mode: 1,
            seed_map: 0,
            seed_offset: LatticePos::ORIGIN,
        }
    }
    pub fn seed(&mut self, mode: i32, map: i32, offset: LatticePos, restore_id: u64) {
        self.restore_id = restore_id;
        self.seed_mode = mode;
        self.seed_map = map;
        self.seed_offset = offset;
    }
    pub fn begin(&mut self, boundary: ConfigurationBoundary) {
        if boundary.steady_time < self.boundary.steady_time + i64::from(self.boundary.frames) {
            self.history.clear();
        }
        self.boundary = boundary;
        self.adopted = false;
    }
    pub fn adopt(&mut self, config: ResolvedConfig) {
        // Copies only fixed geometry, never names or heap storage, on audio.
        if let Some(doc) = self.document.try_read() {
            self.bank = std::array::from_fn(|id| doc.map(id));
        }
        if let Some(mut editor) = self.editor.try_lock() {
            editor.restore(self.restore_id);
            self.working = editor.working;
        }
        self.set_engine(engine(self.seed_mode));
        self.playback.selected = self.seed_map.clamp(0, 127) as usize;
        self.playback.offset = self.seed_offset;
        self.resolve();
        self.adopted = true;
        self.push(self.boundary.steady_time, config);
    }
    fn set_engine(&mut self, engine: TuningEngine) {
        if self.playback.engine != engine {
            self.engine_revision = self.engine_revision.saturating_add(1);
            self.playback.engine = engine;
        }
    }
    fn resolve(&mut self) {
        self.playback.audition = self.working.is_some();
        self.playback.map = self
            .working
            .or(self.bank[self.playback.selected])
            .map(|shape| translated(shape, self.playback.offset));
        if let Some(mut published) = self.published.try_lock() {
            *published = self.playback;
        }
    }
    pub fn observe(&mut self, event: OwnedInput, config: ResolvedConfig) {
        if let InputValue::Parameter { id, value, modulation: false } = event.value {
            if !value.is_finite() {
                return;
            }
            if id == nice_plug::wrapper::hash_param_id("lattice-map") {
                self.playback.selected = (value.round() as i32).clamp(0, 127) as usize;
            } else if id == nice_plug::wrapper::hash_param_id("tuning-engine") {
                self.set_engine(engine(value.round() as i32));
            } else {
                // CLAP stepped values are indices from zero, unlike saved/plain params.
                let step = (value.round() as i32).clamp(0, 2 * COORDINATE_LIMIT) - COORDINATE_LIMIT;
                if id == nice_plug::wrapper::hash_param_id("map-fifths") {
                    self.playback.offset.threes = step;
                } else if id == nice_plug::wrapper::hash_param_id("map-thirds") {
                    self.playback.offset.fives = step;
                } else if id == nice_plug::wrapper::hash_param_id("map-sevenths") {
                    self.playback.offset.sevens = step;
                } else {
                    return;
                }
            }
            self.resolve();
            self.push(event.sample.unwrap_or(self.boundary.steady_time), config);
        }
    }
    pub fn tuning_changed(&mut self, sample: i64, config: ResolvedConfig) {
        if self.adopted {
            self.push(sample, config);
        }
    }
    fn push(&mut self, sample: i64, config: ResolvedConfig) {
        let state =
            AttackState { config, playback: self.playback, engine_revision: self.engine_revision };
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
        if sample >= self.boundary.steady_time + i64::from(self.boundary.frames) {
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
    pub fn restored(&mut self) {
        self.working = None;
    }
    pub fn reset(&mut self) {
        self.history.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use harmonigraph_core::configuration::ConfigReducer;

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

    #[test]
    fn changes_over_an_empty_window_yields_nothing() {
        let maps = filled();
        // `record` asks for `changes(start, start)` whenever a segment is zero
        // frames long, and `range` panics on a backwards bound.
        assert_eq!(maps.changes(4000, 4000).count(), 0);
        assert_eq!(maps.changes(4000, 3990).count(), 0);
    }
}
