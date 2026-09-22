//! Writer-thread command pump, ordered lane draining, and file finalization.
//! File-backed fixtures live below this boundary so pass state stays private.

use super::*;
use configuration::RECORD_PASSES;
use harmonigraph_core::notes::NoteEvent;

#[cfg(all(test, feature = "test-support"))]
mod audio_tests;
#[cfg(all(test, feature = "test-support"))]
mod canonical_tests;
#[cfg(test)]
mod tests;

/// How long the writer thread sleeps when it finds the ring empty.
const DRAIN_IDLE: std::time::Duration = std::time::Duration::from_millis(20);

/// Build the ring and start the writer thread. Called once, from the
/// plugin's `Default`, so neither arming nor disarming ever has to touch
/// the audio thread's producer.
pub fn channel() -> (Recorder, Control) {
    let (producer, mut consumer) = rtrb::RingBuffer::new(TAKE_RING_CAPACITY);
    let (publication, mut publications) = publication::channel();
    let (display, display_consumer) = publication::channel();
    let (audio_producer, mut audio_consumer) = rtrb::RingBuffer::new(AUDIO_RING_CAPACITY);
    let (commands, orders) = mpsc::channel::<Command>();
    let dropped = Arc::new(AtomicU64::new(0));
    let recording = Arc::new(AtomicBool::new(false));
    let rolling = Arc::new(AtomicBool::new(false));
    let with_audio = Arc::new(AtomicBool::new(false));
    let end_at_rewind = Arc::new(AtomicBool::new(false));
    let latches = Arc::new(TakeLatches::default());
    let status = Arc::new(Mutex::new(String::new()));
    let last_take = Arc::new(Mutex::new(None));
    let progress = Arc::new(Progress::default());
    let render = Arc::new(RenderControl::default());

    let fence = Arc::new(RecordFence::default());
    let thread_fence = fence.clone();
    let thread_status = status.clone();
    let thread_last_take = last_take.clone();
    let thread_progress = progress.clone();
    let thread_render = render.clone();
    let _ = std::thread::Builder::new().name("harmonigraph-take-writer".into()).spawn(move || {
        let mut pump = Pump::default();
        loop {
            #[cfg(feature = "test-support")]
            thread_fence.worker_before_commands.reach();
            let mut waiting_for_start = false;
            #[cfg(all(test, feature = "test-support"))]
            let mut processed_stop = false;
            if !pump.disconnected {
            match orders.try_recv() {
                Ok(Command::Start(epoch, header, path, spec)) => {
                    if pump.pending_stop.is_some() {
                        thread_fence.fail();
                    } else {
                        pump.open =
                            Recording::create(*header, path, 1, spec, &thread_status).map(|mut open| {
                                #[cfg(all(test, feature = "test-support"))]
                                { open.fail_marker_on_pass = *thread_fence.test_marker_failure.lock(); }
                                open.epoch = epoch;
                                open.configuration_enabled = thread_fence.enabled.load(Ordering::Acquire);
                                open.source_enabled =
                                    thread_fence.canonical_enabled.load(Ordering::Acquire);
                                open
                            });
                        #[cfg(feature = "test-support")]
                        if let Some(audio) = pump.open.as_mut().and_then(|o| o.current.audio.as_mut())
                        {
                            if let Some(limit) = *thread_fence.test_wav_limit.lock() {
                                audio.limit_frames_for_test(limit);
                            }
                            if thread_fence.test_wav_finish_failure.load(Ordering::Acquire) {
                                audio.fail_finish_for_test();
                            }
                        }
                        if pump
                            .open
                            .as_ref()
                            .is_none_or(|o| spec.is_some() && o.current.audio.is_none())
                        {
                            thread_fence.fail_with_message(thread_status.lock().clone());
                            pump.failure.account(&mut pump.open, epoch, &thread_status, Some(&thread_fence), harmonigraph_take::IncompleteRecord {
                                reason: harmonigraph_take::canonical::GapReasonRecord::ProducerLost,
                                ..Default::default()
                            });
                        }
                    }
                }
                Ok(Command::Stop(epoch, render)) => {
                    #[cfg(all(test, feature = "test-support"))]
                    { processed_stop = true; }
                    pump.pending_stop = Some((epoch, render));
                    if !pump.failure.contains(epoch) {
                        *thread_status.lock() =
                            "finishing — waiting for the recording prefix".into();
                    }
                }
                Err(mpsc::TryRecvError::Empty) => {
                    // Start can arrive just after this poll and arm a producer
                    // before the drains below. With no file yet, its records
                    // and audio still belong to that pending command. An
                    // accounted failure can dispose its remaining entries.
                    waiting_for_start = pump.open.is_none()
                        && !pump.failure.contains(thread_fence.epoch());
                    #[cfg(feature = "test-support")]
                    {
                        thread_fence.worker_empty_visits.fetch_add(1, Ordering::AcqRel);
                        thread_fence.worker_after_empty.reach();
                    }
                }
                Err(mpsc::TryRecvError::Disconnected) => pump.disconnected = true,
            }
            }
            let pumped = pump.pass(
                &mut consumer,
                &mut audio_consumer,
                &mut publications,
                &thread_status,
                &thread_fence,
                waiting_for_start,
            );
            // Before the shutdown check below, so a take sealed by the same
            // pass that observed the disconnect is published and rendered
            // rather than going with the thread.
            if let Some((path, render)) = pumped.finished {
                *thread_last_take.lock() = Some(path.clone());
                if let Some(render) = render {
                    spawn_render(*render, path, thread_status.clone(),
                        thread_progress.clone(), thread_render.clone());
                }
            }
            #[cfg(all(test, feature = "test-support"))]
            if processed_stop {
                thread_fence.worker_stop_processed.store(true, Ordering::Release);
            }
            if pumped.shutdown {
                #[cfg(feature = "test-support")]
                thread_fence.worker_finished.store(true, Ordering::Release);
                return;
            }
            if !pumped.worked {
                std::thread::sleep(DRAIN_IDLE);
            }
        }
    });

    (
        Recorder {
            _writer_lifetime: Some(commands.clone()),
            publication,
            display,
            outage: publication::Lanes::default(),
            fence: fence.clone(),
            record_epoch: 0,
            record_pass: 1,
            closed_epoch: 0,
            producer,
            dropped: dropped.clone(),
            last_params: [f32::NAN; ParamKey::ALL.len()],
            lifecycle: State::Disarmed,
            history: History::default(),
            rolling: rolling.clone(),
            audio_started: false,
            audio: audio_producer,
            with_audio: with_audio.clone(),
            end_at_rewind: end_at_rewind.clone(),
            latches: latches.clone(),
        },
        Control {
            display: Arc::new(Mutex::new(Some(display_consumer))),
            fence,
            commands,
            dropped,
            status,
            last_take,
            recording,
            rolling,
            with_audio,
            end_at_rewind,
            latches,
            progress,
            render,
        },
    )
}

/// Everything one writer carries between passes of the pump: the file it is
/// writing, the two lanes' accounting, and the two terminal conditions.
///
/// One struct rather than five locals because [`Pump::pass`] is driven from two
/// places — the thread above and `testing::FileWriter`, which stands in for it
/// in every dependent crate's take coverage. Those two used to hold their own
/// copies of the drain/failure/stop sequence, and the copies had drifted apart:
/// the test one accounted a failure with no file open, never cleared
/// `pending_stop` or `finishing`, and had no disconnect tier at all, so four
/// files across two crates were testing a pump the plugin does not run (#895).
#[derive(Default)]
struct Pump {
    open: Option<Recording>,
    fanout: CanonicalFanout,
    failure: FailureAccount,
    /// The Stop whose prefix is not closed yet, and the render it asked for.
    pending_stop: Option<(u64, Option<Box<RenderRequest>>)>,
    /// No further command can arrive, so unresolved ownership is lost.
    disconnected: bool,
}

/// What one [`Pump::pass`] leaves for its driver to do.
#[derive(Default)]
struct Pumped {
    /// Either lane had something, so the writer must not sleep yet.
    worked: bool,
    /// A take sealed this pass: where it landed, and the render Stop carried.
    finished: Option<(std::path::PathBuf, Option<Box<RenderRequest>>)>,
    /// Both lanes are drained and no command can arrive: the thread returns.
    shutdown: bool,
}

impl Pump {
    /// One cross-lane pass: drain the record ring and the publication lane in
    /// order, then resolve failure, a ready Stop, and shutdown.
    ///
    /// `waiting_for_start` retains this iteration's records for a `Start` that
    /// may already have armed a producer — only the command half above can
    /// observe that, so it is passed in rather than recomputed here.
    fn pass(
        &mut self,
        consumer: &mut rtrb::Consumer<Entry>,
        audio: &mut rtrb::Consumer<f32>,
        publications: &mut publication::Consumer,
        status: &Mutex<String>,
        fence: &RecordFence,
        waiting_for_start: bool,
    ) -> Pumped {
        let mut pumped = Pumped::default();
        // Acquire idle ownership BEFORE draining: the callback's release
        // follows its last AudioSamples record. A later callback's RMW
        // sees disarmed, so it cannot extend this prefix behind the drain.
        if let Some(recording) = self.open.as_mut() {
            recording.observe_idle_producer(fence);
        }
        let fanout = &mut self.fanout;
        let failure = &self.failure;
        let had_records = !waiting_for_start
            && drain_with_boundaries(
                consumer,
                Some(audio),
                &mut self.open,
                status,
                Some(fence),
                failure,
                |open| {
                    fanout.drain(publications, open, fence, failure);
                },
            );
        let had_publications =
            self.fanout.drain(publications, &mut self.open, fence, &self.failure) != 0;
        pumped.worked = had_records || had_publications;
        #[cfg(feature = "test-support")]
        if self.pending_stop.is_some() {
            fence.worker_after_stop.reach();
        }

        // Failure is pending until both lanes, including the independent
        // loss snapshot, have delivered their retained prefix to its file.
        if fence.failed.load(Ordering::Acquire) {
            #[cfg(feature = "test-support")]
            fence.worker_before_retirement_check.reach();
            // Acquire the terminal ownership release BEFORE checking the
            // lanes again; the release may follow their last publication.
            if !fence.retirement_hold.load(Ordering::Acquire)
                && !self.failure.contains(fence.epoch())
                && consumer.is_empty()
                && publications.settled()
                && (self.open.is_some() || self.disconnected)
            {
                self.failure.account(
                    &mut self.open,
                    fence.epoch(),
                    status,
                    Some(fence),
                    harmonigraph_take::IncompleteRecord {
                        reason: harmonigraph_take::canonical::GapReasonRecord::ProducerLost,
                        ..Default::default()
                    },
                );
            }
            if self.failure.contains(fence.epoch()) {
                // Stop may arrive after the files were already accounted.
                // Its render request is disposed here on the worker, and
                // the failure status remains visible without another callback.
                self.pending_stop = None;
                fence.finishing.store(false, Ordering::Release);
                #[cfg(all(test, feature = "test-support"))]
                fence.worker_failure_accounted.store(true, Ordering::Release);
            }
        } else if self
            .pending_stop
            .as_ref()
            .is_some_and(|(epoch, _)| self.open.as_ref().is_some_and(|o| o.ready(*epoch)))
        {
            let (_, render) = self.pending_stop.take().unwrap();
            if let Some(path) = finish_ready(&mut self.open, fence.epoch(), fence) {
                fence.finishing.store(false, Ordering::Release);
                pumped.finished = Some((path, render));
            }
        }
        // Shutdown uses the same cross-lane pump and honors a now-ready
        // Stop first. Only ownership still unresolved after that is lost.
        if self.disconnected
            && !fence.retirement_hold.load(Ordering::Acquire)
            && consumer.is_empty()
        {
            if publications.settled() {
                if self.open.is_some() {
                    fence.fail();
                    self.failure.account(
                        &mut self.open,
                        fence.epoch(),
                        status,
                        Some(fence),
                        harmonigraph_take::IncompleteRecord {
                            reason: harmonigraph_take::canonical::GapReasonRecord::ProducerLost,
                            ..Default::default()
                        },
                    );
                    *status.lock() =
                        "recording incomplete: producer disconnected before finalization".into();
                }
                pumped.shutdown = true;
                return pumped;
            }
            if self.fanout.waiting_file {
                // No remaining command can materialize this addressed file.
                fence.fail();
                self.failure.account(
                    &mut self.open,
                    fence.epoch(),
                    status,
                    Some(fence),
                    harmonigraph_take::IncompleteRecord {
                        reason: harmonigraph_take::canonical::GapReasonRecord::InvalidRecord,
                        ..Default::default()
                    },
                );
            }
        }
        pumped
    }
}

/// In-memory endpoints for dependent crates that need to reach their real
/// [`Recorder`] wiring without starting this crate's file-writer thread.
/// Compiled only when a dev-dependency explicitly asks for it.
#[cfg(feature = "test-support")]
#[doc(hidden)]
pub mod testing;

/// Worker-owned recording disposal proof. A producer's failure flag requests
/// failure; only flushed file markers (or a reported I/O refusal) account it.
/// Recording cannot restart after failure until reload, so one exact epoch
/// suffices. This state grants no source-journal acknowledgement.
#[derive(Default)]
struct FailureAccount(std::cell::Cell<Option<u64>>);

impl FailureAccount {
    fn contains(&self, epoch: u64) -> bool {
        self.0.get() == Some(epoch)
    }

    fn account(
        &self,
        open: &mut Option<Recording>,
        epoch: u64,
        status: &Mutex<String>,
        fence: Option<&RecordFence>,
        record: harmonigraph_take::IncompleteRecord,
    ) {
        *status.lock() = CONFIGURATION_FAILURE.into();
        if let Some(recording) = open.as_mut() {
            if let Err(error) = recording.mark_incomplete(record) {
                if let Some(fence) = fence {
                    fence.fail_with_message(error.to_string());
                } else {
                    *status.lock() = format!("recording incomplete: {error}");
                }
            }
        }
        self.0.set(Some(epoch));
        *open = None;
    }
}

/// The take's half of publication. The display has its own lane straight off
/// the audio thread (#712 §4), so nothing here forwards, repairs or requests
/// anything on its behalf.
#[derive(Default)]
struct CanonicalFanout {
    waiting_file: bool,
    /// A gap that arrived with no file open, kept until one does.
    ///
    /// **The marker's lifetime is the recording, not the file:** it waits here
    /// until the recording has a file, is written into that one, and is then
    /// carried by [`Recording`] into every pass it opens afterwards. The
    /// two halves are what put it beyond a caller's memory — a gap cannot be
    /// dropped for arriving too early, and it cannot be left behind by a
    /// rollover, so the set of files a marked recording can export unmarked is
    /// empty.
    ///
    /// An unaddressed gap names no pass, so it deliberately waits for nothing
    /// and marks whatever is open. When NOTHING is open its marker used to be
    /// dropped where it stood, and since a gap no longer fails the fence that
    /// made the loss silent: an outage merging a disarmed route with an armed
    /// one keeps neither address, and a drain running concurrently with the
    /// audio thread can consume it before the worker polls the `Start` that
    /// would have given it a file. The take then sealed and exported with
    /// history missing and nothing saying so (#712).
    ///
    /// One record rather than a queue, coalesced over the widest serial range
    /// seen — the same shape the publisher's own outage takes, and for the
    /// same reason: the consumer clears its whole held set either way, so a
    /// second gap has nothing to add that the first has not already said.
    unplaced: Option<harmonigraph_take::IncompleteRecord>,
    /// Non-RT deduplication only. These cuts authorize no musical reclamation.
    cursors: std::collections::BTreeMap<SourceId, (u64, u64, u64)>,
}

impl CanonicalFanout {
    fn drain(
        &mut self,
        publications: &mut publication::Consumer,
        open: &mut Option<Recording>,
        fence: &RecordFence,
        failure: &FailureAccount,
    ) -> usize {
        use harmonigraph_core::canonical::CanonicalEvent;
        self.waiting_file = false;
        // The first file to open after an unplaceable gap carries its warning,
        // and from there the recording carries it — `Recording::next_pass`
        // writes it into every pass opened after this one, so the marker
        // survives a rollover the same way it survives having arrived too early.
        //
        // STATED RESIDUAL rather than a reconciler: that file need not be the
        // one whose history was lost. An outage entirely inside a disarmed
        // stretch marks the next take too, which warns about a hole that take
        // does not have. Conservative in the direction #712 asks for — the
        // export still runs, and the alternative is the silence this repairs.
        if let (Some(record), Some(recording)) = (self.unplaced, open.as_mut()) {
            if let Err(error) = recording.mark_incomplete(record) {
                fence.fail_with_message(error.to_string());
            }
            self.unplaced = None;
        }
        publications.drain(|delivery, route| {
            // A record can reach this lane before its independently queued
            // Start/NewPass control has drained. Retain its whole payload.
            let address = match delivery {
                publication::Delivery::PassComplete(a) => Some(a),
                publication::Delivery::EpochComplete(epoch) => {
                    Some(RecordAddress { epoch, pass: 0 })
                }
                // An unaddressed gap deliberately resolves to nothing and so
                // never waits for a file. It names no pass because its outage
                // spanned more than one, or because the caller could not route
                // it at all; the marker below still lands on whatever file is
                // open, which is the whole still-owned recording. Waiting
                // instead would stall the lane behind a file that may never
                // open, and a gap no longer fails the fence to unblock itself.
                publication::Delivery::Event(_) => route.address,
            };
            if let Some(address) = address {
                if !failure.contains(address.epoch)
                    && open.as_ref().is_none_or(|o| {
                        o.epoch < address.epoch
                            || (o.epoch == address.epoch && o.current.number < address.pass)
                    })
                {
                    self.waiting_file = true;
                    return false;
                }
            }
            match delivery {
                publication::Delivery::PassComplete(address) => {
                    if failure.contains(address.epoch) {
                        return true;
                    }
                    if let Some(recording) = open.as_mut() {
                        if let Some(pass) = recording.addressed(address) {
                            pass.source_complete = true;
                            if let Err(error) = recording.finish_completed_passes() {
                                fence.fail_with_message(error.to_string());
                            }
                        } else {
                            fence.fail();
                        }
                    } else {
                        fence.fail();
                    }
                }
                publication::Delivery::EpochComplete(epoch) => {
                    if failure.contains(epoch) {
                        return true;
                    }
                    if let Some(recording) = open.as_mut().filter(|o| o.epoch == epoch) {
                        recording.current.source_closed = true;
                    } else {
                        fence.fail();
                    }
                }
                publication::Delivery::Event(event) => {
                    match event {
                        CanonicalEvent::Note(delta) if delta.sequence != 0 => {
                            let cursor = self.cursors.entry(delta.event.source).or_default();
                            if delta.sequence <= cursor.0 {
                                return true;
                            }
                            if delta.sequence <= cursor.2 {
                                fence.fail();
                                return true;
                            }
                            cursor.0 = delta.sequence;
                        }
                        CanonicalEvent::Baseline(frame) => {
                            let cursor = self.cursors.entry(frame.source).or_default();
                            if frame.id <= cursor.1 {
                                return true;
                            }
                            if cursor.0 > frame.output_cut {
                                fence.fail();
                                return true;
                            }
                            cursor.1 = frame.id;
                            cursor.2 = frame.output_cut;
                        }
                        _ => {}
                    }
                    let mut record = harmonigraph_take::CanonicalRecord::from_event(event);
                    if let Some(address) = route.address.filter(|a| !failure.contains(a.epoch)) {
                        record.translate(route.time_offset);
                        if let Some(pass) = open.as_mut().and_then(|o| o.addressed(address)) {
                            if pass.source_complete {
                                fence.fail();
                            } else {
                                pass.voiced |= record.voiced();
                                if pass.writer.canonical(record).is_err() {
                                    fence.fail();
                                }
                            }
                        } else {
                            fence.fail();
                        }
                    }
                    // #712: missing note history marks the recording and lets
                    // the export warn. It does not fail the fence, so the take
                    // finalises, `last_take` moves on and Stop-and-render still
                    // reaches the renderer that was made permissive for exactly
                    // this file. Audio, ownership and I/O failures are untouched
                    // and still refuse.
                    if let CanonicalEvent::Gap(gap) = event {
                        let record = harmonigraph_take::IncompleteRecord {
                            first_publication: gap.first,
                            last_publication: gap.last,
                            reason: harmonigraph_take::canonical::GapRecord::from(gap).reason,
                        };
                        match open.as_mut() {
                            Some(recording) => {
                                if let Err(error) = recording.mark_incomplete(record) {
                                    fence.fail_with_message(error.to_string());
                                }
                            }
                            // Nowhere to write it YET. Held rather than
                            // dropped: see `unplaced`.
                            None => {
                                self.unplaced = Some(match self.unplaced {
                                    Some(held) => harmonigraph_take::IncompleteRecord {
                                        first_publication: held
                                            .first_publication
                                            .min(record.first_publication),
                                        last_publication: held
                                            .last_publication
                                            .max(record.last_publication),
                                        reason: held.reason,
                                    },
                                    None => record,
                                })
                            }
                        }
                    }
                }
            }
            true
        })
    }
}

/// Everything the RECORDING owns: the state that outlives any one of its
/// files, plus the pass currently being written.
///
/// **The split is the whole point.** Every field here crosses a pass boundary
/// and every field on [`Pass`] is reset by one, so [`Recording::next_pass`]
/// swaps `current` and carries the rest structurally. This used to be one
/// struct with seven recording-scoped fields re-assigned by hand at the
/// boundary and nothing checking the list was complete — `incomplete` failing
/// to cross it was #712's silent export bug, and the shape of that bug was
/// "the carry list is a list".
struct Recording {
    epoch: u64,
    /// Whether the configuration and source lanes publish for this recording,
    /// read off the fence once at `Start`. They decide whether a rolled-over
    /// pass is retained until its lane reports complete or finished on the
    /// spot, so they belong to the recording rather than to any file.
    configuration_enabled: bool,
    source_enabled: bool,
    /// The header every pass of this recording opens with. A pass's own copy
    /// names that file's WAV and alignment; this one names neither.
    header: harmonigraph_take::Header,
    /// The first pass's path; later passes append `-2`, `-3`, ...
    base: std::path::PathBuf,
    spec: Option<AudioSpec>,
    /// The marker this RECORDING carries, not this file: written into
    /// `current`, into everything in `retained`, and by [`Recording::next_pass`]
    /// into every pass opened from here on.
    incomplete: Option<harmonigraph_take::IncompleteRecord>,
    /// Test hook: fail the marker write on the pass with this number, once.
    /// Recording-scoped because that is how it is set — one value at `Start`,
    /// naming a pass this recording has usually not opened yet.
    #[cfg(all(test, feature = "test-support"))]
    fail_marker_on_pass: Option<u32>,
    /// The most recent pass BEFORE `current` that anything played in, and its
    /// number, so a run of unvoiced passes keeps pointing at the music.
    last_voiced: Option<std::path::PathBuf>,
    last_voiced_number: u32,
    /// Passes the transport has rolled past that a lane has not released yet.
    retained: Vec<Pass>,
    current: Pass,
}

/// One file of a recording: its two writers, and the state a rollover resets.
struct Pass {
    /// 1 for the first pass of the recording, 2 for the next, ...
    number: u32,
    path: std::path::PathBuf,
    writer: harmonigraph_take::Writer,
    /// The header THIS file was opened with, kept so the alignment rewrite
    /// below can supersede it without re-deriving the rest.
    header: harmonigraph_take::Header,
    /// The WAV recorded beside this pass, if audio was asked for.
    audio: Option<harmonigraph_take::WavWriter>,
    /// The three closed flags below are addressed to the EPOCH rather than to
    /// this pass, and are only ever read on `current` — but they reset with the
    /// file the way they did before the split, and nothing is lost by it: an
    /// epoch's closing records are the last the producer writes for it, so no
    /// `NewPass` can follow one and clear it.
    producer_closed: bool,
    configuration_closed: bool,
    source_closed: bool,
    /// These two are addressed to this pass by number, and a later pass's
    /// completion says nothing about this one's.
    configuration_complete: bool,
    source_complete: bool,
    /// Whether a note has STARTED in this pass. A pass without one draws an
    /// empty lattice however many parameter records it holds, so it is not a
    /// pass worth rendering; see [`Recording::finish`].
    voiced: bool,
    /// Whether this FILE already holds the recording's incomplete marker.
    marked: bool,
}

impl Recording {
    fn create(
        header: harmonigraph_take::Header,
        base: std::path::PathBuf,
        pass: u32,
        spec: Option<AudioSpec>,
        status: &Mutex<String>,
    ) -> Option<Recording> {
        let current = Pass::create(header.clone(), &base, pass, spec, status)?;
        Some(Recording {
            epoch: 0,
            configuration_enabled: false,
            source_enabled: false,
            header,
            base,
            spec,
            incomplete: None,
            #[cfg(all(test, feature = "test-support"))]
            fail_marker_on_pass: None,
            last_voiced: None,
            last_voiced_number: 0,
            retained: Vec::new(),
            current,
        })
    }

    /// Close both of the current pass's files and hand back the take to render.
    ///
    /// **That is the last VOICED pass, not simply the last one opened.** A take
    /// can end on a pass that holds parameter records and no notes — a host
    /// restoring the playhead when an audio export finishes lands as a backward
    /// jump, and a split rewrites every parameter into the pass it opens — and
    /// rendering that one produces a video of an empty lattice while the pass
    /// with the music sits unused beside it. An unvoiced tail is left on disk
    /// rather than deleted: it is evidence about what the host did, and it costs
    /// a few hundred bytes.
    fn finish(&mut self) -> std::io::Result<std::path::PathBuf> {
        let path = self.take_path();
        self.current.finish().map(|_| path)
    }

    /// Which of the current pass and its predecessors is the take: this one if
    /// anything played in it, else the last one where something did. A first
    /// pass with no notes has nothing to fall back to and stands as the (empty)
    /// take, which is what "you recorded nothing" looks like.
    fn take_path(&self) -> std::path::PathBuf {
        match &self.last_voiced {
            Some(previous) if !self.current.voiced => previous.clone(),
            _ => self.current.path.clone(),
        }
    }

    /// Open the next pass's files and make it `current`, retaining or closing
    /// the one it replaces.
    ///
    /// Nothing is copied across the boundary here: everything the new pass
    /// inherits is a field of `self` that neither moved nor was rewritten, so
    /// there is no list to keep in step with the struct.
    fn next_pass(&mut self, status: &Mutex<String>) -> std::io::Result<()> {
        if self.epoch != 0 && self.retained.len() + 1 >= RECORD_PASSES {
            return Err(std::io::Error::other("recording pass capacity exhausted"));
        }
        let number = self
            .current
            .number
            .checked_add(1)
            .ok_or_else(|| std::io::Error::other("recording pass number exhausted"))?;
        let Some(mut next) =
            Pass::create(self.header.clone(), &self.base, number, self.spec, status)
        else {
            return Err(std::io::Error::other(status.lock().clone()));
        };
        if self.spec.is_some() && next.audio.is_none() {
            return Err(std::io::Error::other(status.lock().clone()));
        }
        // Incompleteness belongs to the RECORDING, so the new file takes the
        // marker before it becomes the one being written into.
        //
        // A gap that named no pass covers whatever this recording still owns,
        // and that includes passes it has not opened yet: an outage spanning a
        // boundary is exactly the one that arrives address-less. Marking only
        // the passes that existed when it arrived left the take EXPORTABLE and
        // unmarked whenever a later pass was the voiced one, because
        // [`Recording::take_path`] picks the last voiced pass and
        // `mark_incomplete` reaches downward into `retained` rather than
        // forward in time (#712).
        if let Some(record) = self.incomplete {
            #[cfg(all(test, feature = "test-support"))]
            if self.fail_marker_on_pass == Some(next.number) {
                next.writer.make_read_only_for_test(&next.path)?;
                self.fail_marker_on_pass = None;
            }
            next.write_incomplete(record)?;
        }
        // Read before the swap, applied after it: a pass that cannot close is
        // not a pass this recording has rolled past.
        let voiced = self.current.voiced.then(|| (self.current.path.clone(), self.current.number));
        // Keep the old owner until the next file and its inherited marker exist.
        let mut previous = std::mem::replace(&mut self.current, next);
        if self.configuration_enabled || self.source_enabled {
            self.retained.push(previous);
        } else if let Err(error) = previous.finish() {
            self.current = previous;
            return Err(error);
        }
        if let Some((path, number)) = voiced {
            self.last_voiced = Some(path);
            self.last_voiced_number = number;
        }
        Ok(())
    }

    fn observe_idle_producer(&mut self, fence: &RecordFence) {
        if !self.configuration_enabled && fence.intent.load(Ordering::Acquire) == self.epoch << 1 {
            self.current.producer_closed = true;
        }
    }

    fn ready(&self, epoch: u64) -> bool {
        self.epoch == epoch
            && self.current.producer_closed
            && (!self.configuration_enabled || self.current.configuration_closed)
            && (!self.source_enabled || self.current.source_closed)
            && self.retained.is_empty()
    }

    fn finish_completed_passes(&mut self) -> std::io::Result<()> {
        let mut index = 0;
        while index < self.retained.len() {
            if (!self.configuration_enabled || self.retained[index].configuration_complete)
                && (!self.source_enabled || self.retained[index].source_complete)
            {
                self.retained[index].finish()?;
                let old = self.retained.remove(index);
                if old.voiced && old.number > self.last_voiced_number {
                    self.last_voiced = Some(old.path);
                    self.last_voiced_number = old.number;
                }
            } else {
                index += 1;
            }
        }
        Ok(())
    }

    /// Mark this recording — the current pass, every pass it still retains, and
    /// by [`Recording::next_pass`] every pass it opens from here on.
    ///
    /// **The FIRST gap is the one the file names**, and a later one adds
    /// nothing: the marker says this take has a hole, and the reader has to
    /// name the same hole the writer did. `Take::parse` reads it back under the
    /// same rule, which it did not before (#895). A gap that arrives second is
    /// still written into its pass as an ordinary `Gap` record, so nothing
    /// about the hole is lost — only the marker is singular.
    fn mark_incomplete(
        &mut self,
        record: harmonigraph_take::IncompleteRecord,
    ) -> std::io::Result<()> {
        let mut result = Ok(());
        if self.incomplete.is_none() {
            #[cfg(all(test, feature = "test-support"))]
            if self.fail_marker_on_pass == Some(self.current.number) {
                self.current.writer.make_read_only_for_test(&self.current.path)?;
                self.fail_marker_on_pass = None;
            }
            result = self.current.write_incomplete(record);
            if result.is_ok() {
                self.incomplete = Some(record);
            }
        }
        for pass in &mut self.retained {
            #[cfg(all(test, feature = "test-support"))]
            if self.fail_marker_on_pass == Some(pass.number) {
                pass.writer.make_read_only_for_test(&pass.path)?;
                self.fail_marker_on_pass = None;
            }
            // Visit every retained pass, keeping the first useful I/O cause.
            result = result.and(pass.write_incomplete(record));
        }
        result
    }

    fn addressed(&mut self, address: RecordAddress) -> Option<&mut Pass> {
        if self.epoch != address.epoch {
            return None;
        }
        if self.current.number == address.pass {
            return Some(&mut self.current);
        }
        self.retained.iter_mut().find(|pass| pass.number == address.pass)
    }
}

impl Pass {
    fn create(
        mut header: harmonigraph_take::Header,
        base: &std::path::Path,
        number: u32,
        spec: Option<AudioSpec>,
        status: &Mutex<String>,
    ) -> Option<Pass> {
        let path = Self::path_for(base, number);

        // The WAV opens first, so its name can go in the take's header —
        // which is the take's first line and cannot be revised later.
        let audio = spec.and_then(|spec| {
            let wav = path.with_extension("wav");
            match harmonigraph_take::WavWriter::create(&wav, spec.sample_rate, spec.channels) {
                Ok(writer) => {
                    header.audio_file = wav.file_name().and_then(|n| n.to_str()).map(str::to_owned);
                    Some(writer)
                }
                Err(err) => {
                    *status.lock() = format!("cannot write {}: {err}", wav.display());
                    None
                }
            }
        });

        match harmonigraph_take::Writer::create(&path, &header) {
            Ok(writer) => {
                if spec.is_none() || audio.is_some() {
                    *status.lock() = if number <= 1 {
                        format!("recording to {}", path.display())
                    } else {
                        format!("pass {number} -> {}", path.display())
                    };
                }
                Some(Pass {
                    number,
                    path,
                    writer,
                    header,
                    audio,
                    producer_closed: false,
                    configuration_closed: false,
                    source_closed: false,
                    configuration_complete: false,
                    source_complete: false,
                    voiced: false,
                    marked: false,
                })
            }
            Err(err) => {
                *status.lock() = format!("cannot write {}: {err}", path.display());
                None
            }
        }
    }

    fn path_for(base: &std::path::Path, pass: u32) -> std::path::PathBuf {
        if pass <= 1 {
            base.to_path_buf()
        } else {
            let stem = base.file_stem().and_then(|s| s.to_str()).unwrap_or("take");
            base.with_file_name(format!("{stem}-{pass}.{}", harmonigraph_take::EXTENSION))
        }
    }

    /// Close this pass's two files.
    fn finish(&mut self) -> std::io::Result<()> {
        // Attempt both even if one fails. Keep this owner available so the
        // caller can write its incomplete marker before retiring it.
        let notes = self.writer.flush().map_err(|error| {
            std::io::Error::new(
                error.kind(),
                format!("cannot finalize {}: {error}", self.path.display()),
            )
        });
        let audio = self.finish_audio();
        notes.and(audio)
    }

    fn finish_audio(&mut self) -> std::io::Result<()> {
        self.audio.take().map_or(Ok(()), |audio| {
            audio.finish().map_err(|error| {
                std::io::Error::new(
                    error.kind(),
                    format!(
                        "cannot finalize {}: {error}",
                        self.path.with_extension("wav").display()
                    ),
                )
            })
        })
    }

    fn write_audio(&mut self, first: &[f32], second: &[f32]) -> std::io::Result<()> {
        let Some(audio) = self.audio.as_mut() else { return Ok(()) };
        if let Err(error) = audio.write(first).and_then(|_| audio.write(second)) {
            let path = self.path.with_extension("wav");
            let repair = self.finish_audio();
            let repair = repair.err().map(|error| format!("; {error}")).unwrap_or_default();
            return Err(std::io::Error::new(
                error.kind(),
                format!("cannot write {}: {error}{repair}", path.display()),
            ));
        }
        Ok(())
    }

    /// Write the recording's marker into THIS file, once.
    fn write_incomplete(
        &mut self,
        record: harmonigraph_take::IncompleteRecord,
    ) -> std::io::Result<()> {
        if self.marked {
            return Ok(());
        }
        let result =
            self.writer.incomplete(record).and_then(|_| self.writer.flush()).map_err(|error| {
                std::io::Error::new(
                    error.kind(),
                    format!("cannot write incomplete marker {}: {error}", self.path.display()),
                )
            });
        if result.is_ok() {
            self.marked = true;
        }
        result
    }
}

fn finish_ready(
    open: &mut Option<Recording>,
    epoch: u64,
    fence: &RecordFence,
) -> Option<std::path::PathBuf> {
    if !open.as_ref().is_some_and(|o| o.ready(epoch)) {
        return None;
    }
    finish_open(open, fence)
}

fn finish_open(open: &mut Option<Recording>, fence: &RecordFence) -> Option<std::path::PathBuf> {
    match open.as_mut()?.finish() {
        Ok(path) => {
            *open = None;
            Some(path)
        }
        Err(error) => {
            fence.fail_with_message(error.to_string());
            None
        }
    }
}

/// Move everything queued into the writer (discarding it if none is
/// open). Returns whether anything was there.
#[cfg(test)]
fn drain(
    consumer: &mut rtrb::Consumer<Entry>,
    open: &mut Option<Recording>,
    status: &Mutex<String>,
) -> bool {
    drain_with_audio(consumer, None, open, status, None)
}

#[cfg(test)]
fn drain_with_audio(
    consumer: &mut rtrb::Consumer<Entry>,
    audio: Option<&mut rtrb::Consumer<f32>>,
    open: &mut Option<Recording>,
    status: &Mutex<String>,
    fence: Option<&RecordFence>,
) -> bool {
    drain_with_boundaries(consumer, audio, open, status, fence, &FailureAccount::default(), |_| {})
}

fn drain_with_boundaries(
    consumer: &mut rtrb::Consumer<Entry>,
    mut audio: Option<&mut rtrb::Consumer<f32>>,
    open: &mut Option<Recording>,
    status: &Mutex<String>,
    fence: Option<&RecordFence>,
    failure: &FailureAccount,
    mut before_new_pass: impl FnMut(&mut Option<Recording>),
) -> bool {
    // Start is a separate off-thread message, published before arming. Retain
    // records if this iteration observed the ring before that command.
    if open.is_none()
        && fence.is_some_and(|f| f.enabled.load(Ordering::Acquire) && !failure.contains(f.epoch()))
    {
        return false;
    }
    let fail = || {
        if let Some(fence) = fence {
            fence.fail();
        }
        *status.lock() = "recording incomplete: missing or exhausted pass ownership".into();
    };
    let mut any = false;
    while let Ok(entry) = consumer.pop() {
        any = true;
        if matches!(entry, Entry::NewPass) {
            if fence.is_some_and(|f| f.failed.load(Ordering::Acquire)) {
                continue;
            }
            // A completed source cut in the other lane can release a pass
            // before this allocation. Queue ordering alone is not exhaustion.
            before_new_pass(open);
            if let Some(error) = open.as_mut().and_then(|r| r.next_pass(status).err()) {
                if let Some(fence) = fence {
                    fence.fail_with_message(error.to_string());
                }
                *status.lock() = error.to_string();
                let epoch = open.as_ref().map_or(0, |o| o.epoch);
                failure.account(
                    open,
                    epoch,
                    status,
                    fence,
                    harmonigraph_take::IncompleteRecord {
                        reason: harmonigraph_take::canonical::GapReasonRecord::InvalidRecord,
                        ..Default::default()
                    },
                );
            }
            continue;
        }
        if fence.is_some_and(|f| failure.contains(f.epoch()))
            && !matches!(entry, Entry::AudioSamples(_))
        {
            continue;
        }
        match entry {
            Entry::ConfigurationAt { address, config } => {
                match open.as_mut().and_then(|o| o.addressed(address)) {
                    Some(pass) if !pass.configuration_complete => {
                        if pass.writer.configuration(config).is_err() {
                            fail();
                        }
                    }
                    _ => fail(),
                }
                continue;
            }
            Entry::ConfigurationPassComplete(address) => {
                if let Some(recording) = open.as_mut().filter(|o| o.epoch == address.epoch) {
                    match recording.addressed(address) {
                        Some(pass) => pass.configuration_complete = true,
                        None => fail(),
                    }
                    if let Err(error) = recording.finish_completed_passes() {
                        if let Some(fence) = fence {
                            fence.fail_with_message(error.to_string());
                        }
                        *status.lock() = error.to_string();
                    }
                } else {
                    fail();
                }
                continue;
            }
            Entry::ProducerClosed(epoch) | Entry::ConfigurationEpochComplete(epoch) => {
                if let Some(recording) = open.as_mut().filter(|o| o.epoch == epoch) {
                    if matches!(entry, Entry::ProducerClosed(_)) {
                        recording.current.producer_closed = true;
                    } else {
                        recording.current.configuration_closed = true;
                    }
                } else {
                    fail();
                }
                continue;
            }
            Entry::AudioSamples(count) => {
                if let Some(consumer) = audio.as_deref_mut() {
                    if consumer.slots() < count {
                        fail();
                        continue;
                    }
                    if let Ok(chunk) = consumer.read_chunk(count) {
                        if let Some(recording) = open.as_mut() {
                            let (first, second) = chunk.as_slices();
                            if let Err(error) = recording.current.write_audio(first, second) {
                                if let Some(fence) = fence {
                                    fence.fail_with_message(error.to_string());
                                }
                                *status.lock() = error.to_string();
                            }
                        }
                        chunk.commit_all();
                    } else {
                        fail();
                    }
                } else {
                    fail();
                }
                continue;
            }
            _ => {}
        }
        if let Entry::AudioStart(t) = entry {
            // Rewrite the header now that the WAV's alignment is known.
            // The format is line-oriented and the reader takes the LAST
            // Header record, so a corrected one simply supersedes the
            // first — no seeking, no fixed-width fields.
            if let Some(pass) = open.as_mut().map(|recording| &mut recording.current) {
                pass.header.audio_start = Some(t);
                let header = pass.header.clone();
                if pass.writer.write(&harmonigraph_take::Record::Header(header)).is_err() {
                    fail();
                }
            }
            continue;
        }
        let Some(pass) = open.as_mut().map(|recording| &mut recording.current) else { continue };
        // A note starting is what makes this pass the one worth rendering.
        if matches!(entry, Entry::Note { kind: NoteEventKind::On { .. }, .. }) {
            pass.voiced = true;
        }
        let writer = &mut pass.writer;
        let result = match entry {
            Entry::Note { t, source, channel, note, kind } => {
                writer.note(NoteEvent { time: t, source, channel, note, kind }.into())
            }
            Entry::Param { t, key, value } => writer.param(harmonigraph_take::ParamRecord {
                t,
                id: ParamKey::ALL[key].id().to_string(),
                value,
            }),
            // Both handled above; the writer never sees them.
            Entry::NewPass
            | Entry::AudioStart(_)
            | Entry::ConfigurationAt { .. }
            | Entry::ConfigurationPassComplete(_)
            | Entry::ConfigurationEpochComplete(_)
            | Entry::ProducerClosed(_)
            | Entry::AudioSamples(_) => Ok(()),
        };
        if result.is_err() {
            fail();
        }
    }
    any
}
