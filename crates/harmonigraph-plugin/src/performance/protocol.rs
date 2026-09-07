//! Runtime messages for admitted ordinary aggregation. There are no fabricated
//! assignment plans here: #616 extends the retained request/disposition boundary.
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, AtomicU64, AtomicUsize};
use std::sync::Arc;

use harmonigraph_core::canonical::SourceBaseline;
use harmonigraph_core::SourceId;

use super::{clock::Coverage, event::Event, slots::Slots};

pub const TUNERS: usize = 16;
pub const HELD_SESSION: usize = 256;
pub const INTENT_RING: usize = 1024;
pub const REPLY_RING: usize = 1024;
pub const OUTPUT_RING: usize = 2048;
pub const OUTCOME_JOURNAL: usize = 4096;
pub const PENDING_EVENTS: usize = 8192;
pub const LIFETIMES: usize = 8192;
pub const DELAY: i64 = 512;

/// Source-local request binding. decision zero is unbound; all other fields are
/// copied as one addressed reply and preserved through Off and resubmission.
#[derive(Clone, Copy, Debug)]
pub struct Assignment {
    pub configuration: harmonigraph_core::configuration::ResolvedConfig,
    pub decision: u64,
    pub emission: u64,
    pub correction: i64,
    pub initial_player: f64,
}
impl Default for Assignment {
    fn default() -> Self {
        Self {
            configuration: harmonigraph_core::configuration::ConfigReducer::default().resolved(),
            decision: 0,
            emission: 0,
            correction: 0,
            initial_player: 0.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lease {
    pub session: u64,
    pub source: SourceId,
    pub incarnation: u64,
    pub slot: u8,
}

/// Hub-owned immutable emission fence. Generation is the packed OPEN word;
/// closing preserves its BUSY bit until the Source durably completes the group.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fence {
    pub lease: Lease,
    pub epoch: u64,
    pub transaction: u64,
    pub generation: u64,
    pub from_decision: u64,
    pub terminal: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestOutcome {
    Retained,
    Accepted,
    Partial,
    Canceled,
}

/// Reconciliation identity and outcome. Original input timing remains owned by
/// Source Life/Capture and accepted OutputDelta, not this inventory projection.
#[derive(Clone, Copy, Debug)]
pub struct RequestInventory {
    pub lifetime: u64,
    pub on_serial: u64,
    pub decision: u64,
    pub configuration: harmonigraph_core::configuration::ResolvedConfig,
    pub life: u16,
    pub outcome: RequestOutcome,
}

/// One owned 64-entry window, independent of Capture ingress. Common identity
/// stays in the header; records confer no remote CaptureArena read permission.
pub struct InventoryChunk {
    pub fence: Fence,
    pub arena: usize,
    pub input_cut: u64,
    pub lifetime_cut: u64,
    pub output_cut: u64,
    pub sequence: u32,
    pub total: u32,
    pub first: u32,
    pub count: u8,
    pub records: [Option<RequestInventory>; 64],
}

#[derive(Debug)]
pub enum Intent {
    Capture(super::capture::Token),
    /// Hub-local phase of the same retained ingress slot, never Source output.
    CaptureRetirement(super::capture::Retirement),
    Coverage {
        incarnation: u64,
        epoch: u64,
        membership: u64,
        coverage: Coverage,
        input_cut: u64,
    },
    /// Contiguous local musical settlement, independent of original capture
    /// retention for channel reconstruction and of Hub reader retirement.
    InputSettled {
        incarnation: u64,
        epoch: u64,
        input_cut: u64,
        output_cut: u64,
    },
}

#[derive(Clone, Copy, Debug)]
pub enum Reply {
    InventoryComplete {
        fence: Fence,
        input_cut: u64,
        total: u32,
        chunks: u32,
    },
    Fence(Fence),
    RecoveryComplete {
        fence: Fence,
        generation: u64,
        boundary: i64,
    },
    CohortCommitted {
        lease: Lease,
        epoch: u64,
        through: u64,
    },
    CaptureStatusQuery(super::capture::Key),
    PlanRetired {
        incarnation: u64,
        epoch: u64,
        life: u16,
        lifetime: u64,
        decision: u64,
    },
    Assignment {
        key: super::capture::Key,
        life: u16,
        lifetime: u64,
        binding: Assignment,
    },
    /// All immutable reads and remote references ended for exactly this capture.
    CaptureRetired(super::capture::Key),
    Baseline {
        incarnation: u64,
        epoch: u64,
        transaction: u64,
        cut: u64,
        // The producer already publishes the revision it owns. #616 consumes
        // this acknowledgement when binding assignment/input cohorts.
        #[allow(dead_code)]
        membership: u64,
        start: i64,
    },
    OutputRetained {
        incarnation: u64,
        epoch: u64,
        cut: u64,
        complete_through: i64,
    },
    SealedStreamRetained {
        incarnation: u64,
        epoch: u64,
        generation: u64,
        cut: u64,
    },
    Disposition {
        incarnation: u64,
        transaction: u64,
        input_cut: u64,
    },
}

/// Original-consumer identity is meaningful only under the receiver's retained
/// lease/epoch/capture permissions and monotonic output cut. Synthetic output
/// has no original consumer, including injected tuning and channel setup replay.
#[derive(Clone, Copy, Debug)]
pub struct OutputOrigin {
    pub parent: u16,
}
impl OutputOrigin {
    pub const NONE: Self = Self { parent: u16::MAX };
}

/// Explicit compact discriminator avoids paying a second aligned enum tag in
/// every 128-byte output cell. 0/1 are physical wire facts;120/123 are logical
/// terminals whose separately accepted raw controller owns wire_sequence.
#[derive(Clone, Copy, Debug)]
pub struct Outcome {
    wire_sequence: u64,
    pub request: u16,
    pub origin: OutputOrigin,
    tag: u8,
}
impl Outcome {
    pub fn wire(request: u16, origin: OutputOrigin, partial: bool) -> Self {
        Self { wire_sequence: 0, request, origin, tag: u8::from(partial) }
    }
    pub fn channel(wire_sequence: u64, controller: u8, request: u16, origin: OutputOrigin) -> Self {
        assert!(matches!(controller, 120 | 123));
        Self { wire_sequence, request, origin, tag: controller }
    }
    pub fn is_wire(self) -> bool {
        self.tag <= 1
    }
    pub fn partial(self) -> bool {
        self.tag == 1
    }
    pub fn channel_terminal(self) -> Option<(u64, u8)> {
        matches!(self.tag, 120 | 123).then_some((self.wire_sequence, self.tag))
    }
}
const _: () = assert!(std::mem::size_of::<Outcome>() <= 16);

#[derive(Clone, Copy, Debug)]
pub struct OutputDelta {
    pub decision: u64,
    pub player: f64,
    pub incarnation: u64,
    pub sequence: u64,
    pub lifetime: u64,
    pub input: i64,
    pub actual: i64,
    pub epoch: u64,
    /// False preserves raw values after a discontinuity without claiming an old-clock timestamp.
    pub mapped: bool,
    pub discontinuity_generation: u64,
    pub event: Event,
    pub outcome: Outcome,
}

#[derive(Clone, Copy, Debug)]
pub enum Control {
    RevokeAck {
        fence: Fence,
        input_cut: u64,
        output_cut: u64,
        settled_attempt: u64,
    },
    CaptureStatus {
        key: super::capture::Key,
        status: Option<CaptureStatus>,
    },
    Disposition {
        incarnation: u64,
        epoch: u64,
        transaction: u64,
        input_cut: u64,
        lifetime: u64,
        request: u16,
        original_on: bool,
    },
    Adopt {
        lease: Lease,
        epoch: u64,
        coverage: Coverage,
        output_cut: u64,
        input_start_cut: u64,
    },
    Progress {
        incarnation: u64,
        epoch: u64,
        coverage: Coverage,
        output_cut: u64,
    },
    Seal {
        incarnation: u64,
        epoch: u64,
        generation: u64,
        cut: u64,
    },
    /// Callback join fixes the last actually accepted output, even when a
    /// missing physical release still pins musical credit. No sample coverage.
    ProducerJoined {
        incarnation: u64,
        epoch: u64,
        cut: u64,
        unknown_wire: bool,
    },
    Detach {
        incarnation: u64,
        epoch: u64,
        cut: u64,
    },
}

/// Copied only by the Source owner, after every indicated original effect has
/// durably recorded its actual output or received its exact no-wire settlement.
/// The cut bounds those effects; later unrelated output cannot move this proof.
#[derive(Clone, Copy, Debug)]
pub struct CaptureStatus {
    pub output_cut: u64,
    pub work_done: u64,
    pub inline_done: bool,
}

/// Partition the existing pair: ordinary progress owns cell zero, while exact
/// capture status and cancellation acknowledgements always have cell one.
/// Attachment and baseline pairs retain their existing two-cell semantics.
pub struct SourceSlots<T>(Slots<T>);
impl<T> Default for SourceSlots<T> {
    fn default() -> Self {
        Self(Slots::default())
    }
}
impl<T> SourceSlots<T> {
    pub fn reserve(&self) -> Option<super::slots::Reservation<'_, T>> {
        self.0.reserve_at(0)
    }
    pub fn take(&self) -> Option<T> {
        self.0.take_at(0)
    }
    pub fn publish(&self, value: T) -> Result<(), T> {
        match self.reserve() {
            Some(cell) => {
                cell.publish(value);
                Ok(())
            }
            None => Err(value),
        }
    }
    pub fn reserve_repair(&self) -> Option<super::slots::Reservation<'_, T>> {
        self.0.reserve_at(1)
    }
}
impl<T: Copy> SourceSlots<T> {
    pub fn take_repair_if(&self, accept: impl FnOnce(T) -> bool) -> Option<T> {
        self.0.take_at_if(1, accept)
    }
}

#[derive(Clone, Copy)]
pub struct Baseline {
    pub incarnation: u64,
    pub epoch: u64,
    pub frame: SourceBaseline,
    pub start: i64,
}

pub struct SourceControl {
    pub expected_incarnation: AtomicU64,
    pub faults: AtomicU32,
    pub withdrawn: AtomicBool,
    pub hub_detached: AtomicBool,
    pub source_detached: AtomicBool,
    pub emission_gate: AtomicU64,
    pub to_hub: SourceSlots<Control>,
    pub to_source: SourceSlots<Reply>,
    pub baselines: Slots<Baseline>,
    pub inventory: Arc<Slots<InventoryChunk, 1>>,
}
impl Default for SourceControl {
    fn default() -> Self {
        Self {
            expected_incarnation: AtomicU64::new(0),
            faults: AtomicU32::new(0),
            withdrawn: AtomicBool::new(false),
            hub_detached: AtomicBool::new(true),
            source_detached: AtomicBool::new(true),
            emission_gate: AtomicU64::new(2),
            to_hub: SourceSlots::default(),
            to_source: SourceSlots::default(),
            baselines: Slots::default(),
            inventory: Arc::new(Slots::default()),
        }
    }
}

pub struct SessionControl {
    pub runtime: u64,
    pub credits: AtomicUsize,
    pub faults: AtomicU32,
    pub alive: AtomicBool,
    pub epoch: AtomicU64,
    /// Zero is open. A pending hub setup generation fences new admission while
    /// the old epoch's complete output and recording routes finish.
    pub closing: AtomicU64,
    pub hub_through: AtomicI64,
    pub rows: [Arc<SourceControl>; TUNERS],
}

/// Actual rtrb endpoints own the allocations. Every move, including failed or
/// stale adoption, preserves this entire bundle until off-audio reclamation.
/// An Arc to SessionControl alone would NOT pin any ring allocation.
pub struct SourceEndpoints {
    pub intents: rtrb::Producer<Intent>,
    pub replies: rtrb::Consumer<Reply>,
    pub outputs: rtrb::Producer<OutputDelta>,
}
pub struct HubEndpoints {
    pub intents: rtrb::Consumer<Intent>,
    pub replies: rtrb::Producer<Reply>,
    pub outputs: rtrb::Consumer<OutputDelta>,
}
pub struct HubBank {
    pub rows: [HubEndpoints; TUNERS],
}

/// All sixteen endpoint triples are allocated together off audio. The hub gets
/// ONE boxed bank, so adoption does not require sixteen offers through two slots.
pub fn bank() -> (Box<HubBank>, [Option<SourceEndpoints>; TUNERS]) {
    let mut hubs = Vec::with_capacity(TUNERS);
    let mut sources = Vec::with_capacity(TUNERS);
    for _ in 0..TUNERS {
        let (intents, input) = rtrb::RingBuffer::new(INTENT_RING);
        let (reply, replies) = rtrb::RingBuffer::new(REPLY_RING);
        let (outputs, output) = rtrb::RingBuffer::new(OUTPUT_RING);
        hubs.push(HubEndpoints { intents: input, replies: reply, outputs: output });
        sources.push(Some(SourceEndpoints { intents, replies, outputs }));
    }
    (Box::new(HubBank { rows: hubs.try_into().ok().unwrap() }), sources.try_into().ok().unwrap())
}

const _: () = assert!(std::mem::size_of::<OutputDelta>() <= 128);
const _: () = assert!(std::mem::size_of::<Intent>() <= 128);
const _: () = assert!(std::mem::size_of::<Reply>() <= 256);
const _: () = assert!(std::mem::size_of::<Control>() <= 256);
const _: () = assert!(std::mem::size_of::<Baseline>() <= 16384);
// Hub windows and journals allocate Option payloads, not just the bare wire type.
const _: () = assert!(std::mem::size_of::<Option<OutputDelta>>() <= 128);
const _: () = assert!(std::mem::size_of::<Option<Intent>>() <= 128);
const _: () = assert!(std::mem::size_of::<Option<Baseline>>() <= 16384);
const _: () = assert!(std::mem::align_of::<Option<OutputDelta>>() <= 8);
const _: () = assert!(std::mem::align_of::<Option<Intent>>() <= 8);
const _: () = assert!(std::mem::align_of::<Option<Baseline>>() <= 8);
