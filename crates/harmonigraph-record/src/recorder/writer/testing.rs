use super::*;

/// Only fence/status observation. This deliberately retains neither a
/// Control nor a command sender, so teardown fixtures cannot pin the writer.
pub struct WorkerProbe {
    fence: Arc<RecordFence>,
}
pub fn worker_probe(control: &Control, directory: std::path::PathBuf) -> WorkerProbe {
    *control.fence.test_directory.lock() = Some(directory);
    WorkerProbe { fence: control.fence.clone() }
}
impl WorkerProbe {
    pub fn pause_boundary(&self, enabled: bool) {
        self.fence.boundary_pause.enabled.store(enabled, Ordering::Release);
    }
    pub fn boundary_entered(&self) -> bool {
        self.fence.boundary_pause.entered.load(Ordering::Acquire)
    }
    pub fn empty_visits(&self) -> u64 {
        self.fence.worker_empty_visits.load(Ordering::Acquire)
    }
    pub fn finished(&self) -> bool {
        self.fence.worker_finished.load(Ordering::Acquire)
    }
    pub fn failed(&self) -> bool {
        self.fence.failed.load(Ordering::Acquire)
    }
    pub fn pause_retirement_check(&self) {
        self.fence.worker_before_retirement_check.entered.store(false, Ordering::Release);
        self.fence.worker_before_retirement_check.enabled.store(true, Ordering::Release);
    }
    pub fn retirement_check_paused(&self) -> bool {
        self.fence.worker_before_retirement_check.entered.load(Ordering::Acquire)
    }
    pub fn resume_retirement_check(&self) {
        self.fence.worker_before_retirement_check.enabled.store(false, Ordering::Release);
    }
}

pub struct Capture {
    publications: publication::Consumer,
    /// The editor's end of the second lane, so a test sees exactly what
    /// the display would without standing a writer thread up.
    displayed: publication::Consumer,
    fence: Arc<RecordFence>,
    _records: rtrb::Consumer<Entry>,
    audio: rtrb::Consumer<f32>,
    with_audio: Arc<AtomicBool>,
}

impl Capture {
    pub fn display_events(&mut self) -> Vec<harmonigraph_take::CanonicalRecord> {
        let mut events = Vec::new();
        self.displayed.drain(|delivery, _| {
            if let publication::Delivery::Event(event) = delivery {
                events.push(harmonigraph_take::CanonicalRecord::from_event(event));
            }
            true
        });
        events
    }
    /// Feed the display lane into a real tracker, exactly as the editor
    /// does, and report what each delivery left behind.
    pub fn display_into(
        &mut self,
        tracker: &mut harmonigraph_core::NoteTracker,
        mut watch: impl FnMut(
            &harmonigraph_core::canonical::CanonicalEvent<'_>,
            &harmonigraph_core::NoteTracker,
        ),
    ) -> usize {
        self.displayed.drain(|delivery, _| {
            if let publication::Delivery::Event(event) = delivery {
                watch(&event, tracker);
                tracker.handle_canonical(event).unwrap();
            }
            true
        })
    }
    pub fn pause_boundary(&self, enabled: bool) {
        self.fence.boundary_pause.enabled.store(enabled, Ordering::Release);
    }
    pub fn boundary_entered(&self) -> bool {
        self.fence.boundary_pause.entered.load(Ordering::Acquire)
    }
    pub fn pause_producer_close(&self, enabled: bool) {
        self.fence.producer_close_pause.enabled.store(enabled, Ordering::Release);
    }
    pub fn producer_close_entered(&self) -> bool {
        self.fence.producer_close_pause.entered.load(Ordering::Acquire)
    }
    pub fn arm(&self) {
        self.fence.intent.store((self.fence.epoch() + 1) << 1 | 1, Ordering::Release);
    }
    pub fn stop(&self) {
        self.fence.intent.fetch_and(!1, Ordering::AcqRel);
        self.fence.finishing.store(true, Ordering::Release);
    }
    pub fn drain_entries(&mut self) -> Vec<Entry> {
        std::iter::from_fn(|| self._records.pop().ok()).collect()
    }
    pub fn drain_canonical(&mut self) -> Vec<harmonigraph_take::CanonicalRecord> {
        let mut events = Vec::new();
        self.publications.drain(|delivery, _| {
            if let publication::Delivery::Event(event) = delivery {
                events.push(harmonigraph_take::CanonicalRecord::from_event(event));
            }
            true
        });
        events
    }
    pub fn arm_audio(&self) {
        self.arm();
        self.with_audio.store(true, Ordering::Relaxed);
    }

    pub fn drain_audio(&mut self) -> Vec<f32> {
        let mut samples = Vec::new();
        while let Ok(sample) = self.audio.pop() {
            samples.push(sample);
        }
        samples
    }
}

/// File-backed consumer of the very same captured producer stream and
/// completion gate used by the worker. No synthetic musical resolution.
pub struct FileWriter {
    open: Option<Open>,
    fence: Arc<RecordFence>,
    status: Mutex<String>,
    stopping: bool,
    pub finished: Option<std::path::PathBuf>,
    fanout: CanonicalFanout,
    failure: FailureAccount,
}
impl FileWriter {
    pub fn retained_passes(&self) -> usize {
        self.open.as_ref().map_or(0, |o| o.retained.len())
    }
    pub fn current_pass(&self) -> Option<u32> {
        self.open.as_ref().map(|o| o.pass)
    }
    pub fn new(capture: &Capture, path: std::path::PathBuf, spec: Option<AudioSpec>) -> Self {
        let status = Mutex::new(String::new());
        let mut open =
            Open::create(harmonigraph_take::Header::default(), path, 1, spec, &status).unwrap();
        open.epoch = capture.fence.epoch();
        open.configuration_enabled = capture.fence.enabled.load(Ordering::Acquire);
        open.source_enabled = capture.fence.canonical_enabled.load(Ordering::Acquire);
        Self {
            open: Some(open),
            fence: capture.fence.clone(),
            status,
            stopping: false,
            finished: None,
            fanout: CanonicalFanout::default(),
            failure: FailureAccount::default(),
        }
    }
    pub fn stop(&mut self) {
        self.stopping = true;
    }
    pub fn drain(&mut self, capture: &mut Capture) {
        if let Some(current) = self.open.as_mut() {
            current.observe_idle_producer(&self.fence);
        }
        drain_with_boundaries(
            &mut capture._records,
            Some(&mut capture.audio),
            &mut self.open,
            &self.status,
            Some(&self.fence),
            &self.failure,
            |open| {
                self.fanout.drain(&mut capture.publications, open, &self.fence, &self.failure);
            },
        );
        self.fanout.drain(&mut capture.publications, &mut self.open, &self.fence, &self.failure);
        if self.fence.failed.load(Ordering::Acquire) {
            if !self.fence.retirement_hold.load(Ordering::Acquire)
                && capture._records.is_empty()
                && capture.publications.settled()
                && !self.failure.contains(self.fence.epoch())
            {
                self.failure.account(
                    &mut self.open,
                    self.fence.epoch(),
                    &self.status,
                    Some(&self.fence),
                    harmonigraph_take::IncompleteRecord {
                        reason: harmonigraph_take::canonical::GapReasonRecord::ProducerLost,
                        ..Default::default()
                    },
                );
            }
        } else if self.stopping {
            self.finished = finish_ready(&mut self.open, self.fence.epoch(), &self.fence)
                .or_else(|| self.finished.take());
        }
    }
    pub fn failed(&self) -> bool {
        self.fence.failed.load(Ordering::Acquire)
    }
}

pub fn channel() -> (Recorder, Capture) {
    let (producer, records) = rtrb::RingBuffer::new(TAKE_RING_CAPACITY);
    let (publication, publications) = publication::channel();
    let (display, displayed) = publication::channel();
    let (audio, audio_consumer) = rtrb::RingBuffer::new(AUDIO_RING_CAPACITY);
    let with_audio = Arc::new(AtomicBool::new(false));
    let dropped = Arc::new(AtomicU64::new(0));
    let rolling = Arc::new(AtomicBool::new(false));
    let end_at_rewind = Arc::new(AtomicBool::new(false));
    let hit_rewind = Arc::new(AtomicBool::new(false));
    let fence = Arc::new(RecordFence::default());
    let recorder = Recorder {
        _writer_lifetime: None,
        publication,
        display,
        outage: publication::Lanes::default(),
        fence: fence.clone(),
        record_epoch: 0,
        record_pass: 1,
        closed_epoch: 0,
        last_configuration: None,
        producer,
        audio,
        with_audio: with_audio.clone(),
        dropped,
        last_params: [f32::NAN; ParamKey::ALL.len()],
        was_armed: false,
        last_position: None,
        rolling,
        audio_started: false,
        end_at_rewind,
        captured: Arc::new(AtomicU64::new(0)),
        hit_rewind,
        stop_at_bar: Arc::new(StopAtBar::default()),
        last_bar: None,
        finished: false,
        advanced: false,
        pending_split: false,
        rolled: Arc::new(AtomicBool::new(false)),
    };
    let capture = Capture {
        fence,
        publications,
        displayed,
        _records: records,
        audio: audio_consumer,
        with_audio,
    };
    (recorder, capture)
}
