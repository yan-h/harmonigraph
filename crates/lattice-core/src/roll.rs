//! What was played and *when*: a time-stamped log of note segments, the
//! raw material for a piano-roll display.
//!
//! This is the third view of the same MIDI stream, and the three are
//! deliberately different shapes:
//!
//! - [`NoteTracker`](crate::NoteTracker) holds what is sounding *now*
//!   (plus a release tail) — no past, one entry per key.
//! - [`NoteHistory`](crate::NoteHistory) holds *which pitches* have been
//!   visited — no time, one entry per pitch.
//! - [`NoteRoll`] (here) holds *when each note sounded* — one entry per
//!   press, kept in time order, with the pitch it traced while held.
//!
//! A roll entry is opened on note-on and closed on note-off, so unlike
//! history it is written the moment a note starts rather than when its
//! fade completes — a piano roll has to draw the note that is sounding
//! right now.
//!
//! Bounded on both ends (a note count and an age), so an idle plugin left
//! open for a day does not grow without limit. Nothing here knows it will
//! be drawn.

use std::collections::{HashMap, VecDeque};

use crate::notes::Time;

/// One press of one key: when it started, when it stopped, and the pitch
/// it traced in between.
#[derive(Clone, Debug)]
pub struct RollNote {
    pub channel: u8,
    pub note: u8,
    pub velocity: f32,
    pub start: Time,
    /// When it was released, or `None` while it is still sounding.
    pub end: Option<Time>,
    /// Sounding pitch (MIDI note units, per-note tuning included) over
    /// time, oldest first. Never empty: note-on seeds it with the key's
    /// own pitch. Between breakpoints the pitch is read as a straight
    /// line — MPE bends arrive densely enough that interpolating looks
    /// like the glide it is, where holding each value would draw stairs.
    bends: Vec<(Time, f32)>,
}

impl RollNote {
    /// Breakpoints kept per note. Past this the newest overwrites the
    /// previous one, so a long continuous bend keeps an accurate *current*
    /// pitch and loses only middle detail. Generous: a note has to be bent
    /// for a while to reach it.
    pub const MAX_BENDS: usize = 64;

    /// The pitch this note started on.
    pub fn start_pitch(&self) -> f32 {
        self.bends[0].1
    }

    /// The pitch it is sounding (or last sounded) at.
    pub fn end_pitch(&self) -> f32 {
        self.bends[self.bends.len() - 1].1
    }

    /// True while the note is still held.
    pub fn is_live(&self) -> bool {
        self.end.is_none()
    }

    /// When this note stops: its release, or `now` while it is still
    /// sounding (a held note's segment reaches the present moment).
    pub fn stop(&self, now: Time) -> Time {
        self.end.unwrap_or(now).max(self.start)
    }

    /// The note as straight-line segments `((t0, pitch0), (t1, pitch1))`,
    /// in time order, ending at [`stop`](Self::stop). A note that was
    /// never bent yields exactly one flat segment.
    pub fn segments(&self, now: Time) -> impl Iterator<Item = ((Time, f32), (Time, f32))> + '_ {
        let stop = self.stop(now);
        let last = self.bends.len() - 1;
        (0..=last).filter_map(move |i| {
            let (t0, p0) = self.bends[i];
            let (t1, p1) = if i == last { (stop, p0) } else { self.bends[i + 1] };
            // A breakpoint recorded after `stop` (only reachable if the
            // shell hands out non-monotonic times) collapses to nothing.
            let t1 = t1.clamp(t0, stop.max(t0));
            (t1 > t0 || i == last).then_some(((t0, p0), (t1, p1)))
        })
    }

    /// Append a bend, folding it into the last breakpoint once
    /// [`MAX_BENDS`](Self::MAX_BENDS) is reached.
    fn bend(&mut self, at: Time, pitch: f32) {
        let last = self.bends.len() - 1;
        if self.bends[last].1 == pitch {
            return;
        }
        if self.bends.len() >= Self::MAX_BENDS {
            self.bends[last] = (at, pitch);
        } else {
            self.bends.push((at, pitch));
        }
    }
}

/// Every note press the roll still remembers, in time order.
#[derive(Default)]
pub struct NoteRoll {
    /// Finished notes, oldest release first.
    past: VecDeque<RollNote>,
    /// Still-sounding notes, keyed the way the tracker keys its held
    /// voices. Bounded by the number of keys, so never trimmed.
    live: HashMap<(u8, u8), RollNote>,
}

impl NoteRoll {
    /// Finished notes kept. The bound is on per-frame drawing work, not
    /// memory: the pane tests every note against the visible window.
    pub const MAX_NOTES: usize = 4096;
    /// Seconds of history kept. Comfortably past the longest window the
    /// display offers, so the display bound is the one that shows.
    pub const MAX_AGE: Time = 900.0;

    /// Open an entry for a new press. A retrigger with no intervening
    /// note-off closes the previous entry at the same instant, matching
    /// [`NoteTracker`](crate::NoteTracker)'s replace-the-voice rule.
    pub fn note_on(&mut self, channel: u8, note: u8, velocity: f32, pitch: f32, at: Time) {
        self.close(channel, note, at);
        self.live.insert(
            (channel, note),
            RollNote { channel, note, velocity, start: at, end: None, bends: vec![(at, pitch)] },
        );
    }

    pub fn note_off(&mut self, channel: u8, note: u8, at: Time) {
        self.close(channel, note, at);
    }

    /// Record a per-note tuning change on a sounding note. Unknown keys
    /// are ignored (the bend arrived after the note-off).
    pub fn bend(&mut self, channel: u8, note: u8, at: Time, pitch: f32) {
        if let Some(note) = self.live.get_mut(&(channel, note)) {
            note.bend(at, pitch);
        }
    }

    /// Close every sounding note (transport reset).
    pub fn all_off(&mut self, at: Time) {
        for (_, mut note) in self.live.drain() {
            note.end = Some(at.max(note.start));
            self.past.push_back(note);
        }
        self.enforce_cap();
    }

    fn close(&mut self, channel: u8, note: u8, at: Time) {
        if let Some(mut note) = self.live.remove(&(channel, note)) {
            note.end = Some(at.max(note.start));
            self.past.push_back(note);
            self.enforce_cap();
        }
    }

    fn enforce_cap(&mut self) {
        while self.past.len() > Self::MAX_NOTES {
            self.past.pop_front();
        }
    }

    /// Forget notes that finished more than [`MAX_AGE`](Self::MAX_AGE) ago.
    /// Cheap: `past` is release-ordered, so this stops at the first keeper.
    pub fn trim(&mut self, now: Time) {
        let cutoff = now - Self::MAX_AGE;
        while self.past.front().is_some_and(|n| n.end.is_some_and(|e| e < cutoff)) {
            self.past.pop_front();
        }
    }

    /// Every remembered note, finished ones first. Not sorted by start
    /// time: overlapping notes draw as overlapping ribbons either way.
    pub fn notes(&self) -> impl Iterator<Item = &RollNote> {
        self.past.iter().chain(self.live.values())
    }

    /// The most recent moment any remembered note was sounding — `now`
    /// while anything is held — or `None` if the roll is empty.
    ///
    /// This is how a display decides whether it still has to animate: a
    /// roll scrolls for as long as its window reaches back to something,
    /// which outlasts the last voice's release fade. O(1): `past` is
    /// release-ordered, so the latest release is the one on the end.
    pub fn latest_activity(&self, now: Time) -> Option<Time> {
        if !self.live.is_empty() {
            return Some(now);
        }
        self.past.back().and_then(|note| note.end)
    }

    pub fn len(&self) -> usize {
        self.past.len() + self.live.len()
    }

    pub fn is_empty(&self) -> bool {
        self.past.is_empty() && self.live.is_empty()
    }

    /// Forget everything, sounding notes included. A note held across a
    /// clear is gone until it is played again — its bends land nowhere.
    /// That is what "clear the roll" is asked for; the alternative (keep
    /// the held ones) leaves a stub whose start time is already off-screen.
    pub fn clear(&mut self) {
        self.past.clear();
        self.live.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notes::{NoteEvent, NoteEventKind, NoteTracker};

    fn on(time: Time, note: u8) -> NoteEvent {
        NoteEvent { time, channel: 0, note, kind: NoteEventKind::On { velocity: 0.8 } }
    }

    fn off(time: Time, note: u8) -> NoteEvent {
        NoteEvent { time, channel: 0, note, kind: NoteEventKind::Off }
    }

    fn bend(time: Time, note: u8, semitones: f32) -> NoteEvent {
        NoteEvent { time, channel: 0, note, kind: NoteEventKind::Tuning { semitones } }
    }

    #[test]
    fn a_note_is_in_the_roll_the_moment_it_sounds() {
        // Unlike history, which only takes a note once its fade completes:
        // a piano roll has to draw the note being held right now.
        let mut tracker = NoteTracker::new();
        tracker.handle_event(on(0.0, 60));
        let roll = tracker.roll();
        assert_eq!(roll.len(), 1);
        let note = roll.notes().next().unwrap();
        assert_eq!(note.start, 0.0);
        assert!(note.is_live());
        assert_eq!(note.end, None);
    }

    #[test]
    fn a_held_note_reaches_the_present_moment() {
        let mut tracker = NoteTracker::new();
        tracker.handle_event(on(1.0, 60));
        let note = tracker.roll().notes().next().unwrap();
        assert_eq!(note.stop(9.0), 9.0);
        tracker.handle_event(off(4.0, 60));
        let note = tracker.roll().notes().next().unwrap();
        assert_eq!(note.stop(9.0), 4.0, "a released note stops at its release");
    }

    #[test]
    fn repeats_of_one_pitch_stay_separate_entries() {
        // The opposite of NoteHistory, which folds them into one visit.
        let mut tracker = NoteTracker::new();
        tracker.handle_event(on(0.0, 60));
        tracker.handle_event(off(1.0, 60));
        tracker.handle_event(on(2.0, 60));
        tracker.handle_event(off(3.0, 60));
        assert_eq!(tracker.roll().len(), 2);
    }

    #[test]
    fn a_retrigger_without_an_off_closes_the_previous_entry() {
        let mut tracker = NoteTracker::new();
        tracker.handle_event(on(0.0, 60));
        tracker.handle_event(on(2.0, 60));
        let roll = tracker.roll();
        assert_eq!(roll.len(), 2);
        let closed = roll.notes().find(|n| !n.is_live()).unwrap();
        assert_eq!((closed.start, closed.end), (0.0, Some(2.0)));
    }

    #[test]
    fn bends_become_breakpoints_and_interpolate() {
        let mut tracker = NoteTracker::new();
        tracker.handle_event(on(0.0, 60));
        tracker.handle_event(bend(1.0, 60, 2.0));
        tracker.handle_event(off(2.0, 60));
        let roll = tracker.roll();
        let note = roll.notes().next().unwrap();
        assert_eq!(note.start_pitch(), 60.0);
        assert_eq!(note.end_pitch(), 62.0);
        let segments: Vec<_> = note.segments(5.0).collect();
        assert_eq!(segments.len(), 2, "{segments:?}");
        // The glide, then the flat tail at the bent pitch.
        assert_eq!(segments[0], ((0.0, 60.0), (1.0, 62.0)));
        assert_eq!(segments[1], ((1.0, 62.0), (2.0, 62.0)));
    }

    #[test]
    fn an_unbent_note_is_one_flat_segment() {
        let mut tracker = NoteTracker::new();
        tracker.handle_event(on(0.0, 67));
        tracker.handle_event(off(1.5, 67));
        let roll = tracker.roll();
        let segments: Vec<_> = roll.notes().next().unwrap().segments(9.0).collect();
        assert_eq!(segments, vec![((0.0, 67.0), (1.5, 67.0))]);
    }

    #[test]
    fn breakpoints_are_capped_and_the_newest_wins() {
        let mut tracker = NoteTracker::new();
        tracker.handle_event(on(0.0, 60));
        for i in 1..(RollNote::MAX_BENDS as i32 * 2) {
            tracker.handle_event(bend(i as Time, 60, i as f32 * 0.01));
        }
        let roll = tracker.roll();
        let note = roll.notes().next().unwrap();
        assert!(note.segments(999.0).count() <= RollNote::MAX_BENDS);
        // The current pitch survives the fold, which is the point of it.
        let last = (RollNote::MAX_BENDS as i32 * 2 - 1) as f32 * 0.01;
        assert!((note.end_pitch() - (60.0 + last)).abs() < 1e-4);
    }

    #[test]
    fn all_off_closes_every_sounding_note() {
        let mut tracker = NoteTracker::new();
        tracker.handle_event(on(0.0, 60));
        tracker.handle_event(on(0.0, 64));
        tracker.handle_event(NoteEvent {
            time: 3.0,
            channel: 0,
            note: 0,
            kind: NoteEventKind::AllOff,
        });
        let roll = tracker.roll();
        assert_eq!(roll.len(), 2);
        assert!(roll.notes().all(|n| n.end == Some(3.0)));
    }

    #[test]
    fn ignored_channels_never_reach_the_roll() {
        let mut tracker = NoteTracker::new();
        tracker.handle_event(NoteEvent {
            time: 0.0,
            channel: 15,
            note: 60,
            kind: NoteEventKind::On { velocity: 0.8 },
        });
        assert!(tracker.roll().is_empty());
    }

    #[test]
    fn old_notes_age_out_but_sounding_ones_never_do() {
        let mut tracker = NoteTracker::new();
        tracker.handle_event(on(0.0, 60));
        tracker.handle_event(off(1.0, 60));
        tracker.handle_event(on(1.0, 72)); // still held at the far future
        tracker.prune(NoteRoll::MAX_AGE + 100.0, 1.0);
        let roll = tracker.roll();
        assert_eq!(roll.len(), 1);
        assert!(roll.notes().next().unwrap().is_live());
    }

    #[test]
    fn the_note_cap_drops_the_oldest_releases_first() {
        let mut tracker = NoteTracker::new();
        for i in 0..(NoteRoll::MAX_NOTES + 8) {
            let t = i as Time;
            tracker.handle_event(on(t, 60));
            tracker.handle_event(off(t + 0.5, 60));
        }
        let roll = tracker.roll();
        assert_eq!(roll.len(), NoteRoll::MAX_NOTES);
        assert!(
            roll.notes().all(|n| n.start >= 8.0),
            "the first eight presses should have been dropped"
        );
    }

    #[test]
    fn clear_forgets_everything() {
        let mut tracker = NoteTracker::new();
        tracker.handle_event(on(0.0, 60));
        tracker.handle_event(off(1.0, 60));
        tracker.handle_event(on(1.0, 64));
        assert!(!tracker.roll().is_empty());
        tracker.clear_roll();
        assert!(tracker.roll().is_empty());
    }
}
