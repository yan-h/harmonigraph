//! The nice-plug shell: parameters, MIDI ingestion, and plugin-format
//! exports. All interesting logic lives in the `lattice-*` crates; this
//! crate only adapts them to the plugin world.

use std::num::NonZeroU32;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;

use harmonigraph_core::notes::{NoteEvent as CoreNoteEvent, NoteEventKind};
use harmonigraph_ui::params::{ParamBackend, ParamKey};
use nice_plug::prelude::*;
use parking_lot::Mutex;

mod editor;
mod take;

/// Capacity of the audio→GUI note event ring buffer. Events are dropped
/// (silently) if the GUI stalls long enough to fill it.
const EVENT_RING_CAPACITY: usize = 4096;

/// Capacity of the audio→GUI sample ring feeding the Spectral pane's
/// analyzer: >1 s of STEREO at 48 kHz. Overflow just drops frames — a spectrum
/// meter would rather skip than stall the audio thread.
///
/// Doubled when the ring stopped carrying a mono mixdown and started carrying
/// the channels themselves (see `process`), so the seconds it holds — which is
/// what bounds the GUI's catch-up work after a stall, see
/// `AudioSpectrum::push_samples` — stayed where it was.
const AUDIO_RING_CAPACITY: usize = 131_072;

/// Sample rate assumed until the host reports the real one in `initialize`.
/// Held in two representations (the f64 `sample_rate` field and the f32 bit
/// pattern in `sample_rate_bits`), so derive both from one constant.
const DEFAULT_SAMPLE_RATE: f64 = 44_100.0;

pub struct Harmonigraph {
    params: Arc<HarmonigraphParams>,
    note_producer: rtrb::Producer<CoreNoteEvent>,
    /// The input bus, interleaved, for the GUI's spectrum analyzer.
    audio_producer: rtrb::Producer<f32>,
    /// Current sample rate as f32 bits, so the GUI folds FFT bins under
    /// the clock the samples were actually taken at.
    sample_rate_bits: Arc<AtomicU32>,
    /// How many channels the ring's frames carry, so the GUI can de-interleave
    /// them. Written every block; the host can renegotiate the bus layout, and a
    /// GUI de-interleaving by a stale count would read one channel as two.
    audio_channels: Arc<AtomicU32>,
    /// State shared with the editor; created eagerly so the ring buffer's
    /// consumer end has somewhere to live before the GUI opens.
    editor_shared: Arc<Mutex<editor::EditorShared>>,
    sample_rate: f64,
    samples_processed: u64,
    /// Take recording (see `take`). The recorder is always present; it
    /// only writes while the user has armed it from the Video pane.
    take: take::Recorder,
    /// Count of events recorded in the current take, for the UI's status
    /// line. Reset when recording starts.
    take_events: Arc<AtomicU64>,
}

#[derive(Params)]
pub struct HarmonigraphParams {
    /// Window size in logical pixels, persisted with the plugin state.
    #[persist = "editor-state"]
    pub editor_state: Arc<editor::EguiState>,

    /// Serialized UI state (dock layout, camera, view settings), persisted
    /// with the plugin state. See SharedState::save_persist.
    #[persist = "ui-state"]
    pub ui_state: Arc<parking_lot::RwLock<String>>,

    #[id = "tuning-c-offset"]
    pub c_offset: FloatParam,
    #[id = "tuning-three"]
    pub three: FloatParam,
    #[id = "tuning-five"]
    pub five: FloatParam,
    #[id = "tuning-seven"]
    pub seven: FloatParam,
    #[id = "tuning-tolerance"]
    pub tolerance: FloatParam,
    /// Keeps the pre-merge id: this used to be the pitch class's own fade,
    /// and it now drives every layer. Reusing the id means projects
    /// automating it keep their value instead of snapping back to the
    /// default. (The retired "octave-fade" id is simply ignored on load.)
    #[id = "pitch-class-fade"]
    pub fade: FloatParam,
    #[id = "darkest-pitch"]
    pub darkest_pitch: FloatParam,
    #[id = "brightest-pitch"]
    pub brightest_pitch: FloatParam,
}

/// Build a FloatParam entirely from the ParamKey metadata (name, default,
/// range, skew) so the host-facing parameters can never drift from what
/// the UI clamps and displays.
fn param_for_key(key: ParamKey) -> FloatParam {
    let range = key.range();
    let (min, max) = (*range.start(), *range.end());
    let range = if key.logarithmic() {
        FloatRange::Skewed { min, max, factor: FloatRange::skew_factor(key.skew_steepness()) }
    } else {
        FloatRange::Linear { min, max }
    };
    FloatParam::new(key.host_name(), key.default_value(), range)
}

impl Default for HarmonigraphParams {
    fn default() -> Self {
        HarmonigraphParams {
            editor_state: editor::EguiState::from_size(
                editor::DEFAULT_SIZE.0,
                editor::DEFAULT_SIZE.1,
            ),
            ui_state: Arc::new(parking_lot::RwLock::new(String::new())),
            c_offset: param_for_key(ParamKey::COffset),
            three: param_for_key(ParamKey::Three),
            five: param_for_key(ParamKey::Five),
            seven: param_for_key(ParamKey::Seven),
            tolerance: param_for_key(ParamKey::Tolerance),
            fade: param_for_key(ParamKey::Fade),
            darkest_pitch: param_for_key(ParamKey::DarkestPitch),
            brightest_pitch: param_for_key(ParamKey::BrightestPitch),
        }
    }
}

impl HarmonigraphParams {
    fn param_for(&self, key: ParamKey) -> &FloatParam {
        match key {
            ParamKey::COffset => &self.c_offset,
            ParamKey::Three => &self.three,
            ParamKey::Five => &self.five,
            ParamKey::Seven => &self.seven,
            ParamKey::Tolerance => &self.tolerance,
            ParamKey::Fade => &self.fade,
            ParamKey::DarkestPitch => &self.darkest_pitch,
            ParamKey::BrightestPitch => &self.brightest_pitch,
        }
    }
}

/// Adapts nice-plug's parameter system to the UI's `ParamBackend`, so panes
/// don't know they're running inside a plugin.
pub(crate) struct PluginParamBackend<'a> {
    pub params: &'a HarmonigraphParams,
    pub setter: &'a ParamSetter<'a>,
    /// The key currently inside an explicit begin_set/end_set gesture, if
    /// any. Lives in EditorShared so it survives across frames (this
    /// adapter is rebuilt every frame).
    pub gesture: &'a std::cell::Cell<Option<ParamKey>>,
}

impl ParamBackend for PluginParamBackend<'_> {
    fn get(&self, key: ParamKey) -> f32 {
        self.params.param_for(key).value()
    }

    fn set(&self, key: ParamKey, value: f32) {
        let param = self.params.param_for(key);
        if self.gesture.get() == Some(key) {
            // Inside an explicit gesture (drag): just set.
            self.setter.set_parameter(param, value);
        } else {
            // One-shot change: implicit single-value gesture.
            self.setter.begin_set_parameter(param);
            self.setter.set_parameter(param, value);
            self.setter.end_set_parameter(param);
        }
    }

    fn begin_set(&self, key: ParamKey) {
        // Close a dangling gesture first (shouldn't happen, but a host
        // seeing unbalanced begin/end is worse than a spurious end).
        if let Some(previous) = self.gesture.get() {
            self.setter.end_set_parameter(self.params.param_for(previous));
        }
        self.setter.begin_set_parameter(self.params.param_for(key));
        self.gesture.set(Some(key));
    }

    fn end_set(&self, key: ParamKey) {
        if self.gesture.get() == Some(key) {
            self.setter.end_set_parameter(self.params.param_for(key));
            self.gesture.set(None);
        }
    }
}

impl Default for Harmonigraph {
    fn default() -> Self {
        let (producer, consumer) = rtrb::RingBuffer::new(EVENT_RING_CAPACITY);
        let (audio_producer, audio_consumer) = rtrb::RingBuffer::new(AUDIO_RING_CAPACITY);
        let sample_rate_bits = Arc::new(AtomicU32::new((DEFAULT_SAMPLE_RATE as f32).to_bits()));
        // Mono until a block says otherwise: the safe guess, since reading a
        // stereo ring as mono only halves the pitch of what it draws for one
        // block, while reading mono as stereo would de-interleave silence.
        let audio_channels = Arc::new(AtomicU32::new(1));
        let (take, take_control) = take::channel();
        let take_events = Arc::new(AtomicU64::new(0));
        Harmonigraph {
            params: Arc::new(HarmonigraphParams::default()),
            note_producer: producer,
            audio_producer,
            sample_rate_bits: sample_rate_bits.clone(),
            audio_channels: audio_channels.clone(),
            editor_shared: Arc::new(Mutex::new(editor::EditorShared::new(
                consumer,
                audio_consumer,
                sample_rate_bits,
                audio_channels,
                take_control,
                take_events.clone(),
            ))),
            sample_rate: DEFAULT_SAMPLE_RATE,
            samples_processed: 0,
            take,
            take_events,
        }
    }
}

impl Plugin for Harmonigraph {
    const NAME: &'static str = "Harmonigraph";
    const VENDOR: &'static str = "Yan Han";
    const URL: &'static str = env!("CARGO_PKG_HOMEPAGE");
    const EMAIL: &'static str = "yanhan13@gmail.com";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    // Audio passes through untouched, but is tapped (mono mixdown) for the
    // GUI's spectrum analyzer; MIDI is forwarded verbatim.
    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: NonZeroU32::new(2),
        main_output_channels: NonZeroU32::new(2),
        ..AudioIOLayout::const_default()
    }];

    const MIDI_INPUT: MidiConfig = MidiConfig::Basic;
    const MIDI_OUTPUT: MidiConfig = MidiConfig::Basic;

    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        editor::create(self.params.clone(), self.editor_shared.clone())
    }

    fn initialize(
        &mut self,
        _audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        _context: &mut impl InitContext<Self>,
    ) -> bool {
        self.sample_rate = f64::from(buffer_config.sample_rate);
        self.sample_rate_bits.store(buffer_config.sample_rate.to_bits(), Ordering::Relaxed);
        true
    }

    fn reset(&mut self) {
        self.samples_processed = 0;
        // After a transport reset, note-offs for held notes may never
        // arrive; tell the GUI to release everything. Time 0.0 is the
        // restarted sample clock's epoch (the editor's ClockMapper snaps
        // its offset on a jump this large).
        let _ = self.note_producer.push(CoreNoteEvent {
            time: 0.0,
            channel: 0,
            note: 0,
            kind: NoteEventKind::AllOff,
        });

    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let block_start = self.samples_processed;
        // Take timestamps come from the TRANSPORT, not the plugin's own
        // sample counter, so a take lines up with a bounce of the same
        // song without an offset to work out. Nothing is recorded while
        // the transport is stopped; a host that reports no position at
        // all falls back to the local clock so "just record what I play"
        // still works.
        let transport = context.transport();
        let take_origin = if !self.take.is_armed() {
            None
        } else if let Some(seconds) = transport.pos_seconds() {
            // observe_transport decides whether the transport is rolling (and
            // splits the take on a loop wrap, or ends it at the loop end under
            // AtLoopEnd). It is deliberately more permissive than the host's
            // `playing` flag — see there.
            self.take.observe_transport(seconds, transport.playing).then_some(seconds)
        } else {
            // No transport at all: fall back to the plugin's own clock so
            // "just record what I play" still works.
            Some(block_start as f64 / self.sample_rate)
        };

        while let Some(event) = context.next_event() {
            let mapped = match event {
                NoteEvent::NoteOn { timing, channel, note, velocity, .. } => {
                    Some((timing, channel, note, NoteEventKind::On { velocity }))
                }
                NoteEvent::NoteOff { timing, channel, note, .. }
                | NoteEvent::Choke { timing, channel, note, .. } => {
                    Some((timing, channel, note, NoteEventKind::Off))
                }
                // Per-note tuning (CLAP note expression / MPE via the
                // host); v1 supported this as PolyTuning.
                NoteEvent::PolyTuning { timing, channel, note, tuning, .. } => {
                    Some((timing, channel, note, NoteEventKind::Tuning { semitones: tuning }))
                }
                _ => None,
            };
            if let Some((timing, channel, note, kind)) = mapped {
                let time = (block_start + u64::from(timing)) as f64 / self.sample_rate;
                // Full ring = GUI stalled; dropping visualization events is
                // the right failure mode for the audio thread.
                let _ = self.note_producer.push(CoreNoteEvent { time, channel, note, kind });
                if let Some(origin) = take_origin {
                    self.take.note(
                        origin + f64::from(timing) / self.sample_rate,
                        channel,
                        note,
                        kind,
                    );
                    self.take_events.fetch_add(1, Ordering::Relaxed);
                }
            }
            // Behave as a transparent MIDI effect.
            context.send_event(event);
        }

        // The (pass-through) input for the GUI's spectrum analyzer, INTERLEAVED:
        // it analyzes the channels separately and combines them in the power
        // domain, so a mixdown here would cancel anti-phase content before it
        // could (see `harmonigraph_core::spectrum::ChannelBank`). A full ring — editor
        // closed, or its thread stalled — silently drops frames, the same
        // failure mode as the note ring.
        let channels = buffer.channels();
        if channels > 0 {
            self.audio_channels.store(channels as u32, Ordering::Relaxed);
            // Reserve the block's slots once and fill them, rather than a
            // per-sample push(): one ring-atomic touch per block instead of
            // ~48k/s on the audio thread. Bound the reservation by free space
            // so a near-full ring (GUI stalled/closed) drops the block's tail
            // — the same "drop rather than stall" failure mode as before, just
            // at block granularity. `slots()` only grows as the consumer
            // drains, so the reservation of `want` never fails; the `if let`
            // is defensive.
            //
            // Rounded DOWN TO WHOLE FRAMES, which is the one thing this must not
            // get wrong: a tail dropped mid-frame would leave the ring one sample
            // out of phase, and every later frame would hand the left channel's
            // data to the right for as long as the plugin ran.
            let free = self.audio_producer.slots() / channels * channels;
            let want = (buffer.samples() * channels).min(free);
            if want > 0 {
                // Interleaved from the per-channel planes the host gave us. A
                // shared slice-of-slices rather than `iter_samples`, because a
                // frame view cannot be flat-mapped without collecting it — and
                // this is the audio thread, where the allocation that would take
                // is exactly what is not allowed.
                let planes = buffer.as_slice_immutable();
                let frames = want / channels;
                if let Ok(chunk) = self.audio_producer.write_chunk_uninit(want) {
                    chunk.fill_from_iter(
                        (0..frames).flat_map(|f| (0..channels).map(move |c| planes[c][f])),
                    );
                }
            }
        }

        if let Some(origin) = take_origin {
            self.take
                .params(origin, ParamKey::ALL.map(|key| self.params.param_for(key).value()));

            // The take's own audio, when asked for: the input bus exactly
            // as it arrives, so the render gets a spectrum and a
            // soundtrack without a separate bounce. Always stereo,
            // matching AUDIO_IO_LAYOUTS — a mono host input is
            // duplicated rather than desyncing the WAV's frames.
            if self.take.wants_audio() && channels > 0 {
                self.take.mark_audio_start(origin);
                let right = usize::from(channels > 1);
                let wanted = buffer.samples() * 2;
                let mut interleaved = buffer.iter_samples().flat_map(|mut frame| {
                    let l = frame.get_mut(0).map_or(0.0, |s| *s);
                    let r = frame.get_mut(right).map_or(0.0, |s| *s);
                    [l, r]
                });
                self.take.audio(&mut interleaved, wanted);
            }
        }

        self.samples_processed += buffer.samples() as u64;
        ProcessStatus::Normal
    }
}

impl ClapPlugin for Harmonigraph {
    const CLAP_ID: &'static str = "com.yan-h.harmonigraph";
    const CLAP_DESCRIPTION: Option<&'static str> =
        Some("Tonnetz harmony and spectrum visualizer");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::NoteEffect,
        ClapFeature::Analyzer,
        ClapFeature::Utility,
    ];
}

impl Vst3Plugin for Harmonigraph {
    const VST3_CLASS_ID: [u8; 16] = *b"HarmonigraphYanH";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Fx, Vst3SubCategory::Analyzer];
}

nice_export_clap!(Harmonigraph);
nice_export_vst3!(Harmonigraph);

#[cfg(test)]
mod tests {
    use super::*;

    /// `ParamKey::id` is what a recorded take names its automation by, and
    /// what the host names its automation lanes by. They are declared in
    /// two places — the enum's `id()` and the `#[id = "..."]` attributes
    /// here — and if they drift, takes silently replay with default
    /// parameters and projects silently lose their automation. Neither
    /// failure is visible until someone watches a render and wonders why
    /// the tuning is wrong.
    #[test]
    fn every_param_key_id_matches_the_host_facing_id() {
        let params = HarmonigraphParams::default();
        let host_ids: Vec<String> =
            params.param_map().into_iter().map(|(id, _, _)| id).collect();
        for key in ParamKey::ALL {
            assert!(
                host_ids.iter().any(|id| id == key.id()),
                "ParamKey::{key:?} claims id {:?}, which no #[id] attribute declares; \
                 the host exposes {host_ids:?}",
                key.id(),
            );
        }
        assert_eq!(
            host_ids.len(),
            ParamKey::ALL.len(),
            "the plugin exposes parameters ParamKey doesn't know about: {host_ids:?}"
        );
    }
}
