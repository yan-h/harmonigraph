//! The first adaptive musical policy, independent of sequencing and host output.
//!
//! The owner supplies the next canonical onset, authoritative context at that
//! onset, and the separate source/channel/key history cell. It incorporates the
//! returned assignment and history before the next call. This module owns no
//! held voices, clock, history map, configuration lifecycle or player expression.

use crate::configuration::{PolicyConfig, ResolvedConfig};
use crate::tuning::CENTS_TO_MICROCENTS;
use crate::{positions_within, LatticePos, PitchClass, Tempered, Tuning};

pub const RAW_THREES: i32 = 6;
pub const RAW_FIVES: i32 = 2;
pub const RAW_SEVENS: i32 = 0;
pub const MAX_DOMAIN_NODES: usize =
    ((2 * RAW_THREES + 1) * (2 * RAW_FIVES + 1) * (2 * RAW_SEVENS + 1)) as usize;
pub const MAX_CANDIDATES: usize = MAX_DOMAIN_NODES;
/// Includes held voices and explicitly scheduled predecessors, counted together.
pub const MAX_CONTEXT: usize = 256;
/// The central owner may evaluate this many onsets sequentially in one cohort.
/// This is an input/work ceiling, not a callback throughput claim.
pub const MAX_COHORT_ONSETS: usize = 256;
pub const CANDIDATE_RADIUS: u32 = 50 * CENTS_TO_MICROCENTS;
pub const CONTEXT_RADIUS: u32 = 50 * CENTS_TO_MICROCENTS;
pub const CONTEXT_WEIGHT: u16 = 4;
pub const HISTORY_WEIGHT: u16 = 1;
pub const ORIGIN_WEIGHT: u16 = 1;

/// Fixed descriptor for later production configuration binding. The current
/// configuration reducer still emits version zero; this module does not wire
/// the engine into playback or silently change that owner's configuration.
pub const CONFIG: PolicyConfig = PolicyConfig {
    version: 1,
    domain: [RAW_THREES as u16, RAW_FIVES as u16, RAW_SEVENS as u16],
    candidate_radius: CANDIDATE_RADIUS,
    context_radius: CONTEXT_RADIUS,
    context_weight: CONTEXT_WEIGHT,
    history_weight: HISTORY_WEIGHT,
    origin_weight: ORIGIN_WEIGHT,
};

/// Already-resolved musical axes in microcents. Locked axes must already obey
/// their exact comma identities. Display tolerance, auto/learning modes and
/// configuration revisions do not decide a musical score.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MusicalConfig {
    pub c_offset: i32,
    pub axes: [i32; 3],
    pub tempered: Tempered,
}

impl From<ResolvedConfig> for MusicalConfig {
    fn from(config: ResolvedConfig) -> Self {
        let tuning = config.tuning;
        Self {
            c_offset: tuning.c_offset,
            axes: [tuning.three, tuning.five, tuning.seven],
            tempered: config.modes.tempered,
        }
    }
}

impl MusicalConfig {
    fn tuning(self) -> Tuning {
        Tuning {
            c_offset: self.c_offset,
            three: self.axes[0],
            five: self.axes[1],
            seven: self.axes[2],
            tolerance: 0,
        }
    }
}

/// The pitch class of current emitted output, or an explicitly scheduled
/// predecessor, including its current player expression. Octave is irrelevant
/// to lattice scoring. The owner retains absolute pitch and provenance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ContextPitch {
    pub pitch: PitchClass,
    /// Optional attack metadata, never authority for `pitch`. A revision alone
    /// cannot validate it: membership and exact pitch are checked on every call.
    pub node: Option<LatticePos>,
}

/// An onset already chosen by the central sequencer. Source/channel/lifetime
/// identity and ordering stay with that owner; `history` must be this key's cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OrderedOnset {
    pub key: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Assignment {
    Selected {
        node: LatticePos,
        correction_microcents: i32,
    },
    /// Completed policy evaluation with zero adaptive correction. Player
    /// expression is still composed by the output owner, as for any assignment.
    NoCandidate,
}

impl Assignment {
    pub fn correction_microcents(self) -> i32 {
        match self {
            Self::Selected { correction_microcents, .. } => correction_microcents,
            Self::NoCandidate => 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistoryUpdate {
    Set(LatticePos),
    Clear,
}

impl HistoryUpdate {
    /// Apply only to the onset's source/channel/key cell in the appropriate
    /// prospective or confirmed owner. This does not implement that lifecycle.
    pub fn apply(self, cell: &mut Option<LatticePos>) {
        *cell = match self {
            Self::Set(node) => Some(node),
            Self::Clear => None,
        };
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Decision {
    pub assignment: Assignment,
    pub history_update: HistoryUpdate,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputError {
    InvalidMidiKey,
    TooMuchContext,
}

#[derive(Clone, Copy)]
struct DomainNode {
    position: LatticePos,
    pitch: PitchClass,
}

/// Caller-owned fixed scratch, fully rebuilt for each evaluation. There is no
/// cache key or carry-forward musical state. Construct outside a callback if
/// stack space is tight; no call allocates, sorts a heap collection or frees.
pub struct PolicyScratch {
    domain: [DomainNode; MAX_DOMAIN_NODES],
    candidates: [u8; MAX_CANDIDATES],
    context: [LatticePos; MAX_CONTEXT],
    domain_len: usize,
    candidate_len: usize,
    context_len: usize,
}

impl Default for PolicyScratch {
    fn default() -> Self {
        Self {
            domain: [DomainNode {
                position: LatticePos::ORIGIN,
                pitch: PitchClass::from_microcents(0),
            }; MAX_DOMAIN_NODES],
            candidates: [0; MAX_CANDIDATES],
            context: [LatticePos::ORIGIN; MAX_CONTEXT],
            domain_len: 0,
            candidate_len: 0,
            context_len: 0,
        }
    }
}

/// Evaluate one canonically ordered onset with direct bounded enumeration.
/// History is a separate transient node selected under this configuration;
/// the owner clears it at revision/source/session/recovery boundaries. With
/// no usable context it is ignored, including after a preceding phrase.
///
/// Input errors are owner contract violations, distinct from a completed
/// [`Assignment::NoCandidate`]. Context is never silently truncated.
pub fn assign_new_note(
    config: MusicalConfig,
    context: &[ContextPitch],
    history: Option<LatticePos>,
    onset: OrderedOnset,
    scratch: &mut PolicyScratch,
) -> Result<Decision, InputError> {
    scratch.domain_len = 0;
    scratch.candidate_len = 0;
    scratch.context_len = 0;
    if onset.key > 127 {
        return Err(InputError::InvalidMidiKey);
    }
    if context.len() > MAX_CONTEXT {
        return Err(InputError::TooMuchContext);
    }
    let tuning = config.tuning();
    for raw in
        positions_within(-RAW_THREES..=RAW_THREES, -RAW_FIVES..=RAW_FIVES, -RAW_SEVENS..=RAW_SEVENS)
    {
        let position = raw.respell(config.tempered);
        // Deduplicate coordinates, not arbitrary pitch equality. Canonical
        // positions can lie outside the raw box, notably with syntonic locked.
        if !scratch.domain[..scratch.domain_len].iter().any(|node| node.position == position) {
            scratch.domain[scratch.domain_len] =
                DomainNode { position, pitch: tuning.pitch_class(position) };
            scratch.domain_len += 1;
        }
    }
    let domain = &scratch.domain[..scratch.domain_len];
    let target = PitchClass::from_midi_note(onset.key);
    for (index, node) in domain.iter().enumerate() {
        if node.pitch.signed_microcents_from(target).unsigned_abs() <= CANDIDATE_RADIUS {
            scratch.candidates[scratch.candidate_len] = index as u8;
            scratch.candidate_len += 1;
        }
    }
    if scratch.candidate_len == 0 {
        return Ok(Decision {
            assignment: Assignment::NoCandidate,
            history_update: HistoryUpdate::Clear,
        });
    }
    for voice in context {
        if let Some(position) = project(domain, *voice) {
            scratch.context[scratch.context_len] = position;
            scratch.context_len += 1;
        }
    }
    let usable = &scratch.context[..scratch.context_len];
    let best = scratch.candidates[..scratch.candidate_len]
        .iter()
        .map(|&index| domain[usize::from(index)])
        .min_by_key(|node| (score(node.position, usable, history), tie(node.position)))
        .expect("nonempty candidates checked above");
    Ok(Decision {
        assignment: Assignment::Selected {
            node: best.position,
            correction_microcents: best.pitch.signed_microcents_from(target),
        },
        history_update: HistoryUpdate::Set(best.position),
    })
}

fn project(domain: &[DomainNode], voice: ContextPitch) -> Option<LatticePos> {
    if let Some(position) = voice.node {
        if domain.iter().any(|node| node.position == position && node.pitch == voice.pitch) {
            return Some(position);
        }
    }
    domain
        .iter()
        .filter_map(|node| {
            let distance = node.pitch.signed_microcents_from(voice.pitch).unsigned_abs();
            (distance <= CONTEXT_RADIUS).then_some((distance, tie(node.position), node.position))
        })
        .min_by_key(|&(distance, tie, _)| (distance, tie))
        .map(|(_, _, position)| position)
}

fn distance(a: LatticePos, b: LatticePos) -> u64 {
    // Widen before subtraction: even malformed external history metadata must
    // not overflow the deterministic integer score.
    i64::from(a.threes).abs_diff(i64::from(b.threes))
        + i64::from(a.fives).abs_diff(i64::from(b.fives))
        + i64::from(a.sevens).abs_diff(i64::from(b.sevens))
}

fn score(node: LatticePos, context: &[LatticePos], history: Option<LatticePos>) -> u64 {
    if context.is_empty() {
        u64::from(ORIGIN_WEIGHT) * distance(node, LatticePos::ORIGIN)
    } else {
        u64::from(CONTEXT_WEIGHT) * context.iter().map(|&other| distance(node, other)).sum::<u64>()
            + u64::from(HISTORY_WEIGHT) * history.map_or(0, |other| distance(node, other))
    }
}

fn tie(node: LatticePos) -> (u32, u32, u32, i32, i32, i32) {
    (
        node.threes.unsigned_abs(),
        node.fives.unsigned_abs(),
        node.sevens.unsigned_abs(),
        node.threes,
        node.fives,
        node.sevens,
    )
}

#[cfg(test)]
mod tests;
