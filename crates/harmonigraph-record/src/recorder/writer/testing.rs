use super::*;

/// Override only this recorder's subprocess launch, leaving production requests
/// and recorded appearance intact. Set before a fixture starts its export.
pub fn set_renderer_program(control: &Control, program: std::path::PathBuf) {
    *control.render.test_program.lock() = Some(program);
}

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
    pub fn finishing(&self) -> bool {
        self.fence.finishing.load(Ordering::Acquire)
    }

    pub fn pause_boundary(&self, enabled: bool) {
        self.fence.boundary_pause.enabled.store(enabled, Ordering::Release);
    }
    pub fn boundary_entered(&self) -> bool {
        self.fence.boundary_pause.entered.load(Ordering::Acquire)
    }
    pub fn finished(&self) -> bool {
        self.fence.worker_finished.load(Ordering::Acquire)
    }
    pub fn failed(&self) -> bool {
        self.fence.failed.load(Ordering::Acquire)
    }
}

pub struct Capture {
    publications: publication::Consumer,
    /// The editor's end of the second lane, so a test sees exactly what
    /// the display would without standing a writer thread up.
    displayed: publication::Consumer,
    fence: Arc<RecordFence>,
    end_at_rewind: Arc<AtomicBool>,
    _records: rtrb::Consumer<Entry>,
    audio: rtrb::Consumer<f32>,
}

impl Capture {
    pub fn display_events(&mut self) -> Vec<harmonigraph_take::CanonicalRecord> {
        let mut events = Vec::new();
        self.displayed.drain(|event| {
            events.push(harmonigraph_take::CanonicalRecord::from_event(event));
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
        self.displayed.drain(|event| {
            watch(&event, tracker);
            tracker.handle_canonical(event).unwrap();
            true
        })
    }
    pub fn pause_boundary(&self, enabled: bool) {
        self.fence.boundary_pause.enabled.store(enabled, Ordering::Release);
    }
    pub fn boundary_entered(&self) -> bool {
        self.fence.boundary_pause.entered.load(Ordering::Acquire)
    }
    pub fn arm(&self) {
        self.fence.arm(self.fence.epoch() + 1);
    }
    /// What the GUI publishes for Loop end: a rewind after forward motion ends
    /// the take rather than recording on.
    pub fn end_at_rewind(&self, on: bool) {
        self.end_at_rewind.store(on, Ordering::Relaxed);
    }
    pub fn stop(&self) {
        self.fence.disarm();
        self.fence.finishing.store(true, Ordering::Release);
    }
    pub fn drain_entries(&mut self) -> Vec<Entry> {
        std::iter::from_fn(|| self._records.pop().ok()).collect()
    }
    pub fn drain_canonical(&mut self) -> Vec<harmonigraph_take::CanonicalRecord> {
        let mut events = Vec::new();
        self.publications.drain(|event| {
            events.push(harmonigraph_take::CanonicalRecord::from_event(event));
            true
        });
        events
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
///
/// **The pump this drives is the worker's own** ([`Pump::pass`]), not a copy of
/// it: what the writer thread does between polling for a command and sleeping
/// is exactly what [`FileWriter::drain`] does here. The only halves it does
/// not have are the ones a
/// command channel owns — no `Start` opens its file (the constructor does),
/// and nothing can disconnect it.
pub struct FileWriter {
    pump: Pump,
    fence: Arc<RecordFence>,
    status: Mutex<String>,
    pub finished: Option<std::path::PathBuf>,
}
impl FileWriter {
    /// Opens `path` and the WAV beside it, as every `Start` does.
    pub fn new(capture: &Capture, path: std::path::PathBuf) -> Self {
        let open = Recording::create(harmonigraph_take::Header::default(), path).unwrap();
        assert!(open.audio.is_some());
        Self {
            pump: Pump { open: Some(open), ..Default::default() },
            fence: capture.fence.clone(),
            status: Mutex::default(),
            finished: None,
        }
    }
    /// The Stop the GUI sends. Nothing is rendered: there is no worker loop
    /// here to hand the finished take to `spawn_render`, which has its own
    /// tests, so [`drain`] keeps only where the take landed.
    ///
    /// [`drain`]: Self::drain
    pub fn stop(&mut self) {
        self.pump.pending_stop = true;
    }
    pub fn drain(&mut self, capture: &mut Capture) {
        let stop_counted = self.pump.pending_stop;
        let counts = (capture._records.slots(), capture.publications.ready());
        let pumped = self.pump.pass(
            &mut capture._records,
            &mut capture.audio,
            &mut capture.publications,
            counts,
            stop_counted,
            &self.status,
            &self.fence,
        );
        if let Some(path) = pumped.finished {
            self.finished = Some(path);
        }
    }
    /// The status line the writer left, as `Control::status` reads it.
    pub fn status(&self) -> String {
        self.status.lock().clone()
    }
    pub fn failed(&self) -> bool {
        self.fence.failed.load(Ordering::Acquire)
    }
}

/// The shipped rings with no writer thread, their far ends held by the
/// [`Capture`] instead.
pub fn channel() -> (Recorder, Capture) {
    let (recorder, control, ends) = endpoints(TAKE_RING_CAPACITY, AUDIO_RING_CAPACITY);
    let capture = Capture {
        fence: control.fence.clone(),
        end_at_rewind: control.end_at_rewind.clone(),
        publications: ends.publications,
        displayed: control.take_display().expect("a fresh control holds its display lane"),
        _records: ends.records,
        audio: ends.audio,
    };
    (recorder, capture)
}
