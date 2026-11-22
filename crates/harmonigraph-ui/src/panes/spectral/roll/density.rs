//! Resolve repeated notes in screen space before making their ribbons.
//!
//! Each note reserves a one-point core. If two cores cannot leave a two-point
//! gap, they belong to one dense run. Otherwise the cut sits as close to the
//! factual gap as those cores allow. No MIDI times are changed.

use harmonigraph_core::{RollNote, VoiceKey};
use std::collections::HashMap;

pub(super) const GAP_PT: f64 = 2.0;
pub(super) type Segment = ((f64, f32), (f64, f32));

pub(super) struct Ribbon<'a> {
    pub notes: Vec<&'a RollNote>,
    pub start: f64,
    pub stop: f64,
    /// Where its run began: its first note's onset, or an earlier one's that
    /// the window's cull left out of `notes` ([`reach_back`]). What the
    /// ribbons paint in order of.
    onset: f64,
}

impl<'a> Ribbon<'a> {
    pub fn last(&self) -> &'a RollNote {
        self.notes[self.notes.len() - 1]
    }

    pub fn dense(&self) -> bool {
        self.notes.len() > 1
    }

    /// Close only the small rests inside a dense run, holding the preceding
    /// pitch. Merge flat neighbours into one box so subpixel attacks do not
    /// leave individually antialiased ends inside the continuous body.
    ///
    /// Into `out`, which it clears first: one buffer serves every ribbon.
    pub fn segments(&self, now: f64, out: &mut Vec<Segment>) {
        out.clear();
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
    }
}

/// A note's drawn start, floored to `minimum` before its stop, and the
/// one-point core centred on that body.
fn body(note: &RollNote, now: f64, minimum: f64) -> (f64, f64, f64) {
    let stop = note.stop(now);
    let start = note.start.min(stop - minimum);
    let center = (start + stop) * 0.5;
    (start, center - minimum * 0.5, center + minimum * 0.5)
}

/// Where the cut between `last` and the next note of its key may fall, as
/// `(earliest, latest)`. Latest before earliest leaves the cut no room, and
/// the two join one dense run. `None` where observation is missing, which is
/// neither a repeated attack nor a rest to fill.
fn cuts(last: &RollNote, note: &RollNote, now: f64, minimum: f64, gap: f64) -> Option<(f64, f64)> {
    (last.observed_until.is_none() && last.history_complete && note.history_complete).then(|| {
        let (_, _, last_core) = body(last, now, minimum);
        let (_, core, _) = body(note, now, minimum);
        (last_core + gap * 0.5, core - gap * 0.5)
    })
}

fn joins(last: &RollNote, note: &RollNote, now: f64, minimum: f64, gap: f64) -> bool {
    cuts(last, note, now, minimum, gap).is_some_and(|(earliest, latest)| earliest > latest)
}

/// Input is in onset order, and so is the output, by each ribbon's first
/// note: the order the ribbons paint in, until [`reach_back`] finds a run
/// that began before the window's notes.
/// The protected cores make both cuts around a middle note compatible.
pub(super) fn layout<'a>(
    notes: &[&'a RollNote],
    now: f64,
    minimum: f64,
    gap: f64,
) -> Vec<Ribbon<'a>> {
    let mut out: Vec<Ribbon<'a>> = Vec::with_capacity(notes.len());
    let mut previous: HashMap<VoiceKey, usize> = HashMap::new();
    for &note in notes {
        let stop = note.stop(now);
        let (mut start, _, _) = body(note, now, minimum);
        if let Some(&index) = previous.get(&note.key()) {
            let prev = &mut out[index];
            if let Some((earliest, latest)) = cuts(prev.last(), note, now, minimum, gap) {
                if earliest > latest {
                    prev.notes.push(note);
                    prev.stop = stop;
                    continue;
                }
                if start - prev.stop < gap {
                    let cut = ((prev.last().stop(now) + note.start) * 0.5).clamp(earliest, latest);
                    prev.stop = prev.stop.min(cut - gap * 0.5);
                    start = start.max(cut + gap * 0.5);
                }
            }
        }
        previous.insert(note.key(), out.len());
        out.push(Ribbon { notes: vec![note], start, stop, onset: note.start });
    }
    out
}

/// Date each ribbon from where its run truly began, and keep the ribbons in
/// that order: a dense run paints whole at its first onset, beneath a voice
/// that began between two of its attacks.
///
/// `layout` sees only the notes the window's cull kept. A run leaving the
/// window would otherwise start at its oldest attack still in, and pass over
/// such a voice as that attack moved past the voice's onset. So the first
/// ribbon of each key, where its first note is near enough the cull to join a
/// note it left out, walks back through `culled`, the notes stopping before
/// `culled_before`, for as long as each joins the next by `layout`'s own rule.
///
/// `culled` is in the roll's order, which is the order the notes were
/// released, and for one key the order they started; walked newest first,
/// each key's notes come in the order the walk needs, and a ribbon whose
/// predecessor does not join it is done at the first note of its key.
pub(super) fn reach_back<'a>(
    ribbons: &mut [Ribbon<'a>],
    culled: &[&'a RollNote],
    culled_before: f64,
    now: f64,
    minimum: f64,
    gap: f64,
) {
    // A note left out stops before `culled_before`, and so does its core, so
    // it can join only a note whose core starts within a gap of that. A later
    // ribbon of a key has its predecessor in the window, which `layout` did
    // not join it to.
    let mut walking: Vec<(usize, &'a RollNote)> = (0..ribbons.len())
        .map(|i| (i, ribbons[i].notes[0]))
        .filter(|&(i, first)| {
            body(first, now, minimum).1 - gap < culled_before
                && ribbons[..i].iter().all(|r| r.notes[0].key() != first.key())
        })
        .collect();
    let mut moved = false;
    for &note in culled.iter().rev() {
        if walking.is_empty() {
            break;
        }
        walking.retain_mut(|(i, first)| {
            if note.key() != first.key() || note.start >= first.start {
                return true;
            }
            if joins(note, first, now, minimum, gap) {
                *first = note;
                return true;
            }
            moved |= first.start < ribbons[*i].onset;
            ribbons[*i].onset = first.start;
            false
        });
    }
    // Walked back past every note left out.
    for (i, first) in walking {
        moved |= first.start < ribbons[i].onset;
        ribbons[i].onset = first.start;
    }
    if moved {
        ribbons.sort_by(|a, b| {
            a.onset.total_cmp(&b.onset).then_with(|| a.notes[0].key().cmp(&b.notes[0].key()))
        });
    }
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
        let mut segments = Vec::new();
        ribbon.segments(3.0, &mut segments);
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
