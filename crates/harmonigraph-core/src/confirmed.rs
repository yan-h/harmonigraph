//! Reusable confirmed-pitch state for the hub. Callers validate protocol identity,
//! admission and complete source intervals before feeding actual-output batches.
//! Direct input is explicitly observed input, never claimed accepted forwarding.

use crate::{LearnedTuning, NoteEvent, NoteEventKind, PitchClass, SourceId, VoiceKey};

pub const HELD_SESSION: usize = 256;
pub const HELD_PER_SOURCE: usize = 64;
pub const MAX_LEARNING_PAIRS: usize = HELD_SESSION * (HELD_SESSION - 1) / 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PitchProvenance {
    ObservedDirect,
    AcceptedOutput,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConfirmedPitch {
    pub key: VoiceKey,
    /// Supplied by the validated source owner; absent for today's direct input.
    pub lifetime: Option<u64>,
    pub pitch_microcents: i64,
    pub onset_sample: i64,
    pub provenance: PitchProvenance,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfirmedError {
    Capacity,
    InvalidBatch,
    Incomplete,
}

pub struct ConfirmedPitches {
    rows: [Option<ConfirmedPitch>; HELD_SESSION],
    incomplete: [Option<SourceId>; 17],
    unknown_incomplete: bool,
}

impl Default for ConfirmedPitches {
    fn default() -> Self {
        Self { rows: [None; HELD_SESSION], incomplete: [None; 17], unknown_incomplete: false }
    }
}

impl ConfirmedPitches {
    pub fn rows(&self) -> impl Iterator<Item = &ConfirmedPitch> {
        self.rows.iter().flatten()
    }

    pub fn is_complete(&self) -> bool {
        !self.unknown_incomplete && self.incomplete.iter().all(Option::is_none)
    }

    fn invalidate(&mut self, source: SourceId) -> ConfirmedError {
        if !self.incomplete.contains(&Some(source)) {
            if let Some(cell) = self.incomplete.iter_mut().find(|cell| cell.is_none()) {
                *cell = Some(source);
            } else {
                self.unknown_incomplete = true;
            }
        }
        ConfirmedError::Capacity
    }

    pub fn on(&mut self, row: ConfirmedPitch) -> Result<(), ConfirmedError> {
        if row.key.channel >= 16
            || row.key.note >= 128
            || (row.provenance == PitchProvenance::ObservedDirect
                && row.key.source != SourceId::DIRECT)
        {
            self.invalidate(row.key.source);
            return Err(ConfirmedError::InvalidBatch);
        }
        if let Some(cell) =
            self.rows.iter_mut().find(|cell| cell.is_some_and(|old| old.key == row.key))
        {
            *cell = Some(row);
            return Ok(());
        }
        if self.rows().filter(|old| old.key.source == row.key.source).count() >= HELD_PER_SOURCE {
            return Err(self.invalidate(row.key.source));
        }
        if let Some(cell) = self.rows.iter_mut().find(|cell| cell.is_none()) {
            *cell = Some(row);
            Ok(())
        } else {
            Err(self.invalidate(row.key.source))
        }
    }

    /// A supplied lifetime must match. Protocol admission and old-incarnation
    /// rejection remain the session adapter's responsibility.
    pub fn pitch(&mut self, key: VoiceKey, lifetime: Option<u64>, microcents: i64) {
        if let Some(row) =
            self.rows.iter_mut().flatten().find(|row| row.key == key && row.lifetime == lifetime)
        {
            row.pitch_microcents = microcents;
        }
    }

    pub fn release(&mut self, key: VoiceKey, lifetime: Option<u64>) {
        for cell in &mut self.rows {
            if cell.is_some_and(|row| row.key == key && row.lifetime == lifetime) {
                *cell = None;
            }
        }
    }

    pub fn clear_source(&mut self, source: SourceId) {
        for cell in &mut self.rows {
            if cell.is_some_and(|row| row.key.source == source) {
                *cell = None;
            }
        }
        for cell in &mut self.incomplete {
            if *cell == Some(source) {
                *cell = None;
            }
        }
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Validate the complete replacement before touching live rows. This is the
    /// current-state component of recovery, not a baseline protocol or publication
    /// acknowledgement. It neither fabricates attacks nor retires actual history.
    pub fn replace_source(
        &mut self,
        source: SourceId,
        rows: &[ConfirmedPitch],
    ) -> Result<(), ConfirmedError> {
        if rows.len() > HELD_PER_SOURCE
            || rows.len() + self.rows().filter(|row| row.key.source != source).count()
                > HELD_SESSION
        {
            return Err(self.invalidate(source));
        }
        for (i, row) in rows.iter().enumerate() {
            if row.key.source != source
                || row.key.channel >= 16
                || row.key.note >= 128
                || (row.provenance == PitchProvenance::ObservedDirect && source != SourceId::DIRECT)
                || rows[..i].iter().any(|old| old.key == row.key)
            {
                self.invalidate(source);
                return Err(ConfirmedError::InvalidBatch);
            }
        }
        self.clear_source(source);
        for &row in rows {
            self.on(row)?;
        }
        Ok(())
    }

    /// Feed mapped input before the lossy display ring. Learning is invoked by
    /// the caller only after the whole same-sample group, including initial tuning.
    pub fn observe_direct(&mut self, event: NoteEvent, sample: i64) -> Result<(), ConfirmedError> {
        if event.source != SourceId::DIRECT {
            return Err(ConfirmedError::InvalidBatch);
        }
        let key = event.key();
        match event.kind {
            NoteEventKind::On { .. } => self.on(ConfirmedPitch {
                key,
                lifetime: None,
                onset_sample: sample,
                pitch_microcents: i64::from(event.note) * 100_000_000,
                provenance: PitchProvenance::ObservedDirect,
            }),
            NoteEventKind::Tuning { semitones } => {
                if !semitones.is_finite() {
                    self.invalidate(SourceId::DIRECT);
                    return Err(ConfirmedError::InvalidBatch);
                }
                let pitch =
                    ((f64::from(event.note) + f64::from(semitones)) * 100_000_000.0).round() as i64;
                self.pitch(key, None, pitch);
                Ok(())
            }
            NoteEventKind::Off => {
                self.release(key, None);
                Ok(())
            }
            NoteEventKind::SourceReset => {
                self.clear_source(SourceId::DIRECT);
                Ok(())
            }
            NoteEventKind::SessionReset => {
                self.reset();
                Ok(())
            }
        }
    }

    pub fn classes<'a>(
        &self,
        scratch: &'a mut [PitchClass; HELD_SESSION],
    ) -> Result<&'a [PitchClass], ConfirmedError> {
        if !self.is_complete() {
            return Err(ConfirmedError::Incomplete);
        }
        let mut count = 0;
        for row in self.rows() {
            scratch[count] = PitchClass::from_microcents(row.pitch_microcents);
            count += 1;
        }
        scratch[..count].sort_unstable();
        let mut unique = 0;
        for i in 0..count {
            if unique == 0 || scratch[i] != scratch[unique - 1] {
                scratch[unique] = scratch[i];
                unique += 1;
            }
        }
        Ok(&scratch[..unique])
    }
}

/// Fixed scratch and exact class-set memo. Row order, generation, octave,
/// provenance and unrelated configuration do not alter interval inference.
/// Re-arming intentionally reopens the same chord. An empty chord is remembered
/// without changing any axis; incomplete input is never learned.
pub struct LearningState {
    scratch: [PitchClass; HELD_SESSION],
    previous: [PitchClass; HELD_SESSION],
    previous_len: Option<usize>,
    pub last_pair_visits: usize,
}

impl Default for LearningState {
    fn default() -> Self {
        Self {
            scratch: [PitchClass::from_midi_note(0); HELD_SESSION],
            previous: [PitchClass::from_midi_note(0); HELD_SESSION],
            previous_len: None,
            last_pair_visits: 0,
        }
    }
}

impl LearningState {
    pub fn infer(
        &mut self,
        confirmed: &ConfirmedPitches,
        armed: bool,
    ) -> Result<Option<LearnedTuning>, ConfirmedError> {
        self.last_pair_visits = 0;
        if !armed {
            self.previous_len = None;
            return Ok(None);
        }
        let classes = confirmed.classes(&mut self.scratch)?;
        if self.previous_len == Some(classes.len()) && self.previous[..classes.len()] == *classes {
            return Ok(None);
        }
        self.previous[..classes.len()].copy_from_slice(classes);
        self.previous_len = Some(classes.len());
        if classes.is_empty() {
            return Ok(None);
        }
        Ok(Some(crate::tuning::learn_sorted_classes_counted(classes, &mut self.last_pair_visits)))
    }
}

const _: () = assert!(std::mem::size_of::<ConfirmedPitch>() <= 256);
const _: () = assert!(std::mem::align_of::<ConfirmedPitch>() <= 8);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complete_maximum_rows_reach_all_32640_pairs_and_cache_exact_classes() {
        let mut confirmed = ConfirmedPitches::default();
        for i in 0..HELD_SESSION {
            confirmed
                .on(ConfirmedPitch {
                    key: VoiceKey {
                        source: SourceId(1 + (i / 64) as u64),
                        channel: 0,
                        note: (i % 64) as u8,
                    },
                    lifetime: Some(i as u64),
                    onset_sample: 0,
                    pitch_microcents: (i as i64) * 4_687_499,
                    provenance: PitchProvenance::AcceptedOutput,
                })
                .unwrap();
        }
        let mut learning = LearningState::default();
        let result = learning.infer(&confirmed, true).unwrap().unwrap();
        assert_eq!(learning.last_pair_visits, 32640);
        assert_eq!(MAX_LEARNING_PAIRS, 32640);
        let mut classes = [PitchClass::from_midi_note(0); HELD_SESSION];
        let classes = confirmed.classes(&mut classes).unwrap();
        assert_eq!(classes.len(), 256);
        assert_eq!(result, crate::learn_tuning(classes));
        assert_eq!(learning.infer(&confirmed, true).unwrap(), None);
        assert_eq!(learning.last_pair_visits, 0);
    }

    #[test]
    fn direct_exhaustion_disables_learning_until_complete_replacement() {
        let mut confirmed = ConfirmedPitches::default();
        for note in 0..64 {
            confirmed
                .observe_direct(NoteEvent::on(0.0, SourceId::DIRECT, 0, note, 1.0), 0)
                .unwrap();
        }
        assert_eq!(
            confirmed.observe_direct(NoteEvent::on(0.0, SourceId::DIRECT, 0, 64, 1.0), 0),
            Err(ConfirmedError::Capacity)
        );
        let mut learning = LearningState::default();
        assert_eq!(learning.infer(&confirmed, true), Err(ConfirmedError::Incomplete));
        confirmed.observe_direct(NoteEvent::off(0.0, SourceId::DIRECT, 0, 0), 0).unwrap();
        assert!(!confirmed.is_complete(), "a release cannot repair truncated state");
        confirmed.replace_source(SourceId::DIRECT, &[]).unwrap();
        assert_eq!(learning.infer(&confirmed, true), Ok(None));
    }
}
