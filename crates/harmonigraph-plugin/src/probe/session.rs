//! One process-local fixture, bounded SPSC endpoints, and one central +50-cent owner.
//! Membership is fixed for a run: fewer/more active sources stalls completeness.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use nice_plug::prelude::*;
use nice_plug::wrapper::clap::ProcessTrace;

use super::trace::{Data, Trace};
use super::{Config, MAX_EVENTS, MAX_SOURCES, QUEUE_CAPACITY};

#[derive(Clone, Copy, Debug)]
pub struct Request {
    pub source: usize,
    pub generation: u64,
    pub epoch: u64,
    pub id: u64,
    pub input: i64,
    pub deadline: i64,
    pub channel: u8,
    pub key: u8,
}

#[derive(Clone, Copy)]
pub enum Intent {
    Request(Request),
    Progress { generation: u64, epoch: u64, from: i64, through: i64 },
}

#[derive(Clone, Copy)]
pub struct Reply {
    pub request: Request,
    pub correction: f32,
}

pub struct SourcePorts {
    pub input: rtrb::Producer<Intent>,
    pub replies: rtrb::Consumer<Reply>,
}

pub struct Shared {
    pub active: [AtomicBool; MAX_SOURCES],
    pub generations: [AtomicU64; MAX_SOURCES],
    pub epoch: AtomicU64,
    pub hub_active: AtomicBool,
    // Deterministically exercise a main-thread deactivation after collection.
    #[cfg(test)]
    pub deactivate_before_frontier: AtomicBool,
}

struct HubPorts {
    inputs: [rtrb::Consumer<Intent>; MAX_SOURCES],
    replies: [rtrb::Producer<Reply>; MAX_SOURCES],
}

pub struct Registry {
    pub shared: Arc<Shared>,
    sources: [Option<SourcePorts>; MAX_SOURCES],
    hub: Option<HubPorts>,
}

impl Registry {
    fn new() -> Self {
        let mut sources = Vec::with_capacity(MAX_SOURCES);
        let mut inputs = Vec::with_capacity(MAX_SOURCES);
        let mut replies = Vec::with_capacity(MAX_SOURCES);
        for _ in 0..MAX_SOURCES {
            let (input, receiver) = rtrb::RingBuffer::new(QUEUE_CAPACITY);
            let (sender, reply) = rtrb::RingBuffer::new(QUEUE_CAPACITY);
            sources.push(Some(SourcePorts { input, replies: reply }));
            inputs.push(receiver);
            replies.push(sender);
        }
        Self {
            shared: Arc::new(Shared {
                active: std::array::from_fn(|_| AtomicBool::new(false)),
                generations: std::array::from_fn(|_| AtomicU64::new(0)),
                epoch: AtomicU64::new(1),
                hub_active: AtomicBool::new(false),
                #[cfg(test)]
                deactivate_before_frontier: AtomicBool::new(false),
            }),
            sources: sources.try_into().ok().unwrap(),
            hub: Some(HubPorts {
                inputs: inputs.try_into().ok().unwrap(),
                replies: replies.try_into().ok().unwrap(),
            }),
        }
    }

    pub fn source(&mut self) -> Option<(usize, SourcePorts)> {
        self.sources
            .iter_mut()
            .enumerate()
            .find_map(|(i, slot)| slot.take().map(|ports| (i, ports)))
    }

    pub fn return_source(&mut self, source: usize, ports: SourcePorts) {
        self.sources[source] = Some(ports);
    }
}

pub fn registry() -> &'static Mutex<Registry> {
    static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(Registry::new()))
}

pub struct Hub {
    pub trace: Trace,
    shared: Arc<Shared>,
    ports: Option<HubPorts>,
    config: Config,
    progress: [i64; MAX_SOURCES],
    coverage_start: [i64; MAX_SOURCES],
    generations: [u64; MAX_SOURCES],
    pending: Vec<Request>,
    held_replies: Vec<(Request, i64)>,
    order: u64,
    epoch: u64,
    pub enabled: bool,
}

impl Default for Hub {
    fn default() -> Self {
        let mut registry = registry().lock().unwrap();
        Self {
            trace: Trace::new("hub", None),
            shared: registry.shared.clone(),
            ports: registry.hub.take(),
            config: Config::default(),
            progress: [i64::MIN; MAX_SOURCES],
            coverage_start: [i64::MIN; MAX_SOURCES],
            generations: [0; MAX_SOURCES],
            pending: Vec::with_capacity(MAX_EVENTS),
            held_replies: Vec::with_capacity(MAX_EVENTS),
            order: 0,
            epoch: 0,
            enabled: false,
        }
    }
}

impl Hub {
    pub fn initialize(&mut self, buffer: &BufferConfig, api: PluginApi) -> bool {
        self.enabled = api == PluginApi::Clap;
        if !self.enabled {
            return true;
        }
        let Ok(config) = Config::read() else {
            self.trace.fault("invalid_config", 0, -1, true);
            return false;
        };
        self.config = config;
        if self.ports.is_none() {
            self.trace.fault("multiple_hubs", 0, -1, true);
            return false;
        }
        self.shared.hub_active.store(true, Ordering::Release);
        self.trace.record(Data::Activation {
            rate: buffer.sample_rate,
            min: buffer.min_buffer_size,
            max: buffer.max_buffer_size,
            offline: buffer.process_mode == ProcessMode::Offline,
            delay: 0,
            sources: self.config.expected_sources,
            clock_offset: self.config.hub_clock_offset,
        });
        !self.trace.fatal
    }

    pub fn reset(&mut self) {
        if !self.enabled {
            return;
        }
        self.trace.record(Data::Lifecycle {
            name: "hub_reset",
            generation: self.epoch,
            pending: self.pending.len() + self.held_replies.len(),
            held: 0,
        });
        self.epoch = self.shared.epoch.fetch_add(1, Ordering::AcqRel) + 1;
        self.progress.fill(i64::MIN);
        self.coverage_start.fill(i64::MIN);
        self.pending.clear();
        self.held_replies.clear();
        self.order = 0;
    }

    pub fn hook(&mut self, event: ProcessTrace<'_>) {
        self.trace.hook(event);
    }

    pub fn process(&mut self, transport: &Transport, audio: &Buffer) -> bool {
        if !self.enabled {
            return true;
        }
        // Observe downstream voice energy without enabling physical monitoring.
        // The negotiated block bounds this scan; only the off-thread trace writer
        // serializes it, and no audio is played or copied into the trace.
        let mut peak = 0.0_f32;
        let mut energy = 0.0_f64;
        let mut values = 0;
        let mut nonfinite = 0;
        for channel in audio.as_slice_immutable().iter().take(2) {
            for &sample in channel.iter() {
                values += 1;
                if sample.is_finite() {
                    peak = peak.max(sample.abs());
                    energy += f64::from(sample) * f64::from(sample);
                } else {
                    nonfinite += 1;
                }
            }
        }
        self.trace.record(Data::AudioLevel { peak, energy, values, nonfinite });
        self.trace.record(Data::FrameworkTransport {
            seconds: transport.pos_seconds(),
            beats: transport.pos_beats(),
            playing: transport.playing,
        });
        let Some(now) = self.trace.sample(self.config.hub_clock_offset) else {
            return false;
        };
        let Some(ports) = self.ports.as_mut() else {
            return false;
        };
        // Membership must stay consistent between collection and completeness.
        // A concurrent deactivation may stall the next callback, but cannot
        // remove a source from this callback's minimum after it was counted.
        let active_sources: [bool; MAX_SOURCES] =
            std::array::from_fn(|i| self.shared.active[i].load(Ordering::Acquire));
        let mut active = 0;
        for (source, &is_active) in active_sources.iter().enumerate() {
            let generation = self.shared.generations[source].load(Ordering::Acquire);
            if generation != self.generations[source] {
                self.generations[source] = generation;
                self.progress[source] = i64::MIN;
                self.coverage_start[source] = i64::MIN;
                self.pending.retain(|r| r.source != source);
                self.held_replies.retain(|(r, _)| r.source != source);
            }
            active += usize::from(is_active);
            for _ in 0..QUEUE_CAPACITY {
                let Ok(intent) = ports.inputs[source].pop() else {
                    break;
                };
                match intent {
                    Intent::Request(r)
                        if is_active && r.generation == generation && r.epoch == self.epoch =>
                    {
                        if self.pending.len() == MAX_EVENTS {
                            self.trace.fault("hub_pending_capacity", r.id, now, true);
                            break;
                        }
                        self.pending.push(r);
                    }
                    Intent::Progress { generation: g, epoch, from, through }
                        if is_active && g == generation && epoch == self.epoch =>
                    {
                        if through <= from
                            || (self.progress[source] != i64::MIN && from != self.progress[source])
                        {
                            self.trace.fault("noncontiguous_source_progress", 0, from, true);
                            break;
                        }
                        if self.progress[source] == i64::MIN {
                            self.coverage_start[source] = from;
                        }
                        self.progress[source] = through;
                        self.trace.record(Data::Progress { source, generation: g, from, through });
                    }
                    _ => {}
                }
            }
        }
        if self.trace.fatal {
            return false;
        }
        #[cfg(test)]
        if self.shared.deactivate_before_frontier.swap(false, Ordering::Relaxed) {
            self.shared.active[0].store(false, Ordering::Release);
        }
        if active == self.config.expected_sources {
            let frontier = (0..MAX_SOURCES)
                .filter(|&i| active_sources[i])
                .map(|i| self.progress[i])
                .min()
                .unwrap_or(i64::MIN);
            let coverage_start = (0..MAX_SOURCES)
                .filter(|&i| active_sources[i])
                .map(|i| self.coverage_start[i])
                .max()
                .unwrap_or(i64::MIN);
            // A new incarnation only covers intervals it actually processed.
            // A high first watermark cannot fill its preceding absence gap.
            // This invalidates the experiment, not a production note-drop policy.
            if frontier != i64::MIN {
                if let Some(r) = self.pending.iter().find(|r| r.input < coverage_start) {
                    self.trace.fault("input_before_source_coverage", r.id, r.input, true);
                    return false;
                }
            }
            // No allocation: unstable in-place sort, with a total deterministic tie break.
            self.pending.sort_unstable_by_key(|r| (r.input, r.key, r.channel, r.source, r.id));
            let complete = self.pending.partition_point(|r| r.input < frontier);
            for r in self.pending.drain(..complete) {
                self.order += 1;
                let release_after = if self.config.hold_extra_samples > 0
                    && r.source == self.config.hold_source
                    && r.id == self.config.hold_request
                {
                    r.deadline + i64::from(self.config.hold_extra_samples)
                } else {
                    now
                };
                self.trace.record(Data::Assignment {
                    source: r.source,
                    generation: r.generation,
                    request: r.id,
                    input: r.input,
                    deadline: r.deadline,
                    order: self.order,
                    release_after,
                });
                if self.held_replies.len() == MAX_EVENTS {
                    self.trace.fault("held_reply_capacity", r.id, now, true);
                    break;
                }
                self.held_replies.push((r, release_after));
            }
        }
        let mut i = 0;
        while i < self.held_replies.len() {
            let (r, release_after) = self.held_replies[i];
            if now < release_after {
                i += 1;
                continue;
            }
            // The only assignment producer. The source never computes this value.
            if ports.replies[r.source].push(Reply { request: r, correction: 0.5 }).is_err() {
                self.trace.fault("reply_queue_full", r.id, now, true);
                break;
            }
            self.trace.record(Data::ReplyPublished {
                source: r.source,
                generation: r.generation,
                request: r.id,
                hub_sample: now,
            });
            self.held_replies.remove(i);
        }
        !self.trace.fatal
    }

    pub fn deactivate(&mut self) {
        if self.enabled {
            self.shared.hub_active.store(false, Ordering::Release);
            self.reset();
            self.trace.record(Data::Lifecycle {
                name: "hub_deactivate",
                generation: self.epoch,
                pending: 0,
                held: 0,
            });
        }
    }

    pub fn keep_alive(&self) -> bool {
        self.enabled && self.config.keep_alive
    }
}

impl Drop for Hub {
    fn drop(&mut self) {
        self.deactivate();
        if let Some(ports) = self.ports.take() {
            registry().lock().unwrap().hub = Some(ports);
        }
    }
}
