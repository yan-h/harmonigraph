//! Editor-owned musical document, bounded audio snapshot and sample-indexed
//! next-attack state. No editor window is needed for restore or automation.
use harmonigraph_core::configuration::ResolvedConfig;
use harmonigraph_core::lattice_map::{LatticeMap, TuningEngine};
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

pub fn view(params: &crate::HarmonigraphParams) -> MapView {
    let document = params.maps.read();
    let mut editor = params.map_editor.lock();
    if let Some(mailbox) = params.configuration.get() {
        editor.restore(mailbox.accepted_restore.load(std::sync::atomic::Ordering::Acquire));
    }
    let adopted = *params.map_playback.lock();
    let selected = params.map.value().clamp(0, 127) as usize;
    let playback = MapPlayback {
        engine: engine(params.tuning_engine.value()),
        selected,
        map: editor.working.or_else(|| document.map(selected)),
        audition: editor.working.is_some(),
    };
    MapView {
        playback,
        pending: playback != adopted,
        names: document.names(),
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
        MapEdit::Position(position) => {
            let mut editor = params.map_editor.lock();
            if let Some(mut map) = editor.working {
                map.position = position;
                editor.change(map);
            }
        }
        MapEdit::Replace(destination) => {
            let mut editor = params.map_editor.lock();
            if editor.edit_shape {
                if let Some(mut map) = editor.working {
                    map.replace(destination);
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
        MapEdit::Rename(id, name) => {
            if let Some(map) = params.maps.write().slots.get_mut(id) {
                map.name = name;
            }
        }
        MapEdit::Delete(id) => {
            if let Some(map) = params.maps.write().slots.get_mut(id) {
                map.deleted = true;
            }
        }
        MapEdit::MoveEarlier(id) => {
            let mut doc = params.maps.write();
            let mut order: Vec<_> = doc.names().into_iter().map(|(id, _)| id).collect();
            if let Some(index) = order.iter().position(|&item| item == id) {
                if index > 0 {
                    order.swap(index, index - 1);
                }
            }
            doc.order = order;
        }
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
        }
    }
    pub fn seed(&mut self, mode: i32, map: i32, restore_id: u64) {
        self.restore_id = restore_id;
        self.seed_mode = mode;
        self.seed_map = map;
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
        self.playback.map = self.working.or(self.bank[self.playback.selected]);
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
                return;
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
    pub fn changes(&self, start: i64, end: i64) -> impl Iterator<Item = (i64, AttackState)> + '_ {
        self.history
            .iter()
            .filter(move |entry| entry.sample > start && entry.sample < end)
            .map(|entry| (entry.sample, entry.state))
    }
    pub fn restored(&mut self) {
        self.working = None;
    }
    pub fn reset(&mut self) {
        self.history.clear();
    }
}
