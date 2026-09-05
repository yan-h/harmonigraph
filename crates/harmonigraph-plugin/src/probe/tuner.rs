use std::collections::VecDeque;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use nice_plug::prelude::*;
use nice_plug::wrapper::clap::ProcessTrace;

use super::session::{registry, Intent, Request, Shared, SourcePorts};
use super::trace::{Data, Trace};
use super::{event_key, event_name, retime, Config, MAX_EVENTS, QUEUE_CAPACITY};

#[derive(Params)]
struct ProbeParams {
    /// This value deliberately does no musical work. Automating it exercises
    /// the framework's sub-block splitter in the real host.
    #[id = "probe-split"]
    split: FloatParam,
}

impl Default for ProbeParams {
    fn default() -> Self {
        Self {
            split: FloatParam::new("Probe split", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 }),
        }
    }
}

#[derive(Clone, Copy)]
struct Pending {
    event: NoteEvent<()>,
    request: u64,
    input: i64,
    deadline: i64,
    correction: Option<f32>,
    diagnosed: bool,
}

#[derive(Clone, Copy, Default)]
struct Voice {
    request: u64,
    note_id: Option<i32>,
    correction: f32,
}

pub struct HarmonigraphTune {
    params: Arc<ProbeParams>,
    trace: Trace,
    shared: Arc<Shared>,
    source: usize,
    ports: Option<SourcePorts>,
    config: Config,
    generation: u64,
    epoch: u64,
    sequence: u64,
    next_request: u64,
    pending: VecDeque<Pending>,
    input_voices: Box<[u64; 2048]>,
    output_voices: Box<[Voice; 2048]>,
    extra_shift: i64,
    max_frames: u32,
}

impl Default for HarmonigraphTune {
    fn default() -> Self {
        let mut registry = registry().lock().unwrap();
        let (source, ports) = match registry.source() {
            Some((i, p)) => (i, Some(p)),
            None => (super::MAX_SOURCES, None),
        };
        Self {
            params: Arc::new(ProbeParams::default()),
            trace: Trace::new("tuner", Some(source)),
            shared: registry.shared.clone(),
            source,
            ports,
            config: Config::default(),
            generation: 0,
            epoch: 0,
            sequence: 0,
            next_request: 0,
            pending: VecDeque::with_capacity(MAX_EVENTS),
            input_voices: Box::new([0; 2048]),
            output_voices: Box::new([Voice::default(); 2048]),
            extra_shift: 0,
            max_frames: 0,
        }
    }
}

impl HarmonigraphTune {
    fn held(&self) -> usize {
        self.output_voices.iter().filter(|v| v.request != 0).count()
    }

    fn index(event: &NoteEvent<()>) -> Option<usize> {
        match (event.channel(), event_key(event)) {
            (Some(channel @ 0..=15), Some(key @ 0..=127)) => {
                Some(usize::from(channel) * 128 + usize::from(key))
            }
            _ => None,
        }
    }

    fn process_events(&mut self, context: &mut impl ProcessContext<Self>) -> bool {
        self.trace.record(Data::FrameworkTransport {
            seconds: context.transport().pos_seconds(),
            beats: context.transport().pos_beats(),
            playing: context.transport().playing,
        });
        let offset = self.config.source_clock_offsets[self.source];
        let Some(now) = self.trace.sample(offset) else {
            return false;
        };
        let end = now + i64::from(self.trace.clock.length);
        if self.trace.clock.frames > self.max_frames || self.trace.clock.length == 0 {
            self.trace.fault("processing_interval_outside_activation", 0, now, true);
            return false;
        }
        let epoch = self.shared.epoch.load(Ordering::Acquire);
        if epoch != self.epoch {
            if !self.pending.is_empty() || self.held() > 0 {
                self.trace.fault("epoch_changed_with_lifetimes_requires_host_reset", 0, now, true);
                return false;
            }
            self.epoch = epoch;
        }
        for _ in 0..MAX_EVENTS {
            let Some(event) = context.next_event() else {
                break;
            };
            if event.timing() >= self.trace.clock.length
                || event.channel().is_some_and(|c| c > 15)
                || event_key(&event).is_some_and(|k| k > 127)
            {
                self.trace.fault("unsupported_event_address_or_time", 0, now, true);
                return false;
            }
            if self.pending.len() == MAX_EVENTS {
                self.trace.fault("pending_capacity_run_aborted", 0, now, true);
                return false;
            }
            self.sequence += 1;
            let input = now + i64::from(event.timing());
            let deadline = input + i64::from(self.config.delay_samples);
            let index = Self::index(&event);
            let mut request = index.map(|i| self.input_voices[i]).unwrap_or(0);
            let attack = matches!(event, NoteEvent::NoteOn { .. });
            if let NoteEvent::NoteOn { channel, note, .. } = event {
                self.next_request += 1;
                request = self.next_request;
                let index = index.unwrap();
                if self.input_voices[index] != 0 {
                    self.trace.fault("same_key_overlap_without_voice_info", request, input, true);
                    return false;
                }
                self.input_voices[index] = request;
                let onset = Request {
                    source: self.source,
                    generation: self.generation,
                    epoch: self.epoch,
                    id: request,
                    input,
                    deadline,
                    channel,
                    key: note,
                };
                if self.ports.as_mut().unwrap().input.push(Intent::Request(onset)).is_err() {
                    self.trace.fault("input_queue_full_no_watermark", request, input, true);
                    return false;
                }
            }
            self.trace.record(Data::Input {
                sequence: self.sequence,
                request,
                sample: input,
                deadline,
                event_kind: event_name(&event),
                note_id: event.voice_id(),
                channel: event.channel(),
                key: event_key(&event),
            });
            if matches!(event, NoteEvent::NoteOff { .. } | NoteEvent::Choke { .. }) {
                if let Some(index) = index {
                    self.input_voices[index] = 0;
                }
            }
            if matches!(event, NoteEvent::PolyTuning { .. }) && request == 0 {
                self.trace.fault("tuning_without_addressed_attack", 0, input, true);
                return false;
            }
            self.pending.push_back(Pending {
                event,
                request,
                input,
                deadline,
                correction: if attack { None } else { Some(0.0) },
                diagnosed: false,
            });
        }
        // Raw input count is capped at the enclosing callback, so reaching this
        // bound cannot hide an unconsumed event behind a published watermark.
        if self.trace.fatal {
            return false;
        }
        if self
            .ports
            .as_mut()
            .unwrap()
            .input
            .push(Intent::Progress {
                generation: self.generation,
                epoch: self.epoch,
                from: now,
                through: end,
            })
            .is_err()
        {
            self.trace.fault("progress_queue_full", 0, end, true);
            return false;
        }
        self.trace.record(Data::Progress {
            source: self.source,
            generation: self.generation,
            from: now,
            through: end,
        });

        for _ in 0..QUEUE_CAPACITY {
            let Ok(reply) = self.ports.as_mut().unwrap().replies.pop() else {
                break;
            };
            let r = reply.request;
            if r.generation != self.generation || r.epoch != self.epoch {
                self.trace.fault("obsolete_reply_rejected", r.id, now, false);
                continue;
            }
            let pending = self
                .pending
                .iter_mut()
                .find(|p| p.request == r.id && matches!(p.event, NoteEvent::NoteOn { .. }));
            if let Some(pending) = pending {
                pending.correction = Some(reply.correction);
                self.trace.record(Data::ReplyVisible {
                    generation: self.generation,
                    request: r.id,
                    input: r.input,
                    deadline: r.deadline,
                });
            } else {
                self.trace.fault("reply_without_pending_attack", r.id, now, false);
            }
        }

        for _ in 0..MAX_EVENTS {
            let Some(mut head) = self.pending.front().copied() else {
                break;
            };
            let planned = head.deadline + self.extra_shift;
            if planned >= end {
                break;
            }
            if head.correction.is_none() {
                if !head.diagnosed {
                    self.trace.fault(
                        "assignment_deadline_missed",
                        head.request,
                        head.deadline,
                        false,
                    );
                    self.pending.front_mut().unwrap().diagnosed = true;
                }
                break;
            }
            if planned < now {
                if matches!(head.event, NoteEvent::NoteOn { .. }) && !head.diagnosed {
                    self.trace.fault(
                        "assignment_or_callback_late",
                        head.request,
                        head.deadline,
                        false,
                    );
                }
                // Probe-only rule: translate the remaining stream together and
                // keep that extra delay until reset. #616 chooses recovery.
                self.extra_shift += now - planned;
            }
            let actual = head.deadline + self.extra_shift;
            let timing = (actual - now) as u32;
            if !retime(&mut head.event, timing) {
                self.trace.fault("unknown_event_type", head.request, actual, true);
                return false;
            }
            let index = Self::index(&head.event);
            if let NoteEvent::PolyTuning { tuning, voice_id, .. } = &mut head.event {
                if let Some(index) = index {
                    let voice = self.output_voices[index];
                    if voice.request != head.request
                        || (voice_id.is_some() && voice.note_id != *voice_id)
                    {
                        self.trace.fault(
                            "expression_lifetime_mismatch",
                            head.request,
                            actual,
                            true,
                        );
                        return false;
                    }
                    *tuning += voice.correction;
                }
            }
            self.trace.record(Data::PlannedOutput {
                request: head.request,
                input: head.input,
                deadline: head.deadline,
                actual,
                extra_shift: self.extra_shift,
            });
            context.send_event(head.event);
            match head.event {
                NoteEvent::NoteOn { voice_id, channel, note, .. } => {
                    let correction = head.correction.unwrap();
                    self.output_voices[index.unwrap()] =
                        Voice { request: head.request, note_id: voice_id, correction };
                    context.send_event(NoteEvent::PolyTuning {
                        timing,
                        voice_id,
                        channel,
                        note,
                        tuning: correction,
                    });
                }
                NoteEvent::NoteOff { .. } | NoteEvent::Choke { .. } => {
                    if let Some(index) = index {
                        self.output_voices[index] = Voice::default();
                    }
                }
                _ => {}
            }
            self.pending.pop_front();
        }
        !self.trace.fatal
    }
}

impl Plugin for HarmonigraphTune {
    const NAME: &'static str = "Harmonigraph Tune — probe";
    const VENDOR: &'static str = "Yan Han";
    const URL: &'static str = "https://github.com/yan-h/harmonigraph/issues/615";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");
    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[];
    const MIDI_INPUT: MidiConfig = MidiConfig::MidiCCs;
    const MIDI_OUTPUT: MidiConfig = MidiConfig::MidiCCs;
    const SAMPLE_ACCURATE_AUTOMATION: bool = true;
    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn initialize(
        &mut self,
        _layout: &AudioIOLayout,
        buffer: &BufferConfig,
        context: &mut impl InitContext<Self>,
    ) -> bool {
        if self.ports.is_none() {
            self.trace.fault("source_capacity", 0, -1, true);
            return false;
        }
        let Ok(config) = Config::read() else {
            self.trace.fault("invalid_config", 0, -1, true);
            return false;
        };
        self.config = config;
        self.max_frames = buffer.max_buffer_size;
        context.set_latency_samples(self.config.delay_samples);
        self.shared.active[self.source].store(true, Ordering::Release);
        self.trace.record(Data::Activation {
            rate: buffer.sample_rate,
            min: buffer.min_buffer_size,
            max: buffer.max_buffer_size,
            offline: buffer.process_mode == ProcessMode::Offline,
            delay: self.config.delay_samples,
            sources: self.config.expected_sources,
            clock_offset: self.config.source_clock_offsets[self.source],
        });
        !self.trace.fatal
    }

    fn reset(&mut self) {
        self.trace.record(Data::Lifecycle {
            name: "source_reset",
            generation: self.generation,
            pending: self.pending.len(),
            held: self.held(),
        });
        if self.source >= super::MAX_SOURCES {
            return;
        }
        self.generation = self.shared.generations[self.source].fetch_add(1, Ordering::AcqRel) + 1;
        self.epoch = self.shared.epoch.load(Ordering::Acquire);
        self.pending.clear();
        self.input_voices.fill(0);
        self.output_voices.fill(Voice::default());
        self.next_request = 0;
        self.extra_shift = 0;
    }

    fn process(
        &mut self,
        _buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        if self.trace.fatal || !self.process_events(context) {
            return ProcessStatus::Error("#615 probe run invalid; inspect trace");
        }
        if self.config.keep_alive {
            ProcessStatus::KeepAlive
        } else {
            ProcessStatus::Normal
        }
    }

    fn deactivate(&mut self) {
        if self.source < super::MAX_SOURCES {
            self.shared.active[self.source].store(false, Ordering::Release);
        }
        self.trace.record(Data::Lifecycle {
            name: "source_deactivate",
            generation: self.generation,
            pending: self.pending.len(),
            held: self.held(),
        });
    }
}

impl ClapPlugin for HarmonigraphTune {
    const CLAP_ID: &'static str = "com.yanhan.harmonigraph-tune-probe";
    const CLAP_DESCRIPTION: Option<&'static str> =
        Some("Disposable #615 central +50-cent timing experiment");
    const CLAP_MANUAL_URL: Option<&'static str> = Some(Self::URL);
    const CLAP_SUPPORT_URL: Option<&'static str> = Some(Self::URL);
    const CLAP_FEATURES: &'static [ClapFeature] = &[ClapFeature::NoteEffect, ClapFeature::Utility];
    const CLAP_PROCESS_TRACE: bool = true;
    fn clap_process_trace(&mut self, event: ProcessTrace<'_>) {
        self.trace.hook(event);
    }
}

impl Drop for HarmonigraphTune {
    fn drop(&mut self) {
        self.deactivate();
        if let Some(ports) = self.ports.take() {
            registry().lock().unwrap().return_source(self.source, ports);
        }
    }
}
