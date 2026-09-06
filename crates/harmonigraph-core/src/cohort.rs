//! Canonical traversal of one caller-frozen, same-sample cohort.
//!
//! The caller proves membership, lease/epoch identity and complete input coverage,
//! freezes configuration, and pins original payloads and captured target spans.
//! This module proves only local metadata validity and dependency order. It owns
//! no policy, context, plans, output schedule or capture-time address resolver.

pub const COHORT_EVENTS: usize = 1024;
pub const COHORT_ONSETS: usize = 256;
pub const SOURCE_ORDINALS: u8 = 17;
pub const TARGETS_PER_EVENT: usize = 64;
pub const EVENT_CELL_BYTES: usize = 128;
pub const COHORT_PHASES: usize = COHORT_EVENTS + COHORT_ONSETS;
pub const COHORT_BYTES: usize = 294_912;
pub const ADJACENCY_BYTES: usize = 204_800;
pub const COMPACT_EVENT_BYTES: usize = 56;
pub const TRAVERSAL_BYTES: usize = 32_768;
const WORDS: usize = COHORT_PHASES / 64;
const NONE: u16 = u16::MAX;

/// Ordinal zero is DIRECT. Original input identity survives derived release
/// phases unchanged. Sequence zero is invalid.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct InputId {
    pub source: u8,
    pub sequence: u64,
}

/// Caller-issued freeze generation scoped to one persistent Scratch owner.
/// Begin requires a nonzero generation greater than its previous binding. The
/// owner uses checked generations and never changes metadata/targets under one
/// binding. This is not a membership, lease or clock-completeness proof.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrozenInputId(pub u64);

/// Identity within the event's already validated source lease and clock binding.
/// Host ID is original address metadata, never the lifetime identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Target {
    pub lifetime: u64,
    pub host_note_id: i32,
    pub channel: u8,
    pub key: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Exactly one captured target, including for observed DIRECT input.
    Onset,
    /// Original finite CLAP f64 bits; no pitch conversion occurs here.
    Tuning {
        value_bits: u64,
    },
    Expression,
    Terminal,
    /// Shared-channel MIDI (including pedals). `terminal` describes captured
    /// lifetime termination, not accepted output or pedal neutralization.
    Channel {
        channel: u8,
        terminal: bool,
    },
    /// An event with no lifetime/channel effect, e.g. unrelated system MIDI.
    Independent,
}

impl Kind {
    fn terminal(self) -> bool {
        matches!(self, Self::Terminal | Self::Channel { terminal: true, .. })
    }
}

/// Stable index in the caller's immutable captured-reference view.
/// This is not a pointer or a runtime lifetime identity.
pub type TargetHandle = u32;
pub const NO_TARGET: TargetHandle = u32::MAX;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TargetSpan {
    pub first: TargetHandle,
    pub len: u8,
}

impl Default for TargetSpan {
    fn default() -> Self {
        Self { first: NO_TARGET, len: 0 }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct TargetLink {
    pub target: Target,
    pub next: TargetHandle,
}

/// The hub must own/pin a captured view for this borrow's entire lifetime.
/// Never implement this by querying another audio owner's mutable sounding set.
/// One lookup must be bounded O(1), allocation-free and side-effect-free. A linked
/// reference pool needs no contiguous or sorted per-event copy; traversal sorts
/// at most ONE event's targets into its own fixed scratch. Handles may be shared
/// only when they denote the same immutable chain. Source/Hub transfer and safe
/// retention of this view are integration obligations, not supplied by this trait.
pub trait TargetAccess {
    fn get(&self, source: u8, handle: TargetHandle) -> Option<TargetLink>;
}

/// Compact original-input metadata, not a second performance payload owner.
/// Targets were captured at original input position. Their order is arbitrary.
/// Empty sets stay empty; neither wildcards nor host IDs are re-resolved.
#[derive(Clone, Copy, Debug)]
pub struct Event {
    pub id: InputId,
    pub sample: i64,
    pub kind: Kind,
    pub targets: TargetSpan,
    /// At most one old same-key lifetime captured for a replacement onset.
    /// Its virtual release phase precedes this input's onset without consuming
    /// another original-input slot. Empty for every other kind of event.
    pub replaced: TargetSpan,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidBinding,
    CohortInProgress,
    TooManyEvents,
    TooManyOnsets,
    TooManyTargets { event: usize },
    InvalidMetadata { event: usize },
    InvalidTarget { event: usize },
    DuplicateTarget { event: usize },
    InvalidTargetSpan { event: usize },
    DuplicateInput { first: usize, second: usize },
    InconsistentLifetime { first: usize, second: usize },
    DuplicateOnset { first: usize, second: usize },
    TargetBeforeOnset { first: usize, second: usize },
    Cycle,
    NoSelectedEvent,
    InvalidReplacement { event: usize },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Event,
    TunerOnset,
    ObservedDirectOnset,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InitialTuning {
    pub event_index: usize,
    pub id: InputId,
    pub value_bits: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventPhase {
    ReplacedRelease,
    Original,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selected {
    pub event_index: usize,
    pub id: InputId,
    pub role: Role,
    pub phase: EventPhase,
    pub initial_tuning: Option<InitialTuning>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Progress {
    /// Work slice spent; all input/visited/remaining state is still owned.
    Pending,
    /// Stable until commit; repeated advance calls do not choose another event.
    Event(Selected),
    Complete {
        vertices: usize,
        original_inputs: usize,
    },
}

/// Executed structural work, not elapsed time or a callback/WCET guarantee.
/// Each advance unit increments exactly one of the structural counters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Work {
    pub prepared_inputs: u64,
    pub cleared_words: u64,
    pub validated_nodes: u64,
    pub validated_targets: u64,
    pub node_pairs: u64,
    /// Captured-target reads after validation (one O(1) provider lookup each).
    pub target_reads: u64,
    /// One insertion-sort comparison/shift or binary-search comparison.
    pub target_steps: u64,
    pub finalized_nodes: u64,
    pub selection_visits: u64,
    pub retired_words: u64,
    pub retired_edges: u64,
    pub built_edges: u64,
    /// Equal immutable nonempty spans on two non-onsets prove an edge without
    /// another cross-set address comparison. Every row is still validated.
    pub identical_target_pairs: u64,
    pub committed_nodes: u64,
}

impl Work {
    pub fn units(self) -> u64 {
        self.prepared_inputs
            + self.cleared_words
            + self.validated_nodes
            + self.validated_targets
            + self.node_pairs
            + self.target_reads
            + self.target_steps
            + self.finalized_nodes
            + self.selection_visits
            + self.retired_words
            + self.retired_edges
    }
}

/// Persistent owned graph AND cursor. Preallocate once outside processing.
/// No borrowed fields, owning heap handles or destructors.
/// Inactive adjacency rows are never read; each build clears every active row.
pub struct Scratch {
    state: Cursor,
    edges: [[u64; WORDS]; COHORT_PHASES],
    degree: [u16; COHORT_PHASES],
    tuning: [u16; COHORT_PHASES],
    terminal: [u16; COHORT_PHASES],
    channels: [u16; COHORT_PHASES],
    visited: [bool; COHORT_PHASES],
    release_parent: [u16; COHORT_ONSETS],
    release_vertex: [u16; COHORT_EVENTS],
    onset: [Target; COHORT_ONSETS],
    onset_slot: [u16; COHORT_PHASES],
    sorted: [Target; TARGETS_PER_EVENT],
    keys: [u128; 16],
}

impl Default for Scratch {
    fn default() -> Self {
        Self {
            state: Cursor::unbound(FrozenInputId(0)),
            edges: [[0; WORDS]; COHORT_PHASES],
            degree: [0; COHORT_PHASES],
            tuning: [NONE; COHORT_PHASES],
            terminal: [NONE; COHORT_PHASES],
            channels: [0; COHORT_PHASES],
            visited: [false; COHORT_PHASES],
            release_parent: [0; COHORT_ONSETS],
            release_vertex: [NONE; COHORT_EVENTS],
            onset: [EMPTY_TARGET; COHORT_ONSETS],
            onset_slot: [NONE; COHORT_PHASES],
            sorted: [EMPTY_TARGET; TARGETS_PER_EVENT],
            keys: [0; 16],
        }
    }
}

const EMPTY_TARGET: Target = Target { lifetime: 0, host_note_id: -1, channel: 0, key: 0 };

#[derive(Clone, Copy)]
enum Phase {
    Unbound,
    Prepare(usize),
    Clear(usize),
    Validate { node: usize, target: Option<(usize, TargetHandle)> },
    Row { i: usize, handle: TargetHandle, loaded: usize },
    Insert { i: usize, value: Target, next: TargetHandle, loaded: usize, position: usize },
    Pair { i: usize, j: usize },
    Targets { i: usize, j: usize, handle: TargetHandle, read: usize, edge: bool },
    Search { i: usize, j: usize, link: TargetLink, read: usize, lo: usize, hi: usize, edge: bool },
    Finalize(usize),
    Select { cursor: usize, best: Option<usize> },
    Offered(usize),
    Retire { node: usize, word: usize, bits: u64 },
    Complete,
    Failed(Error),
}

struct Cursor {
    binding: FrozenInputId,
    inputs: usize,
    sample: i64,
    phase: Phase,
    onsets: usize,
    vertices: usize,
    committed: usize,
    work: Work,
}

impl Cursor {
    fn unbound(binding: FrozenInputId) -> Self {
        Self {
            binding,
            inputs: 0,
            sample: 0,
            phase: Phase::Unbound,
            onsets: 0,
            vertices: 0,
            committed: 0,
            work: Work::default(),
        }
    }
}

impl Scratch {
    /// Explicitly discard persistent traversal state after the owner has handled
    /// its external prospective effects. The last generation remains reserved;
    /// the next begin needs a strictly newer caller-issued binding. Matrix clear
    /// is charged incrementally by the next build, not hidden in this operation.
    pub fn discard(&mut self) {
        self.state = Cursor::unbound(self.state.binding);
    }
}

/// Ephemeral access to persistent traversal state. Drop at callback return and
/// use resume next callback: all cursors, work and offered/committed ownership
/// live in Scratch, which can move with its serialized owner between threads.
/// The caller pins its immutable input subowner across these ephemeral borrows.
/// Incorporate each selected event into prospective context/history before
/// commit. None of these operations acknowledges actual downstream output.
/// No output/policy call is automatic, and reset/discard cannot roll back the
/// external effects the caller already incorporated.
pub struct Cohort<'events, 'scratch> {
    events: &'events [Event],
    targets: &'events dyn TargetAccess,
    scratch: &'scratch mut Scratch,
}

impl<'events, 'scratch> Cohort<'events, 'scratch> {
    pub fn begin(
        binding: FrozenInputId,
        sample: i64,
        events: &'events [Event],
        targets: &'events dyn TargetAccess,
        scratch: &'scratch mut Scratch,
    ) -> Result<Self, Error> {
        if events.len() > COHORT_EVENTS {
            return Err(Error::TooManyEvents);
        }
        let replaceable = match scratch.state.phase {
            Phase::Unbound | Phase::Complete => true,
            Phase::Failed(_) => scratch.state.committed == 0,
            _ => false,
        };
        if !replaceable {
            return Err(Error::CohortInProgress);
        }
        if binding.0 == 0 || binding.0 <= scratch.state.binding.0 {
            return Err(Error::InvalidBinding);
        }
        scratch.state = Cursor {
            binding,
            inputs: events.len(),
            sample,
            phase: if events.is_empty() { Phase::Complete } else { Phase::Prepare(0) },
            onsets: 0,
            vertices: events.len(),
            committed: 0,
            work: Work::default(),
        };
        Ok(Self { events, targets, scratch })
    }

    /// Reborrow the SAME frozen input owner after a callback return. The owner
    /// must keep every metadata cell/target link immutable under this generation;
    /// changing data while reusing a matching ID violates the caller contract.
    /// Mismatches leave cursor, work and any offered event completely intact.
    pub fn resume(
        binding: FrozenInputId,
        sample: i64,
        events: &'events [Event],
        targets: &'events dyn TargetAccess,
        scratch: &'scratch mut Scratch,
    ) -> Result<Self, Error> {
        if matches!(scratch.state.phase, Phase::Unbound)
            || binding != scratch.state.binding
            || sample != scratch.state.sample
            || events.len() != scratch.state.inputs
        {
            return Err(Error::InvalidBinding);
        }
        Ok(Self { events, targets, scratch })
    }

    pub fn work(&self) -> Work {
        self.scratch.state.work
    }
    pub fn committed(&self) -> usize {
        self.scratch.state.committed
    }
    /// Phase count is final only after construction has offered its first event.
    pub fn remaining(&self) -> usize {
        self.scratch.state.vertices - self.scratch.state.committed
    }
    pub fn was_committed(&self, index: usize) -> Option<bool> {
        (index < self.events.len()).then(|| {
            // Old scratch bits have no meaning until this build reaches traversal.
            self.scratch.state.committed != 0 && self.scratch.visited[index]
        })
    }

    /// Restart this same immutable cohort, for an owner that has separately
    /// discarded/rebuilt its prospective effects. Clearing remains incremental.
    pub fn reset(&mut self) {
        self.scratch.state.phase =
            if self.events.is_empty() { Phase::Complete } else { Phase::Prepare(0) };
        self.scratch.state.onsets = 0;
        self.scratch.state.vertices = self.events.len();
        self.scratch.state.committed = 0;
        self.scratch.state.work = Work::default();
    }

    pub fn advance(&mut self, budget: usize) -> Result<Progress, Error> {
        for _ in 0..budget {
            if let Some(result) = self.progress() {
                return result;
            }
            if let Err(error) = self.tick() {
                self.scratch.state.phase = Phase::Failed(error);
                return Err(error);
            }
        }
        self.progress().unwrap_or(Ok(Progress::Pending))
    }

    /// Acknowledge exactly the offered event after incorporating its effects.
    /// Retirement of successor degrees is charged by subsequent advance calls.
    pub fn commit(&mut self) -> Result<(), Error> {
        let Phase::Offered(node) = self.scratch.state.phase else {
            return Err(Error::NoSelectedEvent);
        };
        self.scratch.visited[node] = true;
        self.scratch.state.committed += 1;
        self.scratch.state.work.committed_nodes += 1;
        self.scratch.state.phase = Phase::Retire { node, word: 0, bits: 0 };
        Ok(())
    }

    fn progress(&self) -> Option<Result<Progress, Error>> {
        Some(match self.scratch.state.phase {
            Phase::Offered(index) => Ok(Progress::Event(self.selected(index))),
            Phase::Complete => Ok(Progress::Complete {
                vertices: self.scratch.state.vertices,
                original_inputs: self.events.len(),
            }),
            Phase::Failed(error) => Err(error),
            _ => return None,
        })
    }

    fn selected(&self, index: usize) -> Selected {
        let event = self.event(index);
        let initial_tuning = (self.scratch.tuning[index] != NONE).then(|| {
            let event_index = self.parent(usize::from(self.scratch.tuning[index]));
            let tuning = self.event(event_index);
            let Kind::Tuning { value_bits } = tuning.kind else { unreachable!() };
            InitialTuning { event_index, id: tuning.id, value_bits }
        });
        Selected {
            event_index: self.parent(index),
            id: event.id,
            initial_tuning,
            phase: if index < self.events.len() {
                EventPhase::Original
            } else {
                EventPhase::ReplacedRelease
            },
            role: if event.kind != Kind::Onset {
                Role::Event
            } else if event.id.source == 0 {
                Role::ObservedDirectOnset
            } else {
                Role::TunerOnset
            },
        }
    }

    fn parent(&self, index: usize) -> usize {
        if index < self.events.len() {
            index
        } else {
            usize::from(self.scratch.release_parent[index - self.events.len()])
        }
    }

    fn event(&self, index: usize) -> Event {
        let mut event = self.events[self.parent(index)];
        if index >= self.events.len() {
            event.kind = Kind::Terminal;
            event.targets = event.replaced;
            event.replaced = TargetSpan::default();
        }
        event
    }

    fn position(&self, index: usize) -> (InputId, bool) {
        (self.event(index).id, index < self.events.len())
    }

    /// Whether a replacement release has been incorporated, separately from
    /// the original event's commit bit. None means no such captured phase.
    pub fn was_replacement_committed(&self, index: usize) -> Option<bool> {
        let event = self.events.get(index)?;
        if event.replaced.len == 0 {
            return None;
        }
        Some(
            self.scratch.state.committed != 0
                && self.scratch.visited[usize::from(self.scratch.release_vertex[index])],
        )
    }

    fn next_pair(&mut self, i: usize, j: usize) {
        self.scratch.state.phase = if j + 1 < self.scratch.state.vertices {
            Phase::Pair { i, j: j + 1 }
        } else if i + 1 < self.scratch.state.vertices {
            self.start_row(i + 1)
        } else {
            Phase::Finalize(0)
        };
    }

    fn start_row(&mut self, i: usize) -> Phase {
        self.scratch.keys = [0; 16];
        Phase::Row { i, handle: self.event(i).targets.first, loaded: 0 }
    }

    fn row_done(&mut self, i: usize) {
        self.scratch.state.phase = if i + 1 < self.scratch.state.vertices {
            Phase::Pair { i, j: i + 1 }
        } else {
            Phase::Finalize(0)
        };
    }

    fn finish_pair(&mut self, i: usize, j: usize, edge: bool) {
        if edge {
            let (early, late) = self.ordered_pair(i, j);
            self.edge(early, late);
        }
        self.next_pair(i, j);
    }

    fn ordered_pair(&self, i: usize, j: usize) -> (usize, usize) {
        if self.position(i) < self.position(j) {
            (i, j)
        } else {
            (j, i)
        }
    }

    fn target(&self, node: usize, handle: TargetHandle, index: usize) -> Result<TargetLink, Error> {
        let event = self.event(node);
        let link = self
            .targets
            .get(event.id.source, handle)
            .ok_or(Error::InvalidTargetSpan { event: node })?;
        if (link.next != NO_TARGET) != (index + 1 < usize::from(event.targets.len)) {
            return Err(Error::InvalidTargetSpan { event: node });
        }
        Ok(link)
    }

    fn edge(&mut self, early: usize, late: usize) {
        self.scratch.edges[early][late / 64] |= 1 << (late % 64);
        self.scratch.degree[late] += 1;
        self.scratch.state.work.built_edges += 1;
    }

    fn earliest(&self, previous: u16, next: usize) -> u16 {
        if previous == NONE || self.event(next).id < self.event(usize::from(previous)).id {
            next as u16
        } else {
            previous
        }
    }

    fn order(&self, index: usize) -> (bool, u8, u8, InputId, bool) {
        let event = self.event(index);
        if event.kind == Kind::Onset {
            {
                let target = self.scratch.onset[usize::from(self.scratch.onset_slot[index])];
                (true, target.key, target.channel, event.id, true)
            }
        } else {
            (false, 0, 0, event.id, index < self.events.len())
        }
    }

    fn tick(&mut self) -> Result<(), Error> {
        let n = self.scratch.state.vertices;
        match self.scratch.state.phase {
            Phase::Prepare(index) => {
                self.scratch.state.work.prepared_inputs += 1;
                let event = self.events[index];
                if event.kind == Kind::Onset {
                    self.scratch.state.onsets += 1;
                    if self.scratch.state.onsets > COHORT_ONSETS {
                        return Err(Error::TooManyOnsets);
                    }
                }
                if (event.replaced.first != NO_TARGET) != (event.replaced.len != 0)
                    || event.replaced.len > 1
                    || (event.replaced.len != 0 && event.kind != Kind::Onset)
                {
                    return Err(Error::InvalidReplacement { event: index });
                }
                if event.replaced.len != 0 {
                    self.scratch.release_parent[self.scratch.state.vertices - self.events.len()] =
                        index as u16;
                    self.scratch.release_vertex[index] = self.scratch.state.vertices as u16;
                    self.scratch.state.vertices += 1;
                } else {
                    self.scratch.release_vertex[index] = NONE;
                }
                self.scratch.state.phase = if index + 1 < self.events.len() {
                    Phase::Prepare(index + 1)
                } else {
                    self.scratch.state.onsets = 0;
                    Phase::Clear(0)
                };
            }
            Phase::Clear(index) => {
                self.scratch.edges[index / WORDS][index % WORDS] = 0;
                self.scratch.state.work.cleared_words += 1;
                self.scratch.state.phase = if index + 1 == n * WORDS {
                    Phase::Validate { node: 0, target: None }
                } else {
                    Phase::Clear(index + 1)
                };
            }
            Phase::Validate { node, target } => {
                let event = self.event(node);
                if let Some((index, handle)) = target {
                    self.scratch.state.work.validated_targets += 1;
                    let link = self.target(node, handle, index)?;
                    let value = link.target;
                    if value.lifetime == 0
                        || value.channel >= 16
                        || value.key >= 128
                        || value.host_note_id < -1
                        || matches!(event.kind, Kind::Channel { channel, .. } if channel != value.channel)
                    {
                        return Err(Error::InvalidTarget { event: node });
                    }
                    if node >= self.events.len() {
                        let parent = self.parent(node);
                        let onset =
                            self.scratch.onset[usize::from(self.scratch.onset_slot[parent])];
                        if value.channel != onset.channel
                            || value.key != onset.key
                            || value.lifetime == onset.lifetime
                        {
                            return Err(Error::InvalidReplacement { event: parent });
                        }
                    }
                    self.scratch.channels[node] |= 1 << value.channel;
                    if event.kind == Kind::Onset {
                        self.scratch.onset[usize::from(self.scratch.onset_slot[node])] = value;
                    }
                    if link.next != NO_TARGET {
                        let next = link.next;
                        self.scratch.state.phase =
                            Phase::Validate { node, target: Some((index + 1, next)) };
                        return Ok(());
                    }
                } else {
                    self.scratch.state.work.validated_nodes += 1;
                    self.scratch.degree[node] = 0;
                    self.scratch.tuning[node] = NONE;
                    self.scratch.terminal[node] = NONE;
                    self.scratch.visited[node] = false;
                    self.scratch.channels[node] = 0;
                    self.scratch.onset_slot[node] = NONE;
                    if event.id.source >= SOURCE_ORDINALS
                        || event.id.sequence == 0
                        || event.sample != self.scratch.state.sample
                        || (event.kind == Kind::Onset && event.targets.len != 1)
                        || (event.kind == Kind::Independent && event.targets.len != 0)
                        || matches!(event.kind, Kind::Channel { channel, .. } if channel >= 16)
                        || matches!(event.kind, Kind::Tuning { value_bits } if !f64::from_bits(value_bits).is_finite())
                    {
                        return Err(Error::InvalidMetadata { event: node });
                    }
                    if usize::from(event.targets.len) > TARGETS_PER_EVENT {
                        return Err(Error::TooManyTargets { event: node });
                    }
                    if (event.targets.first != NO_TARGET) != (event.targets.len != 0) {
                        return Err(Error::InvalidTargetSpan { event: node });
                    }
                    if event.kind == Kind::Onset {
                        self.scratch.onset_slot[node] = self.scratch.state.onsets as u16;
                        self.scratch.state.onsets += 1;
                        if self.scratch.state.onsets > COHORT_ONSETS {
                            return Err(Error::TooManyOnsets);
                        }
                    }
                    if let Kind::Channel { channel, .. } = event.kind {
                        self.scratch.channels[node] = 1 << channel;
                    }
                    if event.targets.first != NO_TARGET {
                        let first = event.targets.first;
                        self.scratch.state.phase =
                            Phase::Validate { node, target: Some((0, first)) };
                        return Ok(());
                    }
                }
                self.scratch.state.phase = if node + 1 < n {
                    Phase::Validate { node: node + 1, target: None }
                } else {
                    self.start_row(0)
                };
            }
            Phase::Row { i, handle, loaded } => {
                self.scratch.state.work.target_reads += 1;
                if handle != NO_TARGET {
                    let link = self.target(i, handle, loaded)?;
                    self.scratch.keys[usize::from(link.target.channel)] |= 1 << link.target.key;
                    self.scratch.state.phase = Phase::Insert {
                        i,
                        value: link.target,
                        next: link.next,
                        loaded,
                        position: loaded,
                    };
                } else {
                    self.row_done(i);
                }
            }
            Phase::Insert { i, value, next, loaded, position } => {
                self.scratch.state.work.target_steps += 1;
                if position > 0 {
                    let previous = self.scratch.sorted[position - 1];
                    if previous.lifetime == value.lifetime {
                        return Err(Error::DuplicateTarget { event: i });
                    }
                    if previous.lifetime > value.lifetime {
                        self.scratch.sorted[position] = previous;
                        self.scratch.state.phase =
                            Phase::Insert { i, value, next, loaded, position: position - 1 };
                        return Ok(());
                    }
                }
                self.scratch.sorted[position] = value;
                if next != NO_TARGET {
                    self.scratch.state.phase = Phase::Row { i, handle: next, loaded: loaded + 1 };
                } else {
                    self.row_done(i);
                }
            }
            Phase::Pair { i, j } => {
                self.scratch.state.work.node_pairs += 1;
                let (early, late) = self.ordered_pair(i, j);
                let a = self.event(early);
                let b = self.event(late);
                if a.id == b.id && self.parent(early) != self.parent(late) {
                    return Err(Error::DuplicateInput {
                        first: self.parent(early),
                        second: self.parent(late),
                    });
                }
                if a.id.source != b.id.source {
                    self.next_pair(i, j);
                    return Ok(());
                }
                // These are the exact same immutable cells, not merely equal
                // channels or intersecting lifetimes. Their address consistency
                // cannot differ; per-row duplicate validation still runs. Onsets
                // use the full path for initial tuning and lifecycle checks.
                if a.kind != Kind::Onset
                    && b.kind != Kind::Onset
                    && a.targets.len != 0
                    && a.targets == b.targets
                {
                    self.scratch.state.work.identical_target_pairs += 1;
                    self.finish_pair(i, j, true);
                    return Ok(());
                }
                let shared = |event: Event| match event.kind {
                    Kind::Channel { channel, .. } => 1_u16 << channel,
                    _ => 0,
                };
                let mut edge = self.parent(early) == self.parent(late)
                    || shared(a) & self.scratch.channels[late] != 0
                    || shared(b) & self.scratch.channels[early] != 0;
                if early == i && a.kind.terminal() && b.kind == Kind::Onset {
                    let onset = self.scratch.onset[usize::from(self.scratch.onset_slot[late])];
                    edge |= self.scratch.keys[usize::from(onset.channel)] & (1 << onset.key) != 0;
                }
                if self.event(j).targets.first != NO_TARGET {
                    let handle = self.event(j).targets.first;
                    self.scratch.state.phase = Phase::Targets { i, j, handle, read: 0, edge };
                } else {
                    self.finish_pair(i, j, edge);
                }
            }
            Phase::Targets { i, j, handle, read, mut edge } => {
                self.scratch.state.work.target_reads += 1;
                let link = self.target(j, handle, read)?;
                let (early, late) = self.ordered_pair(i, j);
                if early == j
                    && self.event(early).kind.terminal()
                    && self.event(late).kind == Kind::Onset
                {
                    let onset = self.scratch.onset[usize::from(self.scratch.onset_slot[late])];
                    edge |= link.target.channel == onset.channel && link.target.key == onset.key;
                }
                self.scratch.state.phase = Phase::Search {
                    i,
                    j,
                    link,
                    read,
                    lo: 0,
                    hi: usize::from(self.event(i).targets.len),
                    edge,
                };
            }
            Phase::Search { i, j, link, read, mut lo, mut hi, mut edge } => {
                self.scratch.state.work.target_steps += 1;
                let (early, late) = self.ordered_pair(i, j);
                if lo < hi {
                    let mid = lo + (hi - lo) / 2;
                    let found = self.scratch.sorted[mid];
                    match found.lifetime.cmp(&link.target.lifetime) {
                        std::cmp::Ordering::Less => lo = mid + 1,
                        std::cmp::Ordering::Greater => hi = mid,
                        std::cmp::Ordering::Equal => {
                            if found != link.target {
                                return Err(Error::InconsistentLifetime {
                                    first: early,
                                    second: late,
                                });
                            }
                            let left = self.event(early);
                            let right = self.event(late);
                            if right.kind == Kind::Onset {
                                return Err(if left.kind == Kind::Onset {
                                    Error::DuplicateOnset { first: early, second: late }
                                } else {
                                    Error::TargetBeforeOnset { first: early, second: late }
                                });
                            }
                            edge = true;
                            if left.kind == Kind::Onset {
                                if matches!(right.kind, Kind::Tuning { .. }) {
                                    self.scratch.tuning[early] =
                                        self.earliest(self.scratch.tuning[early], late);
                                }
                                if right.kind.terminal() {
                                    self.scratch.terminal[early] =
                                        self.earliest(self.scratch.terminal[early], late);
                                }
                            }
                            lo = hi;
                        }
                    }
                    if lo < hi {
                        self.scratch.state.phase = Phase::Search { i, j, link, read, lo, hi, edge };
                        return Ok(());
                    }
                }
                if link.next != NO_TARGET {
                    let handle = link.next;
                    self.scratch.state.phase =
                        Phase::Targets { i, j, handle, read: read + 1, edge };
                } else {
                    self.finish_pair(i, j, edge);
                }
            }
            Phase::Finalize(index) => {
                self.scratch.state.work.finalized_nodes += 1;
                let tuning = self.scratch.tuning[index];
                let terminal = self.scratch.terminal[index];
                if tuning != NONE
                    && terminal != NONE
                    && self.event(usize::from(tuning)).id > self.event(usize::from(terminal)).id
                {
                    self.scratch.tuning[index] = NONE;
                }
                self.scratch.state.phase = if index + 1 < n {
                    Phase::Finalize(index + 1)
                } else {
                    Phase::Select { cursor: 0, best: None }
                };
            }
            Phase::Select { cursor, mut best } => {
                self.scratch.state.work.selection_visits += 1;
                if !self.scratch.visited[cursor]
                    && self.scratch.degree[cursor] == 0
                    && best.is_none_or(|old| self.order(cursor) < self.order(old))
                {
                    best = Some(cursor);
                }
                self.scratch.state.phase = if cursor + 1 < n {
                    Phase::Select { cursor: cursor + 1, best }
                } else if let Some(node) = best {
                    Phase::Offered(node)
                } else {
                    return Err(Error::Cycle);
                };
            }
            Phase::Retire { node, word, bits } => {
                if bits != 0 {
                    let successor = (word - 1) * 64 + bits.trailing_zeros() as usize;
                    self.scratch.degree[successor] -= 1;
                    self.scratch.state.work.retired_edges += 1;
                    self.scratch.state.phase =
                        Phase::Retire { node, word, bits: bits & (bits - 1) };
                } else {
                    self.scratch.state.work.retired_words += 1;
                    let bits = self.scratch.edges[node][word];
                    self.scratch.state.phase = Phase::Retire { node, word: word + 1, bits };
                }
                if let Phase::Retire { word, bits: 0, .. } = self.scratch.state.phase {
                    if word == n.div_ceil(64) {
                        self.scratch.state.phase = if self.scratch.state.committed == n {
                            Phase::Complete
                        } else {
                            Phase::Select { cursor: 0, best: None }
                        };
                    }
                }
            }
            Phase::Unbound | Phase::Offered(_) | Phase::Complete | Phase::Failed(_) => {
                unreachable!()
            }
        }
        Ok(())
    }
}

// Charge actual optional metadata cells and the entire traversal wrapper, not
// just their visible payload fields. Borrowed target owners are charged elsewhere.
const _: () = assert!(std::mem::size_of::<Option<Event>>() <= COMPACT_EVENT_BYTES);
const _: () = assert!(std::mem::size_of::<[[u64; WORDS]; COHORT_PHASES]>() == ADJACENCY_BYTES);
const _: () = assert!(
    std::mem::size_of::<Scratch>() + std::mem::size_of::<Cohort<'static, 'static>>()
        <= ADJACENCY_BYTES + TRAVERSAL_BYTES
);
const _: () = assert!(
    std::mem::size_of::<([Option<Event>; COHORT_EVENTS], Scratch, Cohort<'static, 'static>)>()
        <= COHORT_BYTES
);

#[cfg(test)]
mod tests;
