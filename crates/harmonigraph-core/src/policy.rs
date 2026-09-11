//! Moving, register-aware adaptive tuning. The sequencer owns voice lifetimes;
//! this module owns bounded selection and released-pitch memory. No audio call
//! allocates. Pitch is absolute microcents; displacement is never octave-folded.
use crate::configuration::{PolicyConfig, ResolvedConfig};
use crate::{LatticePos, PitchClass, Tempered};

pub const MAX_CONTEXT: usize = 256;
pub const MAX_MEMORY: usize = 24;
pub const MAX_COHORT_ONSETS: usize = 256;
/// Distinct pitch classes Keep tuning can hold. A twelve-note controller uses
/// twelve; past this, a new class is scored and left unpinned.
pub const MAX_PINS: usize = 128;
/// Explicit resource ceiling, not a musical truncation. The owner must report
/// exhaustion instead of scoring an incomplete neighbourhood.
pub const MAX_CANDIDATES: usize = 4096;
pub const CONFIG: PolicyConfig = PolicyConfig {
    version: 2,
    radius: 3,
    axes: 2,
    memory: 6,
    harmonic: 6000,
    pitch_scale: 20,
    released: 100,
    recency: 700,
    register_floor: 400,
    register_falloff: 800,
    tolerance: 500_000,
    silence_ms: 0,
    reset_stop: false,
    reset_loop: false,
    keep_tuning: false,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MusicalConfig {
    pub c_offset: i32,
    pub axes: [i32; 3],
    pub tempered: Tempered,
    pub policy: PolicyConfig,
}
impl From<ResolvedConfig> for MusicalConfig {
    fn from(c: ResolvedConfig) -> Self {
        Self {
            c_offset: c.tuning.c_offset,
            axes: [c.tuning.three, c.tuning.five, c.tuning.seven],
            tempered: c.modes.tempered,
            policy: c.policy,
        }
    }
}
impl MusicalConfig {
    pub fn cents(self, node: LatticePos) -> f64 {
        4800.0
            + (f64::from(self.c_offset)
                + f64::from(node.threes) * f64::from(self.axes[0])
                + f64::from(node.fives) * f64::from(self.axes[1])
                + f64::from(node.sevens) * f64::from(self.axes[2]))
                / 1_000_000.0
    }
}

/// Tuned onset, including register. Subsequent expression must not overwrite it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ContextPitch {
    pub pitch: i64,
    pub node: Option<LatticePos>,
    pub weight: f64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OrderedOnset {
    pub pitch: i64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Assignment {
    Selected { node: LatticePos, correction_microcents: i64 },
    NoCandidate,
}
impl Assignment {
    pub fn correction_microcents(self) -> i64 {
        match self {
            Self::Selected { correction_microcents, .. } => correction_microcents,
            Self::NoCandidate => 0,
        }
    }
    pub fn node(self) -> Option<LatticePos> {
        match self {
            Self::Selected { node, .. } => Some(node),
            Self::NoCandidate => None,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Decision {
    pub assignment: Assignment,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputError {
    TooMuchContext,
    TooManyCandidates,
    CoordinateOverflow,
    InvalidPitch,
}

#[derive(Clone, Copy, Debug, Default)]
struct Released {
    pitch: ContextPitch,
    source: u8,
}
#[derive(Clone, Copy, Debug)]
struct Pin {
    class: PitchClass,
    assignment: Assignment,
}
/// New activity replaces released entries. Repetition refreshes the same actual
/// onset pitch, never a keyboard key or octave-folded class.
///
/// Keep tuning's pins live here too, so every reset that clears the context
/// clears them with it: they last exactly as long as the rest of memory.
#[derive(Clone, Debug)]
pub struct Memory {
    recent: [Released; MAX_MEMORY],
    len: usize,
    pub reference: i64,
    pins: [Pin; MAX_PINS],
    pin_count: usize,
}
impl Default for Memory {
    fn default() -> Self {
        Self {
            recent: [Released::default(); MAX_MEMORY],
            len: 0,
            reference: 0,
            pins: [Pin {
                class: PitchClass::from_microcents(0),
                assignment: Assignment::NoCandidate,
            }; MAX_PINS],
            pin_count: 0,
        }
    }
}
impl Memory {
    pub fn clear(&mut self) {
        self.len = 0;
        self.reference = 0;
        self.pin_count = 0;
    }
    pub fn forget_source(&mut self, source: u8) {
        let mut n = 0;
        for i in 0..self.len {
            if self.recent[i].source != source {
                self.recent[n] = self.recent[i];
                n += 1;
            }
        }
        self.len = n;
    }
    fn remove_match(&mut self, pitch: i64, tolerance: u32) {
        let mut n = 0;
        for i in 0..self.len {
            if self.recent[i].pitch.pitch.abs_diff(pitch) > u64::from(tolerance) {
                self.recent[n] = self.recent[i];
                n += 1;
            }
        }
        self.len = n;
    }
    pub fn attack(&mut self, input: i64, assignment: Assignment, config: PolicyConfig) {
        let correction = assignment.correction_microcents();
        self.remove_match(input + correction, config.tolerance);
        self.reference = correction;
        if !config.keep_tuning {
            // Pins exist only while the control is on, so switching it back
            // on starts from the current context rather than a pre-drift one.
            self.pin_count = 0;
        } else if self.pin_count < MAX_PINS && self.pinned(input, config).is_none() {
            let class = PitchClass::from_microcents(input);
            self.pins[self.pin_count] = Pin { class, assignment };
            self.pin_count += 1;
        }
    }
    /// With Keep tuning on, the first assignment this input's pitch class
    /// received since the context last reset. Octaves share it: the stored
    /// correction is added to the new input unchanged.
    pub fn pinned(&self, input: i64, config: PolicyConfig) -> Option<Assignment> {
        if !config.keep_tuning {
            return None;
        }
        let class = PitchClass::from_microcents(input);
        self.pins[..self.pin_count]
            .iter()
            .find(|pin| class.signed_microcents_from(pin.class).unsigned_abs() <= config.tolerance)
            .map(|pin| pin.assignment)
    }
    pub fn release(&mut self, pitch: ContextPitch, source: u8, config: PolicyConfig) {
        self.remove_match(pitch.pitch, config.tolerance);
        let capacity = usize::from(config.memory).min(MAX_MEMORY);
        if capacity == 0 {
            self.len = 0;
            return;
        }
        self.len = (self.len + 1).min(capacity);
        self.recent.copy_within(0..self.len - 1, 1);
        self.recent[0] = Released { pitch, source };
    }
    pub fn append(&self, held: &mut Vec<ContextPitch>, config: PolicyConfig) {
        let mut rank = 0;
        for entry in self.recent.iter().take(self.len.min(usize::from(config.memory))) {
            if held
                .iter()
                .any(|v| v.pitch.abs_diff(entry.pitch.pitch) <= u64::from(config.tolerance))
            {
                continue;
            }
            let weight = f64::from(config.released) / 1000.0
                * (f64::from(config.recency) / 1000.0).powi(rank);
            rank += 1;
            if weight > 0.0 {
                held.push(ContextPitch { weight, ..entry.pitch });
            }
        }
    }
}

/// Fixed-capacity vectors are allocated at construction and never grown by
/// selection. Candidates are sorted/deduplicated in-place, with no cached key.
pub struct PolicyScratch {
    pub candidates: Vec<LatticePos>,
    pub context: Vec<ContextPitch>,
}
impl Default for PolicyScratch {
    fn default() -> Self {
        Self {
            candidates: Vec::with_capacity(MAX_CANDIDATES),
            context: Vec::with_capacity(MAX_CONTEXT + MAX_MEMORY),
        }
    }
}
fn key(n: LatticePos) -> (i32, i32, i32) {
    (n.threes, n.fives, n.sevens)
}
pub fn distance(a: LatticePos, b: LatticePos) -> f64 {
    (i64::from(a.threes).abs_diff(i64::from(b.threes))
        + i64::from(a.fives).abs_diff(i64::from(b.fives))
        + i64::from(a.sevens).abs_diff(i64::from(b.sevens))) as f64
}
fn local_nodes(
    config: MusicalConfig,
    anchor: LatticePos,
    mut visit: impl FnMut(LatticePos) -> Result<(), InputError>,
) -> Result<(), InputError> {
    let radius = i32::from(config.policy.radius);
    let f = if config.policy.axes >= 2 { radius } else { 0 };
    let s = if config.policy.axes >= 3 { radius } else { 0 };
    for t in -radius..=radius {
        for f in -f..=f {
            for s in -s..=s {
                if t.abs() + f.abs() + s.abs() > radius {
                    continue;
                }
                let n = LatticePos::new(
                    anchor.threes.checked_add(t).ok_or(InputError::CoordinateOverflow)?,
                    anchor.fives.checked_add(f).ok_or(InputError::CoordinateOverflow)?,
                    anchor.sevens.checked_add(s).ok_or(InputError::CoordinateOverflow)?,
                );
                // Coordinates near the machine limit cannot be safely respelled.
                if [n.threes, n.fives, n.sevens]
                    .iter()
                    .any(|v| v.unsigned_abs() > (i32::MAX / 32) as u32)
                {
                    return Err(InputError::CoordinateOverflow);
                }
                visit(n.respell(config.tempered))?;
            }
        }
    }
    Ok(())
}
/// Build eligibility independently of input. Nodes without assignment metadata
/// (DIRECT input) are projected locally, never into an absolute origin box.
pub fn prepare(
    config: MusicalConfig,
    context: &[ContextPitch],
    scratch: &mut PolicyScratch,
) -> Result<(), InputError> {
    scratch.context.clear();
    scratch.candidates.clear();
    if context.len() > MAX_CONTEXT + MAX_MEMORY {
        return Err(InputError::TooMuchContext);
    }
    for v in context.iter().filter(|v| v.weight > 0.0) {
        if !scratch
            .context
            .iter()
            .any(|other| other.pitch.abs_diff(v.pitch) <= u64::from(config.policy.tolerance))
        {
            scratch.context.push(*v);
        }
    }
    if scratch.context.is_empty() {
        scratch.context.push(ContextPitch {
            pitch: 4_800_000_000 + i64::from(config.c_offset),
            node: Some(LatticePos::ORIGIN),
            weight: 1.0,
        });
    }
    let anchor = scratch.context.iter().find_map(|v| v.node).unwrap_or(LatticePos::ORIGIN);
    for v in &mut scratch.context {
        if v.node.is_none() {
            let mut best = (f64::INFINITY, LatticePos::ORIGIN);
            local_nodes(config, anchor, |n| {
                let error = ((config.cents(n) - v.pitch as f64 / 1_000_000.0 + 600.0)
                    .rem_euclid(1200.0)
                    - 600.0)
                    .abs();
                if error < best.0 || (error == best.0 && key(n) < key(best.1)) {
                    best = (error, n);
                }
                Ok(())
            })?;
            v.node = Some(best.1);
        } else {
            v.node = v.node.map(|n| n.respell(config.tempered));
        }
    }
    for v in &scratch.context {
        local_nodes(config, v.node.unwrap(), |n| {
            // Deduplicate each ball against the current sorted union. No heap
            // growth and no incomplete set is ever scored on exhaustion.
            if let Err(i) = scratch.candidates.binary_search_by_key(&key(n), |n| key(*n)) {
                if scratch.candidates.len() == MAX_CANDIDATES {
                    return Err(InputError::TooManyCandidates);
                }
                scratch.candidates.insert(i, n);
            }
            Ok(())
        })?;
    }
    Ok(())
}
pub fn harmonic_cost(
    config: MusicalConfig,
    node: LatticePos,
    output: f64,
    context: &[ContextPitch],
) -> f64 {
    let floor = f64::from(config.policy.register_floor) / 1000.0;
    let falloff = f64::from(config.policy.register_falloff) / 1000.0;
    let mut total = 0.0;
    let mut sum = 0.0;
    for v in context {
        let register = floor
            + (1.0 - floor)
                * (-falloff * (output - v.pitch as f64 / 1_000_000.0).abs() / 1200.0).exp();
        let weight = v.weight * register;
        total += weight * distance(node, v.node.unwrap());
        sum += weight;
    }
    f64::from(config.policy.harmonic) / 1000.0 * total / sum
}
pub fn select_prepared(
    config: MusicalConfig,
    reference: i64,
    onset: OrderedOnset,
    scratch: &PolicyScratch,
) -> Result<Decision, InputError> {
    let input = onset.pitch as f64 / 1_000_000.0;
    let target = input + reference as f64 / 1_000_000.0;
    let mut best = (f64::INFINITY, LatticePos::ORIGIN, 0.0);
    for &node in &scratch.candidates {
        let base = config.cents(node);
        let output = base + ((target - base) / 1200.0 + 0.5).floor() * 1200.0;
        let score = ((output - target) / f64::from(config.policy.pitch_scale)).powi(2)
            + harmonic_cost(config, node, output, &scratch.context);
        if score < best.0 || (score == best.0 && key(node) < key(best.1)) {
            best = (score, node, output);
        }
    }
    if !best.0.is_finite() {
        return Err(InputError::InvalidPitch);
    }
    let correction = (best.2 - input) * 1_000_000.0;
    if correction.abs() >= i64::MAX as f64 {
        return Err(InputError::InvalidPitch);
    }
    Ok(Decision {
        assignment: Assignment::Selected {
            node: best.1,
            correction_microcents: correction.round() as i64,
        },
    })
}
/// A pitch class Keep tuning has pinned replays its first assignment; anything
/// else is scored against the context.
pub fn assign_new_note(
    config: MusicalConfig,
    context: &[ContextPitch],
    memory: &Memory,
    onset: OrderedOnset,
    scratch: &mut PolicyScratch,
) -> Result<Decision, InputError> {
    if let Some(assignment) = memory.pinned(onset.pitch, config.policy) {
        return Ok(Decision { assignment });
    }
    prepare(config, context, scratch)?;
    select_prepared(config, memory.reference, onset, scratch)
}

#[cfg(test)]
mod tests;

pub mod channel;
pub mod reach;
