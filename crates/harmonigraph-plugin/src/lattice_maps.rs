//! Editor-owned musical document, bounded audio snapshot and sample-indexed
//! next-attack state. No editor window is needed for restore or automation.
use harmonigraph_core::configuration::ResolvedConfig;
use harmonigraph_core::lattice_map::{Follow, LatticeMap, TuningEngine};
use harmonigraph_core::LatticePos;
use harmonigraph_ui::lattice_maps::*;
use nice_plug::prelude::*;
use nice_plug::wrapper::clap::configuration::{ConfigurationBoundary, InputValue, OwnedInput};
use parking_lot::{Mutex, RwLock};
use std::collections::VecDeque;
use std::sync::Arc;

fn engine(value: i32) -> TuningEngine {
    match value {
        1 => TuningEngine::Adaptive,
        2 => TuningEngine::LatticeMap,
        _ => TuningEngine::Off,
    }
}

fn follow(value: i32) -> Follow {
    match value {
        1 => Follow::Thirds,
        2 => Follow::ThirdsAndFifths,
        _ => Follow::Off,
    }
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
    let following = engine(params.tuning_engine.value()) == TuningEngine::LatticeMap
        && follow(params.map_follow.value()) != Follow::Off;
    if following {
        unpack(params.map_followed.load(std::sync::atomic::Ordering::Acquire))
    } else {
        LatticePos::ORIGIN
    }
}

pub fn offsets(params: &crate::HarmonigraphParams) -> MapOffsets {
    MapOffsets {
        fine: LatticePos::new(
            params.map_fifths.unmodulated_plain_value(),
            params.map_thirds.unmodulated_plain_value(),
            params.map_sevenths.unmodulated_plain_value(),
        ),
        extension: LatticePos::new(
            params.map_fifths_extension.unmodulated_plain_value(),
            params.map_thirds_extension.unmodulated_plain_value(),
            params.map_sevenths_extension.unmodulated_plain_value(),
        ),
    }
}

pub fn offset(params: &crate::HarmonigraphParams) -> LatticePos {
    offsets(params).total()
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
    // An arrow press that has landed, or any other change to the lanes,
    // retires the memo of what the last press sent.
    let lanes = offsets(params);
    if editor.translated.is_some_and(|(read, _)| read != lanes) {
        editor.translated = None;
    }
    let selected = params.map.value().clamp(0, 127) as usize;
    let offsets = offsets(params);
    let offset = offsets.total();
    let playback = MapPlayback {
        engine: engine(params.tuning_engine.value()),
        selected,
        offset,
        map: document.map(selected).map(|shape| translated(shape, offset)),
        follow: follow(params.map_follow.value()),
    };
    // Intent is compared before following is added: the Hub's own movement is
    // not host state waiting for audio to adopt it.
    let pending = playback != adopted;
    let followed = followed(params);
    let shown = MapPlayback {
        map: playback.map.map(|map| translated(map, map.position + followed)),
        ..playback
    };
    MapView {
        playback: shown,
        offsets,
        followed,
        pending,
        names: editor.names(&document),
        edit_shape: editor.edit_shape,
        can_undo: playback.map.is_some() && editor.can_undo(selected),
        full: document.is_full(),
    }
}

pub fn edit(params: &crate::HarmonigraphParams, setter: &ParamSetter<'_>, edit: MapEdit) {
    let set = |param: &IntParam, value| {
        setter.begin_set_parameter(param);
        setter.set_parameter(param, value);
        setter.end_set_parameter(param);
    };
    let axis_param = |axis, lane| match (axis, lane) {
        (MapAxis::Fifths, MapOffsetLane::Fine) => &params.map_fifths,
        (MapAxis::Thirds, MapOffsetLane::Fine) => &params.map_thirds,
        (MapAxis::Sevenths, MapOffsetLane::Fine) => &params.map_sevenths,
        (MapAxis::Fifths, MapOffsetLane::Extension) => &params.map_fifths_extension,
        (MapAxis::Thirds, MapOffsetLane::Extension) => &params.map_thirds_extension,
        (MapAxis::Sevenths, MapOffsetLane::Extension) => &params.map_sevenths_extension,
    };
    let selected = params.map.value().clamp(0, 127) as usize;
    // Whether the saved document changed, decided by each arm from what it
    // did rather than by its variant: a no-op edit must not mark the host
    // project modified.
    let document_changed = match edit {
        MapEdit::Engine(mode) => {
            let value = match mode {
                TuningEngine::Off => 0,
                TuningEngine::Adaptive => 1,
                TuningEngine::LatticeMap => 2,
            };
            set(&params.tuning_engine, value);
            false
        }
        MapEdit::Select(id) => {
            if id < MAP_CAPACITY {
                set(&params.map, id as i32);
            }
            false
        }
        MapEdit::EditShape(on) => {
            params.map_editor.lock().edit_shape = on;
            false
        }
        // The document lock is taken before the editor's, in the order `view`
        // takes them.
        MapEdit::Undo => {
            let mut document = params.maps.write();
            let undone = params.map_editor.lock().undo(selected);
            undone.is_some_and(|map| document.reshape(selected, map))
        }
        MapEdit::BeginOffset(axis, lane) => {
            setter.begin_set_parameter(axis_param(axis, lane));
            false
        }
        MapEdit::Offset(axis, lane, value) => {
            setter.set_parameter(axis_param(axis, lane), value);
            false
        }
        MapEdit::EndOffset(axis, lane) => {
            setter.end_set_parameter(axis_param(axis, lane));
            false
        }
        MapEdit::Translate(step, lane) => {
            let seen = offsets(params);
            let from = match params.map_editor.lock().translated {
                Some((read, sent)) if read == seen => sent,
                _ => seen,
            };
            if let Some(to) = from.translated(step, lane) {
                for ((axis, lane, before), (_, _, after)) in
                    from.lanes().into_iter().zip(to.lanes())
                {
                    if before != after {
                        set(axis_param(axis, lane), after);
                    }
                }
                params.map_editor.lock().translated = Some((seen, to));
            }
            false
        }
        MapEdit::Follow(mode) => {
            let value = match mode {
                Follow::Off => 0,
                Follow::Thirds => 1,
                Follow::ThirdsAndFifths => 2,
            };
            set(&params.map_follow, value);
            false
        }
        MapEdit::Replace(destination) => {
            let mut document = params.maps.write();
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
        }
        MapEdit::Duplicate => {
            let mut document = params.maps.write();
            let copy =
                document.map(selected).zip(document.name(selected).map(|n| format!("{n} copy")));
            let id = copy.and_then(|(map, name)| document.capture(map, name));
            drop(document);
            if let Some(id) = id {
                set(&params.map, id as i32);
            }
            id.is_some()
        }
        // Every document mutation goes through a `MapDocument` method, because
        // each one has to move the revision two derived values are keyed on:
        // `MapEditor::names` here, and `AudioMaps::bank` on the audio thread.
        MapEdit::Rename(id, name) => {
            params.maps.write().rename(id, name);
            true
        }
        MapEdit::Delete(id) => {
            params.maps.write().delete(id);
            true
        }
        MapEdit::MoveEarlier(id) => {
            params.maps.write().move_earlier(id);
            true
        }
    };
    if document_changed {
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
    published: Arc<Mutex<MapPlayback>>,
    followed: Arc<std::sync::atomic::AtomicU64>,
    bank: [Option<LatticeMap>; MAP_CAPACITY],
    /// Which document state `bank` was built from, or `None` before the first
    /// read — see [`AudioMaps::adopt`] for what that key does and does not say.
    bank_revision: Option<Revision>,
    pub playback: MapPlayback,
    history: VecDeque<Entry>,
    pub boundary: ConfigurationBoundary,
    adopted: bool,
    engine_revision: u64,
    seed_mode: i32,
    seed_map: i32,
    seed_offset: MapOffsets,
    seed_follow: i32,
    offsets: MapOffsets,
}
impl AudioMaps {
    pub fn new(params: &crate::HarmonigraphParams) -> Self {
        Self {
            document: params.maps.clone(),
            published: params.map_playback.clone(),
            followed: params.map_followed.clone(),
            bank: [None; MAP_CAPACITY],
            bank_revision: None,
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
            seed_mode: params.tuning_engine.value(),
            seed_map: 0,
            seed_offset: MapOffsets::default(),
            seed_follow: 0,
            offsets: MapOffsets::default(),
        }
    }
    pub fn seed(&mut self, mode: i32, map: i32, offset: MapOffsets, follow: i32) {
        self.seed_mode = mode;
        self.seed_map = map;
        self.seed_offset = offset;
        self.seed_follow = follow;
    }
    /// The Hub's follow offset, for the editor to draw the map where it sounds.
    pub fn publish_followed(&self, offset: LatticePos) {
        self.followed.store(pack(offset), std::sync::atomic::Ordering::Release);
    }
    pub fn begin(&mut self, boundary: ConfigurationBoundary) {
        if boundary.steady_time < self.boundary.steady_time + i64::from(self.boundary.frames) {
            self.history.clear();
        }
        self.boundary = boundary;
        self.adopted = false;
    }
    pub fn adopt(&mut self, config: ResolvedConfig) {
        // Copies only fixed geometry, never names or heap storage, on audio —
        // and only when the document is a state this bank was not built from.
        //
        // The key is [`MapDocument::revision`], the same ticket
        // `MapEditor::names` memoizes against, rather than a second counter
        // beside it. Nothing that decides a slot's geometry can move without
        // moving it: geometry reaches a slot only through `capture` and
        // `reshape`, a slot leaves only through `delete`, all bump it, and a
        // document LOADED
        // from saved state is a whole new value whose revision is minted at
        // deserialization. What the key carries beyond this value is a rename
        // and a reorder, neither of which decides any `map(id)`; each costs one
        // extra rebuild at the next callback, which is bounded by the rate a
        // hand types rather than by the rate the host calls this.
        if let Some(doc) = self.document.try_read() {
            let revision = doc.revision();
            if self.bank_revision != Some(revision) {
                self.bank = std::array::from_fn(|id| doc.map(id));
                self.bank_revision = Some(revision);
            }
        }
        self.set_engine(engine(self.seed_mode));
        self.playback.selected = self.seed_map.clamp(0, 127) as usize;
        self.offsets = self.seed_offset;
        self.playback.offset = self.offsets.total();
        self.set_follow(follow(self.seed_follow));
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
        self.playback.map =
            self.bank[self.playback.selected].map(|shape| translated(shape, self.playback.offset));
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
            } else if id == nice_plug::wrapper::hash_param_id("map-follow") {
                self.set_follow(follow(value.round() as i32));
            } else {
                // CLAP stepped values are indices from zero, unlike saved/plain params.
                let step = (value.round() as i32).clamp(0, 2 * OFFSET_LIMIT) - OFFSET_LIMIT;
                let component = [
                    ("map-fifths", &mut self.offsets.fine.threes),
                    ("map-thirds", &mut self.offsets.fine.fives),
                    ("map-sevenths", &mut self.offsets.fine.sevens),
                    ("map-fifths-extension", &mut self.offsets.extension.threes),
                    ("map-thirds-extension", &mut self.offsets.extension.fives),
                    ("map-sevenths-extension", &mut self.offsets.extension.sevens),
                ]
                .into_iter()
                .find(|(name, _)| id == nice_plug::wrapper::hash_param_id(name));
                let Some((_, component)) = component else { return };
                *component = step;
                self.playback.offset = self.offsets.total();
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

    /// Use document mutations to reach the bank, including a capture after the
    /// initial adoption and a delete that must leave the other shape intact.
    #[test]
    fn document_mutations_reach_the_audio_bank_without_changing_slot_identity() {
        let params = crate::HarmonigraphParams::default();
        let config = ConfigReducer::default().resolved();
        let mut second = LatticeMap::default();
        assert!(second.replace(LatticePos::new(4, 0, 0)), "slot 1 must differ from the default");
        let mut maps = AudioMaps::new(&params);
        maps.adopt(config);
        assert_eq!(maps.bank[0], Some(LatticeMap::default()));
        assert_eq!(maps.bank[1], None);

        assert_eq!(params.maps.write().capture(second, "Passage".into()), Some(1));
        maps.adopt(config);
        assert_eq!(maps.bank[1], Some(second), "the fixture never reached a second slot");
        assert_eq!(maps.bank_revision, Some(params.maps.read().revision()));

        let revision = maps.bank_revision;
        maps.adopt(config);
        assert_eq!(maps.bank_revision, revision);
        assert_eq!(maps.bank[1], Some(second));

        params.maps.write().rename(1, "Renamed".into());
        maps.adopt(config);
        assert_ne!(maps.bank_revision, revision);
        assert_eq!(maps.bank_revision, Some(params.maps.read().revision()));
        let revision = maps.bank_revision;
        params.maps.write().move_earlier(1);
        maps.adopt(config);
        assert_ne!(maps.bank_revision, revision);
        assert_eq!(maps.bank_revision, Some(params.maps.read().revision()));
        assert_eq!(maps.bank[0], Some(LatticeMap::default()));
        assert_eq!(maps.bank[1], Some(second));

        params.maps.write().delete(0);
        maps.adopt(config);
        assert_eq!(maps.bank[0], None, "a delete must reach the audio thread's bank");
        assert_eq!(maps.bank[1], Some(second));
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
