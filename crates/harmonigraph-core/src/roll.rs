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

use std::collections::{BTreeMap, VecDeque};

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
    /// The pitch reached by the end of the onset — see
    /// [`settled_pitch`](Self::settled_pitch).
    ///
    /// Kept as a field rather than read back out of `bends`, so that it is
    /// genuinely immutable once the note is [`SETTLE`](Self::SETTLE) old. Read
    /// back, it is not: the [`MAX_BENDS`](Self::MAX_BENDS) fold overwrites the
    /// LAST breakpoint in place, so a note bent hard enough to fill the vector
    /// inside the onset window would have its final in-window breakpoint
    /// replaced by an out-of-window one, and the answer would move — long
    /// after anything derived from it had been told it could not.
    settled: f32,
}

impl RollNote {
    /// Breakpoints kept per note. Past this the newest overwrites the
    /// previous one, so a long continuous bend keeps an accurate *current*
    /// pitch and loses only middle detail. Generous: a note has to be bent
    /// for a while to reach it.
    pub const MAX_BENDS: usize = 64;

    /// How long after the onset a pitch change still counts as part of the
    /// onset rather than as a bend — see [`settled_pitch`](Self::settled_pitch).
    ///
    /// Long enough to cover a per-note tuning that arrives a block or two
    /// behind its note-on (a 512-frame block at 48 kHz is 11 ms), far short of
    /// any glide played on purpose.
    pub const SETTLE: Time = 0.05;

    /// The pitch this note started on.
    pub fn start_pitch(&self) -> f32 {
        self.bends[0].1
    }

    /// The pitch this note SETTLED on: the last breakpoint within
    /// [`SETTLE`](Self::SETTLE) of the onset.
    ///
    /// The pitch that identifies a note, as against
    /// [`start_pitch`](Self::start_pitch), which is where the KEY sits. Under
    /// per-note tuning the two are different notes: a retuned note arrives as
    /// a note-on at its equal-tempered pitch followed by a tuning expression,
    /// so its first breakpoint is 12-EDO however the tuning is set, and every
    /// note in a just-intoned piece would answer `start_pitch` on the same
    /// twelve pitches. This is the plugin's whole subject matter, so a caller
    /// asking "which pitch is this" wants this one.
    ///
    /// Immutable once the note is [`SETTLE`](Self::SETTLE) old, which is what
    /// makes it safe to derive something from and keep — unlike
    /// [`end_pitch`](Self::end_pitch), which moves under a held note for as
    /// long as it is being bent.
    ///
    /// Like [`NoteHistory`](crate::NoteHistory), it prefers a note's SOUNDING
    /// pitch to its key; unlike history, which records where a note ended, it
    /// records where the note began once its tuning had landed. For a glided
    /// note the two name different pitches, and each is right about a
    /// different moment.
    pub fn settled_pitch(&self) -> f32 {
        self.settled
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
        // Still part of the onset: this is how a retuned note reaches the
        // pitch it is actually playing. Recorded as it happens, so the fold
        // above can never take it back.
        if at <= self.start + Self::SETTLE {
            self.settled = pitch;
        }
    }
}

/// Every note press the roll still remembers, in time order.
#[derive(Clone, Default)]
pub struct NoteRoll {
    /// Finished notes, oldest release first.
    past: VecDeque<RollNote>,
    /// Still-sounding notes, keyed and ORDERED the way the tracker keys and
    /// orders its held voices (see [`NoteTracker`](crate::NoteTracker), and
    /// #135). Not for the pane's sake — both readers of [`notes`](Self::notes)
    /// sort for themselves — but because `all_off` drains this map into `past`
    /// in whatever order it iterates, and `past` is what `enforce_cap` pops
    /// from the front of: past `MAX_NOTES`, a hashed order here would decide
    /// which press is forgotten. Bounded by the number of keys, so never
    /// trimmed.
    live: BTreeMap<(u8, u8), RollNote>,
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
    /// [`NoteTracker`](crate::NoteTracker)'s replace-the-voice rule — and if
    /// that instant is the previous entry's own onset, [`close`](Self::close)
    /// drops it, the note having never sounded.
    pub fn note_on(&mut self, channel: u8, note: u8, velocity: f32, pitch: f32, at: Time) {
        self.close(channel, note, at);
        self.live.insert(
            (channel, note),
            RollNote {
                channel,
                note,
                velocity,
                start: at,
                end: None,
                bends: vec![(at, pitch)],
                settled: pitch,
            },
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
        for mut note in std::mem::take(&mut self.live).into_values() {
            note.end = Some(at.max(note.start));
            self.past.push_back(note);
        }
        self.enforce_cap();
    }

    /// Retire the entry for one key — but only if it sounded.
    ///
    /// A note whose off lands at or before its own on never sounded, and the
    /// roll drops it rather than remembering a note of no duration. Hosts do
    /// emit these: one press can arrive as an on, an off and a second on all
    /// stamped at the same sample, and the pair in the middle leaves an entry
    /// that begins and ends together.
    ///
    /// It has to be dropped rather than merely drawn small, because what shows
    /// is not the ribbon. A ribbon of no length is floored to a couple of
    /// pixels and reads as grain, while the NAME the roll writes on it is full
    /// size and anchored at the moment it ended — so a single press leaves a
    /// letter scrolling away from the letter held at the now-line, as though
    /// the key had been played twice.
    ///
    /// The test is `at <= start`, which is a fact about the note rather than a
    /// span short enough to look wrong: a key cannot be released before it was
    /// pressed, and the shortest note anyone can play still ends after it
    /// began. A staccato note is untouched however brief it is.
    ///
    /// [`all_off`](Self::all_off) deliberately does NOT go through this. Its
    /// timestamp is a transport event's, not a statement about any one note, so
    /// a held note it closes did sound — only its length is unknown, and
    /// clamping that to the onset must not erase it.
    fn close(&mut self, channel: u8, note: u8, at: Time) {
        if let Some(mut note) = self.live.remove(&(channel, note)) {
            if at <= note.start {
                return;
            }
            note.end = Some(at);
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

    /// Every remembered note, finished ones first and then the sounding
    /// ones in `(channel, note)` order. Not sorted by start time:
    /// overlapping notes draw as overlapping ribbons either way, and the
    /// pane sorts for itself when the paint order matters.
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
    use crate::notes::{Envelope, NoteEvent, NoteEventKind, NoteTracker};

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

    /// Sounding notes reach the pane through `notes()`, and the transport
    /// reset walks them into `past` — which then holds that order for as
    /// long as it remembers them. Both have to be the music's order, not a
    /// map's; see `NoteTracker::voices` and #135.
    ///
    /// Pressed high to low so only the key order can pass.
    #[test]
    fn sounding_notes_come_back_in_channel_note_order() {
        let mut tracker = NoteTracker::new();
        let expected: Vec<u8> = (40..104u8).collect();
        for &note in expected.iter().rev() {
            tracker.handle_event(on(0.0, note));
        }
        let live: Vec<u8> = tracker.roll().notes().map(|n| n.note).collect();
        assert_eq!(live, expected, "sounding notes must iterate in key order");

        tracker.all_notes_off(1.0);
        let past: Vec<u8> = tracker.roll().notes().map(|n| n.note).collect();
        assert_eq!(past, expected, "the finished tail must inherit key order");
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

    /// A note switched off the instant it started never sounded, and the roll
    /// does not remember it.
    #[test]
    fn a_note_switched_off_the_instant_it_started_never_sounded() {
        let mut tracker = NoteTracker::new();
        tracker.handle_event(on(1.0, 60));
        tracker.handle_event(off(1.0, 60));
        assert!(tracker.roll().is_empty());
    }

    /// The shortest note anyone can play still ends after it began, and is
    /// kept — the rule above is a fact about the note, not a span short enough
    /// to look wrong.
    #[test]
    fn a_staccato_note_is_kept_however_brief() {
        let mut tracker = NoteTracker::new();
        tracker.handle_event(on(1.0, 60));
        tracker.handle_event(off(1.000_001, 60));
        assert_eq!(tracker.roll().len(), 1);
    }

    /// One press, as a host really delivers it: an on, an off and a second on
    /// all stamped at the same sample, then the release nearly two seconds
    /// later. That is ONE note on the roll.
    ///
    /// The pair in the middle otherwise leaves an entry that begins and ends
    /// together — and what shows is not its ribbon, which is floored to a
    /// couple of pixels, but the NAME written on it at full size and anchored
    /// where it ended. So a single press put a letter on the pane that
    /// scrolled away from the letter held at the now-line, as though the key
    /// had been played twice.
    #[test]
    fn one_press_delivered_as_on_off_on_is_one_note() {
        let mut tracker = NoteTracker::new();
        tracker.handle_event(on(9.9314, 60));
        tracker.handle_event(off(9.9314, 60));
        tracker.handle_event(on(9.9314, 60));
        assert_eq!(tracker.roll().len(), 1, "the off/on pair is not a note of its own");
        assert!(tracker.roll().notes().next().unwrap().is_live(), "the press is still sounding");

        tracker.handle_event(off(11.6881, 60));
        let roll = tracker.roll();
        assert_eq!(roll.len(), 1);
        let note = roll.notes().next().unwrap();
        assert_eq!((note.start, note.end), (9.9314, Some(11.6881)));
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
        tracker.prune(NoteRoll::MAX_AGE + 100.0, &Envelope::default());
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

    /// A retuned note is a note-on at its equal-tempered pitch followed by a
    /// tuning expression, so the pitch it was PRESSED at says nothing about
    /// which note it is — in a just-intoned piece every note would answer on
    /// the same twelve. What it settles on within `SETTLE` is the answer; a
    /// bend after that is a glide, and leaves the note's identity alone.
    #[test]
    fn a_note_settles_on_the_pitch_its_tuning_lands_it_at() {
        let mut tracker = NoteTracker::new();
        tracker.handle_event(on(0.0, 64));
        assert_eq!(tracker.roll().notes().next().unwrap().settled_pitch(), 64.0);

        // The tuning, arriving a block behind its note-on.
        tracker.handle_event(bend(0.01, 64, -0.137));
        let note = tracker.roll().notes().next().unwrap();
        assert!((note.settled_pitch() - 63.863).abs() < 1e-4, "{}", note.settled_pitch());
        assert_eq!(note.start_pitch(), 64.0, "the KEY is still where it always was");

        // A glide, well past the window: it moves where the note sounds and
        // not which note it is.
        tracker.handle_event(bend(1.0, 64, 2.0));
        let note = tracker.roll().notes().next().unwrap();
        assert!((note.settled_pitch() - 63.863).abs() < 1e-4);
        assert_eq!(note.end_pitch(), 66.0);
    }

    /// A bend LANDING on the window's edge is still part of the onset, and one
    /// past it is not — the constant is a real boundary, not a rough size.
    #[test]
    fn the_settle_window_has_edges() {
        let settled = |at: Time| -> f32 {
            let mut tracker = NoteTracker::new();
            tracker.handle_event(on(0.0, 60));
            tracker.handle_event(bend(at, 60, 1.0));
            let pitch = tracker.roll().notes().next().unwrap().settled_pitch();
            pitch
        };
        assert_eq!(settled(RollNote::SETTLE), 61.0, "on the edge counts");
        assert_eq!(settled(RollNote::SETTLE * 2.0), 60.0, "past it does not");
    }

    /// The settled pitch does not move once the onset is over — including
    /// under an expression ramp fast enough to fill the breakpoint vector
    /// inside the onset itself.
    ///
    /// Read back out of `bends` it DOES move: the MAX_BENDS fold overwrites
    /// the last breakpoint in place, so the final in-window one is eventually
    /// replaced by an out-of-window one and `take_while` drops back a step.
    /// Small — one sub-50 ms increment — and enough to re-place a name that
    /// had been told the pitch was fixed.
    #[test]
    fn a_ramp_that_fills_the_breakpoints_cannot_move_the_settled_pitch() {
        let mut tracker = NoteTracker::new();
        tracker.handle_event(on(0.0, 60));
        // Fill the vector inside the settle window...
        for i in 1..RollNote::MAX_BENDS {
            let at = RollNote::SETTLE * (i as Time / RollNote::MAX_BENDS as Time);
            tracker.handle_event(bend(at, 60, i as f32 * 0.001));
        }
        let settled = tracker.roll().notes().next().unwrap().settled_pitch();
        assert!(settled > 60.0, "the ramp did land inside the window: {settled}");

        // ...then keep bending, long after. The fold now rewrites the last
        // breakpoint, which was the one inside the window.
        for i in 1..10 {
            tracker.handle_event(bend(1.0 + i as Time, 60, 1.0 + i as f32));
        }
        let note = tracker.roll().notes().next().unwrap();
        assert_eq!(note.settled_pitch(), settled, "the onset is over; the answer is fixed");
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
