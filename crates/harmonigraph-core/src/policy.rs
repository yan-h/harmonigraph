//! Moving, register-aware adaptive tuning. The sequencer owns voice lifetimes;
//! this module owns bounded selection and released-pitch memory. No audio call
//! allocates. Pitch is absolute microcents; displacement is never octave-folded.
use crate::configuration::{PolicyConfig, ResolvedConfig};
use crate::{LatticePos, Tempered};

pub const MAX_CONTEXT: usize = 256;
/// Released notes remembered. A storage bound with no control rather than a
/// musical rule: every entry decays on the half-life, so at one second the
/// entries past the 24th carry under 2% of the weight even at four notes a
/// second, while each one more costs every decision a pass over the
/// candidates on the audio thread.
pub const MAX_MEMORY: usize = 24;
pub const MAX_COHORT_ONSETS: usize = 256;
const OCTAVE: i64 = crate::tuning::OCTAVE_MICROCENTS as i64;
/// How far a key may sit from the keyboard tuning's rendering of a node and
/// still be that node's key. Not the same-note tolerance: a learned fifth
/// multiplied out to fourteen fifths can miss by a few cents, while this only
/// has to stay well under the smallest distinction a meantone or schismatic
/// keyboard makes, about 20¢. A deliberate 21.5¢ attack bend still falls back.
const KEYBOARD_TOLERANCE: i64 = 5_000_000;
/// Explicit resource ceiling, not a musical truncation. The owner must report
/// exhaustion instead of scoring an incomplete neighbourhood.
pub const MAX_CANDIDATES: usize = 4096;
pub const CONFIG: PolicyConfig = PolicyConfig {
    version: 2,
    radius: 3,
    axes: 2,
    harmonic: 6000,
    pitch_scale: 20,
    released: 100,
    half_life_ms: 1000,
    new_note: 700,
    register_floor: 400,
    register_falloff: 800,
    tolerance: 500_000,
    silence_ms: 0,
    reset_stop: false,
    reset_loop: false,
    keyboard: [700_000_000, 400_000_000, 1_000_000_000],
};

/// The share of its weight a contribution keeps `ticks` before the newest
/// attack in context, on a clock of `per_second` ticks a second: it halves once
/// per half-life. Every note's age counts from its attack, held or released:
/// a release only scales it by the released weight, so letting go of a note
/// never raises its weight, and a sustained chord keeps its vote over a note
/// struck before it that has just stopped. Every contribution shares this one
/// clock and the score normalizes weights, so a delay common to all cancels —
/// waiting changes no decision — and measuring from the newest attack rather
/// than from now keeps the newest weight at one instead of letting a long hold
/// underflow it. A chord's notes land milliseconds apart and so weigh alike,
/// where a rank per attack would not.
pub fn decay(config: PolicyConfig, ticks: i64, per_second: f64) -> f64 {
    if config.half_life_ms == 0 || per_second <= 0.0 {
        return 1.0;
    }
    0.5f64.powf(ticks as f64 / per_second * 1000.0 / f64::from(config.half_life_ms))
}

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

/// Whether an attack assigned `node` is a new note: one on a lattice node that
/// no note in `context`, held or remembered, occupies. A node has no octave,
/// so repeating a note in any register is not new and fades nothing.
pub fn is_new(node: Option<LatticePos>, context: &[ContextPitch]) -> bool {
    node.is_some_and(|n| context.iter().all(|v| v.node != Some(n)))
}

#[derive(Clone, Copy, Debug, Default)]
struct Released {
    pitch: ContextPitch,
    source: u8,
    /// When it was struck, on the caller's clock. Its decay counts from its
    /// attack, not its release.
    at: i64,
    /// The new-note count when it was released. Every new note since
    /// multiplies its weight by the new-note factor once.
    new_notes: u64,
}
/// Latest attack first, bounded by [`MAX_MEMORY`]. Repetition refreshes
/// the same actual onset pitch, never a keyboard key or octave-folded class.
#[derive(Clone, Debug)]
pub struct Memory {
    recent: [Released; MAX_MEMORY],
    len: usize,
    pub reference: i64,
    /// Attacks on a lattice node no context note occupied: the clock released
    /// memory fades on by count, as the half-life is the one it fades on by time.
    new_notes: u64,
}
impl Default for Memory {
    fn default() -> Self {
        Self { recent: [Released::default(); MAX_MEMORY], len: 0, reference: 0, new_notes: 0 }
    }
}
impl Memory {
    pub fn clear(&mut self) {
        self.len = 0;
        self.reference = 0;
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
    /// `new` is [`is_new`] for the node this attack was assigned.
    pub fn attack(&mut self, input: i64, correction: i64, tolerance: u32, new: bool) {
        self.remove_match(input + correction, tolerance);
        self.reference = correction;
        self.new_notes += u64::from(new);
    }
    /// Remember a note let go, at the age of its attack `struck`. A full
    /// memory keeps its latest-struck entries, which are its heaviest, so a
    /// note struck before every one of them is not remembered at all.
    pub fn release(&mut self, pitch: ContextPitch, source: u8, config: PolicyConfig, struck: i64) {
        self.remove_match(pitch.pitch, config.tolerance);
        let i = self.recent[..self.len].iter().position(|r| r.at <= struck).unwrap_or(self.len);
        if i == MAX_MEMORY {
            return;
        }
        self.len = (self.len + 1).min(MAX_MEMORY);
        self.recent.copy_within(i..self.len - 1, i + 1);
        self.recent[i] = Released { pitch, source, at: struck, new_notes: self.new_notes };
    }
    /// The latest attack still remembered, on the caller's clock.
    pub fn newest(&self) -> Option<i64> {
        (self.len > 0).then(|| self.recent[0].at)
    }
    /// Released memory after the held context, each entry at the released
    /// weight decayed by its attack's age at `newest` and by the new-note
    /// factor once for every new note since its release.
    pub fn append(
        &self,
        held: &mut Vec<ContextPitch>,
        config: PolicyConfig,
        newest: i64,
        per_second: f64,
    ) {
        for entry in &self.recent[..self.len] {
            if held
                .iter()
                .any(|v| v.pitch.abs_diff(entry.pitch.pitch) <= u64::from(config.tolerance))
            {
                continue;
            }
            let fades = (self.new_notes - entry.new_notes).min(i32::MAX as u64) as i32;
            let weight = f64::from(config.released) / 1000.0
                * decay(config, newest.saturating_sub(entry.at), per_second)
                * (f64::from(config.new_note) / 1000.0).powi(fades);
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
/// Where the keyboard tuning renders `node`, in microcents above the lattice's
/// C offset, within one octave.
pub fn keyboard_class(keyboard: [i32; 3], node: LatticePos) -> i64 {
    (i64::from(node.threes) * i64::from(keyboard[0])
        + i64::from(node.fives) * i64::from(keyboard[1])
        + i64::from(node.sevens) * i64::from(keyboard[2]))
    .rem_euclid(OCTAVE)
}
/// A key may only become a node the keyboard tuning renders within
/// [`KEYBOARD_TOLERANCE`] of the pitch it sent. When none is, the attack was
/// bent off every key and every candidate competes.
pub fn select_prepared(
    config: MusicalConfig,
    reference: i64,
    onset: OrderedOnset,
    scratch: &PolicyScratch,
) -> Result<Decision, InputError> {
    let input = onset.pitch as f64 / 1_000_000.0;
    let target = input + reference as f64 / 1_000_000.0;
    let pressed = onset.pitch.wrapping_sub(i64::from(config.c_offset)).rem_euclid(OCTAVE);
    let mut best = (f64::INFINITY, LatticePos::ORIGIN, 0.0);
    let mut admissible = None::<(f64, LatticePos, f64)>;
    for &node in &scratch.candidates {
        let base = config.cents(node);
        let output = base + ((target - base) / 1200.0 + 0.5).floor() * 1200.0;
        let score = ((output - target) / f64::from(config.policy.pitch_scale)).powi(2)
            + harmonic_cost(config, node, output, &scratch.context);
        let beats =
            |b: (f64, LatticePos, f64)| score < b.0 || (score == b.0 && key(node) < key(b.1));
        if beats(best) {
            best = (score, node, output);
        }
        let off = (keyboard_class(config.policy.keyboard, node) - pressed).rem_euclid(OCTAVE);
        if off.min(OCTAVE - off) <= KEYBOARD_TOLERANCE && admissible.is_none_or(beats) {
            admissible = Some((score, node, output));
        }
    }
    let best = admissible.unwrap_or(best);
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
pub fn assign_new_note(
    config: MusicalConfig,
    context: &[ContextPitch],
    memory: &Memory,
    onset: OrderedOnset,
    scratch: &mut PolicyScratch,
) -> Result<Decision, InputError> {
    prepare(config, context, scratch)?;
    select_prepared(config, memory.reference, onset, scratch)
}

#[cfg(test)]
mod tests;

pub mod channel;
pub mod reach;
