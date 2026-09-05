//! Bounded callback records; only the consumer thread formats or writes them.

use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use clap_sys::events::*;
use nice_plug::wrapper::clap::ProcessTrace;
use serde::Serialize;

const CAPACITY: usize = 65_536;
static ORIGIN: OnceLock<Instant> = OnceLock::new();
static NEXT_INSTANCE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Default, Serialize)]
pub struct Clock {
    pub callback: u64,
    pub host_address: usize,
    pub steady: i64,
    pub frames: u32,
    pub start: u32,
    pub length: u32,
}

#[derive(Clone, Copy, Serialize)]
pub struct Event {
    pub kind: u16,
    pub space: u16,
    pub flags: u32,
    pub offset: u32,
    pub note_id: i32,
    pub channel: i16,
    pub key: i16,
    pub expression: i32,
    pub value: f64,
}

impl Event {
    // The wrapper passes a header borrowed from a complete host/framework event.
    pub fn raw(header: &clap_event_header) -> Self {
        let mut event = Self {
            kind: header.type_,
            space: header.space_id,
            flags: header.flags,
            offset: header.time,
            note_id: -1,
            channel: -1,
            key: -1,
            expression: -1,
            value: 0.0,
        };
        if header.space_id != CLAP_CORE_EVENT_SPACE_ID {
            return event;
        }
        match header.type_ {
            CLAP_EVENT_NOTE_ON
            | CLAP_EVENT_NOTE_OFF
            | CLAP_EVENT_NOTE_CHOKE
            | CLAP_EVENT_NOTE_END
                if header.size as usize >= std::mem::size_of::<clap_event_note>() =>
            {
                // SAFETY: The core event type and its advertised size match the cast.
                let note = unsafe { &*(header as *const _ as *const clap_event_note) };
                event.note_id = note.note_id;
                event.channel = note.channel;
                event.key = note.key;
                event.value = note.velocity;
            }
            CLAP_EVENT_NOTE_EXPRESSION
                if header.size as usize >= std::mem::size_of::<clap_event_note_expression>() =>
            {
                let note = unsafe { &*(header as *const _ as *const clap_event_note_expression) };
                event.note_id = note.note_id;
                event.channel = note.channel;
                event.key = note.key;
                event.expression = note.expression_id;
                event.value = note.value;
            }
            _ => {}
        }
        event
    }
}

#[derive(Clone, Copy, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Data {
    Lifecycle {
        name: &'static str,
        generation: u64,
        pending: usize,
        held: usize,
    },
    Activation {
        rate: f32,
        min: Option<u32>,
        max: u32,
        offline: bool,
        delay: u32,
        sources: usize,
        clock_offset: i64,
    },
    CallbackEnter {
        input_latency: Option<u32>,
        output_latency: Option<u32>,
        inputs: u32,
        outputs: u32,
        latency_queries: u64,
        reported_latency: u32,
    },
    Transport {
        offset: u32,
        flags: u32,
        seconds: i64,
        beats: i64,
        tempo: f64,
        tempo_inc: f64,
        loop_start_seconds: i64,
        loop_end_seconds: i64,
    },
    FrameworkTransport {
        seconds: Option<f64>,
        beats: Option<f64>,
        playing: bool,
    },
    AudioLevel {
        peak: f32,
        energy: f64,
        values: usize,
        nonfinite: usize,
    },
    SubBlock {
        enter: bool,
    },
    CallbackExit {
        status: i32,
    },
    RawInput {
        event: Event,
    },
    RawOutput {
        event: Event,
        accepted: bool,
    },
    Input {
        #[serde(rename = "input_sequence")]
        sequence: u64,
        request: u64,
        sample: i64,
        deadline: i64,
        event_kind: &'static str,
        note_id: Option<i32>,
        channel: Option<u8>,
        key: Option<u8>,
    },
    Progress {
        source: usize,
        generation: u64,
        through: i64,
    },
    Assignment {
        source: usize,
        generation: u64,
        request: u64,
        input: i64,
        deadline: i64,
        order: u64,
        release_after: i64,
    },
    ReplyPublished {
        source: usize,
        generation: u64,
        request: u64,
        hub_sample: i64,
    },
    ReplyVisible {
        generation: u64,
        request: u64,
        input: i64,
        deadline: i64,
    },
    PlannedOutput {
        request: u64,
        input: i64,
        deadline: i64,
        actual: i64,
        extra_shift: i64,
    },
    Fault {
        reason: &'static str,
        request: u64,
        sample: i64,
    },
    ClockGap {
        expected: i64,
        actual: i64,
    },
}

#[derive(Clone, Copy, Serialize)]
struct Record {
    sequence: u64,
    ns: u64,
    thread: u64,
    clock: Clock,
    #[serde(flatten)]
    data: Data,
}

pub struct Trace {
    pub clock: Clock,
    pub fatal: bool,
    origin: Instant,
    sequence: u64,
    producer: rtrb::Producer<Record>,
    lost: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

fn thread_id() -> u64 {
    #[cfg(target_os = "macos")]
    {
        let mut id = 0;
        // SAFETY: Null selects the calling thread; the out-pointer is valid.
        unsafe { libc::pthread_threadid_np(0, &mut id) };
        id
    }
    #[cfg(target_os = "linux")]
    {
        unsafe { libc::syscall(libc::SYS_gettid) as u64 }
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        0
    }
}

impl Trace {
    pub fn new(class: &'static str, source: Option<usize>) -> Self {
        let origin = *ORIGIN.get_or_init(Instant::now);
        let instance = NEXT_INSTANCE.fetch_add(1, Ordering::Relaxed);
        let (producer, mut consumer) = rtrb::RingBuffer::<Record>::new(CAPACITY);
        let stop = Arc::new(AtomicBool::new(false));
        let lost = Arc::new(AtomicU64::new(0));
        let worker_stop = stop.clone();
        let worker_lost = lost.clone();
        let dir = super::directory();
        let file = fs::create_dir_all(&dir).and_then(|()| {
            File::create(dir.join(format!("trace-{}-{instance}.jsonl", std::process::id())))
        });
        let fatal = file.is_err();
        let worker = match file {
            Ok(file) => Some(std::thread::spawn(move || {
                let mut out = BufWriter::new(file);
                let header = serde_json::json!({"kind":"header", "schema":1, "pid":std::process::id(), "instance":instance, "class":class, "source":source, "build":harmonigraph_perf::BUILD_TAG, "trace_capacity":CAPACITY, "clock_mapping":"candidate: raw steady_time plus configured offset; cross-instance validity unproven"});
                let mut io_ok = writeln!(out, "{header}").is_ok();
                let mut last_lost = 0;
                loop {
                    while let Ok(record) = consumer.pop() {
                        if matches!(record.data, Data::Fault { .. }) {
                            eprintln!(
                                "Harmonigraph #615 probe: {}",
                                serde_json::to_string(&record).unwrap_or_default()
                            );
                        }
                        io_ok &= serde_json::to_writer(&mut out, &record).is_ok();
                        io_ok &= out.write_all(b"\n").is_ok();
                    }
                    let dropped = worker_lost.load(Ordering::Acquire);
                    if dropped != last_lost {
                        io_ok &=
                            writeln!(out, "{{\"kind\":\"trace_loss\",\"lost\":{dropped}}}").is_ok();
                        eprintln!(
                            "Harmonigraph #615 probe: trace lost {dropped} records; run invalid"
                        );
                        last_lost = dropped;
                    }
                    io_ok &= out.flush().is_ok();
                    if !io_ok {
                        eprintln!("Harmonigraph #615 probe: trace write failed; run invalid");
                        break;
                    }
                    if worker_stop.load(Ordering::Acquire) && consumer.is_empty() {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
                let _ = writeln!(
                    out,
                    "{{\"kind\":\"footer\",\"lost\":{},\"io_ok\":{io_ok}}}",
                    worker_lost.load(Ordering::Acquire)
                );
                let _ = out.flush();
            })),
            Err(error) => {
                eprintln!("Harmonigraph #615 probe: cannot open trace: {error}");
                None
            }
        };
        Self {
            clock: Clock { steady: -1, ..Clock::default() },
            fatal,
            origin,
            sequence: 0,
            producer,
            lost,
            stop,
            worker,
        }
    }

    pub fn record(&mut self, data: Data) {
        self.at(Instant::now(), data);
    }

    fn at(&mut self, at: Instant, data: Data) {
        self.sequence += 1;
        let record = Record {
            sequence: self.sequence,
            ns: at.saturating_duration_since(self.origin).as_nanos() as u64,
            thread: thread_id(),
            clock: self.clock,
            data,
        };
        if self.producer.push(record).is_err() {
            self.lost.fetch_add(1, Ordering::Release);
            self.fatal = true;
        }
    }

    pub fn fault(&mut self, reason: &'static str, request: u64, sample: i64, fatal: bool) {
        self.fatal |= fatal;
        self.record(Data::Fault { reason, request, sample });
    }

    pub fn hook(&mut self, event: ProcessTrace<'_>) {
        match event {
            ProcessTrace::Enter { process, at, latency_queries, reported_latency } => {
                let expected = self.clock.steady.checked_add(i64::from(self.clock.frames));
                let previous = self.clock.callback;
                self.clock = Clock {
                    callback: previous + 1,
                    host_address: process as *const _ as usize,
                    steady: process.steady_time,
                    frames: process.frames_count,
                    start: 0,
                    length: process.frames_count,
                };
                // SAFETY: CLAP owns these arrays and transport for this callback.
                let input_latency =
                    if process.audio_inputs_count > 0 && !process.audio_inputs.is_null() {
                        Some(unsafe { (*process.audio_inputs).latency })
                    } else {
                        None
                    };
                let output_latency =
                    if process.audio_outputs_count > 0 && !process.audio_outputs.is_null() {
                        Some(unsafe { (*process.audio_outputs).latency })
                    } else {
                        None
                    };
                self.at(
                    at,
                    Data::CallbackEnter {
                        input_latency,
                        output_latency,
                        inputs: process.audio_inputs_count,
                        outputs: process.audio_outputs_count,
                        latency_queries,
                        reported_latency,
                    },
                );
                if previous > 0 && expected != Some(process.steady_time) {
                    self.record(Data::ClockGap {
                        expected: expected.unwrap_or(-1),
                        actual: process.steady_time,
                    });
                }
                if let Some(transport) = unsafe { process.transport.as_ref() } {
                    self.transport(transport);
                }
                if let Some(events) = unsafe { process.in_events.as_ref() } {
                    if let (Some(size), Some(get)) = (events.size, events.get) {
                        let count = unsafe { size(events) };
                        if count > super::MAX_EVENTS as u32 {
                            self.fault("raw_input_capacity", 0, process.steady_time, true);
                        }
                        for index in 0..count.min(super::MAX_EVENTS as u32) {
                            if let Some(header) = unsafe { get(events, index).as_ref() } {
                                self.record(Data::RawInput { event: Event::raw(header) });
                                if header.space_id == CLAP_CORE_EVENT_SPACE_ID
                                    && header.type_ == CLAP_EVENT_TRANSPORT
                                    && header.size as usize
                                        >= std::mem::size_of::<clap_event_transport>()
                                {
                                    self.transport(unsafe {
                                        &*(header as *const _ as *const clap_event_transport)
                                    });
                                }
                            }
                        }
                    }
                }
            }
            ProcessTrace::SubBlockEnter { start, length } => {
                self.clock.start = start;
                self.clock.length = length;
                self.record(Data::SubBlock { enter: true });
            }
            ProcessTrace::SubBlockExit { .. } => self.record(Data::SubBlock { enter: false }),
            ProcessTrace::Output { event, accepted } => {
                self.record(Data::RawOutput { event: Event::raw(event), accepted });
                if !accepted {
                    self.fault(
                        "host_output_rejected",
                        0,
                        self.clock.steady + i64::from(event.time),
                        true,
                    );
                }
            }
            ProcessTrace::Exit { status } => self.record(Data::CallbackExit { status }),
            ProcessTrace::Start => self.record(Data::Lifecycle {
                name: "start_processing",
                generation: 0,
                pending: 0,
                held: 0,
            }),
            ProcessTrace::Stop => self.record(Data::Lifecycle {
                name: "stop_processing",
                generation: 0,
                pending: 0,
                held: 0,
            }),
        }
    }

    fn transport(&mut self, t: &clap_event_transport) {
        self.record(Data::Transport {
            offset: t.header.time,
            flags: t.flags,
            seconds: t.song_pos_seconds,
            beats: t.song_pos_beats,
            tempo: t.tempo,
            tempo_inc: t.tempo_inc,
            loop_start_seconds: t.loop_start_seconds,
            loop_end_seconds: t.loop_end_seconds,
        });
    }

    pub fn sample(&mut self, offset: i64) -> Option<i64> {
        if self.clock.callback == 0 || self.clock.steady < 0 {
            self.fault("missing_raw_steady_clock", 0, -1, true);
            return None;
        }
        self.clock.steady.checked_add(i64::from(self.clock.start))?.checked_add(offset)
    }
}

impl Drop for Trace {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
