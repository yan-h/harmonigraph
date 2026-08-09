//! The spectrum analyzer that keeps running while the editor window is closed.
//!
//! `process` fills both audio→GUI rings whether or not anyone is watching, but
//! only a GUI frame ever drained them — and the two rings fail very differently
//! when nothing does. The note ring holds 4096 events, minutes of playing, so a
//! reopened window replays what it missed and the roll fills in. The audio ring
//! holds [`AUDIO_RING_CAPACITY`](crate::AUDIO_RING_CAPACITY) samples, 1.37 s of
//! stereo at 48 kHz, so it saturates within seconds and every sample after that
//! is dropped on the floor. That asymmetry is what this exists to remove:
//! reopening a window shut for a minute gave back a roll full of notes over a
//! heatmap with a minute-wide hole in it.
//!
//! Both rings are drained here, not just the audio one, and the second is not a
//! bonus. A full ring drops the NEWEST, so once the note ring saturates the
//! events that survive are the OLD ones, and `ClockMapper` then snaps the
//! batch's newest to `now` and shifts the whole backlog forward. Leaving the
//! notes to the reopen while the columns are captured at their true times would
//! draw ribbons and ridges that disagree — and a heatmap that disagrees with
//! the roll above it is worse than one that admits the gap.
//!
//! **What it costs is a continuous FFT.** 0.42 ms per stereo column at the
//! default 8192-point window, 125 columns a second: about 5% of a core for as
//! long as the plugin is instantiated, paid now even by a project nobody opens
//! the editor of, and paid continuously because a DAW streams silence as
//! diligently as it streams music. Memory does not move — [`SpectrumHistory`]
//! bounds itself at ~30 MB and already outlived the window, so this only
//! reaches that bound sooner rather than raising it. The hop is the only lever
//! if that ever needs to come down (32 ms instead of 8 puts it at 1.2%), but it
//! is not one to reach for casually: the store's tiers, `live_slab`'s ladder
//! and `COLUMNS_PER_SLAB` are all rungs of one shared `FFT_INTERVAL`, and a
//! second hop rate is the exact shape of the duplicated-column bug those
//! comments warn about.
//!
//! Nothing here analyzes anything itself. It calls
//! [`EditorShared::catch_up`](crate::editor::EditorShared::catch_up), the same
//! drain a frame runs; see there for why sharing that one path is the point.
//!
//! [`SpectrumHistory`]: harmonigraph_core::SpectrumHistory

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use parking_lot::Mutex;

use crate::editor::{EditorShared, EguiState};

/// How often the analyzer looks for audio the host has left in the ring.
///
/// Bounded above by the ring itself, and tightly: it holds
/// [`AUDIO_RING_CAPACITY`](crate::AUDIO_RING_CAPACITY) interleaved samples,
/// which is 1.37 s of stereo at 48 kHz but only 0.34 s at 192 kHz — and a poll
/// that ever slipped past that span would drop audio silently and for good,
/// which is the failure this module exists to fix, reintroduced by its own
/// pacing. 20 ms leaves 17x margin at the worst rate a host can hand us, so the
/// scheduler has to lose this thread for a third of a second before anything is
/// lost. `the_ring_holds_many_polls_of_audio_at_any_rate_a_host_offers` is what
/// keeps the two numbers in that relation.
///
/// Bounded below by nothing that matters — the drain is the same work whenever
/// it happens, and 50 wakeups a second against a 5%-of-a-core FFT load is not
/// where the cost is. What a shorter poll would buy is a shorter lock hold, and
/// at 20 ms that is already about a millisecond of FFT.
pub(crate) const POLL: Duration = Duration::from_millis(20);

/// The thread, and the flag that ends it.
///
/// Held by the plugin purely for its [`Drop`]: a plugin removed from a project
/// must not leave a thread behind doing FFTs on a ring nobody fills again.
pub(crate) struct BackgroundAnalyzer {
    stop: Arc<AtomicBool>,
    /// `None` when the thread could not be spawned, which costs this feature
    /// and nothing else — an open editor still drains both rings itself.
    thread: Option<JoinHandle<()>>,
}

impl BackgroundAnalyzer {
    pub(crate) fn spawn(
        shared: Arc<Mutex<EditorShared>>,
        editor_state: Arc<EguiState>,
    ) -> BackgroundAnalyzer {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let worker = shared.clone();
        let thread = std::thread::Builder::new()
            .name("harmonigraph-background-analyzer".to_string())
            .spawn(move || run(&worker, &editor_state, &flag));
        let thread = match thread {
            Ok(thread) => Some(thread),
            // Audible rather than silent, on the same argument as a refused
            // persist blob: what is lost is the spectrogram over every stretch
            // the window was shut, and a heatmap with holes in it looks exactly
            // like one nobody was playing into.
            Err(err) => {
                shared.lock().ui.console.log(format!(
                    "background analyzer not started ({err}) — the spectrogram \
                     will only cover time the editor window was open",
                ));
                None
            }
        };
        BackgroundAnalyzer { stop, thread }
    }
}

impl Drop for BackgroundAnalyzer {
    /// Stops the thread and waits for it, so the plugin never outlives its own
    /// worker. The wait is up to one [`POLL`] — the flag is read at the top of
    /// each round, and a round is only a drain.
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// One round: drain the rings if no frame is already doing it.
///
/// Split out from the sleep loop so what it DECIDES can be tested without a
/// clock, which is most of what there is to get wrong here.
///
/// **Both checks exist to keep this thread off a lock a frame is holding.**
/// `frame` takes the `EditorShared` mutex for its whole run, on an argument
/// spelled out there as "uncontended by design" — and issue #296's leading
/// hypothesis for a close-time host hang is that this argument is an assumption
/// and not a mechanism. A second party on that lock is precisely what that
/// hypothesis is about, so this one is arranged to be a party only when the
/// other cannot be: the window is checked BEFORE the lock is asked for, and
/// asked for with `try_lock`, so an open editor never waits here and this never
/// waits on a frame.
///
/// The transitions are ordered to match. `LatticeEditorHandle::drop` takes the
/// lock with `open` still true, so the save-and-close path is never behind this
/// thread. `Editor::spawn` sets `open` after the window is built, which leaves
/// window construction as the one overlap — and there the wait is a single
/// drain, one poll's worth of audio, rather than a whole frame.
///
/// A skipped round costs nothing: the ring carries seventeen of them.
fn tick(shared: &Mutex<EditorShared>, editor_state: &EguiState) {
    if editor_state.is_open() {
        return;
    }
    let Some(mut shared) = shared.try_lock() else {
        return;
    };
    let now = shared.now();
    shared.catch_up_unwatched(now);
}

/// Drain, sleep, repeat, until the plugin goes away. Drains FIRST, so a plugin
/// instantiated into a running project starts its history at once rather than a
/// poll later.
fn run(shared: &Mutex<EditorShared>, editor_state: &EguiState, stop: &AtomicBool) {
    while !stop.load(Ordering::Relaxed) {
        tick(shared, editor_state);
        std::thread::sleep(POLL);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicU32;
    use std::time::Instant;

    use harmonigraph_core::notes::{NoteEvent as CoreNoteEvent, NoteEventKind};

    use super::*;

    /// What a shell owns between the audio thread and the GUI: the state, the
    /// producer ends of both rings, and the window's open flag.
    struct Harness {
        shared: Arc<Mutex<EditorShared>>,
        notes: rtrb::Producer<CoreNoteEvent>,
        audio: rtrb::Producer<f32>,
        editor_state: Arc<EguiState>,
    }

    fn harness() -> Harness {
        let (notes, note_consumer) = rtrb::RingBuffer::new(crate::EVENT_RING_CAPACITY);
        let (audio, audio_consumer) = rtrb::RingBuffer::new(crate::AUDIO_RING_CAPACITY);
        let (_recorder, take_control) = harmonigraph_record::channel();
        let shared = EditorShared::new(
            note_consumer,
            audio_consumer,
            Arc::new(AtomicU32::new(48_000.0f32.to_bits())),
            Arc::new(AtomicU32::new(1)),
            take_control,
            Arc::new(std::sync::atomic::AtomicU64::new(0)),
        );
        Harness {
            shared: Arc::new(Mutex::new(shared)),
            notes,
            audio,
            editor_state: EguiState::from_size(800, 600),
        }
    }

    impl Harness {
        /// Mono audio at 48 kHz, enough for the analyzer to fill a window and
        /// cross hop boundaries, so a drain of it must produce columns.
        fn push_audio(&mut self, frames: usize) {
            for i in 0..frames {
                let t = i as f32 / 48_000.0;
                self.audio
                    .push((std::f32::consts::TAU * 440.0 * t).sin() * 0.5)
                    .expect("the ring is sized for this");
            }
        }

        /// One note-on, stamped on the audio thread's own sample clock.
        fn push_note(&mut self, time: f64, note: u8) {
            self.push_kind(time, note, NoteEventKind::On { velocity: 1.0 });
        }

        fn push_note_off(&mut self, time: f64, note: u8) {
            self.push_kind(time, note, NoteEventKind::Off);
        }

        fn push_kind(&mut self, time: f64, note: u8, kind: NoteEventKind) {
            self.notes
                .push(CoreNoteEvent { time, channel: 0, note, kind })
                .expect("the ring is sized for this");
        }

        fn columns(&self) -> usize {
            self.shared.lock().ui.spectrum.history().len()
        }

        fn voices(&self) -> usize {
            self.shared.lock().ui.tracker.voices().count()
        }

        /// What a GUI frame does with the rings, which is the same call this
        /// module makes — see `EditorShared::catch_up`.
        fn frame(&self) {
            let mut shared = self.shared.lock();
            let now = shared.now();
            shared.catch_up(now);
        }
    }

    /// The whole point: audio that arrives with nobody watching still becomes
    /// spectrogram history.
    #[test]
    fn a_closed_window_still_reaches_the_spectrogram() {
        let mut h = harness();
        h.push_audio(20_000);
        assert_eq!(h.columns(), 0, "nothing is analyzed before a drain");

        tick(&h.shared, &h.editor_state);

        assert!(h.columns() > 0, "a closed window dropped its audio on the floor");
    }

    /// The notes travel with them, and not as a bonus: a full ring drops the
    /// NEWEST, so notes left to pile up until the reopen are the ones
    /// `ClockMapper` then shifts forward to meet `now`. Captured here instead,
    /// the ribbons stay over the ridges they made.
    #[test]
    fn a_closed_window_still_reaches_the_roll() {
        let mut h = harness();
        h.push_note(1.0, 60);
        h.push_note(1.5, 64);
        assert_eq!(h.voices(), 0, "nothing is tracked before a drain");

        tick(&h.shared, &h.editor_state);

        assert_eq!(h.voices(), 2, "a closed window dropped its notes on the floor");
    }

    /// Feeding the tracker and AGEING it are one job, and this is the drainer
    /// that has to do both halves itself.
    ///
    /// `NoteTracker` parks every note-off in a released tail that only
    /// [`NoteTracker::prune`] empties, and the frame reaches `prune` through
    /// `begin_frame` rather than through the drain. A window that never opens
    /// therefore feeds that tail and nothing ever empties it — which
    /// `notes.rs` already names as the hazard behind its whole envelope
    /// design: "the released tail would accumulate for the whole session
    /// against an O(nodes x voices) loop".
    #[test]
    fn a_closed_window_ages_the_voices_it_tracks() {
        let mut h = harness();
        // Played and released far behind the batch's newest event, which is
        // what `ClockMapper` pins to `now` — so this voice's 1 s fade is 99 s
        // over by the time it lands, and nothing but a missing prune can keep
        // it alive.
        h.push_note(0.0, 60);
        h.push_note_off(0.1, 60);
        // Still held, so it is never a prune candidate: it is here to pin the
        // batch's newest timestamp, and to prove the count below is measuring
        // the released tail rather than an empty tracker.
        h.push_note(100.0, 64);

        tick(&h.shared, &h.editor_state);

        assert_eq!(
            h.voices(),
            1,
            "a voice whose fade finished 99 s ago is still in the tracker: the drain \
             feeds the released tail and nothing empties it while the window is shut",
        );
    }

    /// And the mirror image, which is what keeps this thread off the lock a
    /// frame holds: while the window is open the frame owns the drain, and this
    /// leaves both rings completely alone rather than racing it for them.
    #[test]
    fn an_open_window_is_left_to_drain_its_own_rings() {
        let mut h = harness();
        h.push_audio(20_000);
        h.push_note(1.0, 60);
        h.editor_state.set_open(true);

        tick(&h.shared, &h.editor_state);
        assert_eq!(h.columns(), 0, "analyzed audio the frame was going to take");
        assert_eq!(h.voices(), 0, "tracked notes the frame was going to take");

        // Nothing was consumed either — both rings are still there for whoever
        // drains next, which is the part a `return` after the pop would get
        // silently wrong.
        h.editor_state.set_open(false);
        tick(&h.shared, &h.editor_state);
        assert!(h.columns() > 0, "the open round ate the audio ring");
        assert_eq!(h.voices(), 1, "the open round ate the note ring");
    }

    /// The handover is the user-visible half: audio split across a window
    /// closing and reopening must yield exactly the columns it would have
    /// yielded had the window never moved — no column dropped at the boundary,
    /// none analyzed twice.
    ///
    /// Against a baseline rather than a number, because what is being claimed
    /// is an EQUIVALENCE. A count spelled out here would go stale with the hop
    /// or the window length, and worse, would still pass if both paths were
    /// wrong in the same way; a baseline drained by frames alone is the picture
    /// this feature promises to reproduce.
    ///
    /// It is the column GRID this pins and deliberately not the timestamps.
    /// Those hang off `AudioSpectrum`'s anchor, which tracks the SHELL clock —
    /// so feeding half a second of audio in no time at all, as a test must,
    /// drags the anchor by design and says nothing about the handover. A host
    /// hands this thread one poll of audio per poll of wall clock, and the two
    /// advance together.
    #[test]
    fn the_handover_neither_drops_a_column_nor_repeats_one() {
        // The same audio, drained entirely by frames.
        let baseline = {
            let mut h = harness();
            h.editor_state.set_open(true);
            h.push_audio(20_000);
            h.frame();
            h.push_audio(20_000);
            h.frame();
            h.columns()
        };
        assert!(baseline > 0, "the baseline analyzed nothing");

        // And now split across a window that was shut for the first half.
        let mut h = harness();
        h.push_audio(20_000);
        tick(&h.shared, &h.editor_state);
        let while_closed = h.columns();
        assert!(while_closed > 0, "the closed half analyzed nothing");

        h.editor_state.set_open(true);
        h.push_audio(20_000);
        h.frame();
        assert!(h.columns() > while_closed, "the frame added nothing after the handover");

        assert_eq!(
            h.columns(),
            baseline,
            "a window that closed and reopened left {} columns where an open one leaves \
             {baseline}: the handover is not on the same grid",
            h.columns(),
        );
    }

    /// [`POLL`] against the ring it is pacing itself to. A poll slower than the
    /// ring's span drops audio silently and permanently — the exact failure
    /// this module removes, reintroduced by its own pacing — and nothing about
    /// either constant's declaration hints at the other.
    #[test]
    fn the_ring_holds_many_polls_of_audio_at_any_rate_a_host_offers() {
        // The worst case the plugin's layout allows: the fastest rate a host
        // offers against the most channels `AUDIO_IO_LAYOUTS` declares.
        const FASTEST_RATE: f64 = 192_000.0;
        const CHANNELS: f64 = 2.0;
        let span = crate::AUDIO_RING_CAPACITY as f64 / CHANNELS / FASTEST_RATE;
        assert!(
            span > POLL.as_secs_f64() * 8.0,
            "the ring holds {span:.3} s against a {:.3} s poll: too little slack for a \
             thread the scheduler is free to ignore",
            POLL.as_secs_f64(),
        );
    }

    /// The thread really runs on its own, and really stops when the plugin goes
    /// away. A worker that outlived its plugin would leave one thread per
    /// instance ever loaded, each still spending an FFT's worth of a core on a
    /// ring nothing fills again.
    ///
    /// Stopping is asserted by SILENCE AFTER THE DROP, not by the drop
    /// returning: a `Drop` that does nothing at all returns fastest of any, so
    /// timing it alone reads a leaked thread as the healthy case. Audio pushed
    /// afterwards is what tells them apart — a thread still running takes it.
    #[test]
    fn the_thread_runs_on_its_own_and_stops_when_dropped() {
        let mut h = harness();
        h.push_audio(20_000);

        let analyzer = BackgroundAnalyzer::spawn(h.shared.clone(), h.editor_state.clone());
        // Generous against POLL, so a loaded machine cannot fail this for being
        // slow — it is asserting that rounds happen at all, not how fast.
        std::thread::sleep(POLL * 10);
        let analyzed = h.columns();
        assert!(analyzed > 0, "the thread never took a round");

        let started = Instant::now();
        drop(analyzer);
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "the join took {:?}: the flag is not being read once a round",
            started.elapsed(),
        );

        h.push_audio(20_000);
        std::thread::sleep(POLL * 10);
        assert_eq!(h.columns(), analyzed, "the thread outlived the plugin that owned it");
    }
}
