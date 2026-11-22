//! Writer-thread command pump, lane draining, and file finalization.
//! File-backed fixtures live below this boundary so recording state stays
//! private.
//!
//! One armed take is one file and one WAV beside it. The writer decides
//! nothing about song time: it writes every item that arrives while a file is
//! open, and the reader drops what falls in no run. It seals a stopped take
//! once the take's run has ended.

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
    let (recorder, control) = channel("test");
    *control.fence.test_directory.lock() = Some(directory.to_path_buf());
    testing::set_renderer_program(&control, directory.join("no-such-renderer"));
    (recorder, control)
}

/// The WAV every fixture's take opens: [`TAKE_CHANNELS`], as `Control::start`
/// asks for, at the sample rate of [`harmonigraph_take::Header::default`].
#[cfg(any(test, feature = "test-support"))]
const FIXTURE_SPEC: AudioSpec = AudioSpec { sample_rate: 48_000.0, channels: TAKE_CHANNELS as u16 };

/// How long the writer thread sleeps when it finds the rings empty.
const DRAIN_IDLE: std::time::Duration = std::time::Duration::from_millis(20);

/// What the writer thread drains: the far end of every ring the [`Recorder`]
/// pushes into, and of the [`Control`]'s commands. A test that holds these
/// instead reads exactly what was pushed, with no thread racing it.
struct Ends {
    records: rtrb::Consumer<Entry>,
    audio: rtrb::Consumer<f32>,
    publications: publication::Consumer,
    orders: mpsc::Receiver<Command>,
}

/// Both endpoints over rings of these capacities, wired to each other and to
/// [`Ends`], with no writer thread. The one place a [`Control`] is spelled:
/// [`channel`] starts the writer on the ends, and tests and the in-memory
/// fixtures keep them.
fn endpoints(records: usize, audio: usize) -> (Recorder, Control, Ends) {
    let (producer, record_consumer) = rtrb::RingBuffer::new(records);
    let (publication, publications) = publication::channel();
    let (display, display_consumer) = publication::channel();
    let (audio_producer, audio_consumer) = rtrb::RingBuffer::new(audio);
    let (commands, orders) = mpsc::channel::<Command>();
    let recorder = Recorder::new(producer, audio_producer, publication, display, commands.clone());
    let control = Control {
        stop_wakeup: None,
        display: Arc::new(Mutex::new(Some(display_consumer))),
        fence: recorder.fence.clone(),
        commands,
        dropped: recorder.dropped.clone(),
        status: Arc::default(),
        last_take: Arc::default(),
        recording: Arc::default(),
        rolling: recorder.rolling.clone(),
        end_at_rewind: recorder.end_at_rewind.clone(),
        latches: recorder.latches.clone(),
        progress: Arc::default(),
        render: Arc::default(),
    };
    let ends = Ends { records: record_consumer, audio: audio_consumer, publications, orders };
    (recorder, control, ends)
}

/// Build the ring and start the writer thread. Called once, from the
/// plugin's `Default`, so neither arming nor disarming ever has to touch
/// the audio thread's producer.
pub fn channel(plugin_build: &'static str) -> (Recorder, Control) {
    let (recorder, mut control, ends) = endpoints(TAKE_RING_CAPACITY, AUDIO_RING_CAPACITY);
    control.render = Arc::new(RenderControl::for_plugin(plugin_build));
    let Ends { records: mut consumer, audio: mut audio_consumer, mut publications, orders } = ends;
    let thread_fence = control.fence.clone();
    let thread_status = control.status.clone();
    let thread_last_take = control.last_take.clone();
    let thread_progress = control.progress.clone();
    let thread_render = control.render.clone();
    let _ = std::thread::Builder::new().name("harmonigraph-take-writer".into()).spawn(move || {
        let mut pump = Pump::default();
        loop {
            // What the rings hold, then the commands: an armed item counted
            // here was pushed after the `Start` this poll will see. The record
            // ring is counted first, so a `RunEnd` counted here comes with
            // every publication pushed before it. A Stop seals only once it
            // was pending before the count, which so holds everything pushed
            // before it.
            let stop_counted = pump.pending_stop;
            let records = consumer.slots();
            let items = publications.ready();
            #[cfg(feature = "test-support")]
            thread_fence.worker_before_commands.reach();
            #[cfg(all(test, feature = "test-support"))]
            let mut processed_stop = false;
            if !pump.disconnected {
                match orders.try_recv() {
                    Ok(Command::Start(header, path, spec)) => {
                        pump.start(*header, path, spec, &thread_status, &thread_fence);
                    }
                    Ok(Command::Stop) => {
                        #[cfg(all(test, feature = "test-support"))]
                        {
                            processed_stop = true;
                        }
                        pump.pending_stop = true;
                        if !thread_fence.failed.load(Ordering::Acquire) {
                            *thread_status.lock() = FINISHING.into();
                        }
                    }
                    Err(mpsc::TryRecvError::Empty) => {
                        #[cfg(feature = "test-support")]
                        thread_fence.worker_after_empty.reach();
                    }
                    Err(mpsc::TryRecvError::Disconnected) => pump.disconnected = true,
                }
            }
            let pumped = pump.pass(
                &mut consumer,
                &mut audio_consumer,
                &mut publications,
                (records, items),
                stop_counted,
                &thread_status,
                &thread_fence,
            );
            // Before the shutdown check below, so a take sealed by the same
            // pass that observed the disconnect is published and rendered
            // rather than going with the thread.
            if let Some(path) = pumped.finished {
                *thread_last_take.lock() = Some(path.clone());
                // The take's own look: the renderer reads it from the file.
                spawn_render(None, path, None, thread_progress.clone(), thread_render.clone());
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
    (recorder, control)
}

/// Everything one writer carries between passes of the pump.
///
/// One struct because [`Pump::pass`] is driven from two places — the thread
/// above and `testing::FileWriter`, which stands in for it in every dependent
/// crate's take coverage — and the two must run the same pump (#895).
#[derive(Default)]
struct Pump {
    open: Option<Recording>,
    /// A Stop arrived and the take it stops is not sealed yet.
    pending_stop: bool,
    /// No further command can arrive.
    disconnected: bool,
}

/// What one [`Pump::pass`] leaves for its driver to do.
#[derive(Default)]
struct Pumped {
    /// Either lane had something, so the writer must not sleep yet.
    worked: bool,
    /// A take sealed this pass: where it landed.
    finished: Option<std::path::PathBuf>,
    /// Both lanes are drained and no command can arrive: the thread returns.
    shutdown: bool,
}

impl Pump {
    /// Open the take a `Start` names. A take still open is one whose Stop
    /// never sealed it, which `Control::start` refuses to let happen.
    fn start(
        &mut self,
        header: harmonigraph_take::Header,
        path: std::path::PathBuf,
        spec: AudioSpec,
        status: &Mutex<String>,
        fence: &RecordFence,
    ) {
        if self.open.is_some() {
            fail_with(fence, status, "a take started before the last one sealed".into());
            return;
        }
        self.open = match Recording::create(header, path, spec) {
            Ok(recording) => {
                let mut message = status.lock();
                if !matches!(message.as_str(), FINISHING | FINISHING_PREVIOUS) {
                    *message = format!("recording to {}", recording.path.display());
                }
                Some(recording)
            }
            Err(cause) => {
                fail_with(fence, status, cause);
                None
            }
        };
        #[cfg(feature = "test-support")]
        if let Some(audio) = self.open.as_mut().and_then(|o| o.audio.as_mut()) {
            if let Some(limit) = *fence.test_wav_limit.lock() {
                audio.limit_frames_for_test(limit);
            }
            if fence.test_wav_finish_failure.load(Ordering::Acquire) {
                audio.fail_finish_for_test();
            }
        }
    }

    /// One pass: drain what the rings held when it was counted into the open
    /// take, flush it, mark a failure, and seal a stopped take once its run
    /// has ended. An idle worker checks again
    /// every 20 ms.
    #[allow(clippy::too_many_arguments)]
    fn pass(
        &mut self,
        consumer: &mut rtrb::Consumer<Entry>,
        audio: &mut rtrb::Consumer<f32>,
        publications: &mut publication::Consumer,
        (records, items): (usize, usize),
        stop_counted: bool,
        status: &Mutex<String>,
        fence: &RecordFence,
    ) -> Pumped {
        let mut pumped = Pumped::default();
        let had_records = drain_records(consumer, records, audio, &mut self.open, status, fence);
        let had_publications = self.drain_publications(publications, items, status, fence) != 0;
        pumped.worked = had_records || had_publications;
        let failed = fence.failed.load(Ordering::Acquire);
        if let Some(recording) = self.open.as_mut() {
            if failed && !recording.marked {
                *status.lock() = fence.failure_status();
            }
            let marked = if failed { recording.mark_incomplete() } else { Ok(()) };
            let written = marked.and_then(|()| recording.flush());
            if let Err(error) = written {
                fail_with(fence, status, error.to_string());
            }
        }
        let sealable = self.open.as_ref().is_none_or(|o| !o.run_open);
        if self.pending_stop && stop_counted && (sealable || self.disconnected) {
            self.pending_stop = false;
            let finished = self.open.take().map(|mut open| (open.finish(), open.marked));
            let failed = fence.failed.load(Ordering::Acquire);
            match finished {
                Some((Ok(path), false)) if !failed => {
                    let mut status = status.lock();
                    if matches!(status.as_str(), FINISHING | FINISHING_PREVIOUS) {
                        status.clear();
                    }
                    pumped.finished = Some(path);
                }
                Some((Err(error), _)) => fence.fail_with_message(error.to_string()),
                _ => {}
            }
            // Under the status lock Start reads `finishing` in, so a Start
            // refused as finishing cannot overwrite the failure after it.
            let mut status = status.lock();
            if fence.failed.load(Ordering::Acquire) {
                *status = fence.failure_status();
            }
            fence.finishing.store(false, Ordering::Release);
        }
        if self.disconnected && consumer.is_empty() && publications.ready() == 0 {
            // Nothing can stop or feed the take still open: it was destroyed
            // mid-take, which the recorder's drop has already failed.
            if let Some(mut open) = self.open.take() {
                fence.fail();
                let _ = open.mark_incomplete();
                let _ = open.finish();
                *status.lock() =
                    "recording incomplete: producer disconnected before finalization".into();
            }
            pumped.shutdown = true;
        }
        pumped
    }

    /// The take's half of publication: every item drained goes to the open
    /// take, if any. The display has its own lane (#712 §4).
    fn drain_publications(
        &mut self,
        publications: &mut publication::Consumer,
        items: usize,
        status: &Mutex<String>,
        fence: &RecordFence,
    ) -> usize {
        let open = &mut self.open;
        publications.drain_up_to(items, |event| {
            // A gap is NOT a failure (#712): the reader warns if it overlaps
            // a run.
            let record = harmonigraph_take::CanonicalRecord::from_event(event);
            if let Some(Err(error)) = open.as_mut().map(|open| open.writer.canonical(record)) {
                fail_with(fence, status, error.to_string());
            }
            true
        })
    }
}

/// A failure the writer found: keep its cause, and write it to the status
/// line, so [`Control::status`] reads the whole failure ("recording incomplete:
/// `<cause>`; no render started") from the pass that found it. Writing the bare
/// cause there instead showed it without the rest until the next GUI refresh,
/// or, after a Stop, until the take sealed (#1843).
fn fail_with(fence: &RecordFence, status: &Mutex<String>, cause: String) {
    fence.fail_with_message(cause);
    *status.lock() = fence.failure_status();
}

/// In-memory endpoints for dependent crates that need to reach their real
/// [`Recorder`] wiring without starting this crate's file-writer thread.
/// Compiled only when a dev-dependency explicitly asks for it.
#[cfg(feature = "test-support")]
#[doc(hidden)]
pub mod testing;

/// One armed take: its file, its WAV, and where its runs stand.
struct Recording {
    path: std::path::PathBuf,
    writer: harmonigraph_take::Writer,
    /// The WAV's file name, which the header names.
    audio_file: Option<String>,
    /// The WAV itself, until it is finalized or a write to it fails.
    audio: Option<harmonigraph_take::WavWriter>,
    /// Whether the file holds a `Run` no `RunEnd` has followed yet: a stopped
    /// take seals only once one has. This is what the ring has delivered, a
    /// ring's length behind [`Recorder::open_run`](super::Recorder::open_run),
    /// so it is the file's own state rather than a copy of the recorder's.
    run_open: bool,
    /// Whether the file already holds its failure marker.
    marked: bool,
}

impl Recording {
    /// Acquire the take file and its WAV beside it, both exclusively, so a
    /// take never writes into another recorder's files or an orphan. A
    /// failure is its cause, for the status line.
    fn create(
        mut header: harmonigraph_take::Header,
        base: std::path::PathBuf,
        spec: AudioSpec,
    ) -> Result<Recording, String> {
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
                Err(err) => return Err(format!("cannot create {}: {err}", path.display())),
            };
            let wav_file = match std::fs::File::create_new(&wav) {
                Ok(file) => file,
                Err(err) => {
                    drop(take_file);
                    // Only the take was acquired by this attempt; never
                    // remove the companion whose creation just failed.
                    if let Err(cleanup) = std::fs::remove_file(&path) {
                        return Err(format!("cannot remove unused {}: {cleanup}", path.display()));
                    }
                    if err.kind() == std::io::ErrorKind::AlreadyExists {
                        continue;
                    }
                    return Err(format!("cannot create {}: {err}", wav.display()));
                }
            };
            let initialized = (|| {
                let audio = harmonigraph_take::WavWriter::from_file(
                    wav_file,
                    spec.sample_rate,
                    spec.channels,
                )?;
                header.audio_file = wav.file_name().and_then(|n| n.to_str()).map(str::to_owned);
                let writer = harmonigraph_take::Writer::from_file(take_file, &header)?;
                Ok::<_, std::io::Error>((writer, audio))
            })();
            let (writer, audio) = match initialized {
                Ok(pair) => pair,
                Err(err) => {
                    // Both handles have closed and both paths belong to us.
                    let cleanup_take = std::fs::remove_file(&path);
                    let cleanup_wav = std::fs::remove_file(&wav);
                    return Err(format!(
                        "cannot initialize {}: {err}; cleanup: {:?}, {:?}",
                        path.display(),
                        cleanup_take.err(),
                        cleanup_wav.err()
                    ));
                }
            };
            return Ok(Recording {
                audio_file: header.audio_file.clone(),
                path,
                writer,
                audio: Some(audio),
                run_open: false,
                marked: false,
            });
        }
        Err("recording filename suffix exhausted".into())
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
    /// take with audio and no MIDI is just as intentional as one with notes.
    /// Attempts both even if one fails.
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

    /// Write the take's failure marker, once.
    fn mark_incomplete(&mut self) -> std::io::Result<()> {
        let record = harmonigraph_take::IncompleteRecord {
            reason: harmonigraph_take::canonical::GapReasonRecord::ProducerLost,
            ..Default::default()
        };
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

/// Drain `count` entries of the record ring into the open take: runs,
/// configuration, parameters and audio, in ring order. With no take open they
/// are consumed and dropped, audio included, so the two rings stay in step.
fn drain_records(
    consumer: &mut rtrb::Consumer<Entry>,
    count: usize,
    audio: &mut rtrb::Consumer<f32>,
    open: &mut Option<Recording>,
    status: &Mutex<String>,
    fence: &RecordFence,
) -> bool {
    let io_failure = |error: std::io::Error| fail_with(fence, status, error.to_string());
    let mut any = false;
    for _ in 0..count {
        let Ok(entry) = consumer.pop() else { break };
        any = true;
        if let Entry::AudioSamples(count) = entry {
            // The audio thread commits a block's samples before the entry
            // that counts them, so this holds them all unless the two rings
            // have fallen out of step.
            let Ok(chunk) = audio.read_chunk(count) else {
                fail_with(fence, status, format!("audio ring short of {count} samples"));
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
        let Some(recording) = open.as_mut() else { continue };
        let written = match entry {
            Entry::Run { at, song } => {
                recording.run_open = true;
                recording.writer.write(&harmonigraph_take::Record::Run(
                    harmonigraph_take::RunRecord { at, song },
                ))
            }
            Entry::RunEnd { at } => {
                recording.run_open = false;
                recording.writer.write(&harmonigraph_take::Record::RunEnd(
                    harmonigraph_take::RunEndRecord { at },
                ))
            }
            Entry::Configuration(config) => recording.writer.configuration(config),
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
