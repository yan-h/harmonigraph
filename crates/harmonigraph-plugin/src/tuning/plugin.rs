//! The production companion. No analyzer, audio rings, recorder, dock, shared
//! visualization state or GPU editor is constructed for this CLAP class.
use nice_plug::prelude::*;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use super::{setup, tune::Tune};

pub struct HarmonigraphTune {
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
        Self { shared, tune: Some(tune), frames: 0 }
    }
}

impl HarmonigraphTune {
    fn multiplier(&self) -> u32 {
        setup::clamp_delay(self.shared.requested_multiplier.load(Ordering::Acquire))
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
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");
    const CLAP_ID: &'static str = "com.yan-h.harmonigraph-tune";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("Shared harmonic session note companion");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> =
        Some("https://github.com/yan-h/harmonigraph/issues");
    const CLAP_FEATURES: &'static [ClapFeature] = &[ClapFeature::NoteEffect, ClapFeature::Utility];
    const PORTS: Ports = Ports::NONE;
    const NOTE_PORTS: bool = true;
    /// A Tune exports no host parameter. Its Tuning delay is a saved setting
    /// (`setup::InstanceSettings`): a change needs the host to reactivate the
    /// Tune, so it cannot usefully be automated, and the saved value is there
    /// for the Tune to report its latency from before any Hub exists.
    fn params(&self) -> Arc<Table> {
        Arc::new(Table::empty())
    }
    fn setup(&self) -> Option<Arc<dyn nice_plug::wrapper::clap::setup::Setup>> {
        Some(Arc::new(setup::Adapter(self.shared.clone())))
    }
    #[cfg(target_os = "macos")]
    fn editor(&mut self) -> Option<Box<dyn Editor>> {
        Some(Box::new(super::native::NativeEditor { shared: self.shared.clone() }))
    }
    fn init(&mut self) -> bool {
        self.tune().register();
        true
    }
    /// The delay is built from the format the host just advertised, and the
    /// latency asked for is that delay: the shell tells the host while the
    /// activation is still in progress, which is what makes it the host's
    /// number. Nothing about pairing is consulted: a Tune that has never seen
    /// a Hub still reports the latency its own saved multiplier implies.
    fn activate(&mut self, config: &BufferConfig) -> bool {
        self.frames = config.max_buffer_size;
        let multiplier = self.multiplier();
        self.shared.active_multiplier.store(multiplier, Ordering::Release);
        self.shared.publish_format(f64::from(config.sample_rate), config.max_buffer_size);
        self.tune().activate(f64::from(config.sample_rate), config.max_buffer_size, multiplier);
        true
    }
    /// A changed multiplier is a changed latency, which the shell turns into
    /// a host restart request WITHOUT changing what the host reads: the
    /// delay is adopted at that reactivation and never mid-activation, so a
    /// reported latency that ran ahead of it would ask the host to
    /// compensate for a delay no note is being given.
    fn latency(&self) -> u32 {
        self.requested_latency()
    }
    /// Destruction gives the row back. The records this Tune left in its ring
    /// carry an epoch the attach that follows has already moved past, so the
    /// Hub refuses them and no drain, seal or acknowledgement is owed.
    fn destroy(&mut self) {
        self.tune.take().unwrap().retire();
    }
    fn stop(&mut self) {
        self.tune().stop();
    }
    fn reset(&mut self) {
        self.tune().stop();
    }
    fn begin(&mut self, boundary: &Boundary) {
        self.tune().begin(boundary);
    }
    fn process(&mut self, block: &mut Block<'_>) {
        for input in block.events() {
            self.tune().input(input);
        }
        self.tune().schedule(&mut block.output);
    }
    fn end(&mut self) {
        self.tune().end();
    }
}
