//! The nice-plug shell: parameters, MIDI ingestion, and plugin-format
//! exports. All interesting logic lives in the `harmonigraph-*` crates; this
//! crate only adapts them to the plugin world.

use std::num::NonZeroU32;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;

use harmonigraph_core::notes::{NoteEvent as CoreNoteEvent, NoteEventKind};
use harmonigraph_record::{interleaved_reservation, TAKE_CHANNELS};
use harmonigraph_ui::params::{AnalysisInput, ParamBackend, ParamKey};
use nice_plug::prelude::*;
use parking_lot::Mutex;

mod background;
mod editor;
#[cfg(feature = "tuning-probe")]
mod probe;
#[cfg(feature = "tuning-probe")]
use probe::HarmonigraphTune;

/// Capacity of the audio→GUI note event ring buffer. Events are dropped
/// (silently) if the GUI stalls long enough to fill it.
pub(crate) const EVENT_RING_CAPACITY: usize = 4096;

/// Capacity of the audio→GUI sample ring feeding the Spectral pane's
/// analyzer: >1 s of STEREO at 48 kHz. Overflow just drops frames — a spectrum
/// meter would rather skip than stall the audio thread.
///
/// Doubled when the ring stopped carrying a mono mixdown and started carrying
/// the channels themselves (see `process`), so the seconds it holds — which is
/// what bounds the GUI's catch-up work after a stall, see
/// `AudioSpectrum::push_samples` — stayed where it was.
///
/// It is also what paces [`background`], whose poll has to stay well inside the
/// span this holds; see `POLL` there.
pub(crate) const AUDIO_RING_CAPACITY: usize = 131_072;

/// Sample rate assumed until the host reports the real one in `initialize`.
/// Held in two representations (the f64 `sample_rate` field and the f32 bit
/// pattern in `sample_rate_bits`), so derive both from one constant.
const DEFAULT_SAMPLE_RATE: f64 = 44_100.0;

pub struct Harmonigraph {
    #[cfg(feature = "tuning-probe")]
    probe: probe::Hub,
    /// Keeps the spectrogram's history running while the editor window is
    /// closed (see [`background`]). Held only to be dropped with the plugin,
    /// which is what stops its thread.
    ///
    /// FIRST, because fields drop in declaration order and this one has to stop
    /// before any of what it touches is torn down. Declared last instead, the
    /// worker holds the surviving `Arc` once `editor_shared` drops, so the
    /// teardown races a live drain and `EditorShared` — `SharedState`, and any
    /// `egui::TextureHandle` still in its spectrogram surfaces — is destroyed on
    /// the analyzer thread rather than on the one dropping the plugin. That
    /// appears to be sound, but PR #112 was a texture freed from the wrong point
    /// and it cost a crash; ordering the field is cheaper than being sure.
    _background: background::BackgroundAnalyzer,
    params: Arc<HarmonigraphParams>,
    note_producer: rtrb::Producer<CoreNoteEvent>,
    /// The selected analysis input, interleaved, for the GUI's spectrum analyzer.
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
    /// Take recording (see `harmonigraph_record`). The recorder is always
    /// present; it only writes while the user has armed it from the Video pane.
    take: harmonigraph_record::Recorder,
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

    /// Which input feeds the analyzer and the audio recorded with a take. This
    /// is a host parameter rather than part of `ui_state`, so the audio thread
    /// can read it directly and a restored project applies it before an editor
    /// has ever opened. It is deliberately not automatable: changing sources is
    /// an editing decision, not a signal to splice into the spectrogram.
    #[id = "analysis-input"]
    pub analysis_input: EnumParam<AnalysisInputParam>,

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
    /// and it now drives every layer, at both ends of the note. Reusing the
    /// id means projects automating it keep their value instead of snapping
    /// back to the default. (The retired "octave-fade" id is simply ignored
    /// on load.)
    #[id = "pitch-class-fade"]
    pub fade: FloatParam,
    #[id = "darkest-pitch"]
    pub darkest_pitch: FloatParam,
    #[id = "brightest-pitch"]
    pub brightest_pitch: FloatParam,
}

/// Host-persisted form of the UI's analysis-input choice. Stable IDs make the
/// saved value independent of declaration order; the UI-side enum stays free
/// of a dependency on nice-plug.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Enum)]
pub enum AnalysisInputParam {
    #[id = "main"]
    Main,
    #[id = "sidechain"]
    Sidechain,
}

impl From<AnalysisInputParam> for AnalysisInput {
    fn from(input: AnalysisInputParam) -> Self {
        match input {
            AnalysisInputParam::Main => AnalysisInput::Main,
            AnalysisInputParam::Sidechain => AnalysisInput::Sidechain,
        }
    }
}

impl From<AnalysisInput> for AnalysisInputParam {
    fn from(input: AnalysisInput) -> Self {
        match input {
            AnalysisInput::Main => AnalysisInputParam::Main,
            AnalysisInput::Sidechain => AnalysisInputParam::Sidechain,
        }
    }
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
            analysis_input: EnumParam::new("Analysis Input", AnalysisInputParam::Main)
                .non_automatable(),
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

    fn analysis_input(&self) -> Option<AnalysisInput> {
        Some(self.params.analysis_input.value().into())
    }

    fn set_analysis_input(&self, input: AnalysisInput) {
        let param = &self.params.analysis_input;
        self.setter.begin_set_parameter(param);
        self.setter.set_parameter(param, input.into());
        self.setter.end_set_parameter(param);
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

// The decisions `process` makes that do not need a host.
//
// A focused fixture below reaches the real process wiring with live buffers,
// but these are the decisions whose edge cases are clearer as pure values — a
// ring left out of phase, a take stamped on the wrong clock — and exactly the
// class a build cannot catch and a listener notices late.

/// What this block's take timestamps hang off, before the recorder gets a say.
#[derive(Debug, Clone, Copy, PartialEq)]
enum OriginSource {
    /// Not armed: nothing is recorded this block. The recorder has still been
    /// consulted by then — see the `armed` argument, which only `is_armed` can
    /// supply and which cannot be skipped on a disarmed block.
    Idle,
    /// The host reports a position. Only `observe_transport` can say whether it
    /// counts, so the caller still has to ask.
    Transport(f64),
    /// The host reports no position at all: fall back to the plugin's own clock,
    /// so "just record what I play" still works.
    LocalClock(f64),
}

/// Take timestamps come from the TRANSPORT, not the plugin's own sample
/// counter, so a take lines up with a bounce of the same song without an offset
/// to work out. Nothing is recorded while the transport is stopped.
///
/// `pos_seconds` is read even when disarmed; it is a plain getter, so asking
/// for it on a block that will not use it costs nothing.
fn origin_source(
    armed: bool,
    pos_seconds: Option<f64>,
    block_start: u64,
    sample_rate: f64,
) -> OriginSource {
    if !armed {
        OriginSource::Idle
    } else if let Some(seconds) = pos_seconds {
        OriginSource::Transport(seconds)
    } else {
        OriginSource::LocalClock(block_start as f64 / sample_rate)
    }
}

/// When an event happened on the RING's clock: the plugin's own sample counter,
/// which is what the editor's `ClockMapper` reads.
fn ring_time(block_start: u64, timing: u32, sample_rate: f64) -> f64 {
    (block_start + u64::from(timing)) as f64 / sample_rate
}

/// When the same event happened on the TAKE's clock, which hangs off the
/// transport instead. The two bases differ by wherever the song sat when the
/// plugin loaded, and a take stamped on the ring's clock lines up with no
/// bounce of the same song.
fn take_time(origin: f64, timing: u32, sample_rate: f64) -> f64 {
    origin + f64::from(timing) / sample_rate
}

/// One host event, in the terms the lattice draws it in.
///
/// Named fields rather than a tuple, because `channel` and `note` are both
/// `u8` and adjacent: a tuple lets the destructuring site transpose them and
/// still compile, and every note-on then draws a channel as a pitch. `process`
/// has no test that would catch it.
#[derive(Debug, Clone, Copy, PartialEq)]
struct MappedNote {
    timing: u32,
    channel: u8,
    note: u8,
    kind: NoteEventKind,
}

/// The events the visualization cares about, and what each becomes. A host
/// sends far more than this; everything unmatched is forwarded untouched and
/// never reaches the lattice.
fn mapped_note(event: NoteEvent<()>) -> Option<MappedNote> {
    match event {
        NoteEvent::NoteOn { timing, channel, note, velocity, .. } => {
            Some(MappedNote { timing, channel, note, kind: NoteEventKind::On { velocity } })
        }
        NoteEvent::NoteOff { timing, channel, note, .. }
        | NoteEvent::Choke { timing, channel, note, .. } => {
            Some(MappedNote { timing, channel, note, kind: NoteEventKind::Off })
        }
        // Per-note tuning (CLAP note expression / MPE via the host); v1
        // supported this as PolyTuning.
        NoteEvent::PolyTuning { timing, channel, note, tuning, .. } => Some(MappedNote {
            timing,
            channel,
            note,
            kind: NoteEventKind::Tuning { semitones: tuning },
        }),
        _ => None,
    }
}

/// Which input plane the take's right channel reads from. A take's WAV is
/// always stereo, matching AUDIO_IO_LAYOUTS, so a mono input is duplicated
/// rather than written as half a frame — which would desync every frame after
/// it.
fn take_right_channel(channels: usize) -> usize {
    usize::from(channels > 1)
}

/// One stereo frame for the take WAV, from the same selected buffer the live
/// analyzer reads. The caller has already established that the buffer has at
/// least one channel and that `frame` is in range.
fn take_stereo_frame(input: &Buffer<'_>, frame: usize) -> [f32; TAKE_CHANNELS] {
    let planes = input.as_slice_immutable();
    [planes[0][frame], planes[take_right_channel(planes.len())][frame]]
}

/// One source decision for the whole process block. Both the live analyzer and
/// the take recorder read the returned buffer, so changing the source can never
/// put the live and offline pictures on opposite inputs. Missing auxiliary data
/// falls back only as a defensive layout guard; an advertised but unrouted
/// sidechain is a real zero-filled buffer and remains silence.
fn selected_analysis_input<'a, T>(
    main: &'a T,
    sidechain: Option<&'a T>,
    input: AnalysisInputParam,
) -> &'a T {
    match input {
        AnalysisInputParam::Main => main,
        AnalysisInputParam::Sidechain => sidechain.unwrap_or(main),
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
        let (take, take_control) = harmonigraph_record::channel();
        let take_events = Arc::new(AtomicU64::new(0));
        let params = Arc::new(HarmonigraphParams::default());
        let editor_shared = Arc::new(Mutex::new(editor::EditorShared::new(
            consumer,
            audio_consumer,
            sample_rate_bits.clone(),
            audio_channels.clone(),
            take_control,
            take_events.clone(),
        )));
        // From HERE, not from `initialize` or the editor: the point of the
        // thread is to cover the stretches nothing else does, and both of those
        // are stretches. `initialize` runs on activation, so a suspended plugin
        // would go dark; the editor is the very thing being covered for.
        //
        // `ui_state` goes with it because the same argument covers the project's
        // saved settings: the host restores them into that field at a moment of
        // its own choosing, and nothing but this thread is there to read them
        // before a window is built. See `background::Restore`.
        let _background = background::BackgroundAnalyzer::spawn(
            editor_shared.clone(),
            params.editor_state.clone(),
            params.ui_state.clone(),
        );
        Harmonigraph {
            #[cfg(feature = "tuning-probe")]
            probe: probe::Hub::default(),
            params,
            note_producer: producer,
            audio_producer,
            sample_rate_bits,
            audio_channels,
            editor_shared,
            sample_rate: DEFAULT_SAMPLE_RATE,
            samples_processed: 0,
            take,
            take_events,
            _background,
        }
    }
}

impl Plugin for Harmonigraph {
    const NAME: &'static str = "Harmonigraph";
    const VENDOR: &'static str = "Yan Han";
    const URL: &'static str = env!("CARGO_PKG_HOMEPAGE");
    // Empty because a personal address in a public tree is an address in a
    // scraper's index. VST3's factory info is the only thing that reads this —
    // CLAP has no email field — and it takes a blank one; contact goes through
    // the issue tracker that CLAP_SUPPORT_URL points hosts at.
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    // Main audio passes through untouched. The selected input is tapped for
    // the GUI's spectrum analyzer — INTERLEAVED, channel by channel, not as a
    // mixdown (see `process`, and `interleaved_reservation` for what that
    // costs). MIDI is forwarded verbatim.
    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: NonZeroU32::new(2),
        main_output_channels: NonZeroU32::new(2),
        aux_input_ports: &[new_nonzero_u32(2)],
        // nice-plug 0.1.5's `main_output_name` accidentally reads
        // `main_input`, so leave both main names unset to get the distinct
        // generated Input/Output pair. The sidechain name is unaffected.
        names: PortNames { aux_inputs: &["Visualization Input"], ..PortNames::const_default() },
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
        #[cfg(feature = "tuning-probe")]
        if !self.probe.initialize(buffer_config, _context.plugin_api()) {
            return false;
        }
        true
    }

    fn reset(&mut self) {
        #[cfg(feature = "tuning-probe")]
        self.probe.reset();
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
        aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        #[cfg(feature = "tuning-probe")]
        if !self.probe.process(context.transport()) {
            return ProcessStatus::Error("#615 hub probe run invalid; inspect trace");
        }
        let block_start = self.samples_processed;
        let block_samples = buffer.samples();
        let transport = context.transport();
        // EVERY block, armed or not. `is_armed` is an edge detector, not a
        // getter: it latches `was_armed` and clears the recorder's per-take
        // state on the arming edge. Skipping it on a disarmed block means the
        // next arm edge never fires and recording silently never resumes.
        let armed = self.take.is_armed();
        let take_origin =
            match origin_source(armed, transport.pos_seconds(), block_start, self.sample_rate) {
                OriginSource::Idle => None,
                // observe_transport decides whether the transport is rolling (and
                // splits the take on a loop wrap, or ends it at a backward
                // jump under the one-file triggers). It is deliberately more
                // permissive than the host's `playing` flag — see there.
                OriginSource::Transport(seconds) => {
                    self.take.observe_transport(seconds, transport.playing).then_some(seconds)
                }
                OriginSource::LocalClock(seconds) => Some(seconds),
            };

        while let Some(event) = context.next_event() {
            if let Some(MappedNote { timing, channel, note, kind }) = mapped_note(event) {
                let time = ring_time(block_start, timing, self.sample_rate);
                // Full ring = GUI stalled; dropping visualization events is
                // the right failure mode for the audio thread.
                let _ = self.note_producer.push(CoreNoteEvent { time, channel, note, kind });
                if let Some(origin) = take_origin {
                    self.take.note(
                        take_time(origin, timing, self.sample_rate),
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

        // The selected input for the GUI's spectrum analyzer, INTERLEAVED:
        // it analyzes the channels separately and combines them in the power
        // domain, so a mixdown here would cancel anti-phase content before it
        // could (see `harmonigraph_core::spectrum::ChannelBank`). A full ring — editor
        // closed, or its thread stalled — silently drops frames, the same
        // failure mode as the note ring.
        // Read the parameter and choose ONCE per block: the same buffer goes to
        // the live ring below and the take recorder after it. The main buffer
        // remains the host's untouched pass-through path either way.
        let analysis_input = selected_analysis_input(
            &*buffer,
            aux.inputs.first(),
            self.params.analysis_input.value(),
        );
        let channels = analysis_input.channels();
        if channels > 0 {
            self.audio_channels.store(channels as u32, Ordering::Relaxed);
            // Reserve the block's slots once and fill them, rather than a
            // per-sample push(): one ring-atomic touch per block instead of
            // ~48k/s on the audio thread. `slots()` only grows as the consumer
            // drains, so the reservation of `want` never fails; the `if let`
            // is defensive.
            let want =
                interleaved_reservation(self.audio_producer.slots(), block_samples, channels);
            if want > 0 {
                // Interleaved from the per-channel planes the host gave us. A
                // shared slice-of-slices rather than `iter_samples`, because a
                // frame view cannot be flat-mapped without collecting it — and
                // this is the audio thread, where the allocation that would take
                // is exactly what is not allowed.
                let planes = analysis_input.as_slice_immutable();
                let frames = want / channels;
                if let Ok(chunk) = self.audio_producer.write_chunk_uninit(want) {
                    chunk.fill_from_iter(
                        (0..frames).flat_map(|f| (0..channels).map(move |c| planes[c][f])),
                    );
                }
            }
        }

        if let Some(origin) = take_origin {
            self.take.params(origin, ParamKey::ALL.map(|key| self.params.param_for(key).value()));

            // The take's own audio, when asked for: the selected analysis input
            // exactly as it arrives, so the render gets a spectrum and a
            // soundtrack without a separate bounce. Sized by TAKE_CHANNELS
            // rather than the host's own channel count — the WAV is stereo
            // whatever arrives, which is what `take_right_channel` duplicates
            // a mono input for.
            if self.take.wants_audio() && channels > 0 {
                self.take.mark_audio_start(origin);
                let wanted = block_samples * TAKE_CHANNELS;
                let mut interleaved =
                    (0..block_samples).flat_map(|frame| take_stereo_frame(analysis_input, frame));
                self.take.audio(&mut interleaved, wanted);
            }
        }

        self.samples_processed += block_samples as u64;
        #[cfg(feature = "tuning-probe")]
        if self.probe.keep_alive() {
            return ProcessStatus::KeepAlive;
        }
        ProcessStatus::Normal
    }

    #[cfg(feature = "tuning-probe")]
    fn deactivate(&mut self) {
        self.probe.deactivate();
    }
}

impl ClapPlugin for Harmonigraph {
    #[cfg(feature = "tuning-probe")]
    const CLAP_PROCESS_TRACE: bool = true;
    #[cfg(feature = "tuning-probe")]
    fn clap_process_trace(&mut self, event: nice_plug::wrapper::clap::ProcessTrace<'_>) {
        self.probe.hook(event);
    }
    const CLAP_ID: &'static str = "com.yan-h.harmonigraph";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("Tonnetz harmony and spectrum visualizer");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> =
        Some("https://github.com/yan-h/harmonigraph/issues");
    const CLAP_FEATURES: &'static [ClapFeature] =
        &[ClapFeature::NoteEffect, ClapFeature::Analyzer, ClapFeature::Utility];
}

impl Vst3Plugin for Harmonigraph {
    const VST3_CLASS_ID: [u8; 16] = *b"HarmonigraphYanH";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Fx, Vst3SubCategory::Analyzer];
}

#[cfg(not(feature = "tuning-probe"))]
nice_export_clap!(Harmonigraph);
#[cfg(feature = "tuning-probe")]
nice_export_clap!(Harmonigraph, HarmonigraphTune);
nice_export_vst3!(Harmonigraph);

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use nice_plug::params::InternalParamMut;

    use super::*;

    /// How long a "the analyzer has done the work" assertion waits before it
    /// gives up — on the analyzer, or on the machine, which is a distinction
    /// [`columns_after`] keeps rather than resolves.
    ///
    /// Two orders of magnitude over the 20 ms poll, because what has to be
    /// bounded is not how fast the analyzer works but how long the scheduler
    /// may ignore it: on a shared CI runner a round can simply fail to happen
    /// inside any window a fixed sleep would pick, which is what took main red
    /// at `f9cbc4e0` on a tree that had already passed (#598). Nothing healthy
    /// ever spends this — [`columns_after`] returns on the round that
    /// lands — so it costs the suite only when it is about to fail anyway.
    const ANALYSIS_DEADLINE: Duration = Duration::from_secs(5);

    struct EmptyProcessContext {
        transport: Transport,
    }

    impl EmptyProcessContext {
        fn new() -> Self {
            Self { transport: Transport::new(DEFAULT_SAMPLE_RATE as f32) }
        }
    }

    impl ProcessContext<Harmonigraph> for EmptyProcessContext {
        fn plugin_api(&self) -> PluginApi {
            PluginApi::Standalone
        }

        fn execute_background(&self, _task: ()) {}

        fn execute_gui(&self, _task: ()) {}

        fn transport(&self) -> &Transport {
            &self.transport
        }

        fn next_event(&mut self) -> Option<PluginNoteEvent<Harmonigraph>> {
            None
        }

        fn send_event(&mut self, _event: PluginNoteEvent<Harmonigraph>) {}

        fn set_latency_samples(&self, _samples: u32) {}

        fn set_current_voice_capacity(&self, _capacity: u32) {}
    }

    fn stereo_buffer<'a>(left: &'a mut [f32], right: &'a mut [f32]) -> Buffer<'a> {
        assert_eq!(left.len(), right.len());
        let frames = left.len();
        let mut buffer = Buffer::default();
        // SAFETY: both slices stay borrowed for the returned buffer's lifetime,
        // and the assertion above establishes nice-plug's equal-length contract.
        unsafe {
            buffer.set_slices(frames, |channels| *channels = vec![left, right]);
        }
        buffer
    }

    /// A column count above `before`, or `None` if
    /// [`ANALYSIS_DEADLINE`] passes without one.
    ///
    /// **`None` does not mean the wiring is broken, and a caller must not word
    /// it that way.** An expired deadline says only that no column arrived in
    /// five seconds, which a flag nobody watches and a runner that never
    /// scheduled the thread produce alike; telling those two apart is exactly
    /// what a deadline gives up in exchange for not being a fixed sleep. The
    /// distinction is worth the `Option`, because #598 cost a session partly
    /// by asserting a cause in its failure message that its evidence did not
    /// support.
    ///
    /// Sound only for asserting that work HAS happened. The reverse claim —
    /// that the analyzer left a ring alone — is not waitable at all, because
    /// every wait ends in "not yet", and a thread the scheduler forgot looks
    /// exactly like a thread correctly holding off. Those assertions keep a
    /// fixed settle and are commented where they stand.
    fn columns_after(plugin: &Harmonigraph, before: usize) -> Option<usize> {
        let started = Instant::now();
        loop {
            let columns = plugin.editor_shared.lock().ui.spectrum.history().len();
            if columns > before {
                return Some(columns);
            }
            if started.elapsed() >= ANALYSIS_DEADLINE {
                return None;
            }
            // Deliberately NOT `POLL`. This samples under the BLOCKING lock
            // while `tick` takes the same one with `try_lock`, so a sample
            // landing inside the analyzer's own hold costs it that whole
            // round — and a test written to stop being flaky should not
            // sample in lockstep with the thread it is watching. Half the
            // period leaves the two with no phase relationship to get wrong,
            // and costs nothing.
            std::thread::sleep(background::POLL / 2);
        }
    }

    /// Whether the analyzer completed a loop round after `before`.
    ///
    /// This is the synchronization a negative assertion cannot obtain from
    /// columns: a round that sees an open editor correctly produces none. The
    /// counter exists only under `cfg(test)`, so observing that silence does not
    /// add state or atomic traffic to the plugin build.
    fn a_round_after(plugin: &Harmonigraph, before: u64) -> bool {
        let started = Instant::now();
        loop {
            if plugin._background.completed_rounds() > before {
                return true;
            }
            if started.elapsed() >= ANALYSIS_DEADLINE {
                return false;
            }
            std::thread::sleep(background::POLL / 2);
        }
    }

    /// The window flag the editor announces itself through has to be the one
    /// the background analyzer watches, and nothing but this says so.
    ///
    /// They are two clones of one `Arc<EguiState>` today — `Default` hands one
    /// to [`background::BackgroundAnalyzer::spawn`] and `editor` hands the same
    /// field to `editor::create`. A second `EguiState` on either side compiles,
    /// passes every other test, and leaves the analyzer reading a flag nobody
    /// sets: it would then race an open editor for both rings on every poll,
    /// which is a timing bug with nothing between it and a release.
    ///
    /// Driven through the REAL spawned thread rather than by calling `tick`,
    /// because the wiring is precisely what a direct call would bypass.
    #[test]
    fn the_background_analyzer_watches_the_flag_the_editor_sets() {
        // A settle rather than a poll, and deliberately: it waits on the
        // assertion that NOTHING happened, and nothing is what a poll cannot
        // wait for. Five polls of it fails safe — an
        // analyzer that is merely late reads as one correctly holding off, so
        // only a drain that really happened trips the assertion it guards.
        let settle = background::POLL * 5;
        let columns =
            |plugin: &Harmonigraph| plugin.editor_shared.lock().ui.spectrum.history().len();

        let mut plugin = Harmonigraph::default();
        // First make construction's in-flight round observable. The fill is
        // big enough to cross hop boundaries, so its drain must grow history.
        for i in 0..20_000 {
            let t = i as f32 / 48_000.0;
            plugin
                .audio_producer
                .push((std::f32::consts::TAU * 440.0 * t).sin() * 0.5)
                .expect("the ring is sized for this");
        }
        let before = columns_after(&plugin, 0).unwrap_or_else(|| {
            panic!(
                "no baseline column in {ANALYSIS_DEADLINE:?}: this runner never scheduled the \
                 analyzer"
            )
        });
        plugin.params.editor_state.set_open(true);
        // A completed round after the store is the missing synchronization:
        // even one that read `open == false` just before the store has now
        // either drained the empty ring or lost `try_lock`, and cannot resume
        // later to race the fill. Every following round sees the open flag.
        let rounds = plugin._background.completed_rounds();
        assert!(
            a_round_after(&plugin, rounds),
            "no analyzer round in {ANALYSIS_DEADLINE:?} after the editor announced itself open: \
             this runner never scheduled it",
        );

        // A second reachable fill is the work an open editor must leave alone.
        for i in 0..20_000 {
            let t = i as f32 / 48_000.0;
            plugin
                .audio_producer
                .push((std::f32::consts::TAU * 440.0 * t).sin() * 0.5)
                .expect("the ring is sized for this");
        }
        std::thread::sleep(settle);
        assert_eq!(
            columns(&plugin),
            before,
            "the analyzer grew history while the editor had claimed the ring: it is not \
             watching the EguiState the editor sets",
        );

        // Polled, not slept: this is the half that needs the thread to have
        // DONE something, so a late round is a real failure of the test and
        // not of the wiring.
        plugin.params.editor_state.set_open(false);
        assert!(
            columns_after(&plugin, before).is_some(),
            "no new column in {ANALYSIS_DEADLINE:?} after the editor announced itself shut: \
             either the analyzer is not watching the EguiState the editor sets, or this \
             runner never scheduled it",
        );
    }

    /// The project's saved settings have to reach the analyzer with no editor
    /// ever constructed, which is the whole of issue #324: `params.ui_state` is
    /// where the host puts them, the editor's window-build closure is the only
    /// other reader of that field, and the plugin analyzes audio from the moment
    /// it is instantiated.
    ///
    /// Driven through the REAL spawned thread and the real `params` field —
    /// nothing here calls [`Plugin::editor`] — because that pairing IS the
    /// claim. A `Restore` handed any other `RwLock<String>` satisfies every unit
    /// test one file over and reaches nothing a host ever writes. Those cover
    /// what the adopt decides; this covers that it is reachable at all.
    #[test]
    fn a_projects_saved_window_reaches_the_analyzer_with_no_editor_built() {
        use harmonigraph_ui::SpectrumWindow;

        // Half the analysis window: what `column_lag` reads back out of the
        // analyzer, on the rate a plugin runs at until a host reports its own.
        let lag_of = |window: SpectrumWindow| {
            0.5 * window.samples() as f64 / f64::from(DEFAULT_SAMPLE_RATE as f32)
        };
        let blob = {
            let mut saved = harmonigraph_ui::SharedState::new(editor::ASSUMED_SURFACE_FORMAT);
            saved.spectrum_config.window = SpectrumWindow::Precise;
            harmonigraph_ui::shell::close(&saved)
        };

        let mut plugin = Harmonigraph::default();
        // What the host does when it restores a project, and all it does: no
        // editor, no activation, just the field.
        *plugin.params.ui_state.write() = blob;
        // A Precise window is 16384 samples; this is more than one windowful,
        // so the analyzer cannot come out of the drain empty.
        for i in 0..40_000 {
            let t = i as f32 / 48_000.0;
            plugin
                .audio_producer
                .push((std::f32::consts::TAU * 440.0 * t).sin() * 0.5)
                .expect("the ring is sized for this");
        }
        assert!(
            columns_after(&plugin, 0).is_some(),
            "nothing was analyzed in {ANALYSIS_DEADLINE:?}: either the restored blob \
             reaches nothing without an editor, or this runner never scheduled the \
             analyzer",
        );

        let shared = plugin.editor_shared.lock();
        let lag = shared.ui.spectrum.column_lag();
        assert!(
            (lag - lag_of(SpectrumWindow::Precise)).abs() < 1e-9,
            "the background analyzer ran at a {:.1} ms window where the project saved \
             {:.1} ms: the restored blob reaches nothing until the editor opens",
            lag * 2000.0,
            lag_of(SpectrumWindow::Precise) * 2000.0,
        );
    }

    /// `ParamKey::id` is what a recorded take names its automation by, and
    /// what the host names its automation lanes by. They are declared in
    /// two places — the enum's `id()` and the `#[id = "..."]` attributes
    /// here — and if they drift, takes silently replay with default
    /// parameters and projects silently lose their automation. The one
    /// operational parameter is named separately: it selects which audio is
    /// recorded rather than describing a value the replay must reconstruct.
    #[test]
    fn every_param_key_id_matches_the_host_facing_id() {
        let params = HarmonigraphParams::default();
        let host_ids: Vec<String> = params.param_map().into_iter().map(|(id, _, _)| id).collect();
        for key in ParamKey::ALL {
            assert!(
                host_ids.iter().any(|id| id == key.id()),
                "ParamKey::{key:?} claims id {:?}, which no #[id] attribute declares; \
                 the host exposes {host_ids:?}",
                key.id(),
            );
        }
        assert!(
            host_ids.iter().any(|id| id == "analysis-input"),
            "the source selector has no host-persisted parameter: {host_ids:?}",
        );
        assert_eq!(
            host_ids.len(),
            ParamKey::ALL.len() + 1,
            "the plugin exposes an unnamed operational parameter: {host_ids:?}"
        );
    }

    /// The source choice belongs to the host's project state and is read on the
    /// audio thread, but is not automation: a lane splicing two unrelated
    /// signals into one spectrum history is not a useful visual control. Main
    /// remains the default so adding the auxiliary port does not restyle an
    /// existing project into silence.
    #[test]
    fn analysis_input_is_a_stable_non_automatable_main_default() {
        let params = HarmonigraphParams::default();
        assert_eq!(params.analysis_input.value(), AnalysisInputParam::Main);
        assert!(params.analysis_input.flags().contains(ParamFlags::NON_AUTOMATABLE));
        assert_eq!(AnalysisInputParam::ids(), Some(["main", "sidechain"].as_slice()));
    }

    /// The descriptor is what makes the host show its sidechain routing control.
    /// One named stereo auxiliary bus is the whole contract; adding a second or
    /// silently narrowing it to mono would make the first-buffer choice in
    /// `process` disagree with what the host presents.
    #[test]
    fn the_audio_layout_exposes_one_named_stereo_sidechain() {
        let [layout] = Harmonigraph::AUDIO_IO_LAYOUTS else {
            panic!("the plugin exposes more than its one fixed audio layout")
        };
        assert_eq!(layout.main_input_channels.map(NonZeroU32::get), Some(2));
        assert_eq!(layout.main_output_channels.map(NonZeroU32::get), Some(2));
        assert_eq!(layout.main_input_name(), "Input");
        assert_eq!(layout.main_output_name(), "Output");
        assert_eq!(layout.aux_input_ports, &[new_nonzero_u32(2)]);
        assert_eq!(layout.aux_input_name(0).as_deref(), Some("Visualization Input"));
    }

    /// Reach the feature through the callback the host actually invokes. The
    /// main and sidechain samples are all distinct, so selecting either wrong
    /// plane or accidentally writing into the pass-through cannot hide behind
    /// equality. An in-memory recorder arms the real take branch, so its
    /// captured samples prove that path uses the same selected buffer without
    /// writing a take into the user's Music directory.
    #[test]
    fn a_sidechain_process_block_feeds_analysis_and_leaves_main_untouched() {
        let mut plugin = Harmonigraph::default();
        // SAFETY: this emulates the host wrapper setting a parameter while the
        // owned parameter object remains alive for the whole process call.
        assert!(unsafe {
            plugin.params.analysis_input._internal_set_plain_value(AnalysisInputParam::Sidechain)
        });
        let (recorder, mut take_capture) = harmonigraph_record::testing::channel();
        plugin.take = recorder;
        take_capture.arm_audio();

        let shared = plugin.editor_shared.clone();
        let mut shared = shared.lock();
        let mut main_left = [1.0, 2.0, 3.0, 4.0];
        let mut main_right = [5.0, 6.0, 7.0, 8.0];
        let expected_main_left = main_left;
        let expected_main_right = main_right;
        let mut sidechain_left = [11.0, 12.0, 13.0, 14.0];
        let mut sidechain_right = [21.0, 22.0, 23.0, 24.0];
        let expected_sidechain = [11.0, 21.0, 12.0, 22.0, 13.0, 23.0, 14.0, 24.0];

        {
            let mut main = stereo_buffer(&mut main_left, &mut main_right);
            let sidechain = stereo_buffer(&mut sidechain_left, &mut sidechain_right);
            let mut inputs = [sidechain];
            let mut outputs = [];
            let mut aux = AuxiliaryBuffers { inputs: &mut inputs, outputs: &mut outputs };
            assert!(matches!(
                plugin.process(&mut main, &mut aux, &mut EmptyProcessContext::new()),
                ProcessStatus::Normal,
            ));
        }

        assert_eq!(main_left, expected_main_left, "process changed the main left pass-through");
        assert_eq!(main_right, expected_main_right, "process changed the main right pass-through");
        assert_eq!(
            shared.drain_analysis_audio_for_test().as_slice(),
            &expected_sidechain,
            "the live analyzer received Main instead of Sidechain",
        );
        assert_eq!(
            take_capture.drain_audio().as_slice(),
            &expected_sidechain,
            "the armed take received Main instead of Sidechain",
        );
        assert_eq!(plugin.samples_processed, expected_main_left.len() as u64);
    }

    /// Source selection is explicit and block-wide. Silence says nothing about
    /// connectivity, so Sidechain chooses the auxiliary value even when it is
    /// zero; only a structurally missing buffer takes the defensive Main path.
    #[test]
    fn analysis_input_selection_never_falls_back_on_signal_content() {
        let (main, sidechain) = (11, 0);
        assert_eq!(
            *selected_analysis_input(&main, Some(&sidechain), AnalysisInputParam::Main),
            main,
        );
        assert_eq!(
            *selected_analysis_input(&main, Some(&sidechain), AnalysisInputParam::Sidechain),
            sidechain,
            "a silent sidechain fell back to the main input",
        );
        assert_eq!(
            *selected_analysis_input(&main, None, AnalysisInputParam::Sidechain),
            main,
            "a missing auxiliary buffer has no safe source except Main",
        );
    }

    /// A note-on carries its velocity through; a note-off and a choke both mean
    /// "this voice is done" and collapse to the same kind. The fields are
    /// asserted by name, which is the pairing `MappedNote` exists to keep.
    #[test]
    fn the_note_events_the_lattice_draws_keep_their_fields() {
        let on =
            NoteEvent::NoteOn { timing: 7, voice_id: None, channel: 2, note: 60, velocity: 0.5 };
        assert_eq!(
            mapped_note(on),
            Some(MappedNote {
                timing: 7,
                channel: 2,
                note: 60,
                kind: NoteEventKind::On { velocity: 0.5 },
            })
        );

        let off =
            NoteEvent::NoteOff { timing: 9, voice_id: None, channel: 2, note: 60, velocity: 0.0 };
        assert_eq!(
            mapped_note(off),
            Some(MappedNote { timing: 9, channel: 2, note: 60, kind: NoteEventKind::Off })
        );

        // A choke is a note-off the host will not follow with one.
        let choke = NoteEvent::Choke { timing: 11, voice_id: None, channel: 3, note: 64 };
        assert_eq!(
            mapped_note(choke),
            Some(MappedNote { timing: 11, channel: 3, note: 64, kind: NoteEventKind::Off })
        );

        let tuning = NoteEvent::PolyTuning {
            timing: 13,
            voice_id: None,
            channel: 1,
            note: 67,
            tuning: -0.25,
        };
        assert_eq!(
            mapped_note(tuning),
            Some(MappedNote {
                timing: 13,
                channel: 1,
                note: 67,
                kind: NoteEventKind::Tuning { semitones: -0.25 },
            })
        );
    }

    /// The negatives are where this goes wrong quietly. `MidiConfig::Basic`
    /// delivers every poly expression below, and each carries the same
    /// `{ timing, channel, note, .. }` shape as the PolyTuning arm — so each is
    /// one copy-pasted arm away from drawing notes nobody played, with
    /// `#[non_exhaustive]` guaranteeing the compiler never points at the
    /// omission.
    #[test]
    fn the_poly_expressions_are_not_notes() {
        let not_notes = [
            NoteEvent::PolyPressure {
                timing: 5,
                voice_id: None,
                channel: 1,
                note: 67,
                pressure: 0.5,
            },
            NoteEvent::PolyVolume { timing: 5, voice_id: None, channel: 1, note: 67, gain: 0.5 },
            NoteEvent::PolyPan { timing: 5, voice_id: None, channel: 1, note: 67, pan: 0.5 },
            NoteEvent::PolyVibrato {
                timing: 5,
                voice_id: None,
                channel: 1,
                note: 67,
                vibrato: 0.5,
            },
            NoteEvent::PolyExpression {
                timing: 5,
                voice_id: None,
                channel: 1,
                note: 67,
                expression: 0.5,
            },
            NoteEvent::PolyBrightness {
                timing: 5,
                voice_id: None,
                channel: 1,
                note: 67,
                brightness: 0.5,
            },
            // Plugin-to-host: this one cannot arrive on the path at all.
            NoteEvent::VoiceTerminated { timing: 5, voice_id: None, channel: 1, note: 67 },
        ];
        for event in not_notes {
            assert_eq!(mapped_note(event), None, "{event:?} must not reach the lattice");
        }
    }

    /// The three ways a block decides where its take timestamps start. The
    /// middle rung is the one that matters: a take stamped on the plugin's own
    /// clock rather than the transport's lines up with no bounce of the same
    /// song, and nothing looks wrong until the audio is dropped underneath it.
    #[test]
    fn a_take_origin_prefers_the_transport_over_the_local_clock() {
        // Disarmed: the recorder is not consulted, whatever the host reports.
        assert_eq!(origin_source(false, Some(12.0), 22_050, 44_100.0), OriginSource::Idle);
        assert_eq!(origin_source(false, None, 22_050, 44_100.0), OriginSource::Idle);

        // Armed, and the host reports a position: that position, untouched.
        assert_eq!(
            origin_source(true, Some(12.0), 22_050, 44_100.0),
            OriginSource::Transport(12.0)
        );

        // Armed, and the host reports nothing at all: the plugin's own sample
        // counter in seconds, so "just record what I play" still works.
        assert_eq!(origin_source(true, None, 22_050, 44_100.0), OriginSource::LocalClock(0.5));
    }

    /// One event, two clocks. The ring is stamped on the plugin's own sample
    /// counter, which is what the editor's ClockMapper reads; the take is
    /// stamped on the transport, which is what lets it be lined up against a
    /// bounce later. Collapsing them into one would look right on screen and
    /// put every recorded take at the wrong song position.
    ///
    /// Every constant here divides to an exact binary fraction, which is what
    /// lets `assert_eq!` compare f64 at all. Picking more realistic numbers — a
    /// 512-sample block, an odd offset at 44.1 kHz — lands off a representable
    /// value and fails on the last bit with the implementation still correct.
    #[test]
    fn an_event_is_stamped_on_two_independent_clocks() {
        let rate = 48_000.0;

        // Both clocks advance by the event's offset within the block...
        assert_eq!(ring_time(0, 24_000, rate) - ring_time(0, 0, rate), 0.5);
        assert_eq!(take_time(90.0, 24_000, rate) - take_time(90.0, 0, rate), 0.5);

        // ...but only the ring's counts the plugin's own blocks,
        assert_eq!(ring_time(48_000, 0, rate), 1.0);
        // and only the take's counts the song position it hangs off.
        assert_eq!(take_time(90.0, 0, rate), 90.0);
    }

    /// A take's WAV is always stereo. A mono host input is DUPLICATED into both
    /// planes rather than written as half a frame, which would put every
    /// subsequent frame out of step with the notes it is meant to match.
    #[test]
    fn a_mono_input_is_duplicated_into_both_take_channels() {
        assert_eq!(take_right_channel(1), 0, "mono reads plane 0 twice");
        assert_eq!(take_right_channel(2), 1);
        // A host offering more than two planes still writes a stereo take.
        assert_eq!(take_right_channel(8), 1);
    }
}
