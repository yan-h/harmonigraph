//! The production companion. No analyzer, audio rings, recorder, dock, shared
//! visualization state or GPU editor is constructed for this CLAP class.
use nice_plug::prelude::*;
use nice_plug::wrapper::clap::{configuration::OwnedInput, performance as api};
use std::sync::atomic::Ordering;
use std::sync::Arc;

use super::{setup, tune::Tune, DELAY_MULTIPLIER_MAX};

#[derive(Params)]
pub struct TuneParams {
    /// Positive integer multiples of the host's advertised maximum callback
    /// size. An ordinary parameter, so the host saves it and each Tune reports
    /// the latency it implies before any Hub exists.
    #[id = "tuning_delay"]
    pub delay: IntParam,
}
impl Default for TuneParams {
    fn default() -> Self {
        Self {
            delay: IntParam::new(
                "Tuning Delay",
                1,
                IntRange::Linear { min: 1, max: DELAY_MULTIPLIER_MAX },
            )
            .with_value_to_string(Arc::new(|value| format!("{value}x buffer"))),
        }
    }
}

pub struct HarmonigraphTune {
    pub params: Arc<TuneParams>,
    pub shared: Arc<setup::Shared>,
    pub tune: Option<Box<Tune>>,
    /// The maximum callback size this activation advertised. One buffer means
    /// this and nothing else -- not the minimum, not the length of the
    /// callback in hand, not an observed typical size.
    frames: u32,
}

impl Default for HarmonigraphTune {
    fn default() -> Self {
        let shared = setup::Shared::source();
        let tune = Tune::new(shared.clone());
        Self { params: Arc::new(TuneParams::default()), shared, tune: Some(tune), frames: 0 }
    }
}

impl HarmonigraphTune {
    fn multiplier(&self) -> u32 {
        self.params.delay.value().clamp(1, DELAY_MULTIPLIER_MAX) as u32
    }
    /// What this Tune asks the host for. Zero frames means no activation has
    /// advertised a format yet, and there is no delay to report until one has.
    /// Asking is not being answered: the wrapper publishes a request only
    /// across an activation, so between a finished edit and the reactivation
    /// the host keeps reading the delay this activation is actually running.
    fn requested_latency(&self) -> u32 {
        self.multiplier().saturating_mul(self.frames)
    }
    fn tune(&mut self) -> &mut Tune {
        self.tune.as_mut().unwrap()
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
    fn initialize(
        &mut self,
        _: &AudioIOLayout,
        _: &BufferConfig,
        context: &mut impl InitContext<Self>,
    ) -> bool {
        // Activation order is `clap_main_activate` then `initialize`, so the
        // frames this reports against are the ones the delay was just built
        // from, and the request equals the delay this activation adopted.
        // Publishing it here is what makes it the host's number. Nothing about
        // pairing is consulted: a Tune that has never seen a Hub still reports
        // the latency its own saved multiplier implies.
        context.set_latency_samples(self.requested_latency());
        true
    }
    #[cfg(target_os = "macos")]
    fn editor(&mut self, _: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        Some(Box::new(super::native::NativeEditor { shared: self.shared.clone() }))
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
        Some(Arc::new(setup::Adapter(self.shared.clone(), Some(self.params.clone()))))
    }
    fn clap_main_init(&mut self) -> bool {
        self.tune().register();
        true
    }
    fn clap_main_activate(&mut self, config: &BufferConfig) -> bool {
        self.frames = config.max_buffer_size;
        let multiplier = self.multiplier();
        self.shared.requested_multiplier.store(multiplier, Ordering::Release);
        self.shared.active_multiplier.store(multiplier, Ordering::Release);
        self.shared.publish_format(f64::from(config.sample_rate), config.max_buffer_size);
        self.tune().activate(f64::from(config.sample_rate), config.max_buffer_size, multiplier);
        true
    }
    /// Destruction gives the row back. The records this Tune left in its ring
    /// carry an epoch the attach that follows has already moved past, so the
    /// Hub refuses them and no drain, seal or acknowledgement is owed.
    fn clap_main_destroy(&mut self) {
        self.tune.take().unwrap().retire();
    }
    fn clap_performance_stop(&mut self) {
        self.tune().stop();
    }
    fn clap_performance_reset(&mut self) {
        self.tune().stop();
    }
    fn clap_performance_begin(&mut self, callback: api::Callback, _output: &mut api::Output<'_>) {
        self.tune().begin(callback);
    }
    fn clap_performance_input(&mut self, input: OwnedInput) {
        self.tune().input(input);
    }
    fn clap_performance_process(
        &mut self,
        _buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
        block: api::Block,
        output: &mut api::Output<'_>,
    ) -> ProcessStatus {
        // A changed multiplier is a changed latency, and nice-plug turns that
        // into a host restart request WITHOUT changing what the host reads:
        // the delay is adopted at that reactivation and never mid-activation,
        // so a reported latency that ran ahead of it would ask the host to
        // compensate for a delay no note is being given. Repeating the same
        // number costs one atomic swap and asks for nothing.
        context.set_latency_samples(self.requested_latency());
        self.tune().schedule(block, output);
        ProcessStatus::KeepAlive
    }
    /// The whole callback's remaining horizon, emitted at its final sample.
    /// `start` is the chronological floor the sub-blocks already passed, not a
    /// new interval: the wrapper's parameter output for the last sub-block is
    /// pinned there too, so anything emitted before it would unsort the list.
    fn clap_performance_finalize(
        &mut self,
        callback: api::Callback,
        _status: i32,
        output: &mut api::Output<'_>,
    ) {
        self.tune().schedule(
            api::Block {
                callback,
                start: callback.frames.saturating_sub(1),
                frames: 1,
                transport: callback.transport,
            },
            output,
        );
    }
    fn clap_performance_end(&mut self, _callback: api::Callback, _summary: api::Summary) {
        self.tune().end();
    }
}
