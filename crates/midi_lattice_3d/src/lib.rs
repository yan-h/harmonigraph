//! The nice-plug shell: parameters, MIDI ingestion, and plugin-format
//! exports. All interesting logic lives in the `lattice-*` crates; this
//! crate only adapts them to the plugin world.

use std::num::NonZeroU32;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use lattice_core::notes::{NoteEvent as CoreNoteEvent, NoteEventKind};
use lattice_ui::params::{ParamBackend, ParamKey};
use nice_plug::prelude::*;
use parking_lot::Mutex;

mod editor;

/// Capacity of the audio→GUI note event ring buffer. Events are dropped
/// (silently) if the GUI stalls long enough to fill it.
const EVENT_RING_CAPACITY: usize = 4096;

/// Capacity of the audio→GUI sample ring feeding the Spectral pane's
/// analyzer: >1 s at 48 kHz. Overflow just drops samples — a spectrum
/// meter would rather skip than stall the audio thread.
const AUDIO_RING_CAPACITY: usize = 65_536;

pub struct MidiLattice3d {
    params: Arc<MidiLattice3dParams>,
    note_producer: rtrb::Producer<CoreNoteEvent>,
    /// Mono mixdown of the input bus, for the GUI's spectrum analyzer.
    audio_producer: rtrb::Producer<f32>,
    /// Current sample rate as f32 bits, so the GUI folds FFT bins under
    /// the clock the samples were actually taken at.
    sample_rate_bits: Arc<AtomicU32>,
    /// State shared with the editor; created eagerly so the ring buffer's
    /// consumer end has somewhere to live before the GUI opens.
    editor_shared: Arc<Mutex<editor::EditorShared>>,
    sample_rate: f64,
    samples_processed: u64,
}

#[derive(Params)]
pub struct MidiLattice3dParams {
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

impl Default for MidiLattice3dParams {
    fn default() -> Self {
        MidiLattice3dParams {
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

impl MidiLattice3dParams {
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
    pub params: &'a MidiLattice3dParams,
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

impl Default for MidiLattice3d {
    fn default() -> Self {
        let (producer, consumer) = rtrb::RingBuffer::new(EVENT_RING_CAPACITY);
        let (audio_producer, audio_consumer) = rtrb::RingBuffer::new(AUDIO_RING_CAPACITY);
        let sample_rate_bits = Arc::new(AtomicU32::new(44_100.0f32.to_bits()));
        MidiLattice3d {
            params: Arc::new(MidiLattice3dParams::default()),
            note_producer: producer,
            audio_producer,
            sample_rate_bits: sample_rate_bits.clone(),
            editor_shared: Arc::new(Mutex::new(editor::EditorShared::new(
                consumer,
                audio_consumer,
                sample_rate_bits,
            ))),
            sample_rate: 44_100.0,
            samples_processed: 0,
        }
    }
}

impl Plugin for MidiLattice3d {
    const NAME: &'static str = "MIDI Lattice 3D";
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
            }
            // Behave as a transparent MIDI effect.
            context.send_event(event);
        }

        // Mono mixdown of the (pass-through) input for the GUI's spectrum
        // analyzer. A full ring — editor closed, or its thread stalled —
        // silently drops samples, the same failure mode as the note ring.
        let channels = buffer.channels();
        if channels > 0 {
            let gain = 1.0 / channels as f32;
            // Reserve the block's slots once and fill them, rather than a
            // per-sample push(): one ring-atomic touch per block instead of
            // ~48k/s on the audio thread. Bound the reservation by free space
            // so a near-full ring (GUI stalled/closed) drops the block's tail
            // — the same "drop rather than stall" failure mode as before, just
            // at block granularity. `slots()` only grows as the consumer
            // drains, so the reservation of `want` never fails; the `if let`
            // is defensive.
            let want = buffer.samples().min(self.audio_producer.slots());
            if want > 0 {
                if let Ok(chunk) = self.audio_producer.write_chunk_uninit(want) {
                    chunk.fill_from_iter(
                        buffer
                            .iter_samples()
                            .map(|mut frame| frame.iter_mut().map(|s| *s).sum::<f32>() * gain),
                    );
                }
            }
        }

        self.samples_processed += buffer.samples() as u64;
        ProcessStatus::Normal
    }
}

impl ClapPlugin for MidiLattice3d {
    const CLAP_ID: &'static str = "com.yan-h.midi-lattice-3d";
    const CLAP_DESCRIPTION: Option<&'static str> =
        Some("3D Tonnetz visualizer for incoming MIDI notes");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::NoteEffect,
        ClapFeature::Analyzer,
        ClapFeature::Utility,
    ];
}

impl Vst3Plugin for MidiLattice3d {
    const VST3_CLASS_ID: [u8; 16] = *b"MidiLatt3dYanHan";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Fx, Vst3SubCategory::Analyzer];
}

nice_export_clap!(MidiLattice3d);
nice_export_vst3!(MidiLattice3d);
