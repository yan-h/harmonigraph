//! Writer-thread command pump, ordered lane draining, and file finalization.
//! File-backed fixtures live below this boundary so recording state stays
//! private.
//!
//! One armed take is one file, holding every pass, with one WAV per pass beside
//! it. The writer decides nothing about passes or song time: it writes what the
//! audio thread stamped, drops what was played disarmed, and rotates the WAV at
//! a `Run` that opens a new pass. The reader chooses the rest (#1760).

use super::*;
#[cfg(test)]
use harmonigraph_core::SourceId;

#[cfg(all(test, feature = "test-support"))]
mod audio_tests;
#[cfg(all(test, feature = "test-support"))]
mod canonical_tests;
#[cfg(test)]
mod tests;

/// A real writer whose takes land in `directory` and whose renders are aimed at
/// a program that does not exist there: a take that finishes launches nothing,
/// and its render leaves only "could not run" on its export job's detail.
#[cfg(all(test, feature = "test-support"))]
fn worker_in(directory: &std::path::Path) -> (Recorder, Control) {
    let (recorder, control) = channel();
    *control.fence.test_directory.lock() = Some(directory.to_path_buf());
    testing::set_renderer_program(&control, directory.join("no-such-renderer"));
    (recorder, control)
}

/// The WAV every fixture's take opens: [`TAKE_CHANNELS`], as `Control::start`
/// asks for, at the sample rate of [`harmonigraph_take::Header::default`].
#[cfg(any(test, feature = "test-support"))]
const FIXTURE_SPEC: AudioSpec = AudioSpec { sample_rate: 48_000.0, channels: TAKE_CHANNELS as u16 };

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
    let recorder =
        Recorder::new(producer, audio_producer, publication, display, Some(commands.clone()));
    let recording = Arc::new(AtomicBool::new(false));
    let status = Arc::new(Mutex::new(String::new()));
    let last_take = Arc::new(Mutex::new(None));
    let progress = Arc::new(Progress::default());
    let render = Arc::new(RenderControl::default());

    let thread_fence = recorder.fence.clone();
    let thread_latches = recorder.latches.clone();
    let thread_status = status.clone();
    let thread_last_take = last_take.clone();
    let thread_progress = progress.clone();
    let thread_render = render.clone();
    let _ = std::thread::Builder::new().name("harmonigraph-take-writer".into()).spawn(move || {
        let mut pump = Pump { latches: thread_latches, ..Default::default() };
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
                            Recording::create(*header, path, epoch, spec, &thread_status);
                        #[cfg(feature = "test-support")]
                        if let Some(audio) = pump.open.as_mut().and_then(|o| o.audio.as_mut())
                        {
                            if let Some(limit) = *thread_fence.test_wav_limit.lock() {
                                audio.limit_frames_for_test(limit);
                            }
                            if thread_fence.test_wav_finish_failure.load(Ordering::Acquire) {
                                audio.fail_finish_for_test();
                            }
                        }
                        if pump.open.is_none()
                        {
                            thread_fence.fail_with_message(thread_status.lock().clone());
                            pump.failure.account(&mut pump.open, epoch, &thread_status, Some(&thread_fence), harmonigraph_take::IncompleteRecord {
                                reason: harmonigraph_take::canonical::GapReasonRecord::ProducerLost,
                                ..Default::default()
                            });
                        }
                    }
                }
                Ok(Command::Stop(epoch)) => {
                    #[cfg(all(test, feature = "test-support"))]
                    { processed_stop = true; }
                    pump.pending_stop = Some(epoch);
                    if !pump.failure.contains(epoch) {
                        *thread_status.lock() = FINISHING.into();
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
            if let Some((path, notice)) = pumped.finished {
                #[cfg(all(test, feature = "test-support"))]
                thread_fence.worker_after_finish.reach();
                *thread_last_take.lock() = Some(path.clone());
                let render = RenderRequest { appearance: None, notice };
                spawn_render(render, path, None, thread_progress.clone(), thread_render.clone());
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

    let control = Control {
        stop_wakeup: None,
        display: Arc::new(Mutex::new(Some(display_consumer))),
        fence: recorder.fence.clone(),
        commands,
        dropped: recorder.dropped.clone(),
        status,
        last_take,
        recording,
        rolling: recorder.rolling.clone(),
        end_at_rewind: recorder.end_at_rewind.clone(),
        latches: recorder.latches.clone(),
        progress,
        render,
    };
    (recorder, control)
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
    /// Shared with the producer: Stop intent can precede the admitted
    /// callback's completion, so the ready prefix owns the final reason.
    latches: Arc<TakeLatches>,
    open: Option<Recording>,
    fanout: CanonicalFanout,
    failure: FailureAccount,
    /// The epoch of the Stop whose prefix is not closed yet.
    pending_stop: Option<u64>,
    /// No further command can arrive, so unresolved ownership is lost.
    disconnected: bool,
}

/// What one [`Pump::pass`] leaves for its driver to do.
#[derive(Default)]
struct Pumped {
    /// Either lane had something, so the writer must not sleep yet.
    worked: bool,
    /// A take sealed this pass: where it landed, and the notice its render
    /// carries — why the take ended early, which only this pass knows.
    finished: Option<(std::path::PathBuf, Option<&'static str>)>,
    /// Both lanes are drained and no command can arrive: the thread returns.
    shutdown: bool,
}

impl Pump {
    /// One cross-lane pass: drain the record ring and the publication lane in
    /// order, flush the open take file, then resolve failure, a ready
    /// Stop, and shutdown. A killed process can lose the buffered part of this
    /// pass; an idle worker checks for the next batch every 20 ms.
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
        let had_records = !waiting_for_start
            && drain_records(
                consumer,
                Some(audio),
                &mut self.open,
                status,
                Some(fence),
                &self.failure,
            );
        let had_publications =
            self.fanout.drain(publications, &mut self.open, fence, &self.failure) != 0;
        pumped.worked = had_records || had_publications;
        if let Some(recording) = self.open.as_mut() {
            if let Err(error) = recording.flush() {
                fence.fail_with_message(error.to_string());
            }
        }
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
                // It is disposed here on the worker, unrendered, and the
                // failure status remains visible without another callback.
                self.pending_stop = None;
                fence.finishing.store(false, Ordering::Release);
                #[cfg(all(test, feature = "test-support"))]
                fence.worker_failure_accounted.store(true, Ordering::Release);
            }
        } else if self
            .pending_stop
            .is_some_and(|epoch| self.open.as_ref().is_some_and(|o| o.ready(epoch)))
        {
            self.pending_stop = None;
            if let Some(path) = finish_ready(&mut self.open, fence.epoch(), fence) {
                // All admitted callbacks have retired. Capture their final
                // reason before releasing finishing: the next Start clears
                // these shared latches, but this render must retain its own.
                let notice = (self.latches.end() == Some(End::ForwardSeek))
                    .then_some("take ended before a forward transport seek");
                // Publish before Start is permitted again. An error may
                // already own the line; completion only retires its own
                // finishing message, under the same lock as a refused Start.
                // Nothing replaces it: the take's export job names the video
                // from here on, beside Open video folder.
                let mut status = status.lock();
                if matches!(status.as_str(), FINISHING | FINISHING_PREVIOUS) {
                    status.clear();
                }
                fence.finishing.store(false, Ordering::Release);
                pumped.finished = Some((path, notice));
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
    /// Non-RT deduplication only. These cuts authorize no musical reclamation.
    order: harmonigraph_core::canonical::CanonicalOrder,
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
        publications.drain(|delivery, span| {
            // The take a record or a closure belongs to. History played
            // disarmed belongs to none, and a loss that only covered such
            // history touched no recorded run: it is dropped, warning nothing.
            let epoch = match delivery {
                publication::Delivery::PassClosed { epoch, .. }
                | publication::Delivery::EpochComplete(epoch) => Some(epoch),
                publication::Delivery::Event(_) => {
                    span.intent.is_armed().then(|| span.intent.epoch())
                }
            };
            // An armed item can reach this lane before its `Start` command
            // has opened the take it belongs to. Retain its whole payload.
            if epoch.is_some_and(|epoch| {
                !failure.contains(epoch) && open.as_ref().is_none_or(|o| o.epoch < epoch)
            }) {
                self.waiting_file = true;
                return false;
            }
            let recording = epoch.and_then(|epoch| open.as_mut().filter(|o| o.epoch == epoch));
            match delivery {
                publication::Delivery::PassClosed { epoch, pass } => {
                    // A failure's marker comes only once both lanes settle,
                    // below this line, and must still reach this pass.
                    if failure.contains(epoch) || fence.failed.load(Ordering::Acquire) {
                        return true;
                    }
                    match recording {
                        Some(recording) => {
                            if let Err(error) = recording.pass_closed(pass) {
                                fence.fail_with_message(error.to_string());
                            }
                        }
                        None => fence.fail(),
                    }
                }
                publication::Delivery::EpochComplete(epoch) => {
                    if failure.contains(epoch) {
                        return true;
                    }
                    match recording {
                        Some(recording) => recording.source_closed = true,
                        None => fence.fail(),
                    }
                }
                publication::Delivery::Event(event) => {
                    let accepted = match event {
                        CanonicalEvent::Note(delta) => {
                            self.order.note(delta.event.source, delta.sequence)
                        }
                        CanonicalEvent::Baseline(frame) => {
                            self.order.baseline(frame.source, frame.id, frame.output_cut)
                        }
                        CanonicalEvent::Gap(_) => Ok(true),
                    };
                    match accepted {
                        Ok(false) => return true,
                        Err(_) => {
                            fence.fail();
                            return true;
                        }
                        Ok(true) => {}
                    }
                    if epoch.is_none_or(|epoch| failure.contains(epoch)) {
                        return true;
                    }
                    // A delta's own timing is its stamp; a snapshot and a gap
                    // carry theirs beside the payload. A gap is NOT a failure
                    // (#712): it marks the passes it touched, and the export
                    // warns.
                    let mut record = harmonigraph_take::CanonicalRecord::from_event(event);
                    let stamp = |at: publication::Stamp| {
                        harmonigraph_take::StampRecord::new(at.clock, at.input)
                    };
                    match &mut record {
                        harmonigraph_take::CanonicalRecord::Baseline(frame) => {
                            frame.at = Some(stamp(span.first));
                        }
                        harmonigraph_take::CanonicalRecord::Gap(gap) => {
                            gap.inputs = Some([stamp(span.first), stamp(span.last)]);
                        }
                        _ => {}
                    }
                    // A take sealed before this arrived, or the write failed.
                    if recording.is_none_or(|recording| recording.writer.canonical(record).is_err())
                    {
                        fence.fail();
                    }
                }
            }
            true
        })
    }
}

/// One armed take: its file, the WAV of the pass being recorded, and how far
/// the three closers have got.
struct Recording {
    epoch: u64,
    path: std::path::PathBuf,
    writer: harmonigraph_take::Writer,
    spec: AudioSpec,
    /// The pass whose WAV is open: 1 until a `Run` opens another.
    pass: u32,
    /// That WAV's file name, which every `Run` of the pass names.
    audio_file: Option<String>,
    /// The WAV itself, until it is finalized or a write to it fails.
    audio: Option<harmonigraph_take::WavWriter>,
    producer_closed: bool,
    configuration_closed: bool,
    source_closed: bool,
    /// Whether the file already holds its failure marker.
    marked: bool,
    /// Test hook: fail the marker write once.
    #[cfg(all(test, feature = "test-support"))]
    fail_marker: bool,
}

impl Recording {
    /// Acquire the take file and pass 1's WAV beside it, both exclusively, so
    /// a take never writes into another recorder's files or an orphan.
    fn create(
        header: harmonigraph_take::Header,
        base: std::path::PathBuf,
        epoch: u64,
        spec: AudioSpec,
        status: &Mutex<String>,
    ) -> Option<Recording> {
        let stem = base.file_stem().and_then(|s| s.to_str()).unwrap_or("take").to_owned();
        for suffix in 0u64.. {
            let path = if suffix == 0 {
                base.clone()
            } else {
                base.with_file_name(format!("{stem}_{suffix}.{}", harmonigraph_take::EXTENSION))
            };
            let wav = path.with_extension("wav");
            let take_file = match std::fs::File::create_new(&path) {
                Ok(file) => file,
                Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(err) => {
                    *status.lock() = format!("cannot create {}: {err}", path.display());
                    return None;
                }
            };
            let wav_file = match std::fs::File::create_new(&wav) {
                Ok(file) => file,
                Err(err) => {
                    drop(take_file);
                    // Only the take was acquired by this attempt; never
                    // remove the companion whose creation just failed.
                    if let Err(cleanup) = std::fs::remove_file(&path) {
                        *status.lock() =
                            format!("cannot remove unused {}: {cleanup}", path.display());
                        return None;
                    }
                    if err.kind() == std::io::ErrorKind::AlreadyExists {
                        continue;
                    }
                    *status.lock() = format!("cannot create {}: {err}", wav.display());
                    return None;
                }
            };
            let initialized = (|| {
                let audio = harmonigraph_take::WavWriter::from_file(
                    wav_file,
                    spec.sample_rate,
                    spec.channels,
                )?;
                let writer = harmonigraph_take::Writer::from_file(take_file, &header)?;
                Ok::<_, std::io::Error>((writer, audio))
            })();
            let (writer, audio) = match initialized {
                Ok(pair) => pair,
                Err(err) => {
                    // Both handles have closed and both paths belong to us.
                    let cleanup_take = std::fs::remove_file(&path);
                    let cleanup_wav = std::fs::remove_file(&wav);
                    *status.lock() = format!(
                        "cannot initialize {}: {err}; cleanup: {:?}, {:?}",
                        path.display(),
                        cleanup_take.err(),
                        cleanup_wav.err()
                    );
                    return None;
                }
            };
            let mut message = status.lock();
            if !matches!(message.as_str(), FINISHING | FINISHING_PREVIOUS) {
                *message = format!("recording to {}", path.display());
            }
            return Some(Recording {
                epoch,
                audio_file: wav.file_name().and_then(|n| n.to_str()).map(str::to_owned),
                path,
                writer,
                spec,
                pass: 1,
                audio: Some(audio),
                producer_closed: false,
                configuration_closed: false,
                source_closed: false,
                marked: false,
                #[cfg(all(test, feature = "test-support"))]
                fail_marker: false,
            });
        }
        *status.lock() = "recording filename suffix exhausted".into();
        None
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.writer.flush().map_err(|error| {
            std::io::Error::new(
                error.kind(),
                format!("cannot flush {}: {error}", self.path.display()),
            )
        })
    }

    /// Close the take's two files and hand the take back for rendering. A
    /// take with audio and no MIDI is just as intentional as one with notes;
    /// the lifecycle defers stopped rewinds so restoration alone opens no
    /// tail. Attempts both even if one fails.
    fn finish(&mut self) -> std::io::Result<std::path::PathBuf> {
        let notes = self.writer.flush().map_err(|error| {
            std::io::Error::new(
                error.kind(),
                format!("cannot finalize {}: {error}", self.path.display()),
            )
        });
        let audio = self.finish_audio();
        notes.and(audio).map(|_| self.path.clone())
    }

    fn wav_path(&self) -> std::path::PathBuf {
        self.path.with_file_name(self.audio_file.as_deref().unwrap_or_default())
    }

    fn finish_audio(&mut self) -> std::io::Result<()> {
        let path = self.wav_path();
        self.audio.take().map_or(Ok(()), |audio| {
            audio.finish().map_err(|error| {
                std::io::Error::new(
                    error.kind(),
                    format!("cannot finalize {}: {error}", path.display()),
                )
            })
        })
    }

    /// Open a run. Its pass's audio goes to that pass's own WAV, which the
    /// first run of the pass opens beside the take: `take-…-2.wav` for pass 2,
    /// suffixed again if that name is taken.
    fn run(&mut self, mut record: harmonigraph_take::RunRecord) -> std::io::Result<()> {
        if record.pass != self.pass {
            self.finish_audio()?;
            let stem = self.path.file_stem().and_then(|s| s.to_str()).unwrap_or("take");
            let name = format!("{stem}-{}", record.pass);
            let (wav, file) = (0u64..)
                .map(|suffix| {
                    let name = if suffix == 0 { name.clone() } else { format!("{name}_{suffix}") };
                    self.path.with_file_name(format!("{name}.wav"))
                })
                .find_map(|wav| match std::fs::File::create_new(&wav) {
                    Ok(file) => Some(Ok((wav, file))),
                    Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => None,
                    Err(err) => Some(Err(std::io::Error::new(
                        err.kind(),
                        format!("cannot create {}: {err}", wav.display()),
                    ))),
                })
                .expect("an unbounded suffix finds a free name")?;
            let audio = harmonigraph_take::WavWriter::from_file(
                file,
                self.spec.sample_rate,
                self.spec.channels,
            )?;
            self.pass = record.pass;
            self.audio_file = wav.file_name().and_then(|n| n.to_str()).map(str::to_owned);
            self.audio = Some(audio);
        }
        record.audio_file = self.audio_file.clone();
        self.writer.write(&harmonigraph_take::Record::Run(record))
    }

    fn pass_closed(&mut self, pass: u32) -> std::io::Result<()> {
        self.writer.write(&harmonigraph_take::Record::PassClosed(pass))
    }

    fn ready(&self, epoch: u64) -> bool {
        self.epoch == epoch
            && self.producer_closed
            && self.configuration_closed
            && self.source_closed
    }

    /// Write the take's failure marker, once. It touches every pass not
    /// closed above it, which the reader works out from its place.
    fn mark_incomplete(
        &mut self,
        record: harmonigraph_take::IncompleteRecord,
    ) -> std::io::Result<()> {
        if self.marked {
            return Ok(());
        }
        #[cfg(all(test, feature = "test-support"))]
        if std::mem::take(&mut self.fail_marker) {
            self.writer.make_read_only_for_test(&self.path)?;
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

    fn write_audio(&mut self, first: &[f32], second: &[f32]) -> std::io::Result<()> {
        let Some(audio) = self.audio.as_mut() else { return Ok(()) };
        if let Err(error) = audio.write(first).and_then(|_| audio.write(second)) {
            let path = self.wav_path();
            let repair = self.finish_audio();
            let repair = repair.err().map(|error| format!("; {error}")).unwrap_or_default();
            return Err(std::io::Error::new(
                error.kind(),
                format!("cannot write {}: {error}{repair}", path.display()),
            ));
        }
        Ok(())
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

/// Drain the record ring into the open take: runs, configuration, parameters,
/// audio and the producer's and configuration's closures, in ring order.
fn drain_records(
    consumer: &mut rtrb::Consumer<Entry>,
    mut audio: Option<&mut rtrb::Consumer<f32>>,
    open: &mut Option<Recording>,
    status: &Mutex<String>,
    fence: Option<&RecordFence>,
    failure: &FailureAccount,
) -> bool {
    // Start is a separate off-thread message, published before arming. Retain
    // records if this iteration observed the ring before that command.
    if open.is_none() && fence.is_some_and(|f| !failure.contains(f.epoch())) {
        return false;
    }
    let fail = || {
        if let Some(fence) = fence {
            fence.fail();
        }
        *status.lock() = "recording incomplete: a record reached no open take".into();
    };
    let io_failure = |error: std::io::Error| {
        if let Some(fence) = fence {
            fence.fail_with_message(error.to_string());
        }
        *status.lock() = error.to_string();
    };
    let mut any = false;
    while let Ok(entry) = consumer.pop() {
        any = true;
        if let Entry::AudioSamples(count) = entry {
            // Consumed even with nothing to write it to, so the audio ring
            // stays in step with this one after an accounted failure.
            let Some(consumer) = audio.as_deref_mut() else {
                fail();
                continue;
            };
            if consumer.slots() < count {
                fail();
                continue;
            }
            let Ok(chunk) = consumer.read_chunk(count) else {
                fail();
                continue;
            };
            if let Some(recording) = open.as_mut() {
                let (first, second) = chunk.as_slices();
                if let Err(error) = recording.write_audio(first, second) {
                    io_failure(error);
                }
            }
            chunk.commit_all();
            continue;
        }
        if fence.is_some_and(|f| failure.contains(f.epoch())) {
            continue;
        }
        let epoch = match entry {
            Entry::Run { epoch, .. }
            | Entry::RunEnd { epoch, .. }
            | Entry::Configuration { epoch, .. }
            | Entry::ProducerClosed(epoch)
            | Entry::ConfigurationEpochComplete(epoch) => Some(epoch),
            Entry::Param { .. } | Entry::AudioSamples(_) => None,
        };
        let Some(recording) = open.as_mut().filter(|o| epoch.is_none_or(|epoch| o.epoch == epoch))
        else {
            if epoch.is_some() {
                fail();
            }
            continue;
        };
        let stamp =
            |at: publication::Stamp| harmonigraph_take::StampRecord::new(at.clock, at.input);
        let written = match entry {
            Entry::Run { pass, at, song, presentation, .. } => {
                recording.run(harmonigraph_take::RunRecord {
                    pass,
                    at: stamp(at),
                    song,
                    presentation,
                    audio_file: None,
                })
            }
            Entry::RunEnd { at, .. } => recording.writer.write(&harmonigraph_take::Record::RunEnd(
                harmonigraph_take::RunEndRecord { at: stamp(at) },
            )),
            Entry::Configuration { config, .. } => recording.writer.configuration(config),
            Entry::ProducerClosed(_) => {
                recording.producer_closed = true;
                Ok(())
            }
            Entry::ConfigurationEpochComplete(_) => {
                recording.configuration_closed = true;
                Ok(())
            }
            Entry::Param { t, key, value } => {
                recording.writer.param(harmonigraph_take::ParamRecord {
                    t,
                    id: ParamKey::ALL[key].id().to_string(),
                    value,
                })
            }
            Entry::AudioSamples(_) => unreachable!("handled above"),
        };
        if let Err(error) = written {
            io_failure(error);
        }
    }
    any
}
