//! The spectrum analyzer that keeps running while the editor window is closed.
//!
//! `process` fills both audio→GUI rings whether or not anyone is watching, and
//! a GUI frame is otherwise the only thing that drains them — which the two
//! rings survive very differently. The note ring holds 4096 events, minutes of
//! playing, so a reopened window replays what it missed and the roll fills in.
//! The audio ring holds [`AUDIO_RING_CAPACITY`](crate::AUDIO_RING_CAPACITY)
//! samples, 1.37 s of stereo at 48 kHz, so it saturates within seconds and
//! every sample after that is dropped on the floor. Without a drainer of its
//! own, reopening a window shut for a minute would give back a roll full of
//! notes over a heatmap with a minute-wide hole in it; removing that asymmetry
//! is what this is for.
//!
//! Both rings are drained here, not just the audio one, and the second is not a
//! bonus. A full ring drops the NEWEST, so once the note ring saturates the
//! events that survive are the OLD ones, and `ClockMapper` then snaps the
//! batch's newest to `now` and shifts the whole backlog forward. Leaving the
//! notes to the reopen while the columns are captured at their true times would
//! draw ribbons and ridges that disagree — and a heatmap that disagrees with
//! the roll above it is worse than one that admits the gap.
//!
//! **What it costs is a continuous FFT.** 0.23 ms per stereo column at the
//! default 8192-point window, 125 columns a second: about 3% of a core for as
//! long as the plugin is instantiated, paid now even by a project nobody opens
//! the editor of, and paid continuously because a DAW streams silence as
//! diligently as it streams music. Memory does not move — [`SpectrumHistory`]
//! bounds itself at ~30 MB and already outlived the window, so this only
//! reaches that bound sooner rather than raising it. The hop is the only lever
//! if that ever needs to come down (32 ms instead of 8 puts it at 0.7%), but it
//! is not one to reach for casually: the store's tiers, `live_slab`'s ladder
//! and `COLUMNS_PER_SLAB` are all rungs of one shared `FFT_INTERVAL`, and a
//! second hop rate is the exact shape of the duplicated-column bug those
//! comments warn about.
//!
//! Nothing here analyzes anything itself. It calls
//! [`catch_up_unwatched`](crate::editor::EditorShared::catch_up_unwatched),
//! which is the frame's own drain plus the ageing a frame instead gets from
//! `begin_frame`; see there for why sharing that one path is the point.
//!
//! **The project's own settings arrive here too**, for the same reason: this
//! is the only thing running before the editor's window exists, and that
//! window's build closure is otherwise the sole reader of `params.ui_state`.
//! Left to it, every column analyzed before the first open is analyzed at
//! [`SpectrumConfig::default`]'s window whatever the project saved — and since
//! `INTERP_BIN_CEILING` is a fixed BIN index, the two stretches differ in the
//! reconstruction rule a band of the spectrum is drawn under and not merely in
//! resolution, so their join scrolls across the heatmap as a seam. See
//! [`Restore`].
//!
//! [`SpectrumHistory`]: harmonigraph_core::SpectrumHistory
//! [`SpectrumConfig::default`]: harmonigraph_ui::SpectrumConfig

#[cfg(test)]
use std::sync::atomic::AtomicU64;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use parking_lot::{Mutex, RwLock};

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
/// where the cost is. A shorter poll would shorten the TYPICAL lock hold, which
/// is already about a millisecond of FFT; it would not shorten the worst one,
/// because that is set by how full the ring got while this thread was away
/// rather than by how often it means to look. See [`tick`] for that bound.
pub(crate) const POLL: Duration = Duration::from_millis(20);

/// The thread, and the flag that ends it.
///
/// Held by the plugin purely for its [`Drop`]: a plugin removed from a project
/// must not leave a thread behind doing FFTs on a ring nobody fills again.
pub(crate) struct BackgroundAnalyzer {
    stop: Arc<AtomicBool>,
    /// Completed loop rounds, exposed only to tests that need an observable
    /// boundary after changing the editor-open flag.
    #[cfg(test)]
    rounds: Arc<AtomicU64>,
    /// `None` when the thread could not be spawned, which costs this feature
    /// and nothing else — an open editor still drains both rings itself.
    thread: Option<JoinHandle<()>>,
}

impl BackgroundAnalyzer {
    pub(crate) fn spawn(
        shared: Arc<Mutex<EditorShared>>,
        editor_state: Arc<EguiState>,
        ui_state: Arc<RwLock<String>>,
    ) -> BackgroundAnalyzer {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let worker = shared.clone();
        #[cfg(test)]
        let rounds = Arc::new(AtomicU64::new(0));
        #[cfg(test)]
        let worker_rounds = rounds.clone();
        let thread = std::thread::Builder::new()
            .name("harmonigraph-background-analyzer".to_string())
            .spawn(move || {
                #[cfg(not(test))]
                run(&worker, &editor_state, Restore::of(ui_state), &flag);
                #[cfg(test)]
                run(&worker, &editor_state, Restore::of(ui_state), &flag, &worker_rounds);
            });
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
        BackgroundAnalyzer {
            stop,
            thread,
            #[cfg(test)]
            rounds,
        }
    }

    #[cfg(test)]
    pub(crate) fn completed_rounds(&self) -> u64 {
        self.rounds.load(Ordering::Acquire)
    }
}

impl Drop for BackgroundAnalyzer {
    /// Stops the thread and waits for it, so the plugin never outlives its own
    /// worker. The wait is up to one [`POLL`] — the flag is read at the top of
    /// each round, and a round is a drain plus, on the rounds a host has just
    /// written state, one RON parse.
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The host's saved UI state, and what of it the shared state already holds.
///
/// `params.ui_state` is where a project's settings arrive — the host writes it
/// on state restore, and `LatticeEditorHandle`'s `Drop` writes it again on the
/// way out of every editor session. Nothing reads it until a window is built,
/// so a project's saved analyzer Window reaches nothing that runs before the
/// first open; applying it here is what closes that gap.
///
/// A MIRROR of the blob rather than a done-once flag, because the field is
/// written more than once: a preset change or an undo pushes fresh state at a
/// plugin whose window is shut, and the round after it should take that too.
/// The mirror is also what keeps the cost of a round at one string compare.
///
/// One consequence, and it is a harmless one: the first round after every
/// editor session adopts the blob that session's `Drop` wrote, which is a
/// reload of the state's own save. That blob is a FIXED POINT of the round
/// trip, so the round that takes it moves nothing — `save_persist` reads back
/// every field `load_persist` writes that a shut window has any reader for, and
/// `a_reloaded_save_leaves_the_state_where_it_was` is what holds the two calls
/// to that. The two writes it does NOT read back, the fold dial and the comma
/// verdicts, are frame-scoped: only a draw reads either, no draw is running,
/// and every window build resets both through `Opening` regardless. What it
/// costs is one RON round trip per window close, where a window close is
/// already spending one.
struct Restore {
    /// The host's field, shared with the plugin and with the editor.
    blob: Arc<RwLock<String>>,
    /// The last blob handed to `load_persist`. Empty until one has been, which
    /// is also what the field itself holds until a project supplies one.
    applied: String,
}

impl Restore {
    fn of(blob: Arc<RwLock<String>>) -> Restore {
        Restore { blob, applied: String::new() }
    }

    /// Apply the host's blob to the shared state, unless the state already
    /// holds it.
    ///
    /// Called with the window SHUT and the state's lock held, and both halves
    /// are load-bearing. `Opening` applies the same blob through the same call
    /// whenever a window is built, so an open window is already served; and
    /// applying it under one would revert everything the user has changed since
    /// they opened it, since the blob is only written on the way out. The lock
    /// is what makes reading `blob` here safe to order this way — the close
    /// path takes the same two in the same order (`shared`, then `ui_state`).
    ///
    /// The mirror is updated whether or not the blob was ACCEPTED, so one this
    /// build refuses — below the version floor, or naming a dropped variant —
    /// says so on the console once rather than fifty times a second.
    fn adopt(&mut self, shared: &mut EditorShared) {
        {
            let blob = self.blob.read();
            // An empty blob is what a project whose editor has never been open
            // carries; reading it as one would put a parse failure on the
            // console of every instance that has never saved.
            if blob.is_empty() || *blob == self.applied {
                return;
            }
            self.applied.clear();
            self.applied.push_str(&blob);
        }
        shared.ui.load_persist(&self.applied);
    }
}

/// One round: adopt whatever the host has restored, then drain the rings —
/// both of them only while no frame is doing either.
///
/// Split out from the sleep loop so what it DECIDES can be tested without a
/// clock, which is most of what there is to get wrong here.
///
/// The window check carries a second job with the adopt behind it, and one that
/// is about the USER rather than about a lock: `params.ui_state` names what the
/// project last saved, so re-applying it under an open window would revert
/// whatever has been changed since. See [`Restore::adopt`].
///
/// **Both checks exist to keep this thread off a lock a frame is holding.**
/// `frame` takes the `EditorShared` mutex for its whole run, on an argument
/// spelled out there as "uncontended by design" — and issue #296's leading
/// hypothesis for a close-time host hang is that this argument is an assumption
/// and not a mechanism. A second party on that lock is precisely what that
/// hypothesis is about, so this one is arranged to be a party only when the
/// other cannot be: the window is checked BEFORE the lock is asked for, and
/// asked for with `try_lock` — so this thread never waits on a frame at all,
/// and a frame waits on this thread only for the width of one drain.
///
/// The transitions are ordered to keep even that rare.
/// `LatticeEditorHandle::drop` takes the lock with `open` still true, so the
/// save-and-close path is never behind this thread. `Editor::spawn` sets `open`
/// after the window is built, which leaves window construction as the one
/// overlap — a check that passed a moment before the window claimed itself.
///
/// **A drain is bounded by the RING, not by the poll**, and the difference is
/// worth stating because the reassuring number is the wrong one. Descheduled
/// past 1.37 s with the window shut, this wakes to a full ring and spends ~170
/// columns of FFT — about 40 ms — under one lock; the steady state is a poll's
/// worth, well under a millisecond. Neither is a hang, but 40 ms on the host's
/// main thread inside `gui_create` is the figure to argue with if #296 is ever
/// re-opened against this lock.
///
/// A skipped round costs nothing: the ring carries seventeen of them even at
/// the fastest rate a host offers.
fn tick(shared: &Mutex<EditorShared>, editor_state: &EguiState, restore: &mut Restore) {
    if editor_state.is_open() {
        return;
    }
    let Some(mut shared) = shared.try_lock() else {
        return;
    };
    // BEFORE the drain: the settings decide how the samples about to be taken
    // are analyzed, so a round that adopted them afterwards would still leave
    // its own columns at the window the blob just replaced.
    restore.adopt(&mut shared);
    let now = shared.now();
    shared.catch_up_unwatched(now);
}

/// Drain, sleep, repeat, until the plugin goes away.
///
/// The flag is read once per round and a round is bounded work, which is what
/// bounds the join in [`BackgroundAnalyzer::drop`] to a single [`POLL`].
///
/// The [`Restore`] mirror lives here, for the length of the thread: it is what
/// the host has already been answered about, and a fresh one every round would
/// re-apply the same blob fifty times a second.
fn run(
    shared: &Mutex<EditorShared>,
    editor_state: &EguiState,
    mut restore: Restore,
    stop: &AtomicBool,
    #[cfg(test)] rounds: &AtomicU64,
) {
    while !stop.load(Ordering::Relaxed) {
        tick(shared, editor_state, &mut restore);
        #[cfg(test)]
        rounds.fetch_add(1, Ordering::Release);
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
    /// producer ends of both rings, the window's open flag, and the field the
    /// host restores a project's settings into.
    struct Harness {
        shared: Arc<Mutex<EditorShared>>,
        notes: rtrb::Producer<CoreNoteEvent>,
        audio: rtrb::Producer<f32>,
        editor_state: Arc<EguiState>,
        /// The host's end of `params.ui_state`.
        ui_state: Arc<RwLock<String>>,
        /// The thread's end of it, held across rounds exactly as [`run`] holds
        /// one — so a test can tick twice and see what the second round skips.
        restore: Restore,
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
        let ui_state = Arc::new(RwLock::new(String::new()));
        Harness {
            shared: Arc::new(Mutex::new(shared)),
            notes,
            audio,
            editor_state: EguiState::from_size(800, 600),
            restore: Restore::of(ui_state.clone()),
            ui_state,
        }
    }

    /// A project blob that differs from a fresh install in its analyzer Window
    /// and nothing else, written by the same call the editor saves through.
    fn blob_with_window(window: harmonigraph_ui::SpectrumWindow) -> String {
        let mut state = harmonigraph_ui::SharedState::new(crate::editor::ASSUMED_SURFACE_FORMAT);
        state.spectrum_config.window = window;
        harmonigraph_ui::shell::close(&state)
    }

    /// Half the analysis window, in seconds, at the harness's rate: what
    /// [`AudioSpectrum::column_lag`] reads back out of the analyzer, and so the
    /// one window size observable from outside it.
    ///
    /// [`AudioSpectrum::column_lag`]: harmonigraph_ui::AudioSpectrum::column_lag
    fn lag_of(window: harmonigraph_ui::SpectrumWindow) -> f64 {
        0.5 * window.samples() as f64 / 48_000.0
    }

    impl Harness {
        /// One round of the loop the thread runs, against a mirror that
        /// survives between rounds.
        fn tick(&mut self) {
            super::tick(&self.shared, &self.editor_state, &mut self.restore);
        }

        /// What the host does when it restores a project — or pushes a preset
        /// at a plugin whose window is shut.
        fn restore(&mut self, blob: &str) {
            *self.ui_state.write() = blob.to_string();
        }

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

        /// The window the SETTING asks for.
        fn configured_window(&self) -> harmonigraph_ui::SpectrumWindow {
            self.shared.lock().ui.spectrum_config.window
        }

        /// The window the ANALYZER is actually running at, which is the claim
        /// worth making: the setting reaching the config and stopping there is
        /// precisely the bug (see `AudioSpectrum::push_samples`, the one thing
        /// that carries one to the other).
        fn analyzed_lag(&self) -> f64 {
            self.shared.lock().ui.spectrum.column_lag()
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

        h.tick();

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

        h.tick();

        assert_eq!(h.voices(), 2, "a closed window dropped its notes on the floor");
    }

    /// Feeding the tracker and AGEING it are one job, and this is the drainer
    /// that has to do both halves itself.
    ///
    /// `NoteTracker` parks every note-off in a released tail that only
    /// [`NoteTracker::prune`](harmonigraph_core::NoteTracker::prune) empties,
    /// and the frame reaches `prune` through
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

        h.tick();

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

        h.tick();
        assert_eq!(h.columns(), 0, "analyzed audio the frame was going to take");
        assert_eq!(h.voices(), 0, "tracked notes the frame was going to take");

        // Nothing was consumed either — both rings are still there for whoever
        // drains next, which is the part a `return` after the pop would get
        // silently wrong.
        h.editor_state.set_open(false);
        h.tick();
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
    /// wrong in the same way.
    ///
    /// What the baseline is NOT is the real `frame`: it is `Harness::frame`,
    /// which stands in for it by making the same `catch_up` call. So this is a
    /// regression guard on `tick` — it catches `tick` growing an analyzer of its
    /// own, a clock of its own, or a `return` placed after the pop — and not
    /// evidence that `frame` still routes through `catch_up`. That claim is
    /// `catch_up_answers_whether_notes_arrived`'s, one file over.
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
        h.tick();
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

    /// Issue #324: the project's saved Window has to reach the analyzer with no
    /// editor anywhere in the picture, because the stretch before the first
    /// open is exactly what this thread exists to cover.
    ///
    /// Asserted on [`AudioSpectrum::column_lag`] — half the analysis window —
    /// rather than on the config, because the two are a step apart and only the
    /// second one is the picture. Nothing but `push_samples` carries the setting
    /// across that step, so a load placed after the drain satisfies an assertion
    /// on the config and still analyzes the round's own audio at the window the
    /// blob replaced.
    ///
    /// [`AudioSpectrum::column_lag`]: harmonigraph_ui::AudioSpectrum::column_lag
    #[test]
    fn a_restored_window_reaches_the_analyzer_with_no_editor_ever_built() {
        use harmonigraph_ui::SpectrumWindow;

        let mut h = harness();
        h.restore(&blob_with_window(SpectrumWindow::Precise));
        // A Precise window is 16384 samples, so this is comfortably more than
        // one windowful and the analyzer cannot come out of it empty.
        h.push_audio(40_000);

        h.tick();

        assert!(h.columns() > 0, "nothing was analyzed at all");
        assert_eq!(h.configured_window(), SpectrumWindow::Precise, "the blob never landed");
        assert!(
            (h.analyzed_lag() - lag_of(SpectrumWindow::Precise)).abs() < 1e-9,
            "columns analyzed at a {:.1} ms window where the project saved {:.1} ms: the \
             restored setting reached the config and not the analyzer",
            h.analyzed_lag() * 2000.0,
            lag_of(SpectrumWindow::Precise) * 2000.0,
        );
    }

    /// The trap in fixing it. `params.ui_state` is only written on the way OUT
    /// of an editor session, so it names what the user had when they last
    /// closed the window — and re-applying that under an open one would revert
    /// everything they have changed since.
    ///
    /// The guard is [`tick`]'s existing open-window check, which is why this
    /// test lives next to the drain it also guards: anything that moved the
    /// adopt ahead of that check would pass every other test here.
    #[test]
    fn an_open_window_keeps_the_settings_it_is_being_used_to_change() {
        use harmonigraph_ui::SpectrumWindow;

        let mut h = harness();
        // What the project saved, against what the user has since dialled in.
        h.restore(&blob_with_window(SpectrumWindow::Precise));
        h.shared.lock().ui.spectrum_config.window = SpectrumWindow::Fast;
        h.editor_state.set_open(true);

        h.tick();

        assert_eq!(
            h.configured_window(),
            SpectrumWindow::Fast,
            "the saved blob was re-applied under an open window, reverting the setting the \
             user is holding the window open to change",
        );
    }

    /// A host writes that field more than once — a preset change or an undo
    /// pushes fresh state at a plugin whose window is shut — so what the thread
    /// keeps has to be a mirror of the blob and not a done-once flag.
    #[test]
    fn a_project_switched_while_the_window_is_shut_is_taken_too() {
        use harmonigraph_ui::SpectrumWindow;

        let mut h = harness();
        h.restore(&blob_with_window(SpectrumWindow::Precise));
        h.tick();
        assert_eq!(h.configured_window(), SpectrumWindow::Precise);

        h.restore(&blob_with_window(SpectrumWindow::Fast));
        h.tick();

        assert_eq!(
            h.configured_window(),
            SpectrumWindow::Fast,
            "the second blob was never read: the thread adopts once and then stops looking",
        );
    }

    /// An EMPTY blob is a project with nothing saved, not a broken one. A
    /// plugin whose editor has never been opened carries one, and a host can
    /// hand it to a live instance — loading such a project over this one with
    /// the window shut. Read as a blob it would put `persist ignored — the blob
    /// did not parse` on the console, which is the noise the emptiness check
    /// exists to keep off it.
    ///
    /// Reached only from a mirror that already holds something: against a fresh
    /// one the check is indistinguishable from the same-blob check beside it,
    /// which is what every other test here exercises.
    #[test]
    fn an_empty_blob_is_a_project_with_nothing_saved() {
        use harmonigraph_ui::SpectrumWindow;

        let mut h = harness();
        h.restore(&blob_with_window(SpectrumWindow::Precise));
        h.tick();

        h.restore("");
        h.tick();

        assert_eq!(
            h.shared.lock().ui.console.lines().filter(|l| l.contains("persist ignored")).count(),
            0,
            "an empty blob was read as a broken one",
        );
        // And nothing was reset to defaults on the strength of it: an empty
        // field says the other project saved nothing, not that this one should
        // forget what it has.
        assert_eq!(h.configured_window(), SpectrumWindow::Precise);
    }

    /// What makes the round after every window close harmless: the blob
    /// `LatticeEditorHandle::drop` writes is a fixed point of the round trip, so
    /// adopting it moves nothing. Asserted on the SAVE either side rather than
    /// on named fields, because the claim is about every persisted field at once
    /// and a list here would go stale the moment one is added.
    #[test]
    fn a_reloaded_save_leaves_the_state_where_it_was() {
        use harmonigraph_ui::SpectrumWindow;

        let mut h = harness();
        // Dialled away from the defaults, the way a session leaves a state —
        // otherwise a load that reset a field to its default would pass.
        {
            let ui = &mut h.shared.lock().ui;
            ui.spectrum_config.window = SpectrumWindow::Precise;
            ui.spectrum_config.floor_db = -72.0;
            ui.fps_cap = Some(90.0);
        }
        // What the close writes into `params.ui_state`.
        let saved = harmonigraph_ui::shell::close(&h.shared.lock().ui);
        h.restore(&saved);

        h.tick();

        assert_eq!(
            harmonigraph_ui::shell::close(&h.shared.lock().ui),
            saved,
            "reloading a state's own save moved something in it",
        );
    }

    /// And the mirror's other half. A blob this build refuses — below the
    /// version floor, or naming a variant that has been dropped — is refused
    /// LOUDLY, on the console, which is a fine thing to say once and a useless
    /// thing to say fifty times a second for as long as the project is loaded.
    #[test]
    fn a_blob_this_build_refuses_says_so_once() {
        let mut h = harness();
        h.restore("this is not a persist blob");

        for _ in 0..5 {
            h.tick();
        }

        let refusals = h
            .shared
            .lock()
            .ui
            .console
            .lines()
            .filter(|line| line.contains("persist ignored"))
            .count();
        assert_eq!(refusals, 1, "the console holds one line per round, not one per blob");
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

        let analyzer =
            BackgroundAnalyzer::spawn(h.shared.clone(), h.editor_state.clone(), h.ui_state.clone());
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
