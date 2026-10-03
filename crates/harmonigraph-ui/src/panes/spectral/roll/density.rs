//! Resolve repeated notes in screen space before making their ribbons.
//!
//! Each note reserves a one-point core. If two cores cannot leave a two-point
//! gap, they belong to one dense run. Otherwise the cut sits as close to the
//! factual gap as those cores allow. No MIDI times are changed.

use harmonigraph_core::{RollNote, VoiceKey};
use std::collections::HashMap;

pub(super) const GAP_PT: f64 = 2.0;
type Segment = ((f64, f32), (f64, f32));

pub(super) struct Ribbon<'a> {
    pub notes: Vec<&'a RollNote>,
    pub start: f64,
    pub stop: f64,
    /// Original onset index and time where another voice has interleaved.
    /// Geometry stays shared; the renderer clips and orders these paint runs.
    pub paint_starts: Vec<(usize, f64)>,
    last_order: usize,
    core_start: f64,
    core_stop: f64,
}

impl<'a> Ribbon<'a> {
    pub fn last(&self) -> &'a RollNote {
        self.notes[self.notes.len() - 1]
    }

    pub fn dense(&self) -> bool {
        self.notes.len() > 1
    }

    pub fn note_at(&self, at: f64) -> &'a RollNote {
        let i = self.notes.partition_point(|n| n.start <= at).saturating_sub(1);
        self.notes[i]
    }

    /// Close only the small rests inside a dense run, holding the preceding
    /// pitch. Merge flat neighbours into one box so subpixel attacks do not
    /// leave individually antialiased ends inside the continuous body.
    pub fn segments(&self, now: f64) -> Vec<Segment> {
        let mut out: Vec<Segment> = Vec::new();
        let mut push = |((t0, p0), (t1, p1)): Segment| {
            if t1 < self.start || t0 > self.stop {
                return;
            }
            let (a, b) = (t0.max(self.start), t1.min(self.stop));
            let pitch = |t| {
                if t1 > t0 {
                    p0 + (p1 - p0) * ((t - t0) / (t1 - t0)) as f32
                } else {
                    p1
                }
            };
            let (pa, pb) = (pitch(a), pitch(b));
            if let Some(last) = out.last_mut() {
                if last.1 .0 == a && last.0 .1 == pa && last.1 .1 == pa && pa == pb {
                    last.1 .0 = b;
                    return;
                }
            }
            out.push(((a, pa), (b, pb)));
        };
        let first = self.notes[0];
        if self.start < first.start {
            // Same-time tuning replaces the raw key before it ever sounds.
            // Padding must meet the first actual segment, not leave a spur at
            // the untuned key. Later bends retain their recorded geometry.
            let pitch = first.segments(now).next().map_or(first.start_pitch(), |s| s.0 .1);
            push(((self.start, pitch), (first.start, pitch)));
        }
        for (i, note) in self.notes.iter().enumerate() {
            for segment in note.segments(now) {
                push(segment);
            }
            if let Some(next) = self.notes.get(i + 1) {
                if note.stop(now) < next.start {
                    push(((note.stop(now), note.end_pitch()), (next.start, note.end_pitch())));
                }
            }
        }
        out
    }
}

/// Input is in onset order; paint runs retain that order across voices.
/// The protected cores make both cuts around a middle note compatible.
pub(super) fn layout<'a>(
    notes: &[&'a RollNote],
    now: f64,
    minimum: f64,
    gap: f64,
) -> Vec<Ribbon<'a>> {
    let mut out: Vec<Ribbon<'a>> = Vec::with_capacity(notes.len());
    let mut previous: HashMap<VoiceKey, usize> = HashMap::new();
    for (order, &note) in notes.iter().enumerate() {
        let stop = note.stop(now);
        let start = note.start.min(stop - minimum);
        let center = (start + stop) * 0.5;
        let mut ribbon = Ribbon {
            notes: vec![note],
            start,
            stop,
            paint_starts: vec![(order, f64::NEG_INFINITY)],
            last_order: order,
            core_start: center - minimum * 0.5,
            core_stop: center + minimum * 0.5,
        };
        if let Some(&index) = previous.get(&note.key()) {
            let prev = &mut out[index];
            let last = prev.last();
            // Missing observation is not a repeated attack or a rest to fill.
            if last.observed_until.is_none() && last.history_complete && note.history_complete {
                let cut_min = prev.core_stop + gap * 0.5;
                let cut_max = ribbon.core_start - gap * 0.5;
                if cut_min > cut_max {
                    if prev.last_order + 1 != order {
                        prev.paint_starts.push((order, note.start));
                    }
                    prev.last_order = order;
                    prev.notes.push(note);
                    prev.stop = stop;
                    prev.core_stop = ribbon.core_stop;
                    continue;
                }
                if ribbon.start - prev.stop < gap {
                    let cut = ((last.stop(now) + note.start) * 0.5).clamp(cut_min, cut_max);
                    prev.stop = prev.stop.min(cut - gap * 0.5);
                    ribbon.start = ribbon.start.max(cut + gap * 0.5);
                }
            }
        }
        previous.insert(note.key(), out.len());
        out.push(ribbon);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use harmonigraph_core::{NoteRoll, SourceId};

    #[test]
    fn dense_onset_padding_uses_the_sounding_pitch() {
        let mut roll = NoteRoll::default();
        let key = VoiceKey { source: SourceId::DIRECT, channel: 0, note: 60 };
        for i in 0..64 {
            let start = 1.0 + f64::from(i) * 0.025;
            roll.note_on(key, 1.0, 60.0, start);
            roll.bend(key, start, 60.3);
            roll.note_off(key, start + 0.001);
        }
        let notes: Vec<_> = roll.notes().collect();
        let ribbons = layout(&notes, 3.0, 0.1, 0.2);
        assert_eq!(ribbons.len(), 1, "fixture must form a dense run");
        let ribbon = &ribbons[0];
        assert!(ribbon.start < notes[0].start, "fixture must need onset padding");
        let segments = ribbon.segments(3.0);
        assert_eq!(segments.len(), 1, "padding left a separate line at the untuned key");
        assert_eq!(segments[0], ((ribbon.start, 60.3), (ribbon.stop, 60.3)));
    }

    #[test]
    fn dense_runs_stop_at_resolvable_rests_and_never_join_other_voices() {
        let mut roll = NoteRoll::default();
        for (source, channel, note) in [(0, 0, 60), (1, 0, 60), (0, 1, 60), (0, 0, 61)] {
            let key = VoiceKey { source: SourceId(source), channel, note };
            for start in [0.0, 0.1, 10.0, 10.1] {
                roll.note_on(key, 1.0, f32::from(note), start);
                roll.note_off(key, start + 0.05);
            }
        }
        let mut notes: Vec<_> = roll.notes().collect();
        notes.sort_by(|a, b| a.start.total_cmp(&b.start).then_with(|| a.key().cmp(&b.key())));
        let ribbons = layout(&notes, 12.0, 1.0, GAP_PT);
        assert_eq!(ribbons.len(), 8);
        for ribbon in ribbons {
            assert_eq!(ribbon.notes.len(), 2);
            assert_eq!(ribbon.notes[0].key(), ribbon.notes[1].key());
            assert!(ribbon.stop - ribbon.start < 2.0, "filled a long rest");
        }
    }
}
