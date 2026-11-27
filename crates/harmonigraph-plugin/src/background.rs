//! The input worker: the one thread that drains the audio thread's rings and
//! runs the spectrum analysis, whether or not the editor window is open.
//!
//! `process` fills both audio→GUI rings whether or not anyone is watching. The
//! note ring holds 4096 events, minutes of playing; the audio ring holds
//! [`AUDIO_RING_CAPACITY`](crate::AUDIO_RING_CAPACITY) samples, 1.37 s of
//! stereo at 48 kHz, so it saturates within seconds and every sample after
//! that is dropped. A frame used to drain both with the window open and this
//! thread with it shut; now this thread drains them always. The window's
//! state sets how often it polls; it decides nothing about what is drained
//! (#1559), nor whether the picture ages and a take's end is polled.
//!
//! **The FFTs run with no lock held.** A step takes the editor's lock to move
//! the notes into the tracker and the audio out of its ring (a copy), lets
//! go, analyzes, and takes it again to hand the columns to the store
//! ([`LiveInput::take`], then the [`Taken`] it returns: `analyze`, `give`).
//! So the host's main thread, which a frame and a window opening run on,
//! waits on this thread for a copy or an append, never for an FFT, and no
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
//! Folding each column into every level of the spectrogram's slab pyramid
//! costs about 11 µs more (#1758), and the pyramid's memory, about 30 MB
//! when full, is bounded by its rings. A different hop is not the only
//! possible cost reduction, and would need to keep the pyramid's levels on
//! the `live_slab` ladder of their shared `FFT_INTERVAL` grid.
//!
//! **Every step also does the two duties no frame may be needed for,** editor
//! open or shut: the ageing a frame gets from `begin_frame`
//! ([`VisualRuntime::advance_time`](harmonigraph_ui::VisualRuntime::advance_time)),
//! and [`EditorShared::poll_take_end`] so a take can finish and render.
//! A frame runs only while the editor can be seen, so an open editor that is
//! covered, minimized or on another Space runs none; were these a frame's
//! alone, the released voices would pile up and an OnTransportStop take would
//! record on past its stop. Frames age the picture too, in `begin_frame`,
//! which is harmless: both run under the editor lock, ageing on one monotone
//! clock and removing only voices whose fade is over. The take's end is polled
//! here alone, its debounce counted in seconds. This completion poll is
//! separate from the GUI's recording commands;
//! `a_take_ends_itself_with_no_frame_running` covers it.
//!
//! **The project's own settings arrive here too**, for the same reason: this
//! is the only thing running before the editor's window exists. A host
//! restore waits in the editor lock's inbox until a holder of that lock
//! installs it ([`EditorLock`]), and this thread's first hold of every step
//! comes before its take. Were it left to the window, every column analyzed
//! before the first open would be analyzed at [`SpectrumConfig::default`]'s
//! window whatever the project saved — and the two stretches would then differ
//! in how long every pitch listened and how far behind its audio each column
//! is dated, so their join would scroll across the heatmap as a seam (#324).
//!
//! [`EditorShared::poll_take_end`]: crate::editor::EditorShared::poll_take_end
//! [`LiveInput::take`]: crate::editor::input::LiveInput::take
//! [`Taken`]: crate::editor::input::Taken
//! [`SpectrumConfig::default`]: harmonigraph_ui::SpectrumConfig

#[cfg(test)]
use std::sync::atomic::AtomicU64;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use crate::editor::input::LiveInput;
use crate::editor::{EditorLock, EguiState};

/// How often the worker looks for input while the editor window is open.
///
/// Under the 8 ms hop, so a column is analyzed within a poll of its audio
/// arriving, where a frame used to analyze it at the next frame: the worst
/// case for a block is now a frame plus a poll. Cheap: a step that finds
/// nothing with the window open is one short hold of an uncontended lock.
pub(crate) const POLL: Duration = Duration::from_millis(4);

/// How often it looks with the window shut, where nothing is drawing what it
/// analyzes, so it need not wake as often; the ageing and the take's poll
/// (see [`step`]) are content with any cadence this bound allows.
///
/// Bounded above by the ring itself: it holds
/// [`AUDIO_RING_CAPACITY`](crate::AUDIO_RING_CAPACITY) interleaved samples,
/// which is 1.37 s of stereo at 48 kHz, the fastest rate the projects run at
/// — and a poll that ever slipped past that span would drop audio silently
/// and for good, reintroduced by this thread's own pacing. 20 ms, the longest
/// it sleeps, leaves 68x margin there, so the scheduler has to lose this
/// thread for over a second before anything is lost.
/// `the_ring_holds_many_polls_of_audio_at_the_projects_rates` is what keeps
/// the two numbers in that relation.
pub(crate) const SHUT_POLL: Duration = Duration::from_millis(20);

/// What a step last saw of the tracker: how many edits it had taken, and the
/// display time the step read ([`Self::edited`]).
#[derive(Default)]
struct Seen {
    edits: u64,
    display: f64,
}

impl Seen {
    /// Whether the window should draw for the tracker's `edits`, at a step
    /// that read the display clock at `display`: for any edit it took since
    /// the last step, a release, a bend or a pressure change as much as a
    /// note, and for as long as the latest is dated at or past the display
    /// time the last step read: so an edit dated ahead of the clock, as a
    /// Hub's are by the Tuning delay, keeps asking until the step whose clock
    /// has passed it, whose pass draws it.
    fn edited(&mut self, edits: harmonigraph_core::Edits, display: f64) -> bool {
        let before = std::mem::replace(self, Seen { edits: edits.count, display });
        edits.count != before.edits || edits.latest > before.display
    }
}

/// The input worker's thread, and the flag that ends it.
///
/// Held by the plugin to stop it: `Plugin::destroy` stops and joins it, so no
/// step or host wake of its outlives the plugin's destruction, and its
/// [`Drop`] does the same for a plugin dropped without one. A plugin removed
/// from a project must not leave a thread behind doing FFTs on a ring nobody
/// fills again.
pub(crate) struct InputWorker {
    stop: Arc<AtomicBool>,
    /// Completed loop rounds, exposed only to tests that need an observable
    /// boundary after changing the editor-open flag.
    #[cfg(test)]
    rounds: Arc<AtomicU64>,
    /// `None` when the thread could not be spawned. Nothing else reads the
    /// rings or ends a take, and only a frame, which runs while the editor can
    /// be seen, ages the picture, so the editor then shows no input until the
    /// plug-in is reloaded: a case outside the use, with a
    /// Console line rather than a fallback that would run the analysis on
    /// the host's main thread. The thread owns the input.
    thread: Option<JoinHandle<Ended>>,
}

/// What the thread hands back as it ends: nothing, so the input it owned is
/// freed there rather than on the thread that drops the plugin; in tests, the
/// input, for them to read (`InputWorker::join`).
#[cfg(not(test))]
type Ended = ();
#[cfg(test)]
type Ended = LiveInput;

#[cfg(not(test))]
fn ended(_: LiveInput) {}
#[cfg(test)]
fn ended(input: LiveInput) -> LiveInput {
    input
}

impl InputWorker {
    pub(crate) fn spawn(
        shared: Arc<EditorLock>,
        editor_state: Arc<EguiState>,
        tape: crate::tape::Worker,
        input: LiveInput,
        lights: Option<crate::board::Lights>,
        learn: Option<Arc<crate::HarmonigraphParams>>,
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
                let (mut tape, mut input, mut lights) = (tape, input, lights);
                let learn = learn.as_deref();
                #[cfg(not(test))]
                run(&worker, &editor_state, &mut tape, &mut input, &mut lights, learn, &flag);
                #[cfg(test)]
                run(
                    &worker,
                    &editor_state,
                    &mut tape,
                    &mut input,
                    &mut lights,
                    learn,
                    &flag,
                    &worker_rounds,
                );
                ended(input)
            });
        let thread = match thread {
            Ok(thread) => Some(thread),
            // Audible rather than silent, on the same argument as a refused
            // persist blob: an empty picture looks exactly like one nobody
            // was playing into.
            Err(err) => {
                shared.lock().ui.picture.runtime.console.log(format!(
                    "input worker not started ({err}) — no input is read until the \
                     plug-in is reloaded",
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

    /// Stop the thread and wait for it, at the plugin's destruction: no step,
    /// nor a host wake one makes, outlives this.
    pub(crate) fn stop(&mut self) {
        self.stop_and_join();
    }

    /// Stops the thread and waits for it, and what it handed back; `None`
    /// when it was never spawned, already joined, or ended in a panic outside
    /// a step. Joining can wait for the current step and one [`SHUT_POLL`]
    /// sleep at most.
    fn stop_and_join(&mut self) -> Option<Ended> {
        self.stop.store(true, Ordering::Relaxed);
        self.thread.take().and_then(|thread| thread.join().ok())
    }

    /// [`Self::stop_and_join`], for a test that drains the input by hand or
    /// reads what its analysis ran at.
    #[cfg(test)]
    pub(crate) fn join(&mut self) -> Option<LiveInput> {
        self.stop_and_join()
    }
}

impl Drop for InputWorker {
    /// Stops the thread, so the plugin never outlives its own worker.
    fn drop(&mut self) {
        self.stop_and_join();
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

#[cfg(test)]
thread_local! {
    /// Run between a step's two holds of the lock, just before its FFTs.
    #[allow(clippy::type_complexity)]
    static BETWEEN_HOLDS: std::cell::RefCell<Option<Box<dyn FnMut(&EditorLock, &crate::editor::input::Taken<'_>)>>> = const { std::cell::RefCell::new(None) };
    /// Run just after them, before the second hold.
    #[allow(clippy::type_complexity)]
    static ANALYZED: std::cell::RefCell<Option<Box<dyn FnMut(&crate::editor::input::Taken<'_>)>>> = const { std::cell::RefCell::new(None) };
}

/// One step: take the notes and the audio under the lock, age the picture and
/// poll a take's end in that same hold, analyze the audio without it, and give
/// the columns back under it. Taking the lock installs a host restore waiting
/// for it ([`EditorLock`]), window open or shut, and the first hold takes it
/// BEFORE the take: the settings decide how the samples about to be taken are
/// analyzed.
///
/// The ageing and the poll run once a step, every [`POLL`] with the window
/// open and every [`SHUT_POLL`] with it shut. `poll_take_end` times its
/// OnTransportStop debounce in seconds
/// ([`STOP_SECONDS`](crate::editor::EditorShared::STOP_SECONDS)), so it holds
/// at whatever rate it is polled.
///
/// Split out from the sleep loop so what it DECIDES can be tested without a
/// clock, which is most of what there is to get wrong here.
///
/// **The lock is waited for, not tried**: this thread never runs on the
/// host's main thread, so waiting out a frame costs the host nothing, and a
/// frame or a window opening waits on this thread only for a take (a copy of
/// what the ring holds, then the ageing, a pass over the fading voices, and
/// the take's poll, a few atomics) or a give (folding its columns into every
/// level of the spectrogram's history, 11 µs a column): a few tens of
/// microseconds for the
/// two or three columns a step usually brings, and a few milliseconds for a
/// catch-up's: the ring's 131,072 samples are 341 mono columns at 48 kHz,
/// after this thread was starved for 2.7 s, and 371 at 44.1 kHz after 3 s
/// (about 4.5 ms), more at lower rates; one such hold may cost a frame. The FFTs
/// between them, some 10 ms for a full stereo ring through Fast
/// at 48 kHz and 36 through Precise (see
/// [`SpectrumAnalysis::feed_source`](harmonigraph_ui::SpectrumAnalysis::feed_source)),
/// hold nothing. The thread runs at the GUI's
/// scheduling class on macOS (see [`InputWorker::spawn`]), so a frame waiting
/// on one of those holds is not also waiting for this thread to be scheduled.
///
/// **The tape** (see [`crate::tape`]) works between the two holds as well:
/// it records the blocks just taken, places the notes the drain recorded
/// beside them, keeps a cue's audio queued for the audio thread, and analyzes
/// the spectrogram Tape mode draws. It takes Tape mode's request and fills its
/// view in the first hold, and plays the notes of the tape the blocks just
/// taken carried into the tracker there, on their audio's clock; its console
/// lines wait for the second.
///
/// **A still window is asked to draw what a step changed**
/// ([`EguiState::request_pass`]): every edit of the tracker, played or from
/// the tape, a release, a bend or a pressure change as much as a note, until
/// the display clock has passed the latest ([`Seen::edited`]), and sound
/// after a still stretch. A still window runs no pass until something asks,
/// so without this an edit over a still picture was drawn at its next idle
/// poll. Sound asks only as it starts: once a picture shows it moving, the
/// window draws as it moves until it stops, and a picture that does not show
/// it has nothing new to draw. What else a step changes (the tape's view, a
/// Console line) is drawn at the idle poll.
fn step(
    shared: &EditorLock,
    editor_state: &EguiState,
    tape: &mut crate::tape::Worker,
    input: &mut LiveInput,
    seen: &mut Seen,
    learn: Option<&crate::HarmonigraphParams>,
) {
    // What the exchange displaced from the view, freed at the end of the step:
    // declared before either hold's guard, so it outlives both, an early
    // return and a panic included. That moves its cost past the step's live
    // work rather than removing it: a large free, up to about 19 ms after a
    // format change clears a 25-minute tape, can delay the next step's drain,
    // though never under the lock.
    let _displaced;
    let (taken, edited, learned) = {
        let mut guard = shared.lock();
        let shared = &mut *guard;
        let now = shared.clock.now();
        let open = editor_state.is_open();
        let settings = &shared.ui.picture.appearance.spectrum;
        _displaced = tape.exchange(&mut shared.ui.picture.runtime.tape, settings, open);
        // The drain's marks go straight to the tape, which `settle` empties
        // every step.
        let taken = input.take(&mut shared.clock, &mut shared.ui.picture, &mut tape.inbox, now);
        if let Some(taken) = &taken {
            tape.feed(taken, &mut shared.ui.picture.runtime);
        }
        let learned =
            learn.is_some_and(|params| self::learn(params, &mut shared.ui.picture.runtime));
        // A frame's duties that must not wait for one: none runs while the
        // editor cannot be seen, open or not. Every trigger but OnDisarm is
        // decided in `poll_take_end`, so without it here an armed take would
        // run to whenever a frame next noticed, and a Bitwig audio export,
        // which leaves the window shut, would render nothing.
        let display = shared.clock.display_now(now);
        let edited = seen.edited(shared.ui.picture.runtime.tracker.edits(), display);
        shared.ui.picture.runtime.advance_time(display, &shared.ui.picture.appearance);
        shared.poll_take_end();
        (taken, edited, learned)
    };
    // Out of the editor lock: the host's wake must not run under it (#1924 C).
    if let Some(params) = learn.filter(|_| learned) {
        params.settings.changed();
    }
    // Asked once the lock is let go, so the pass it runs does not wait on it.
    if edited {
        editor_state.request_pass();
    }
    if let Some(taken) = &taken {
        for (block, samples) in taken.blocks() {
            tape.capture(block, samples);
        }
    }
    tape.settle();
    tape.play();
    tape.roll();
    tape.analyze();
    tape.report();
    let taken = taken.map(|mut taken| {
        #[cfg(test)]
        BETWEEN_HOLDS.with_borrow_mut(|hook| hook.as_mut().map(|hook| hook(shared, &taken)));
        taken.analyze();
        #[cfg(test)]
        ANALYZED.with_borrow_mut(|hook| hook.as_mut().map(|hook| hook(&taken)));
        taken
    });
    if taken.is_none() && tape.logs.is_empty() {
        return;
    }
    let mut guard = shared.lock();
    let shared = &mut *guard;
    let resumed = taken.is_some_and(|taken| taken.give(&mut shared.ui.picture.runtime));
    for line in tape.logs.drain(..) {
        shared.ui.picture.runtime.console.log(line);
    }
    drop(guard);
    if resumed {
        editor_state.request_pass();
    }
}

/// A [`step`] that survives its own panic. One would otherwise end the
/// thread with nothing draining either ring, which freezes the picture and
/// every duty of a step for the session with nothing said. Handing the drain
/// to the frames instead would only move the same panic onto the host's main
/// thread. So the analysis starts fresh, whatever the panic left it in, with
/// no lock held, the source run restarts so its columns re-anchor, the
/// console says so once, and the next step carries on.
///
/// Whether the step completed.
#[allow(clippy::too_many_arguments)]
fn guarded_step(
    shared: &EditorLock,
    editor_state: &EguiState,
    tape: &mut crate::tape::Worker,
    input: &mut LiveInput,
    seen: &mut Seen,
    learn: Option<&crate::HarmonigraphParams>,
    panicked: &mut bool,
) -> bool {
    let stepped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        step(shared, editor_state, tape, input, seen, learn)
    }));
    if stepped.is_err() {
        input.restart_analysis();
        if !std::mem::replace(panicked, true) {
            shared
                .lock()
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

/// One step of Learn (#1938 C), from the notes the step just drained: here
/// rather than in a frame, so it learns with the window covered or shut, as
/// it did on audio. The gate, the inference and the result are one write of
/// the document (`Settings::learn`). The editor's lock is held, then the
/// document's: the order a frame takes them in. Whether what the project
/// saves changed, for the caller to mark once the editor lock is let go.
fn learn(params: &crate::HarmonigraphParams, runtime: &mut harmonigraph_ui::VisualRuntime) -> bool {
    let engine = || {
        harmonigraph_core::lattice_map::TuningEngine::from_index(
            params.int(crate::Row::TuningEngine),
        )
    };
    params.settings.learn(engine, |armed| runtime.learn(armed))
}

/// How many consecutive steps may panic before the worker stops trying.
const MAX_PANICS_IN_A_ROW: u32 = 8;

/// Drain, sleep, repeat, until the plugin goes away.
///
/// The stop flag is read once per step. Joining waits for any current step
/// and sleep; what the rings still hold then goes with the plugin, since
/// nothing reads the picture after its destruction.
// The test build's round counter is the eighth.
#[cfg_attr(test, allow(clippy::too_many_arguments))]
fn run(
    shared: &EditorLock,
    editor_state: &EguiState,
    tape: &mut crate::tape::Worker,
    input: &mut LiveInput,
    lights: &mut Option<crate::board::Lights>,
    learn: Option<&crate::HarmonigraphParams>,
    stop: &AtomicBool,
    #[cfg(test)] rounds: &AtomicU64,
) {
    let mut panicked = false;
    let mut in_a_row = 0;
    let mut seen = Seen::default();
    while !stop.load(Ordering::Relaxed) {
        if guarded_step(shared, editor_state, tape, input, &mut seen, learn, &mut panicked) {
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
        // After the step, so the board shows the notes it just drained.
        // Guarded as a step is, but a panic here ends only the lights.
        if let Some(lit) = lights {
            let polled =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| lit.poll(shared)));
            if polled.is_err() {
                *lights = None;
                shared.lock().ui.picture.runtime.console.log(
                    "the lattice-board's lights panicked and stay off until the plugin is reloaded"
                        .to_string(),
                );
            }
        }
        #[cfg(test)]
        rounds.fetch_add(1, Ordering::Release);
        std::thread::sleep(if editor_state.is_open() { POLL } else { SHUT_POLL });
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use harmonigraph_core::notes::{NoteEvent as CoreNoteEvent, NoteEventKind, SourceId};

    use super::*;
    use crate::editor::EditorShared;

    /// What a shell owns between the audio thread and the GUI: the state, the
    /// producer ends of both rings, and the window's open flag.
    struct Harness {
        shared: Arc<EditorLock>,
        /// The audio thread's half of `shared.take`, for a test that needs a
        /// take to have captured something.
        recorder: harmonigraph_record::Recorder,
        notes: harmonigraph_record::publication::Publisher,
        audio: crate::audio_ingress::Producer,
        audio_frames: u64,
        editor_state: Arc<EguiState>,
        tape: crate::tape::Worker,
        /// The input the thread would own, and what its steps saw.
        input: LiveInput,
        seen: super::Seen,
        /// The plugin whose tuning Learn writes, when a test has it learn.
        learn: Option<Arc<crate::HarmonigraphParams>>,
    }

    fn tape() -> crate::tape::Worker {
        crate::tape::channel(Arc::new(crate::HarmonigraphParams::default())).1
    }

    fn harness() -> Harness {
        let (notes, note_consumer) = harmonigraph_record::publication::channel();
        let (audio, audio_consumer) = crate::audio_ingress::channel(crate::AUDIO_RING_CAPACITY);
        let (recorder, take_control) = harmonigraph_record::channel(crate::BUILD_TAG);
        let shared = EditorShared::new(take_control);
        Harness {
            shared: Arc::new(EditorLock::new(shared, Arc::default())),
            recorder,
            notes,
            audio,
            audio_frames: 0,
            editor_state: EguiState::from_size(800, 600),
            tape: tape(),
            input: LiveInput::new(note_consumer, audio_consumer),
            seen: Default::default(),
            learn: None,
        }
    }

    /// The analysis window's column lag, in seconds, at the harness's rate:
    /// what [`SpectrumAnalysis::lag`] reads back out of the analyzer, and so
    /// the one window setting observable from outside it.
    ///
    /// [`SpectrumAnalysis::lag`]: harmonigraph_ui::SpectrumAnalysis::lag
    fn lag_of(window: harmonigraph_ui::SpectrumWindow) -> f64 {
        harmonigraph_ui::SpectrumConfig { window, ..Default::default() }.column_lag(48_000.0)
    }

    impl Harness {
        /// One round of the loop the thread runs.
        fn step(&mut self) {
            super::step(
                &self.shared,
                &self.editor_state,
                &mut self.tape,
                &mut self.input,
                &mut self.seen,
                self.learn.as_deref(),
            );
        }

        /// Mono audio at 48 kHz, enough for the analyzer to fill a window and
        /// cross hop boundaries, so a drain of it must produce columns.
        fn push_audio(&mut self, frames: usize) {
            self.push_placed(frames, None, None);
        }

        /// [`Self::push_audio`] rolling at song seconds `song`, or carrying
        /// the tape's audio from song seconds `heard`.
        fn push_placed(&mut self, frames: usize, song: Option<f64>, heard: Option<f64>) {
            let tape = heard.map_or(crate::audio_ingress::TapePlay::Idle, |seconds| {
                crate::audio_ingress::TapePlay::Heard(crate::audio_ingress::Heard {
                    cue: 1,
                    seconds,
                    frames,
                    speed: 1.0,
                })
            });
            self.push_tape(frames, song, tape);
        }

        fn push_tape(
            &mut self,
            frames: usize,
            song: Option<f64>,
            tape: crate::audio_ingress::TapePlay,
        ) {
            let start = self.audio_frames;
            self.audio.publish(
                frames,
                crate::audio_ingress::Format {
                    channels: 1,
                    sample_rate: 48_000.0,
                    sidechain: false,
                },
                start as f64 / 48_000.0,
                song.map(|seconds| crate::audio_ingress::Song { seconds, meter: None }),
                tape,
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
                )
                .expect("the ring is sized for this");
        }

        fn columns(&self) -> u64 {
            self.shared.lock().ui.picture.runtime.spectrum.columns_taken()
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
        /// precisely the bug (see `SpectrumAnalysis::feed_source`, the one
        /// thing that carries one to the other).
        fn analyzed_lag(&self) -> f64 {
            self.input.analysis_lag()
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

    /// The tape's notes end to end through the worker's own step: a note the
    /// drain shows while the song rolls is recorded at its song position,
    /// and a later block that carries the tape from just after it puts it
    /// back in the tracker, under the tape's own source. A block the deck
    /// starved is a pause: the note is held on, not struck again.
    #[test]
    fn a_note_recorded_while_rolling_sounds_again_when_the_tape_plays_there() {
        let mut h = harness();
        h.push_placed(4800, Some(0.0), None);
        h.push_note(0.05, 60);
        h.step();
        // The transport stops: the pass and its chunk are committed.
        h.push_placed(4800, None, None);
        h.step();
        let tape_held = |h: &Harness| {
            let shared = h.shared.lock();
            let tracker = &shared.ui.picture.runtime.tracker;
            tracker.held_voices().filter(|v| v.source.is_tape()).map(|v| v.note).collect::<Vec<_>>()
        };
        let onset = |h: &Harness| {
            let shared = h.shared.lock();
            let tracker = &shared.ui.picture.runtime.tracker;
            let onset = tracker.held_voices().find(|v| v.source.is_tape()).map(|v| v.on_time);
            onset
        };
        assert!(tape_held(&h).is_empty(), "nothing plays before the tape does");

        // Inside what is recorded, which ends at 0.1 s: the deck plays no
        // further, and the player lets go there.
        h.push_placed(1200, None, Some(0.06));
        h.step();
        assert_eq!(tape_held(&h), [60], "the tape's block did not chase the note held there");
        assert!(h.shared.lock().ui.picture.runtime.tape.audition.is_some());
        let struck = onset(&h);

        h.push_tape(600, None, crate::audio_ingress::TapePlay::Waiting);
        h.step();
        h.push_placed(600, None, Some(0.085));
        h.step();
        assert_eq!(onset(&h), struck, "a starved block ended the audition and struck it again");

        h.push_placed(2400, None, None);
        h.step();
        assert!(tape_held(&h).is_empty(), "the audition's end released the tape's note");
    }

    /// Feeding the tracker and AGEING it are one job, and this is the drainer
    /// that has to do both halves itself, with the editor open or shut.
    ///
    /// `NoteTracker` parks every note-off in a released tail that only
    /// [`NoteTracker::prune`](harmonigraph_core::NoteTracker::prune) empties,
    /// and the frame reaches `prune` through
    /// `begin_frame` rather than through the drain. A window that runs no
    /// frames, shut or open and unseen, therefore feeds that tail and nothing
    /// else empties it — which `notes.rs` already names as the hazard behind
    /// its whole envelope design: "the released tail would accumulate for the
    /// whole session against an O(nodes x voices) loop".
    #[test]
    fn the_worker_ages_the_voices_it_tracks_with_no_frame_running() {
        for open in [false, true] {
            let mut h = harness();
            h.editor_state.set_open(open);
            // Played and released far behind the batch's newest event, which
            // is what `ClockMapper` pins to `now` — so this voice's 1 s fade
            // is 99 s over by the time it lands, and nothing but a missing
            // prune can keep it alive.
            h.push_note(0.0, 60);
            h.push_note_off(0.1, 60);
            // Still held, so it is never a prune candidate: it is here to pin
            // the batch's newest timestamp, and to prove the count below is
            // measuring the released tail rather than an empty tracker.
            h.push_note(100.0, 64);

            h.step();

            assert_eq!(
                h.voices(),
                1,
                "editor open: {open}. A voice whose fade finished 99 s ago is still in the \
                 tracker: the drain feeds the released tail and nothing empties it",
            );
        }
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
    }

    /// A step that changes the tracker asks the open window to draw it on its
    /// next frame, since no voice holds the loop over a still picture: a note,
    /// one joining a held one, and over the held chord a bend, a pressure
    /// change and a release, each dated where the clock the last step read
    /// already stands, as a block's events often are; and the first sound
    /// after silence. A step that brings nothing new, and sound that goes on,
    /// ask nothing.
    #[test]
    fn a_step_asks_the_window_to_draw_what_it_changes() {
        use harmonigraph_core::Expression;

        let mut h = harness();
        h.editor_state.set_open(true);
        let asked = |h: &Harness| h.editor_state.pass_request.take();
        let voice = |h: &Harness, note: u8| {
            let shared = h.shared.lock();
            let voice = shared.ui.picture.runtime.tracker.voices().find(|v| v.note == note);
            voice.expect("the fixture's note sounds")
        };
        h.push_note(1.0, 60);
        h.step();
        assert!(asked(&h), "the first note waited for the idle poll");
        h.step();
        assert!(!asked(&h), "a step that brought nothing asked");
        h.push_note(1.5, 64);
        h.step();
        assert!(asked(&h), "a note joining a held one waited for the idle poll");
        h.push_kind(1.5, 64, NoteEventKind::Tuning { semitones: 0.3 });
        h.step();
        assert!((voice(&h, 64).pitch - 64.3).abs() < 1e-6, "the fixture's bend must arrive");
        assert!(asked(&h), "a bend waited for the idle poll");
        let pressure = NoteEventKind::Expression { expression: Expression::Pressure, value: 0.7 };
        h.push_kind(1.5, 64, pressure);
        h.step();
        assert_eq!(voice(&h, 64).expressions.pressure, 0.7, "the fixture's pressure must arrive");
        assert!(asked(&h), "a pressure change waited for the idle poll");
        h.push_note_off(1.5, 64);
        h.step();
        let released =
            matches!(voice(&h, 64).state, harmonigraph_core::VoiceState::Released { .. });
        assert!(released, "the fixture's release must arrive");
        assert!(asked(&h), "a release waited for the idle poll");
        h.push_audio(20_000);
        h.step();
        assert!(asked(&h), "the first sound waited for the idle poll");
        h.push_audio(20_000);
        h.step();
        assert!(!asked(&h), "sound that goes on");
    }

    /// An edit dated ahead of the display clock, as a Hub's are by the Tuning
    /// delay, draws nothing new in the pass asked for at its drain, so the
    /// worker goes on asking until the step whose clock has passed it, and
    /// stops after.
    #[test]
    fn an_edit_dated_ahead_asks_until_the_clock_has_passed_it() {
        let mut h = harness();
        h.editor_state.set_open(true);
        let asked = |h: &Harness| h.editor_state.pass_request.take();
        h.push_note(1.0, 60);
        h.step();
        assert!(asked(&h));
        // Published at the clock's 1.0 s, to sound a tenth of a second later.
        let ahead = CoreNoteEvent::on(1.1, SourceId::DIRECT, 0, 64, 1.0);
        h.notes.note(ahead.into()).expect("the ring is sized for this");
        h.step();
        assert!(asked(&h), "the edit's own step asked nothing");
        let (due, mut display) = {
            let mut shared = h.shared.lock();
            let tracker = &shared.ui.picture.runtime.tracker;
            let due = tracker.voices().find(|v| v.note == 64).expect("the edit arrived").on_time;
            let now = shared.clock.now();
            (due, shared.clock.display_now(now))
        };
        assert!(due > display + 0.05, "the fixture's edit must be dated ahead: {due} at {display}");
        let mut steps = 0;
        while display < due {
            std::thread::sleep(POLL);
            h.step();
            assert!(asked(&h), "a step before the clock passed the edit asked nothing");
            let mut shared = h.shared.lock();
            let now = shared.clock.now();
            display = shared.clock.display_now(now);
            steps += 1;
        }
        assert!(steps > 5, "the fixture's clock passed the edit in {steps} steps");
        // The step whose own reading passed it asks once more, and may be the
        // last of those above or the next.
        h.step();
        asked(&h);
        h.step();
        assert!(!asked(&h), "the worker went on asking after the clock passed the edit");
    }

    /// A take ends itself with no frame running: with the window SHUT, which
    /// is the state a Bitwig audio export leaves the plugin in, and with it
    /// open but unseen, where no frame runs either.
    ///
    /// Every trigger but `OnDisarm` is decided in `poll_take_end`, and while
    /// only a GUI frame made that call, an armed take with nobody watching ran
    /// on and rendered nothing, whichever trigger was chosen. Driven here
    /// through `OnTransportStop`'s debounce rather than the bar, because that
    /// path needs no audio thread to latch anything: what is being claimed is
    /// that the DECISION runs at all, not which signal reaches it.
    #[test]
    fn a_take_ends_itself_with_no_frame_running() {
        for open in [false, true] {
            a_take_ends_itself_with_the_editor(open);
        }
    }

    fn a_take_ends_itself_with_the_editor(open: bool) {
        let mut h = harness();
        let directory =
            std::env::temp_dir().join(format!("background-take-end-{open}-{}", std::process::id()));
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
            shared.take.start(appearance);
            assert!(shared.take.is_recording(), "armed");
        }
        // Something captured — one note, as the Hub publishes it into the
        // take's run — and the transport since stopped: the two conditions
        // the debounce needs before it will end a take.
        h.recorder.is_armed();
        h.recorder.record(0.0, Some(0.0));
        let on = CoreNoteEvent::on(0.0, SourceId::DIRECT, 0, 60, 1.0);
        let _ = h.recorder.publish_note(on.into());

        h.editor_state.set_open(open);
        h.step();
        assert!(
            h.shared.lock().take.is_recording(),
            "editor open: {open}. Ended before the debounce"
        );
        std::thread::sleep(Duration::from_secs_f64(EditorShared::STOP_SECONDS));
        h.step();
        assert!(!h.shared.lock().take.is_recording(), "editor open: {open}. The take ran on");
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
        BETWEEN_HOLDS.set(Some(Box::new(move |shared: &EditorLock, taken| {
            seen.set(Some((shared.try_lock().is_some(), taken.unanalyzed_audio())));
        })));
        h.step();
        BETWEEN_HOLDS.set(None);

        assert_eq!(between.get(), Some((true, true)), "the lock was held, or the audio analyzed");
        assert!(h.columns() > 0, "the step analyzed nothing");
    }

    /// A step that panics between its holds, before its FFTs or after them,
    /// leaves the next one analyzing again: with its columns dated where they
    /// belong rather than from the frame zero of the run the panic
    /// interrupted, and only its own columns given to the spectrum, none of
    /// those the panicked step made.
    #[test]
    fn a_panicking_step_leaves_the_next_one_analyzing() {
        // The columns 40 000 frames make from a fresh analysis.
        let fresh = {
            let mut h = harness();
            h.push_audio(40_000);
            h.step();
            h.columns()
        };
        for after_the_ffts in [false, true] {
            let mut h = harness();
            h.push_audio(20_000);
            h.step();
            let before = h.columns();
            assert!(before > 0, "the first step analyzed nothing");
            let newest =
                |h: &Harness| h.shared.lock().ui.picture.runtime.spectrum.newest_column().unwrap();
            let first = newest(&h);

            h.push_audio(20_000);
            if after_the_ffts {
                ANALYZED.set(Some(Box::new(|taken| {
                    assert!(!taken.columns().is_empty(), "the FFTs made no column to leave behind");
                    panic!("a step's bug");
                })));
            } else {
                BETWEEN_HOLDS.set(Some(Box::new(|_: &EditorLock, _| panic!("a step's bug"))));
            }
            let mut panicked = false;
            super::guarded_step(
                &h.shared,
                &h.editor_state,
                &mut h.tape,
                &mut h.input,
                &mut h.seen,
                None,
                &mut panicked,
            );
            BETWEEN_HOLDS.set(None);
            ANALYZED.set(None);
            assert!(panicked, "the hook did not run");
            assert_eq!(h.columns(), before, "the panicked step gave columns");

            h.push_audio(40_000);
            h.step();
            assert_eq!(
                h.columns() - before,
                fresh,
                "after the FFTs: {after_the_ffts}. The step after the panic gave other than \
                 its own columns",
            );
            // Both newest columns lag their audio's end by the same column lag,
            // and the audio ends 60 000 frames apart; the restarted run's hop
            // grid puts its newest column up to a hop (8 ms) short of that.
            let spacing = newest(&h) - first;
            assert!(
                (spacing - 60_000.0 / 48_000.0).abs() < 0.01,
                "after the FFTs: {after_the_ffts}. The newest columns are {spacing:.3} s apart \
                 where their audio is 1.25 s: the columns after the panic are dated from the \
                 interrupted run",
            );
        }
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
        h.step();
        h.push_audio(40_000);
        let ran = std::rc::Rc::new(std::cell::Cell::new(false));
        let between = ran.clone();
        BETWEEN_HOLDS.set(Some(Box::new(move |shared: &EditorLock, _| {
            shared.lock().ui.picture.appearance.spectrum.window = second;
            between.set(true);
        })));
        h.step();
        BETWEEN_HOLDS.set(None);
        assert!(ran.get(), "the hook did not run");
        assert!((h.analyzed_lag() - lag_of(first)).abs() < 1e-9, "analyzed under the edit");

        h.push_audio(40_000);
        h.step();
        assert!((h.analyzed_lag() - lag_of(second)).abs() < 1e-9, "the next step missed the edit");
    }

    /// [`SHUT_POLL`], the longest the worker sleeps, against the ring it is
    /// pacing itself to. A poll slower than the ring's span drops audio
    /// silently and permanently — the exact failure this module removes,
    /// reintroduced by its own pacing — and nothing about either constant's
    /// declaration hints at the other.
    #[test]
    fn the_ring_holds_many_polls_of_audio_at_the_projects_rates() {
        // The worst case within the use: the fastest rate the projects run at
        // against the most channels `Plugin::PORTS` declares.
        const FASTEST_RATE: f64 = 48_000.0;
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

        // The harness's input, which the thread then owns; the harness keeps
        // one nothing feeds.
        let unfed = LiveInput::new(
            harmonigraph_record::publication::channel().1,
            crate::audio_ingress::channel(1).1,
        );
        let input = std::mem::replace(&mut h.input, unfed);
        let analyzer =
            InputWorker::spawn(h.shared.clone(), h.editor_state.clone(), tape(), input, None, None);
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

    /// A processing stop publishes a session reset, which ends every live
    /// source's presses, a Tune's included, at its place in the stream; a
    /// restart's notes after it stay held and Learn hears them, even when
    /// one poll drains the stop, the restart and the new chord (#1948).
    #[test]
    fn a_stop_ends_what_was_held_and_a_restart_in_the_same_poll_keeps_its_notes() {
        use harmonigraph_core::canonical::NoteDelta;
        use harmonigraph_core::configuration::{ConfigEdit, TuningAxis};
        let params = Arc::new(crate::HarmonigraphParams::default());
        params.settings.submit(ConfigEdit::axis(TuningAxis::Three, 690_000_000));
        let mut h = harness();
        h.learn = Some(params.clone());
        // A Tune's fifth, held at the stop.
        let tune = SourceId(1);
        h.notes.observe_clock(0.0);
        for (sequence, note) in [(1, 60u8), (2, 67)] {
            let on = CoreNoteEvent::on(0.0, tune, 0, note, 1.0);
            let delta = NoteDelta {
                event: on,
                sequence,
                lifetime: sequence,
                pitch_microcents: Some(i64::from(note) * 100_000_000),
                assignment: None,
            };
            h.notes.note(delta).expect("the ring is sized for this");
        }
        h.step();
        params.settings.submit(ConfigEdit { learning: Some(true), ..Default::default() });
        // `Plugin::stop`, then `Plugin::reset` and the new run's fifth.
        h.push_kind(0.1, 0, NoteEventKind::SessionReset);
        h.push_kind(0.1, 0, NoteEventKind::SourceReset);
        h.push_note(0.1, 74);
        h.push_note(0.1, 81);
        h.step();
        let held = h.shared.lock().ui.picture.runtime.tracker.held_count();
        assert_eq!(held, 2, "the fifth held at the stop ended, and Learn cannot hear it");
        let three = params.settings.resolved().tuning.three;
        assert_eq!(three, 700_000_000, "the fifth struck after the restart is learned");
    }
}
