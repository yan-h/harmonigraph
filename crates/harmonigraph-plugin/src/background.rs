//! The input worker: the one thread that drains the audio thread's rings and
//! runs the spectrum analysis, whether or not the editor window is open.
//!
//! `process` fills both audio→GUI rings whether or not anyone is watching. The
//! note ring holds 4096 events, minutes of playing; the audio ring holds
//! [`AUDIO_RING_CAPACITY`](crate::AUDIO_RING_CAPACITY) samples, 1.37 s of
//! stereo at 48 kHz, so it saturates within seconds and every sample after
//! that is dropped. A frame used to drain both with the window open and this
//! thread with it shut; now this thread drains them always, so the window's
//! state decides nothing about the input (#1559).
//!
//! **The FFTs run with no lock held.** A step takes the editor's lock to move
//! the notes into the tracker and the audio out of its ring (a copy), lets
//! go, analyzes, and takes it again to hand the columns to the store
//! ([`LiveInput::take`](crate::editor::input::LiveInput::take), `analyze`,
//! `give`). So the host's main thread, which a frame and a window opening run
//! on, waits on this thread for a copy or an append, never for an FFT, and no
//! longer does any FFT itself. The notes and the clock heartbeat go with the
//! audio, in the same hold, because the heartbeat's offset is what dates the
//! audio: observed and then mapped in one hold, as a frame's drain did, where
//! on two threads at two cadences a block after a callback pause could be
//! dated with the offset from before it, a pause too early. (`process`
//! publishes a block before its heartbeat, so a take landing between the two
//! still maps that one block with the previous offset, as a frame's drain
//! always could; it matters only for the first block after a pause.)
//!
//! Both rings are drained here, not just the audio one, and the second is not
//! a bonus: a full ring drops the NEWEST, so a ring left to pile up would lose
//! the events beside the fresh audio.
//!
//! **Analysis keeps progressing, but silence need not run an FFT.**
//! `harmonigraph-analysis` skips transforms when the retained window is exactly
//! zero, while continuing output columns and history. It still transforms a
//! window containing earlier nonzero samples even if the newest input is zero;
//! `silent_windows_keep_readiness_retained_audio_and_resume` covers that boundary.
//! This is no amplitude threshold or permission to suspend the drainer.
//!
//! Historical cost: the 2026-09-03 update (`0bcabce51`) recorded 0.23 ms per
//! stereo column at an 8192-point window, 125 columns a second, about 3% of a
//! core (and estimated 0.7% at a 32 ms hop). The original 2026-08-09
//! [#305](https://github.com/yan-h/harmonigraph/pull/305) measured 0.417 ms,
//! about 5.2%. Neither is a current full-plugin or silent-input measurement.
//! Memory remains bounded by [`SpectrumHistory`]'s ~54 MiB power sums;
//! the history already outlives the window. A different hop is not the only
//! possible cost reduction, and would need to preserve the store tiers,
//! `live_slab` ladder on their shared `FFT_INTERVAL` grid.
//!
//! **With the window shut, a step also does the frame's other duties:** the
//! ageing a frame gets from `begin_frame`
//! ([`VisualRuntime::advance_time`](harmonigraph_ui::VisualRuntime::advance_time)),
//! and [`EditorShared::poll_take_end`] so a take can finish and render without
//! an open editor. This completion poll is separate from the GUI's full
//! recording control synchronization; the
//! `a_take_ends_itself_while_the_editor_window_is_shut` test covers it.
//!
//! **The project's own settings arrive here too**, for the same reason: this
//! is the only thing running before the editor's window exists, and that
//! window's build closure is otherwise the only thing that applies
//! `params.ui_state` to the state (a host save reads it only to store it).
//! Left to it, every column analyzed before the first open is analyzed at
//! [`SpectrumConfig::default`]'s window whatever the project saved — and the
//! two stretches then differ in how long every pitch listened and how far
//! behind its audio each column is dated, so their join scrolls across the
//! heatmap as a seam. See [`Restore`].
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

/// How often the worker looks for input while the editor window is open.
///
/// Under the 8 ms hop, so a column is analyzed within a poll of its audio
/// arriving, where a frame used to analyze it at the next frame: the worst
/// case for a block is now a frame plus a poll. Cheap: a step that finds
/// nothing with the window open is one short hold of an uncontended lock.
pub(crate) const POLL: Duration = Duration::from_millis(4);

/// How often it looks with the window shut, where nothing is drawing what it
/// analyzes, so it need not wake as often; the closed window's duties (see
/// [`step`]) are content with any cadence this bound allows.
///
/// Bounded above by the ring itself, and tightly: it holds
/// [`AUDIO_RING_CAPACITY`](crate::AUDIO_RING_CAPACITY) interleaved samples,
/// which is 1.37 s of stereo at 48 kHz but only 0.34 s at 192 kHz — and a poll
/// that ever slipped past that span would drop audio silently and for good,
/// reintroduced by this thread's own pacing. 20 ms, the longest it sleeps,
/// leaves 17x margin at the worst rate a host can hand us, so the scheduler
/// has to lose this thread for a third of a second before anything is lost.
/// `the_ring_holds_many_polls_of_audio_at_any_rate_a_host_offers` is what keeps
/// the two numbers in that relation.
pub(crate) const SHUT_POLL: Duration = Duration::from_millis(20);

/// The input worker's thread, and the flag that ends it.
///
/// Held by the plugin purely for its [`Drop`]: a plugin removed from a project
/// must not leave a thread behind doing FFTs on a ring nobody fills again.
pub(crate) struct InputWorker {
    stop: Arc<AtomicBool>,
    /// Completed loop rounds, exposed only to tests that need an observable
    /// boundary after changing the editor-open flag.
    #[cfg(test)]
    rounds: Arc<AtomicU64>,
    /// `None` when the thread could not be spawned. The window's frames then
    /// drain both rings themselves, on the host's main thread as before #1559
    /// (see [`EditorShared::frame_drains`]), and only a shut window loses its
    /// input.
    thread: Option<JoinHandle<()>>,
}

impl InputWorker {
    pub(crate) fn spawn(
        shared: Arc<Mutex<EditorShared>>,
        editor_state: Arc<EguiState>,
        ui_state: Arc<RwLock<String>>,
    ) -> InputWorker {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let worker = shared.clone();
        #[cfg(test)]
        let rounds = Arc::new(AtomicU64::new(0));
        #[cfg(test)]
        let worker_rounds = rounds.clone();
        let thread = std::thread::Builder::new()
            .name("harmonigraph-input-worker".to_string())
            .spawn(move || {
                raise_to_gui_priority();
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
                let mut shared = shared.lock();
                shared.frame_drains = true;
                shared.ui.picture.runtime.console.log(format!(
                    "input worker not started ({err}) — the editor window drains \
                     its own input, and the spectrogram will only cover time it \
                     was open",
                ));
                None
            }
        };
        InputWorker {
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

impl Drop for InputWorker {
    /// Stops the thread and waits for it, so the plugin never outlives its own
    /// worker. Joining can wait for the current step, one [`SHUT_POLL`] sleep
    /// at most, and the final drain in [`run`].
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Run this thread at the class the host's GUI thread runs at on macOS:
/// a frame there waits on this thread's holds of the editor lock, and
/// `parking_lot` lends no priority to the holder, so a worker preempted inside
/// one would hold the frame for as long as the scheduler kept it off a core.
fn raise_to_gui_priority() {
    #[cfg(target_os = "macos")]
    {
        // <pthread/qos.h>: QOS_CLASS_USER_INTERACTIVE, the main thread's.
        const QOS_CLASS_USER_INTERACTIVE: u32 = 0x21;
        unsafe extern "C" {
            fn pthread_set_qos_class_self_np(qos_class: u32, relative_priority: i32) -> i32;
        }
        // SAFETY: changes only the calling thread's own scheduling class.
        unsafe {
            pthread_set_qos_class_self_np(QOS_CLASS_USER_INTERACTIVE, 0);
        }
    }
}

/// The host's saved UI state, and what of it the shared state already holds.
///
/// `params.ui_state` is where a project's settings arrive — the host writes it
/// on state restore, and `LatticeEditorHandle`'s `Drop` writes it again on the
/// way out of every editor session. Nothing else writes it: a host save with a
/// window open serializes the live state without storing it (see
/// [`UiState`](crate::editor::UiState)), so a change of blob still means a
/// restore or a close. Nothing reads it until a window is built,
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
    /// whenever a window is built, and the window's frames apply a restore
    /// that lands while it is open (see [`UiState`](crate::editor::UiState)),
    /// so an open window is already served; and applying it under one would
    /// revert everything the user has changed since the window last applied it
    /// (a save with the window open serializes the live state and stores
    /// nothing). The lock
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

#[cfg(test)]
thread_local! {
    /// Pause a step before it obtains shared state.
    static TICK_PAUSE: std::cell::RefCell<Option<Arc<std::sync::Barrier>>> = const { std::cell::RefCell::new(None) };
    /// Run between a step's two holds of the lock, just before its FFTs.
    #[allow(clippy::type_complexity)]
    static BETWEEN_HOLDS: std::cell::RefCell<Option<Box<dyn FnMut(&Mutex<EditorShared>, &crate::editor::input::Taken)>>> = const { std::cell::RefCell::new(None) };
}

/// One step: adopt whatever the host has restored while the window is shut,
/// take the notes and the audio under the lock, analyze the audio without
/// it, and give the columns back under it — then, with the window shut, the
/// frame's other duties.
///
/// Those run once a step, and a shut window's steps come every [`SHUT_POLL`].
/// `poll_take_end` times its OnTransportStop debounce in seconds
/// ([`EditorShared::STOP_SECONDS`]), so it holds at whatever rate it is polled.
///
/// Split out from the sleep loop so what it DECIDES can be tested without a
/// clock, which is most of what there is to get wrong here.
///
/// The window check carries a job that is about the USER rather than about a
/// lock: `params.ui_state` names the last restore or window close, so
/// re-applying it under an open window would revert whatever has been changed
/// since. See [`Restore::adopt`]. It is made under the lock, where the
/// window's build reads the same blob: `LatticeEditorHandle::drop` takes the
/// lock with `open` still true, and `Editor::spawn` sets `open` after the
/// window is built, which leaves window construction as the one overlap — a
/// step that found the window shut a moment before it claimed itself.
///
/// **The lock is waited for, not tried**: this thread never runs on the
/// host's main thread, so waiting out a frame costs the host nothing, and a
/// frame or a window opening waits on this thread only for a take (a copy of
/// what the ring holds) or a give (appending its columns): microseconds for
/// the two or three columns a step usually brings, and a few milliseconds at
/// most for a full ring's 170, reachable only after this thread was starved
/// for 1.37 s. The FFTs between them, some 40 ms for that ring at the
/// historical estimate above, hold nothing. The thread runs at the GUI's
/// scheduling class on macOS (see [`InputWorker::spawn`]), so a frame waiting
/// on one of those holds is not also waiting for this thread to be scheduled.
fn step(shared: &Mutex<EditorShared>, editor_state: &EguiState, restore: &mut Restore) {
    #[cfg(test)]
    if let Some(gate) = TICK_PAUSE.with_borrow_mut(Option::take) {
        gate.wait();
        gate.wait();
    }
    let taken = {
        let mut guard = shared.lock();
        let shared = &mut *guard;
        // BEFORE the take: the settings decide how the samples about to be
        // taken are analyzed, so a step that adopted them afterwards would
        // still leave its own columns at the window the blob just replaced.
        if !editor_state.is_open() {
            restore.adopt(shared);
        }
        let now = shared.input.now();
        let (notes, taken) =
            shared.input.take(&mut shared.ui.picture.runtime, &shared.ui.picture.appearance, now);
        shared.fresh_notes |= notes;
        taken
    };
    let taken = taken.map(|mut taken| {
        #[cfg(test)]
        BETWEEN_HOLDS.with_borrow_mut(|hook| hook.as_mut().map(|hook| hook(shared, &taken)));
        crate::editor::input::LiveInput::analyze(&mut taken);
        taken
    });
    if taken.is_none() && editor_state.is_open() {
        return;
    }
    let mut guard = shared.lock();
    let shared = &mut *guard;
    if let Some(taken) = taken {
        shared.input.give(&mut shared.ui.picture.runtime, taken);
    }
    if !editor_state.is_open() {
        let now = shared.input.now();
        let now = shared.input.display_now(now);
        shared.ui.picture.runtime.advance_time(now, &shared.ui.picture.appearance);
        // A take can end itself while nobody is watching, and an audio export
        // with the plugin window shut is precisely that. Every trigger but
        // OnDisarm is decided in this call, which otherwise only a GUI frame
        // makes — so a shut window meant an armed take ran to whenever it was
        // noticed and no video was ever rendered. See
        // `EditorShared::poll_take_end`.
        shared.poll_take_end();
    }
}

/// A [`step`] that survives its own panic. One would otherwise end the
/// thread with the analysis lent out and nothing draining either ring, which
/// freezes the picture and every shut-window duty for the session with
/// nothing said. Handing the drain to the frames instead would only move the
/// same panic onto the host's main thread. So the analysis is put back fresh,
/// the source run restarts so its columns re-anchor, the console says so
/// once, and the next step carries on.
///
/// Whether the step completed.
fn guarded_step(
    shared: &Mutex<EditorShared>,
    editor_state: &EguiState,
    restore: &mut Restore,
    panicked: &mut bool,
) -> bool {
    let stepped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        step(shared, editor_state, restore)
    }));
    if stepped.is_err() {
        let mut guard = shared.lock();
        let shared = &mut *guard;
        shared.input.recover_analysis(&mut shared.ui.picture.runtime);
        if !std::mem::replace(panicked, true) {
            shared
                .ui
                .picture
                .runtime
                .console
                .log("input worker panicked in a step and restarted its analysis".to_string());
        }
        return false;
    }
    true
}

/// How many consecutive steps may panic before the worker stops trying.
const MAX_PANICS_IN_A_ROW: u32 = 8;

/// Drain, sleep, repeat, until the plugin goes away.
///
/// The stop flag is read once per step. Joining waits for any current step
/// and sleep, followed by the final drain below.
///
/// The [`Restore`] mirror lives here, for the length of the thread: it is what
/// the host has already been answered about, and a fresh one every step would
/// re-apply the same blob hundreds of times a second.
fn run(
    shared: &Mutex<EditorShared>,
    editor_state: &EguiState,
    mut restore: Restore,
    stop: &AtomicBool,
    #[cfg(test)] rounds: &AtomicU64,
) {
    let mut panicked = false;
    let mut in_a_row = 0;
    while !stop.load(Ordering::Relaxed) {
        if guarded_step(shared, editor_state, &mut restore, &mut panicked) {
            in_a_row = 0;
        } else {
            in_a_row += 1;
            // A step that panics every time would unwind every poll, printing
            // a backtrace each, for a picture that is frozen anyway.
            if in_a_row >= MAX_PANICS_IN_A_ROW {
                shared.lock().ui.picture.runtime.console.log(format!(
                    "input worker stopped after {in_a_row} panicking steps in a row — the \
                     editor's input is frozen until the plugin is reloaded",
                ));
                return;
            }
        }
        #[cfg(test)]
        rounds.fetch_add(1, Ordering::Release);
        std::thread::sleep(if editor_state.is_open() { POLL } else { SHUT_POLL });
    }
    // Destruction follows host callback quiescence. Consume the final owned
    // display payloads before the editor's bank is reclaimed off thread.
    let mut shared = shared.lock();
    let shared = &mut *shared;
    let now = shared.input.now();
    shared.input.drain(&mut shared.ui.picture.runtime, &shared.ui.picture.appearance, now);
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicU32;
    use std::time::Instant;

    use harmonigraph_core::notes::{NoteEvent as CoreNoteEvent, NoteEventKind, SourceId};

    use super::*;

    /// What a shell owns between the audio thread and the GUI: the state, the
    /// producer ends of both rings, the window's open flag, and the field the
    /// host restores a project's settings into.
    struct Harness {
        shared: Arc<Mutex<EditorShared>>,
        /// The audio thread's half of `shared.take`, for a test that needs a
        /// take to have captured something.
        recorder: harmonigraph_record::Recorder,
        notes: harmonigraph_record::publication::Publisher,
        audio: crate::audio_ingress::Producer,
        audio_frames: u64,
        editor_state: Arc<EguiState>,
        /// The host's end of `params.ui_state`.
        ui_state: Arc<RwLock<String>>,
        /// The thread's end of it, held across rounds exactly as [`run`] holds
        /// one — so a test can step twice and see what the second step skips.
        restore: Restore,
    }

    fn harness() -> Harness {
        let (notes, note_consumer) = harmonigraph_record::publication::channel();
        let (audio, audio_consumer) = crate::audio_ingress::channel(crate::AUDIO_RING_CAPACITY);
        let (recorder, take_control) = harmonigraph_record::channel();
        let shared = EditorShared::new(
            note_consumer,
            audio_consumer,
            Arc::new(AtomicU32::new(48_000.0f32.to_bits())),
            take_control,
        );
        let ui_state = Arc::new(RwLock::new(String::new()));
        Harness {
            shared: Arc::new(Mutex::new(shared)),
            recorder,
            notes,
            audio,
            audio_frames: 0,
            editor_state: EguiState::from_size(800, 600),
            restore: Restore::of(ui_state.clone()),
            ui_state,
        }
    }

    /// A project blob that differs from a fresh install in its analyzer Window
    /// and nothing else, written by the same call the editor saves through.
    fn blob_with_window(window: harmonigraph_ui::SpectrumWindow) -> String {
        let mut state = harmonigraph_ui::SharedState::new(crate::editor::ASSUMED_SURFACE_FORMAT);
        state.picture.appearance.spectrum.window = window;
        harmonigraph_ui::shell::close(&state)
    }

    /// The analysis window's column lag, in seconds, at the harness's rate:
    /// what [`AudioSpectrum::column_lag`] reads back out of the analyzer, and
    /// so the one window setting observable from outside it.
    ///
    /// [`AudioSpectrum::column_lag`]: harmonigraph_ui::AudioSpectrum::column_lag
    fn lag_of(window: harmonigraph_ui::SpectrumWindow) -> f64 {
        harmonigraph_ui::SpectrumConfig { window, ..Default::default() }.column_lag(48_000.0)
    }

    impl Harness {
        /// One round of the loop the thread runs, against a mirror that
        /// survives between rounds.
        fn step(&mut self) {
            super::step(&self.shared, &self.editor_state, &mut self.restore);
        }

        /// What the host does when it restores a project — or pushes a preset
        /// at a plugin whose window is shut.
        fn restore(&mut self, blob: &str) {
            *self.ui_state.write() = blob.to_string();
        }

        /// Mono audio at 48 kHz, enough for the analyzer to fill a window and
        /// cross hop boundaries, so a drain of it must produce columns.
        fn push_audio(&mut self, frames: usize) {
            let start = self.audio_frames;
            self.audio.publish(
                frames,
                crate::audio_ingress::Format {
                    channels: 1,
                    sample_rate: 48_000.0,
                    sidechain: false,
                },
                start as f64 / 48_000.0,
                (0..frames).map(|i| {
                    let t = (start + i as u64) as f32 / 48_000.0;
                    (std::f32::consts::TAU * 440.0 * t).sin() * 0.5
                }),
            );
            self.audio_frames += frames as u64;
            self.notes.observe_clock(self.audio_frames as f64 / 48_000.0);
        }

        /// One note-on, stamped on the audio thread's own sample clock.
        fn push_note(&mut self, time: f64, note: u8) {
            self.push_kind(time, note, NoteEventKind::On { velocity: 1.0 });
        }

        fn push_note_off(&mut self, time: f64, note: u8) {
            self.push_kind(time, note, NoteEventKind::Off);
        }

        fn push_kind(&mut self, time: f64, note: u8, kind: NoteEventKind) {
            self.notes.observe_clock(time);
            self.notes
                .note(
                    CoreNoteEvent { source: SourceId::DIRECT, time, channel: 0, note, kind }.into(),
                    Default::default(),
                )
                .expect("the ring is sized for this");
        }

        fn columns(&self) -> usize {
            self.shared.lock().ui.picture.runtime.spectrum.history().len()
        }

        fn voices(&self) -> usize {
            self.shared.lock().ui.picture.runtime.tracker.voices().count()
        }

        /// The window the SETTING asks for.
        fn configured_window(&self) -> harmonigraph_ui::SpectrumWindow {
            self.shared.lock().ui.picture.appearance.spectrum.window
        }

        /// The window the ANALYZER is actually running at, which is the claim
        /// worth making: the setting reaching the config and stopping there is
        /// precisely the bug (see `AudioSpectrum::push_samples`, the one thing
        /// that carries one to the other).
        fn analyzed_lag(&self) -> f64 {
            self.shared.lock().ui.picture.runtime.spectrum.column_lag()
        }
    }

    /// The whole point: audio that arrives with nobody watching still becomes
    /// spectrogram history.
    #[test]
    fn a_closed_window_still_reaches_the_spectrogram() {
        let mut h = harness();
        h.push_audio(20_000);
        assert_eq!(h.columns(), 0, "nothing is analyzed before a drain");

        h.step();

        assert!(h.columns() > 0, "a closed window dropped its audio on the floor");
    }

    /// The notes travel with them, and not as a bonus: a full ring drops the
    /// NEWEST, so notes left to pile up until the reopen would lose the recent
    /// ribbons beside the audio. Captured here instead, both streams stay current.
    #[test]
    fn a_closed_window_still_reaches_the_roll() {
        let mut h = harness();
        h.push_note(1.0, 60);
        h.push_note(1.5, 64);
        assert_eq!(h.voices(), 0, "nothing is tracked before a drain");

        h.step();

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

        h.step();

        assert_eq!(
            h.voices(),
            1,
            "a voice whose fade finished 99 s ago is still in the tracker: the drain \
             feeds the released tail and nothing empties it while the window is shut",
        );
    }

    /// And with the window open: the worker drains both rings all the same,
    /// so an open window's picture is fed by the one path a shut one's is.
    #[test]
    fn an_open_window_is_fed_by_the_worker_too() {
        let mut h = harness();
        h.push_audio(20_000);
        h.push_note(1.0, 60);
        h.editor_state.set_open(true);

        h.step();
        assert!(h.columns() > 0, "the worker left the audio to a frame");
        assert_eq!(h.voices(), 1, "the worker left the notes to a frame");
        assert!(h.shared.lock().fresh_notes, "the frame was not told to draw the note");
    }

    /// A take ends itself with the window SHUT, which is the state a Bitwig
    /// audio export leaves the plugin in.
    ///
    /// Every trigger but `OnDisarm` is decided in `poll_take_end`, and until
    /// this call that only ever ran inside a GUI frame — so an armed take with
    /// nobody watching ran on and rendered nothing, whichever trigger was
    /// chosen. Driven here through `OnTransportStop`'s debounce rather than the
    /// bar, because that path needs no audio thread to latch
    /// anything: what is being claimed is that the DECISION runs at all, not
    /// which signal reaches it.
    #[test]
    fn a_take_ends_itself_while_the_editor_window_is_shut() {
        let mut h = harness();
        let directory =
            std::env::temp_dir().join(format!("background-take-end-{}", std::process::id()));
        {
            let mut shared = h.shared.lock();
            harmonigraph_record::testing::set_renderer_program(
                &shared.take,
                directory.join("absent-renderer"),
            );
            let _probe = harmonigraph_record::testing::worker_probe(&shared.take, directory);
            shared.ui.picture.appearance.render.trigger =
                harmonigraph_ui::RenderTrigger::OnTransportStop;
            // Armed through the Control directly, which is what the Video
            // pane's toggle reaches: this test is about what happens with no
            // frame running, so it must not need one to set up either.
            let appearance = shared.ui.picture.appearance.serialize();
            shared.take.start(48_000.0, appearance);
            assert!(shared.take.is_recording(), "armed");
        }
        // Something captured — one note, as the Hub publishes it into the
        // take's pass — and the transport since stopped: the two conditions
        // the debounce needs before it will end a take.
        h.recorder.is_armed();
        let route = harmonigraph_record::publication::Route {
            address: h.recorder.configuration_address(),
            time_offset: 0.0,
        };
        let on = CoreNoteEvent::on(0.0, SourceId::DIRECT, 0, 60, 1.0);
        let _ = h.recorder.publish_note(on.into(), route);

        h.editor_state.set_open(false);
        h.step();
        assert!(h.shared.lock().take.is_recording(), "ended before the debounce");
        std::thread::sleep(Duration::from_secs_f64(EditorShared::STOP_SECONDS));
        h.step();
        assert!(!h.shared.lock().take.is_recording(), "the shut window ended the take");
    }

    /// The point of the worker: its FFTs run with the editor's lock free, so
    /// a frame or a window opening on the host's main thread never waits for
    /// one. Just before them the lock is free and the audio taken is not yet
    /// analyzed, so the first hold did not run them, and the second only
    /// gives back what they made.
    #[test]
    fn the_analysis_runs_with_the_lock_free() {
        let mut h = harness();
        h.push_audio(20_000);
        let between = std::rc::Rc::new(std::cell::Cell::new(None));
        let seen = between.clone();
        BETWEEN_HOLDS.set(Some(Box::new(move |shared: &Mutex<EditorShared>, taken| {
            seen.set(Some((shared.try_lock().is_some(), taken.unanalyzed_audio())));
        })));
        h.step();
        BETWEEN_HOLDS.set(None);

        assert_eq!(between.get(), Some((true, true)), "the lock was held, or the audio analyzed");
        assert!(h.columns() > 0, "the step analyzed nothing");
    }

    /// A Clear the window makes while a step analyzes empties the history of
    /// that step's columns too: they come from audio before the Clear.
    #[test]
    fn a_clear_during_the_analysis_keeps_its_columns_out() {
        let mut h = harness();
        h.push_audio(20_000);
        BETWEEN_HOLDS.set(Some(Box::new(|shared: &Mutex<EditorShared>, _| {
            shared.lock().ui.picture.runtime.spectrum.clear_history();
        })));
        h.step();
        BETWEEN_HOLDS.set(None);
        assert_eq!(h.columns(), 0, "the step put back columns from before the Clear");

        h.push_audio(20_000);
        h.step();
        assert!(h.columns() > 0, "audio after the Clear was not kept");
    }

    /// A step that panics with the analysis lent out leaves the next one
    /// analyzing again, with its columns dated where they belong rather than
    /// from the frame zero of the run the panic interrupted.
    #[test]
    fn a_panicking_step_leaves_the_next_one_analyzing() {
        let mut h = harness();
        h.push_audio(20_000);
        h.step();
        let before = h.columns();
        assert!(before > 0, "the first step analyzed nothing");
        let newest = |h: &Harness| {
            h.shared.lock().ui.picture.runtime.spectrum.history().back().unwrap().time
        };
        let first = newest(&h);

        h.push_audio(20_000);
        BETWEEN_HOLDS.set(Some(Box::new(|_: &Mutex<EditorShared>, _| panic!("a step's bug"))));
        let mut panicked = false;
        super::guarded_step(&h.shared, &h.editor_state, &mut h.restore, &mut panicked);
        BETWEEN_HOLDS.set(None);
        assert!(panicked, "the hook did not run");

        h.push_audio(40_000);
        h.step();
        assert!(h.columns() > before, "nothing was analyzed after the panic");
        // Both newest columns lag their audio's end by the same column lag,
        // and the audio ends 60 000 frames apart; the restarted run's hop
        // grid puts its newest column up to a hop (8 ms) short of that.
        let spacing = newest(&h) - first;
        assert!(
            (spacing - 60_000.0 / 48_000.0).abs() < 0.01,
            "the newest columns are {spacing:.3} s apart where their audio is 1.25 s: the \
             columns after the panic are dated from the interrupted run",
        );
    }

    /// A step analyzes under the settings it took the audio with, even if they
    /// change while it analyzes, and the next step under the new ones.
    #[test]
    fn a_step_analyzes_under_the_settings_it_took() {
        use harmonigraph_ui::SpectrumWindow;

        let mut h = harness();
        let first = h.configured_window();
        let second = if first == SpectrumWindow::Precise {
            SpectrumWindow::Fast
        } else {
            SpectrumWindow::Precise
        };
        h.push_audio(40_000);
        BETWEEN_HOLDS.set(Some(Box::new(move |shared: &Mutex<EditorShared>, _| {
            shared.lock().ui.picture.appearance.spectrum.window = second;
        })));
        h.step();
        BETWEEN_HOLDS.set(None);
        assert!((h.analyzed_lag() - lag_of(first)).abs() < 1e-9, "analyzed under the edit");

        h.push_audio(40_000);
        h.step();
        assert!((h.analyzed_lag() - lag_of(second)).abs() < 1e-9, "the next step missed the edit");
    }

    /// Issue #324: the project's saved Window has to reach the analyzer with no
    /// editor anywhere in the picture, because the stretch before the first
    /// open is exactly what this thread exists to cover.
    ///
    /// Asserted on [`AudioSpectrum::column_lag`] — the analysis window's lag —
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
        // A Precise frame is 24,576 samples, so this is comfortably more than
        // one frameful and the analyzer cannot come out of it empty.
        h.push_audio(40_000);

        h.step();

        assert!(h.columns() > 0, "nothing was analyzed at all");
        assert_eq!(h.configured_window(), SpectrumWindow::Precise, "the blob never landed");
        assert!(
            (h.analyzed_lag() - lag_of(SpectrumWindow::Precise)).abs() < 1e-9,
            "columns analyzed at a {:.1} ms lag where the project's window lags {:.1} ms: the \
             restored setting reached the config and not the analyzer",
            h.analyzed_lag() * 1000.0,
            lag_of(SpectrumWindow::Precise) * 1000.0,
        );
    }

    /// The trap in fixing it. `params.ui_state` is written only by a host
    /// restore and on the way OUT of an editor session — a host save with the
    /// window open serializes the live state rather than storing it — so it
    /// names something the open window has already applied, and re-applying
    /// that under it would revert everything the user has changed since. (A
    /// restore that lands while the window is open reaches it through the
    /// window's own frame instead; see `UiState`.)
    ///
    /// The guard is [`step`]'s existing open-window check, which is why this
    /// test lives next to the drain it also guards: anything that moved the
    /// adopt ahead of that check would pass every other test here.
    #[test]
    fn an_open_window_keeps_the_settings_it_is_being_used_to_change() {
        use harmonigraph_ui::SpectrumWindow;

        let mut h = harness();
        // What the project saved, against what the user has since dialled in.
        h.restore(&blob_with_window(SpectrumWindow::Precise));
        h.shared.lock().ui.picture.appearance.spectrum.window = SpectrumWindow::Fast;
        h.editor_state.set_open(true);

        h.step();

        assert_eq!(
            h.configured_window(),
            SpectrumWindow::Fast,
            "the saved blob was re-applied under an open window, reverting the setting the \
             user is holding the window open to change",
        );
    }

    #[test]
    fn a_tick_admitted_before_opening_cannot_overwrite_a_live_edit() {
        use harmonigraph_ui::SpectrumWindow;
        use std::sync::Barrier;

        let mut h = harness();
        let blob = blob_with_window(SpectrumWindow::Precise);
        h.restore(&blob);
        let gate = Arc::new(Barrier::new(2));
        let shared = h.shared.clone();
        let editor_state = h.editor_state.clone();
        let ui_state = h.ui_state.clone();
        std::thread::scope(|scope| {
            let worker_gate = gate.clone();
            let worker = scope.spawn(move || {
                TICK_PAUSE.with_borrow_mut(|pause| *pause = Some(worker_gate));
                // This worker has not adopted the blob yet.
                let mut restore = Restore::of(ui_state);
                step(&shared, &editor_state, &mut restore);
            });
            gate.wait();
            {
                let mut shared = h.shared.lock();
                shared.ui.load_persist(&blob);
                h.editor_state.set_open(true);
                shared.ui.picture.appearance.spectrum.window = SpectrumWindow::Fast;
            }
            gate.wait();
            worker.join().unwrap();
        });
        assert!(h.editor_state.is_open());
        assert_eq!(h.configured_window(), SpectrumWindow::Fast);
    }

    /// A host writes that field more than once — a preset change or an undo
    /// pushes fresh state at a plugin whose window is shut — so what the thread
    /// keeps has to be a mirror of the blob and not a done-once flag.
    #[test]
    fn a_project_switched_while_the_window_is_shut_is_taken_too() {
        use harmonigraph_ui::SpectrumWindow;

        let mut h = harness();
        h.restore(&blob_with_window(SpectrumWindow::Precise));
        h.step();
        assert_eq!(h.configured_window(), SpectrumWindow::Precise);

        h.restore(&blob_with_window(SpectrumWindow::Fast));
        h.step();

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
        h.step();

        h.restore("");
        h.step();

        assert_eq!(
            h.shared
                .lock()
                .ui
                .picture
                .runtime
                .console
                .lines()
                .filter(|l| l.contains("persist ignored"))
                .count(),
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
            ui.picture.appearance.spectrum.window = SpectrumWindow::Precise;
            ui.picture.appearance.spectrum.floor_db = -72.0;
            ui.workspace.interaction.frame_rate = harmonigraph_ui::FrameRate::Third;
        }
        // What the close writes into `params.ui_state`.
        let saved = harmonigraph_ui::shell::close(&h.shared.lock().ui);
        h.restore(&saved);

        h.step();

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
            h.step();
        }

        let refusals = h
            .shared
            .lock()
            .ui
            .picture
            .runtime
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
            span > SHUT_POLL.as_secs_f64() * 8.0,
            "the ring holds {span:.3} s against a {:.3} s poll: too little slack for a \
             thread the scheduler is free to ignore",
            SHUT_POLL.as_secs_f64(),
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
            InputWorker::spawn(h.shared.clone(), h.editor_state.clone(), h.ui_state.clone());
        // Polled to a deadline, so a loaded machine cannot fail this for being
        // slow — it is asserting that steps happen at all, not how fast.
        let started = Instant::now();
        while h.columns() == 0 {
            assert!(started.elapsed() < Duration::from_secs(5), "the thread never took a step");
            std::thread::sleep(POLL);
        }
        let analyzed = h.columns();

        let started = Instant::now();
        drop(analyzer);
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "the join took {:?}: the flag is not being read once a round",
            started.elapsed(),
        );

        h.push_audio(20_000);
        std::thread::sleep(SHUT_POLL * 10);
        assert_eq!(h.columns(), analyzed, "the thread outlived the plugin that owned it");
    }
}
