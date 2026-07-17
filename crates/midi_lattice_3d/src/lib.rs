//! The nice-plug shell: parameters, MIDI ingestion, and plugin-format
//! exports. All interesting logic lives in the `lattice-*` crates; this
//! crate only adapts them to the plugin world.

use std::num::NonZeroU32;
use std::sync::Arc;

use lattice_core::notes::{NoteEvent as CoreNoteEvent, NoteEventKind};
use lattice_ui::params::{ParamBackend, ParamKey};
use nice_plug::prelude::*;
use parking_lot::Mutex;

mod editor;

/// Capacity of the audio→GUI note event ring buffer. Events are dropped
/// (silently) if the GUI stalls long enough to fill it.
const EVENT_RING_CAPACITY: usize = 4096;

pub struct MidiLattice3d {
    params: Arc<MidiLattice3dParams>,
    note_producer: rtrb::Producer<CoreNoteEvent>,
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
    #[id = "highlight-time"]
    pub highlight_time: FloatParam,
    #[id = "octave-highlight-time"]
    pub octave_highlight_time: FloatParam,
    #[id = "darkest-pitch"]
    pub darkest_pitch: FloatParam,
    #[id = "brightest-pitch"]
    pub brightest_pitch: FloatParam,
}

fn linear_param(name: &str, key: ParamKey, default: f32) -> FloatParam {
    let range = key.range();
    FloatParam::new(
        name,
        default,
        FloatRange::Linear { min: *range.start(), max: *range.end() },
    )
}

impl Default for MidiLattice3dParams {
    fn default() -> Self {
        MidiLattice3dParams {
            editor_state: editor::EguiState::from_size(1000, 700),
            ui_state: Arc::new(parking_lot::RwLock::new(String::new())),
            // 12-TET defaults, like v1: the lattice matches what a plain
            // MIDI keyboard sends until the user dials in a tuning.
            c_offset: linear_param("C Offset (cents)", ParamKey::COffset, 0.0),
            three: linear_param(
                "Perfect Fifth (cents)",
                ParamKey::Three,
                lattice_core::tuning::THREE_12TET,
            ),
            five: linear_param(
                "Major Third (cents)",
                ParamKey::Five,
                lattice_core::tuning::FIVE_12TET,
            ),
            seven: linear_param(
                "Harmonic Seventh (cents)",
                ParamKey::Seven,
                lattice_core::tuning::SEVEN_12TET,
            ),
            tolerance: FloatParam::new(
                "Tuning Tolerance (cents)",
                0.5,
                FloatRange::Skewed {
                    min: 0.001,
                    max: 49.999,
                    factor: FloatRange::skew_factor(-2.5),
                },
            ),
            highlight_time: FloatParam::new(
                "Note Highlight (sec)",
                1.0,
                FloatRange::Skewed {
                    min: 0.0,
                    max: 100.0,
                    factor: FloatRange::skew_factor(-2.0),
                },
            ),
            octave_highlight_time: FloatParam::new(
                "Octave Fade (sec)",
                1.0,
                FloatRange::Skewed {
                    min: 0.0,
                    max: 100.0,
                    factor: FloatRange::skew_factor(-2.0),
                },
            ),
            // Pitch-height coloring range for channels 10-14 (MIDI
            // convention), as in v1.
            darkest_pitch: linear_param("Darkest Pitch", ParamKey::DarkestPitch, 24.0),
            brightest_pitch: linear_param("Brightest Pitch", ParamKey::BrightestPitch, 108.0),
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
            ParamKey::HighlightTime => &self.highlight_time,
            ParamKey::OctaveHighlightTime => &self.octave_highlight_time,
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
        MidiLattice3d {
            params: Arc::new(MidiLattice3dParams::default()),
            note_producer: producer,
            editor_shared: Arc::new(Mutex::new(editor::EditorShared::new(consumer))),
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

    // Audio passes through untouched; the plugin only inspects MIDI.
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
        true
    }

    fn reset(&mut self) {
        self.samples_processed = 0;
        // TODO: signal the GUI to clear held notes (e.g. a Reset event in
        // the ring buffer); after a transport reset, note-offs may never
        // arrive and voices would stick until their next note-on.
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
