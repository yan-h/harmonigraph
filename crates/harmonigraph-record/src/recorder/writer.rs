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
    let rolled = Arc::new(AtomicBool::new(false));
    let captured = Arc::new(AtomicU64::new(0));
    let hit_rewind = Arc::new(AtomicBool::new(false));
    let stop_at_bar = Arc::new(StopAtBar::default());
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
        let mut open: Option<Open> = None;
        let mut pending_stop = None;
        let mut fanout = CanonicalFanout::default();
        let failure = FailureAccount::default();
        let mut disconnected = false;
        loop {
            #[cfg(feature = "test-support")]
            thread_fence.worker_before_commands.reach();
            let mut waiting_for_start = false;
            #[cfg(all(test, feature = "test-support"))]
            let mut processed_stop = false;
            if !disconnected {
            match orders.try_recv() {
                Ok(Command::Start(epoch, header, path, spec)) => {
                    if pending_stop.is_some() {
                        thread_fence.fail();
                    } else {
                        open =
                            Open::create(*header, path, 1, spec, &thread_status).map(|mut open| {
                                #[cfg(all(test, feature = "test-support"))]
                                { open.fail_marker_on_pass = *thread_fence.test_marker_failure.lock(); }
                                open.epoch = epoch;
                                open.configuration_enabled = thread_fence.enabled.load(Ordering::Acquire);
                                open.source_enabled =
                                    thread_fence.canonical_enabled.load(Ordering::Acquire);
                                open
                            });
                        #[cfg(feature = "test-support")]
                        if let Some(audio) = open.as_mut().and_then(|o| o.audio.as_mut()) {
                            if let Some(limit) = *thread_fence.test_wav_limit.lock() {
                                audio.limit_frames_for_test(limit);
                            }
                            if thread_fence.test_wav_finish_failure.load(Ordering::Acquire) {
                                audio.fail_finish_for_test();
                            }
                        }
                        if open.as_ref().is_none_or(|o| spec.is_some() && o.audio.is_none()) {
                            thread_fence.fail_with_message(thread_status.lock().clone());
                            failure.account(&mut open, epoch, &thread_status, Some(&thread_fence), harmonigraph_take::IncompleteRecord {
                                reason: harmonigraph_take::canonical::GapReasonRecord::ProducerLost,
                                ..Default::default()
                            });
                        }
                    }
                }
                Ok(Command::Stop(epoch, render)) => {
                    #[cfg(all(test, feature = "test-support"))]
                    { processed_stop = true; }
                    pending_stop = Some((epoch, render));
                    if !failure.contains(epoch) {
                        *thread_status.lock() =
                            "finishing — waiting for the recording prefix".into();
                    }
                }
                Err(mpsc::TryRecvError::Empty) => {
                    // Start can arrive just after this poll and arm a producer
                    // before the drains below. With no file yet, its records
                    // and audio still belong to that pending command. An
                    // accounted failure can dispose its remaining entries.
                    waiting_for_start = open.is_none()
                        && !failure.contains(thread_fence.epoch());
                    #[cfg(feature = "test-support")]
                    {
                        thread_fence.worker_empty_visits.fetch_add(1, Ordering::AcqRel);
                        thread_fence.worker_after_empty.reach();
                    }
                }
                Err(mpsc::TryRecvError::Disconnected) => disconnected = true,
            }
            }
            // Acquire idle ownership BEFORE draining: the callback's release
            // follows its last AudioSamples record. A later callback's RMW
            // sees disarmed, so it cannot extend this prefix behind the drain.
            if let Some(current) = open.as_mut() {
                current.observe_idle_producer(&thread_fence);
            }
            let had_records = !waiting_for_start && drain_with_boundaries(
                &mut consumer, Some(&mut audio_consumer), &mut open,
                &thread_status, Some(&thread_fence), &failure,
                |open| {
                    fanout.drain(&mut publications, open, &thread_fence, &failure);
                },
            );
            let had_publications =
                fanout.drain(&mut publications, &mut open, &thread_fence, &failure) != 0;
            #[cfg(feature = "test-support")]
            if pending_stop.is_some() { thread_fence.worker_after_stop.reach(); }

            // Failure is pending until both lanes, including the independent
            // loss snapshot, have delivered their retained prefix to its file.
            if thread_fence.failed.load(Ordering::Acquire) {
                #[cfg(feature = "test-support")]
                thread_fence.worker_before_retirement_check.reach();
                // Acquire the terminal ownership release BEFORE checking the
                // lanes again; the release may follow their last publication.
                if !thread_fence.retirement_hold.load(Ordering::Acquire)
                    && !failure.contains(thread_fence.epoch())
                    && consumer.is_empty() && publications.settled()
                    && (open.is_some() || disconnected)
                {
                    failure.account(&mut open, thread_fence.epoch(), &thread_status, Some(&thread_fence),
                        harmonigraph_take::IncompleteRecord {
                            reason: harmonigraph_take::canonical::GapReasonRecord::ProducerLost,
                            ..Default::default()
                        });
                }
                if failure.contains(thread_fence.epoch()) {
                    // Stop may arrive after the files were already accounted.
                    // Its render request is disposed here on the worker, and
                    // the failure status remains visible without another callback.
                    pending_stop = None;
                    thread_fence.finishing.store(false, Ordering::Release);
                    #[cfg(all(test, feature = "test-support"))]
                    thread_fence.worker_failure_accounted.store(true, Ordering::Release);
                }
            } else if pending_stop.as_ref()
                .is_some_and(|(epoch, _)| open.as_ref().is_some_and(|o| o.ready(*epoch)))
            {
                let (_, render) = pending_stop.take().unwrap();
                if let Some(path) = finish_ready(&mut open, thread_fence.epoch(), &thread_fence) {
                    *thread_last_take.lock() = Some(path.clone());
                    thread_fence.finishing.store(false, Ordering::Release);
                    if let Some(render) = render {
                        spawn_render(*render, path, thread_status.clone(),
                            thread_progress.clone(), thread_render.clone());
                    }
                }
            }
            #[cfg(all(test, feature = "test-support"))]
            if processed_stop {
                thread_fence.worker_stop_processed.store(true, Ordering::Release);
            }
            // Shutdown uses the same cross-lane pump and honors a now-ready
            // Stop first. Only ownership still unresolved after that is lost.
            if disconnected && !thread_fence.retirement_hold.load(Ordering::Acquire)
                && consumer.is_empty() {
                if publications.settled() {
                    if open.is_some() {
                        thread_fence.fail();
                        failure.account(&mut open, thread_fence.epoch(), &thread_status, Some(&thread_fence),
                            harmonigraph_take::IncompleteRecord {
                                reason: harmonigraph_take::canonical::GapReasonRecord::ProducerLost,
                                ..Default::default()
                            });
                        *thread_status.lock() = "recording incomplete: producer disconnected before finalization".into();
                    }
                    #[cfg(feature = "test-support")]
                    thread_fence.worker_finished.store(true, Ordering::Release);
                    return;
                }
                if fanout.waiting_file {
                    // No remaining command can materialize this addressed file.
                    thread_fence.fail();
                    failure.account(&mut open, thread_fence.epoch(), &thread_status, Some(&thread_fence),
                        harmonigraph_take::IncompleteRecord {
                            reason: harmonigraph_take::canonical::GapReasonRecord::InvalidRecord,
                            ..Default::default()
                        });
                }
            }
            if !had_records && !had_publications {
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
            last_configuration: None,
            producer,
            dropped: dropped.clone(),
            last_params: [f32::NAN; ParamKey::ALL.len()],
            was_armed: false,
            last_position: None,
            rolling: rolling.clone(),
            audio_started: false,
            audio: audio_producer,
            with_audio: with_audio.clone(),
            end_at_rewind: end_at_rewind.clone(),
            captured: captured.clone(),
            hit_rewind: hit_rewind.clone(),
            stop_at_bar: stop_at_bar.clone(),
            last_bar: None,
            finished: false,
            advanced: false,
            pending_split: false,
            rolled: rolled.clone(),
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
            rolled,
            captured,
            hit_rewind,
            stop_at_bar,
            progress,
            render,
        },
    )
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
        open: &mut Option<Open>,
        epoch: u64,
        status: &Mutex<String>,
        fence: Option<&RecordFence>,
        record: harmonigraph_take::IncompleteRecord,
    ) {
        *status.lock() = CONFIGURATION_FAILURE.into();
        if let Some(current) = open.as_mut() {
            if let Err(error) = current.mark_incomplete(record) {
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
    /// carried by [`Open`] into every pass the recording opens afterwards. The
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
        open: &mut Option<Open>,
        fence: &RecordFence,
        failure: &FailureAccount,
    ) -> usize {
        use harmonigraph_core::canonical::CanonicalEvent;
        self.waiting_file = false;
        // The first file to open after an unplaceable gap carries its warning,
        // and from there the recording carries it — `Open::next_pass` writes it
        // into every pass opened after this one, so the marker survives a
        // rollover the same way it survives having arrived too early.
        //
        // STATED RESIDUAL rather than a reconciler: that file need not be the
        // one whose history was lost. An outage entirely inside a disarmed
        // stretch marks the next take too, which warns about a hole that take
        // does not have. Conservative in the direction #712 asks for — the
        // export still runs, and the alternative is the silence this repairs.
        if let (Some(record), Some(current)) = (self.unplaced, open.as_mut()) {
            if let Err(error) = current.mark_incomplete(record) {
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
                            || (o.epoch == address.epoch && o.pass < address.pass)
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
                    if let Some(current) = open.as_mut() {
                        if let Some(pass) = current.addressed(address) {
                            pass.source_complete = true;
                            if let Err(error) = current.finish_completed_passes() {
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
                    if let Some(current) = open.as_mut().filter(|o| o.epoch == epoch) {
                        current.source_closed = true;
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
                            Some(current) => {
                                if let Err(error) = current.mark_incomplete(record) {
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

/// The file currently being written, and what it takes to open the next
/// one when the transport loops.
struct Open {
    epoch: u64,
    retained: Vec<Open>,
    producer_closed: bool,
    configuration_enabled: bool,
    configuration_closed: bool,
    configuration_complete: bool,
    source_enabled: bool,
    source_closed: bool,
    source_complete: bool,
    last_voiced_number: u32,
    /// The marker this RECORDING carries, not this file: whatever was written
    /// into this pass, so [`Open::next_pass`] can write it into the next one.
    incomplete: Option<harmonigraph_take::IncompleteRecord>,
    #[cfg(all(test, feature = "test-support"))]
    fail_marker_on_pass: Option<u32>,
    writer: harmonigraph_take::Writer,
    header: harmonigraph_take::Header,
    /// The first pass's path; later passes append `-2`, `-3`, ...
    base: std::path::PathBuf,
    pass: u32,
    /// The WAV recorded beside this pass, if audio was asked for.
    audio: Option<harmonigraph_take::WavWriter>,
    spec: Option<AudioSpec>,
    /// Whether a note has STARTED in this pass. A pass without one draws an
    /// empty lattice however many parameter records it holds, so it is not a
    /// pass worth rendering; see [`Open::finish`].
    voiced: bool,
    /// The most recent earlier pass that was voiced, to fall back to.
    last_voiced: Option<std::path::PathBuf>,
}

impl Open {
    fn create(
        mut header: harmonigraph_take::Header,
        base: std::path::PathBuf,
        pass: u32,
        spec: Option<AudioSpec>,
        status: &Mutex<String>,
    ) -> Option<Open> {
        let path = Self::path_for(&base, pass);

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
                    *status.lock() = if pass <= 1 {
                        format!("recording to {}", path.display())
                    } else {
                        format!("pass {pass} -> {}", path.display())
                    };
                }
                Some(Open {
                    epoch: 0,
                    retained: Vec::new(),
                    producer_closed: false,
                    configuration_enabled: false,
                    configuration_closed: false,
                    configuration_complete: false,
                    source_enabled: false,
                    source_closed: false,
                    source_complete: false,
                    last_voiced_number: 0,
                    incomplete: None,
                    #[cfg(all(test, feature = "test-support"))]
                    fail_marker_on_pass: None,
                    writer,
                    header,
                    base,
                    pass,
                    audio,
                    spec,
                    voiced: false,
                    last_voiced: None,
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

    /// Close both files and hand back the take to render.
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
        // Attempt both even if one fails. Keep this owner available so the
        // caller can write its incomplete marker before retiring it.
        let notes = self.writer.flush().map_err(|error| {
            std::io::Error::new(
                error.kind(),
                format!("cannot finalize {}: {error}", self.path().display()),
            )
        });
        let audio = self.finish_audio();
        notes.and(audio).map(|_| path)
    }

    fn finish_audio(&mut self) -> std::io::Result<()> {
        self.audio.take().map_or(Ok(()), |audio| {
            audio.finish().map_err(|error| {
                std::io::Error::new(
                    error.kind(),
                    format!(
                        "cannot finalize {}: {error}",
                        self.path().with_extension("wav").display()
                    ),
                )
            })
        })
    }

    fn write_audio(&mut self, first: &[f32], second: &[f32]) -> std::io::Result<()> {
        let Some(audio) = self.audio.as_mut() else { return Ok(()) };
        if let Err(error) = audio.write(first).and_then(|_| audio.write(second)) {
            let path = self.path().with_extension("wav");
            let repair = self.finish_audio();
            let repair = repair.err().map(|error| format!("; {error}")).unwrap_or_default();
            return Err(std::io::Error::new(
                error.kind(),
                format!("cannot write {}: {error}{repair}", path.display()),
            ));
        }
        Ok(())
    }

    /// Which of this pass and its predecessors is the take: this one if anything
    /// played in it, else the last one where something did. A first pass with no
    /// notes has nothing to fall back to and stands as the (empty) take, which
    /// is what "you recorded nothing" looks like.
    fn take_path(&self) -> std::path::PathBuf {
        match &self.last_voiced {
            Some(previous) if !self.voiced => previous.clone(),
            _ => self.path(),
        }
    }

    /// The file this pass is writing to.
    fn path(&self) -> std::path::PathBuf {
        Self::path_for(&self.base, self.pass)
    }

    /// Close this pass's files and open the next pass's.
    fn next_pass(open: &mut Option<Open>, status: &Mutex<String>) -> std::io::Result<()> {
        let Some(current) = open.as_ref() else { return Ok(()) };
        if current.epoch != 0 && current.retained.len() + 1 >= RECORD_PASSES {
            return Err(std::io::Error::other("recording pass capacity exhausted"));
        }
        let pass = current
            .pass
            .checked_add(1)
            .ok_or_else(|| std::io::Error::other("recording pass number exhausted"))?;
        let mut header = current.header.clone();
        header.audio_start = None;
        let Some(mut next) = Open::create(header, current.base.clone(), pass, current.spec, status)
        else {
            return Err(std::io::Error::other(status.lock().clone()));
        };
        if current.spec.is_some() && next.audio.is_none() {
            return Err(std::io::Error::other(status.lock().clone()));
        }
        #[cfg(all(test, feature = "test-support"))]
        {
            next.fail_marker_on_pass = current.fail_marker_on_pass;
        }
        next.last_voiced = current.voiced_so_far();
        // Incompleteness belongs to the RECORDING, so it crosses the rollover
        // with everything else the new pass inherits.
        //
        // A gap that named no pass covers whatever this recording still owns,
        // and that includes passes it has not opened yet: an outage spanning a
        // boundary is exactly the one that arrives address-less. Marking only
        // the passes that existed when it arrived left the take EXPORTABLE and
        // unmarked whenever a later pass was the voiced one, because
        // [`Open::take_path`] picks the last voiced pass and `mark_incomplete`
        // reaches downward into `retained` rather than forward in time (#712).
        if let Some(record) = current.incomplete {
            next.mark_incomplete(record)?;
        }
        // Keep the old owner until the next file and its inherited marker exist.
        let mut previous = open.take().unwrap();
        next.epoch = previous.epoch;
        next.configuration_enabled = previous.configuration_enabled;
        next.source_enabled = previous.source_enabled;
        if previous.configuration_enabled || previous.source_enabled {
            next.last_voiced_number =
                if previous.voiced { previous.pass } else { previous.last_voiced_number };
            next.retained = std::mem::take(&mut previous.retained);
            next.retained.push(previous);
        } else if let Err(error) = previous.finish() {
            *open = Some(previous);
            return Err(error);
        }
        *open = Some(next);
        Ok(())
    }

    fn observe_idle_producer(&mut self, fence: &RecordFence) {
        if !self.configuration_enabled && fence.intent.load(Ordering::Acquire) == self.epoch << 1 {
            self.producer_closed = true;
        }
    }

    fn ready(&self, epoch: u64) -> bool {
        self.epoch == epoch
            && self.producer_closed
            && (!self.configuration_enabled || self.configuration_closed)
            && (!self.source_enabled || self.source_closed)
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
                if old.voiced && old.pass > self.last_voiced_number {
                    self.last_voiced = Some(old.path());
                    self.last_voiced_number = old.pass;
                }
            } else {
                index += 1;
            }
        }
        Ok(())
    }

    /// Mark this recording — this pass, every pass it still retains, and by
    /// [`Open::next_pass`] every pass it opens from here on.
    fn mark_incomplete(
        &mut self,
        record: harmonigraph_take::IncompleteRecord,
    ) -> std::io::Result<()> {
        let mut result = Ok(());
        if self.incomplete.is_none() {
            #[cfg(all(test, feature = "test-support"))]
            if self.fail_marker_on_pass == Some(self.pass) {
                self.writer.make_read_only_for_test(self.path())?;
                self.fail_marker_on_pass = None;
            }
            result =
                self.writer.incomplete(record).and_then(|_| self.writer.flush()).map_err(|error| {
                    std::io::Error::new(
                        error.kind(),
                        format!(
                            "cannot write incomplete marker {}: {error}",
                            self.path().display()
                        ),
                    )
                });
            if result.is_ok() {
                self.incomplete = Some(record);
            }
        }
        for pass in &mut self.retained {
            // Visit every retained pass, keeping the first useful I/O cause.
            result = result.and(pass.mark_incomplete(record));
        }
        result
    }
    fn addressed(&mut self, address: RecordAddress) -> Option<&mut Open> {
        if self.epoch != address.epoch {
            return None;
        }
        if self.pass == address.pass {
            return Some(self);
        }
        self.retained.iter_mut().find(|pass| pass.pass == address.pass)
    }

    /// The latest pass up to and including this one that anything played in, so
    /// a run of unvoiced passes keeps pointing at the last one carrying music.
    fn voiced_so_far(&self) -> Option<std::path::PathBuf> {
        if self.voiced {
            Some(self.path())
        } else {
            self.last_voiced.clone()
        }
    }
}

fn finish_ready(
    open: &mut Option<Open>,
    epoch: u64,
    fence: &RecordFence,
) -> Option<std::path::PathBuf> {
    if !open.as_ref().is_some_and(|o| o.ready(epoch)) {
        return None;
    }
    finish_open(open, fence)
}

fn finish_open(open: &mut Option<Open>, fence: &RecordFence) -> Option<std::path::PathBuf> {
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
    open: &mut Option<Open>,
    status: &Mutex<String>,
) -> bool {
    drain_with_audio(consumer, None, open, status, None)
}

#[cfg(test)]
fn drain_with_audio(
    consumer: &mut rtrb::Consumer<Entry>,
    audio: Option<&mut rtrb::Consumer<f32>>,
    open: &mut Option<Open>,
    status: &Mutex<String>,
    fence: Option<&RecordFence>,
) -> bool {
    drain_with_boundaries(consumer, audio, open, status, fence, &FailureAccount::default(), |_| {})
}

fn drain_with_boundaries(
    consumer: &mut rtrb::Consumer<Entry>,
    mut audio: Option<&mut rtrb::Consumer<f32>>,
    open: &mut Option<Open>,
    status: &Mutex<String>,
    fence: Option<&RecordFence>,
    failure: &FailureAccount,
    mut before_new_pass: impl FnMut(&mut Option<Open>),
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
            if let Err(error) = Open::next_pass(open, status) {
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
                if let Some(current) = open.as_mut().filter(|o| o.epoch == address.epoch) {
                    if current.pass == address.pass {
                        current.configuration_complete = true;
                    } else if let Some(pass) =
                        current.retained.iter_mut().find(|p| p.pass == address.pass)
                    {
                        pass.configuration_complete = true;
                    } else {
                        fail();
                    }
                    if let Err(error) = current.finish_completed_passes() {
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
                if let Some(current) = open.as_mut().filter(|o| o.epoch == epoch) {
                    if matches!(entry, Entry::ProducerClosed(_)) {
                        current.producer_closed = true;
                    } else {
                        current.configuration_closed = true;
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
                        if let Some(current) = open.as_mut() {
                            let (first, second) = chunk.as_slices();
                            if let Err(error) = current.write_audio(first, second) {
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
            if let Some(current) = open.as_mut() {
                current.header.audio_start = Some(t);
                let header = current.header.clone();
                if current.writer.write(&harmonigraph_take::Record::Header(header)).is_err() {
                    fail();
                }
            }
            continue;
        }
        let Some(current) = open.as_mut() else { continue };
        // A note starting is what makes this pass the one worth rendering.
        if matches!(entry, Entry::Note { kind: NoteEventKind::On { .. }, .. }) {
            current.voiced = true;
        }
        let writer = &mut current.writer;
        let result = match entry {
            Entry::Configuration(config) => writer.configuration(config),
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
