//! The production companion. No analyzer, audio rings, recorder, dock, shared
//! visualization state or GPU editor is constructed for this CLAP class.
use super::{setup, source::Source};
use nice_plug::prelude::*;
use nice_plug::wrapper::clap::{configuration::OwnedInput, performance as api};
use std::sync::Arc;

#[derive(Params)]
pub struct TuneParams {
    #[id = "participating"]
    pub participating: BoolParam,
}
impl Default for TuneParams {
    fn default() -> Self {
        Self {
            participating: BoolParam::new("Participating", true).with_value_to_string(Arc::new(
                |value| if value { "Participating" } else { "Off" }.into(),
            )),
        }
    }
}
pub struct HarmonigraphTune {
    pub params: Arc<TuneParams>,
    pub shared: Arc<setup::Shared>,
    pub source: Option<Box<Source>>,
}
impl Default for HarmonigraphTune {
    fn default() -> Self {
        let shared = setup::Shared::source();
        let source = Source::new(shared.clone());
        Self { params: Arc::new(TuneParams::default()), shared, source: Some(source) }
    }
}
impl Plugin for HarmonigraphTune {
    const NAME: &'static str = "Harmonigraph Tune";
    const VENDOR: &'static str = "Yan Han";
    const URL: &'static str = env!("CARGO_PKG_HOMEPAGE");
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");
    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout::const_default()];
    const MIDI_INPUT: MidiConfig = MidiConfig::MidiCCs;
    const MIDI_OUTPUT: MidiConfig = MidiConfig::MidiCCs;
    type SysExMessage = ();
    type BackgroundTask = ();
    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }
    #[cfg(target_os = "macos")]
    fn editor(&mut self, _: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        Some(Box::new(super::native::NativeEditor {
            shared: self.shared.clone(),
            params: self.params.clone(),
        }))
    }
    fn process(
        &mut self,
        _buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        _context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        ProcessStatus::KeepAlive
    }
}
impl ClapPlugin for HarmonigraphTune {
    const CLAP_ID: &'static str = "com.yan-h.harmonigraph-tune";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("Shared harmonic session note companion");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> =
        Some("https://github.com/yan-h/harmonigraph/issues");
    const CLAP_FEATURES: &'static [ClapFeature] = &[ClapFeature::NoteEffect, ClapFeature::Utility];
    const CLAP_PERFORMANCE: bool = true;
    fn clap_setup(&self) -> Option<Arc<dyn nice_plug::wrapper::clap::setup::Setup>> {
        Some(Arc::new(setup::Adapter(self.shared.clone())))
    }
    fn clap_main_init(&mut self) -> bool {
        self.shared.register();
        true
    }
    fn clap_main_activate(&mut self, config: &BufferConfig) -> bool {
        self.source
            .as_mut()
            .unwrap()
            .activate(f64::from(config.sample_rate), config.max_buffer_size);
        true
    }
    fn clap_main_destroy(&mut self) {
        super::registry::retire_source(self.source.take().unwrap());
    }
    fn clap_performance_stop(&mut self) {
        self.source.as_mut().unwrap().stop();
    }
    fn clap_performance_reset(&mut self) {
        self.source.as_mut().unwrap().stop();
    }
    fn clap_performance_begin(&mut self, callback: api::Callback, _output: &mut api::Output<'_>) {
        self.source.as_mut().unwrap().begin(callback);
    }
    fn clap_performance_input(&mut self, input: OwnedInput) -> api::Consumption {
        self.source.as_mut().unwrap().input(input)
    }
    fn clap_performance_input_boundary(&mut self) {
        self.source.as_mut().unwrap().apply_setup();
    }
    fn clap_performance_process(
        &mut self,
        _buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        _context: &mut impl ProcessContext<Self>,
        block: api::Block,
        output: &mut api::Output<'_>,
    ) -> ProcessStatus {
        self.source.as_mut().unwrap().schedule(block, output);
        ProcessStatus::KeepAlive
    }
    fn clap_performance_prepare(&mut self, group: api::Group) -> bool {
        #[cfg(all(test, debug_assertions))]
        self.source.as_mut().unwrap().test_stale_prepare(group);
        self.source.as_mut().unwrap().prepare(group)
    }
    fn clap_performance_complete(
        &mut self,
        completion: api::Completion,
        output: &mut api::Output<'_>,
    ) {
        #[cfg(all(test, debug_assertions))]
        self.source.as_mut().unwrap().test_stale_complete(completion, output);
        self.source.as_mut().unwrap().complete(completion, output);
    }
    fn clap_performance_finalize(
        &mut self,
        callback: api::Callback,
        _status: i32,
        output: &mut api::Output<'_>,
    ) {
        self.source.as_mut().unwrap().schedule(
            api::Block {
                callback,
                start: 0,
                frames: callback.frames,
                transport: callback.transport,
            },
            output,
        );
    }
    fn clap_performance_end(&mut self, callback: api::Callback, _summary: api::Summary) {
        self.source.as_mut().unwrap().end(callback);
    }
}
