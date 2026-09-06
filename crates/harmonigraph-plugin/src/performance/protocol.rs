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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lease {
    pub session: u64,
    pub source: SourceId,
    pub incarnation: u64,
    pub slot: u8,
}

#[derive(Clone, Copy, Debug)]
pub enum Intent {
    Coverage {
        incarnation: u64,
        epoch: u64,
        coverage: Coverage,
        input_cut: u64,
    },
    /// A separately counted ordinary pending-input disposition, never a held
    /// baseline standing in for unsounded input. Plans/config bindings come later.
    Disposition {
        incarnation: u64,
        transaction: u64,
        input_cut: u64,
        total: u32,
        index: u32,
        // The aggregation-only hub acknowledges cancellation independently of
        // held output; #616 binds this identity to its remote request ledger.
        #[allow(dead_code)]
        lifetime: u64,
        canceled: bool,
    },
}

#[derive(Clone, Copy, Debug)]
pub enum Reply {
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

#[derive(Clone, Copy, Debug)]
pub enum Outcome {
    Wire,
    /// One accepted raw CC is the witness for this logical terminal fact.
    /// This is not a separately attempted or accepted CLAP note output.
    ChannelTerminal {
        wire_sequence: u64,
        controller: u8,
    },
}

#[derive(Clone, Copy, Debug)]
pub struct OutputDelta {
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
    Adopt {
        lease: Lease,
        epoch: u64,
        start: i64,
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
    pub to_hub: Slots<Control>,
    pub to_source: Slots<Reply>,
    pub baselines: Slots<Baseline>,
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
            to_hub: Slots::default(),
            to_source: Slots::default(),
            baselines: Slots::default(),
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
