//! Runtime messages for admitted ordinary aggregation. There are no fabricated
//! assignment plans here: #616 extends the retained request/disposition boundary.
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, AtomicU64, AtomicUsize};
use std::sync::Arc;

use harmonigraph_core::SourceId;

use super::{clock::Coverage, event::Event, slots::Slots};

pub const TUNERS: usize = 16;
pub const HELD_SESSION: usize = 256;
pub const INTENT_RING: usize = 1024;
pub const REPLY_RING: usize = 1024;
pub const OUTPUT_RING: usize = 2048;
pub const OUTCOME_JOURNAL: usize = 4096;
/// A request slot is held for as long as any in-flight input still addresses
/// it, not for as long as its note sounds, so this tracks `PENDING_EVENTS`
/// rather than `HELD_PER_SOURCE`. Both stay where they were; the plan ledger
/// they size belongs to the per-pairing allocation, not to their depth.
pub const PENDING_EVENTS: usize = 8192;
pub const LIFETIMES: usize = 8192;
/// Copied records the Hub stages per source between arrival and sequencing.
/// One input can address every held note, so this is deeper than the number
/// of inputs a callback can carry. Exhaustion is back pressure on the ring,
/// which the Tune retries; it is not made unreachable by the size.
pub const CAPTURES_PER_SOURCE: usize = 1024;
/// Copied records the ordering pass may hold for one sample across all
/// sources: sixteen tracks each choking every held note at the same sample is
/// 1,040. Beyond it the pass latches rather than dropping an input.
pub const BATCH_EVENTS: usize = 2048;
/// The tuning delay is `multiplier x advertised max frames`, chosen per Tune
/// and fixed for the whole activation. Sixteen is the top of the range: it
/// still reaches the 512 samples this delay used to be fixed at even from a
/// 32-frame callback, while 16x a 512-frame callback is already 171 ms at
/// 48 kHz, past any use a live player has for it.
pub const DELAY_MULTIPLIER_MAX: i32 = 16;

/// Four bytes retain all three outcomes without conflating an Off birth with
/// the policy's completed NoCandidate history clear.
#[derive(Clone, Copy, Debug, Default)]
pub enum Selection {
    #[default]
    Unretuned,
    NoCandidate,
    Node([i8; 3]),
}
impl Selection {
    pub fn node(self) -> Option<harmonigraph_core::LatticePos> {
        match self {
            Self::Node(p) => {
                Some(harmonigraph_core::LatticePos::new(p[0].into(), p[1].into(), p[2].into()))
            }
            _ => None,
        }
    }
    pub fn musical(self) -> bool {
        !matches!(self, Self::Unretuned)
    }
}
const _: () = assert!(std::mem::size_of::<Selection>() == 4);

/// Source-local request binding. decision zero is unbound; all other fields are
/// copied as one addressed reply and preserved through Off and resubmission.
#[derive(Clone, Copy, Debug)]
pub struct Assignment {
    pub configuration: harmonigraph_core::configuration::ResolvedConfig,
    pub decision: u64,
    pub emission: u64,
    /// Exact microcents within the policy's inclusive +/-50-cent bound.
    pub correction: i32,
    /// Checked coordinates from the bounded canonical policy domain, or the
    /// explicit completed result. Decision zero alone means no assignment.
    pub selection: Selection,
    pub initial_player: f64,
}
impl Default for Assignment {
    fn default() -> Self {
        Self {
            configuration: harmonigraph_core::configuration::ConfigReducer::default().resolved(),
            decision: 0,
            emission: 0,
            correction: 0,
            selection: Selection::Unretuned,
            initial_player: 0.0,
        }
    }
}

impl Assignment {
    pub fn node(self) -> Option<harmonigraph_core::LatticePos> {
        self.selection.node()
    }
}

#[cfg(test)]
#[test]
fn compact_selection_preserves_every_canonical_policy_coordinate() {
    use harmonigraph_core::{policy, positions_within, Tempered};
    for syntonic in [false, true] {
        for septimal_kleisma in [false, true] {
            for raw in positions_within(
                -policy::RAW_THREES..=policy::RAW_THREES,
                -policy::RAW_FIVES..=policy::RAW_FIVES,
                -policy::RAW_SEVENS..=policy::RAW_SEVENS,
            ) {
                let node = raw.respell(Tempered { syntonic, septimal_kleisma });
                let encoded = Selection::Node([
                    i8::try_from(node.threes).unwrap(),
                    i8::try_from(node.fives).unwrap(),
                    i8::try_from(node.sevens).unwrap(),
                ]);
                assert_eq!(encoded.node(), Some(node));
            }
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

/// One copied, self-contained input record. Everything the Hub needs to order
/// and assign this input is here; nothing points back into Tune storage.
///
/// The Tune emits one record per addressed target, so a wildcard release or a
/// channel termination becomes one record per held note rather than one record
/// carrying a 64-wide fan-out. That keeps the cell fixed-size and bounds the
/// traffic by the notes actually sounding.
#[derive(Clone, Copy, Debug)]
pub struct Capture {
    pub lease: Lease,
    /// Reset generation. A reply minted under a different epoch is obsolete.
    pub epoch: u64,
    /// The original input's own sequence at its source. Several records share
    /// one serial when that one input addressed several notes.
    pub serial: u64,
    /// Input sample already mapped into the Hub's timebase.
    pub sample: i64,
    pub kind: CaptureKind,
    /// Request slot of the addressed note, or `NO_REQUEST`.
    pub request: u16,
    /// Birth serial of the addressed note, or zero when none is addressed.
    pub lifetime: u64,
    pub channel: u8,
    pub key: u8,
    /// This onset asks for an adaptive assignment.
    pub adaptive: bool,
}
pub const NO_REQUEST: u16 = u16::MAX;

impl Capture {
    pub fn onset(self) -> bool {
        matches!(self.kind, CaptureKind::Onset)
    }
    /// Merge order inside one sample: every release and controller from every
    /// source applies before any onset, and onsets then run in the musical
    /// key/channel/source tie-break. Ties inside each half fall back to the
    /// original per-source input order, which copying preserves.
    pub fn order(self) -> (bool, u8, u8, u8, u64) {
        if self.onset() {
            (true, self.key, self.channel, self.lease.slot, self.serial)
        } else {
            (false, 0, 0, self.lease.slot, self.serial)
        }
    }
}

/// What one copied record does to the Hub's musical state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CaptureKind {
    Onset,
    /// Ends the addressed note: its own release, a same-key replacement's
    /// choke, or one held note's share of a channel termination.
    Terminal,
    /// Per-note pitch expression addressed to one note.
    Tuning {
        value_bits: u64,
    },
    /// A shared channel controller, carrying its channel in the record. It
    /// addresses no note of its own; the notes a termination ends arrive as
    /// their own Terminal records.
    Channel,
    Participation(bool),
    /// This source's transport Stop. It ends every note the source is playing
    /// — the sounding ones by emergency release, the unsounded ones by
    /// cancellation — and neither ending becomes an addressed Terminal record.
    Stop,
    /// Note-addressed expression and everything else the Hub keeps only as
    /// chronological context.
    Other,
}

/// Exactly the identity a reply needs to reach one request and to be refused
/// after a reset, a re-pairing or a request-slot reuse.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Request {
    pub lease: Lease,
    pub epoch: u64,
    /// The onset's own input serial.
    pub serial: u64,
    /// The Tune's request slot.
    pub request: u16,
    /// The addressed note's birth serial.
    pub lifetime: u64,
}

#[derive(Clone, Copy, Debug)]
pub enum Intent {
    Capture(Capture),
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
    CohortCommitted {
        lease: Lease,
        epoch: u64,
        through: u64,
    },
    PlanRetired {
        incarnation: u64,
        epoch: u64,
        life: u16,
        lifetime: u64,
        decision: u64,
    },
    Assignment {
        request: Request,
        binding: Assignment,
    },
    /// This Tune is enrolled in the session at revision `membership`, from
    /// coverage `start` onward. A `start` beyond what the Tune adopted with
    /// is a join floor: the Hub has already published past that point, so the
    /// Tune rejoins from there instead. Carries no held-note snapshot — the
    /// pairing boundary is a reset, so there is nothing sounding to hand over.
    Enrolled {
        incarnation: u64,
        epoch: u64,
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

/// Explicit compact discriminator avoids paying a second aligned enum tag in
/// every 128-byte output cell. 0/1 are physical wire facts;120/123 are logical
/// terminals whose separately accepted raw controller owns wire_sequence.
#[derive(Clone, Copy, Debug)]
pub struct Outcome {
    wire_sequence: u64,
    pub request: u16,
    tag: u8,
}
impl Outcome {
    pub fn wire(request: u16, partial: bool) -> Self {
        Self { wire_sequence: 0, request, tag: u8::from(partial) }
    }
    pub fn channel(wire_sequence: u64, controller: u8, request: u16) -> Self {
        assert!(matches!(controller, 120 | 123));
        Self { wire_sequence, request, tag: controller }
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
    Disposition {
        incarnation: u64,
        epoch: u64,
        transaction: u64,
        input_cut: u64,
        lifetime: u64,
        request: u16,
        original_on: bool,
    },
    /// The join request. Everything the Hub needs to enroll this row: no
    /// separate snapshot follows it, because the Tune reset at this boundary.
    Adopt {
        lease: Lease,
        epoch: u64,
        coverage: Coverage,
        output_cut: u64,
        input_start_cut: u64,
        participating: bool,
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

/// Partition the existing pair: ordinary progress owns cell zero, while exact
/// capture status and cancellation acknowledgements always have cell one.
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

pub struct SourceControl {
    pub expected_incarnation: AtomicU64,
    pub faults: AtomicU32,
    /// This Tune's activation-fixed delay in samples. The Source is the only
    /// writer; the Hub reads it for the emission time it reports each output
    /// was planned for, which is the Tune's own input+D and nothing else.
    pub delay: AtomicI64,
    /// Deadline feedback the Tune measures and the Hub aggregates for its own
    /// display: notes counted once each, and the largest lateness seen since
    /// the delay setting was last applied.
    pub deadline_misses: AtomicU64,
    pub worst_lateness: AtomicI64,
    pub withdrawn: AtomicBool,
    pub hub_detached: AtomicBool,
    pub source_detached: AtomicBool,
    pub emission_gate: AtomicU64,
    pub to_hub: SourceSlots<Control>,
    pub to_source: SourceSlots<Reply>,
    /// This row's storage on its way from the registry's pairing boundary to
    /// the Hub. Cell zero only; the Hub keeps what it takes, so a row that is
    /// ever paired is built once, off audio, and reused if it pairs again.
    pub store: Slots<super::hub::RowStore>,
    pub store_held: AtomicBool,
}
impl Default for SourceControl {
    fn default() -> Self {
        Self {
            expected_incarnation: AtomicU64::new(0),
            faults: AtomicU32::new(0),
            delay: AtomicI64::new(0),
            deadline_misses: AtomicU64::new(0),
            worst_lateness: AtomicI64::new(0),
            withdrawn: AtomicBool::new(false),
            hub_detached: AtomicBool::new(true),
            source_detached: AtomicBool::new(true),
            emission_gate: AtomicU64::new(2),
            to_hub: SourceSlots::default(),
            to_source: SourceSlots::default(),
            store: Slots::default(),
            store_held: AtomicBool::new(false),
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
const _: () = assert!(std::mem::size_of::<Capture>() <= 96);
const _: () = assert!(std::mem::size_of::<Option<Capture>>() <= 96);
const _: () = assert!(std::mem::size_of::<Intent>() <= 128);
const _: () = assert!(std::mem::size_of::<Reply>() <= 256);
const _: () = assert!(std::mem::size_of::<Control>() <= 256);
// Hub windows and journals allocate Option payloads, not just the bare wire type.
const _: () = assert!(std::mem::size_of::<Option<OutputDelta>>() <= 128);
const _: () = assert!(std::mem::size_of::<Option<Intent>>() <= 128);
const _: () = assert!(std::mem::align_of::<Option<OutputDelta>>() <= 8);
const _: () = assert!(std::mem::align_of::<Option<Intent>>() <= 8);
