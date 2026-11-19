//! Harmonigraph's Hub and Tune from the exported CLAP factory, on
//! harmonigraph-clap's test host (#1868): the one device the configuration,
//! host-surface and tuning suites drive, through the real entry point, real
//! endpoint capacities and real recorder fanout, without opening an editor.
//!
//! A suite names the host extensions it needs, and chooses the Hub's engine
//! with [`Device::with_retune`] or [`Device::with_adaptive`]; a Hub that
//! chooses neither starts as a fresh project does, Retune off and the engine
//! Off.
use crate::tuning::{event::Event, setup};
use clap_sys::{events::*, ext::params::clap_plugin_params, plugin::clap_plugin, process::*};
use harmonigraph_clap::testing::{self, Audio, Call};
pub(crate) use harmonigraph_clap::testing::{header, Event as Input, Host};
use harmonigraph_clap::wrapper::clap::Wrapper;
use harmonigraph_ui::params::ParamKey;
use std::{cell::Cell, ffi::CStr, ptr, sync::Arc};

/// What the host's output list saw in one callback.
pub(crate) struct Sink {
    /// Every offer in turn: its type, time, parameter id (0 for an event
    /// with none) and whether the host took it.
    pub attempts: Vec<(u16, u32, u32, bool)>,
    /// The notes, expressions and MIDI the host took, and those it refused.
    pub values: Vec<(u32, Event)>,
    pub rejected: Vec<(u32, Event)>,
    /// What the callback returned to the host.
    pub status: clap_process_status,
}

impl Sink {
    fn read(offers: testing::Sink, status: clap_process_status) -> Self {
        let mut sink = Self { attempts: vec![], values: vec![], rejected: vec![], status };
        for offer in offers.offers {
            let flags = offer.flags;
            sink.attempts.push((offer.kind, offer.time, offer.param_id(), offer.taken));
            let event = match offer.event {
                Input::Note(e) => Event::Note {
                    kind: offer.kind,
                    id: e.note_id,
                    port: e.port_index,
                    channel: e.channel,
                    key: e.key,
                    velocity: e.velocity,
                    flags,
                },
                Input::Expression(e) => Event::Expression {
                    kind: e.expression_id,
                    id: e.note_id,
                    port: e.port_index,
                    channel: e.channel,
                    key: e.key,
                    value: e.value,
                    flags,
                },
                Input::Midi(e) => Event::Midi { port: e.port_index, data: e.data, flags },
                _ => continue,
            };
            if offer.taken { &mut sink.values } else { &mut sink.rejected }
                .push((offer.time, event));
        }
        sink
    }
}

pub(crate) struct Device {
    device: testing::Device,
    pub plugin: *const clap_plugin,
    pub tuner: bool,
    /// The most frames per callback [`Device::activate`] advertises.
    max_frames: u32,
    /// The auxiliary input every Hub block carries: silence unless a fixture
    /// needs it told apart from the main input.
    pub sidechain: Cell<[[f32; 64]; 2]>,
}

// The fixtures invoke only CLAP's allowed concurrent main/audio entrypoints;
// activation, destruction, and ordinary processing remain exclusively owned.
unsafe impl Sync for Device {}

impl Device {
    fn new(id: &CStr, tuner: bool, host: Host) -> Self {
        let device = testing::Device::new(&crate::clap_entry, id, host);
        Self {
            plugin: device.plugin,
            device,
            tuner,
            max_frames: 512,
            sidechain: Cell::new([[0.0; 64]; 2]),
        }
    }
    /// A Hub on `host`, initialized and not yet active.
    pub fn hub(host: Host) -> Self {
        Self::new(c"com.yan-h.harmonigraph", false, host)
    }
    /// A Tune on `host`, initialized and not yet active.
    pub fn tune(host: Host) -> Self {
        Self::new(c"com.yan-h.harmonigraph-tune", true, host)
    }
    /// Retune the Hub's own input, which defaults off.
    pub fn with_retune(self) -> Self {
        self.wrapper().test_inspect_plugin(|plugin| {
            plugin.aggregation.as_ref().unwrap().shared.set_retune(true)
        });
        self
    }
    /// Select the Adaptive engine, from the default Off. The Hub adopts a
    /// flush at its next process boundary, ahead of that callback's events.
    pub fn with_adaptive(self) -> Self {
        let engine = Input::Param(clap_event_param_value {
            header: header::<clap_event_param_value>(CLAP_EVENT_PARAM_VALUE, 0),
            param_id: harmonigraph_clap::wrapper::hash_param_id("tuning-engine"),
            cookie: ptr::null_mut(),
            note_id: -1,
            port_index: -1,
            channel: -1,
            key: -1,
            value: 1.0,
        });
        self.device.flush(&[engine], Some(&mut testing::Sink::with_capacity(64)));
        self
    }
    /// Advertise at most `frames` per callback at [`Device::activate`],
    /// rather than 512.
    pub fn with_max_frames(mut self, frames: u32) -> Self {
        self.max_frames = frames;
        self
    }

    /// What the plugin asked of the host.
    pub fn host(&self) -> &Host {
        self.device.host()
    }
    pub fn wrapper(&self) -> &Wrapper<crate::Harmonigraph> {
        assert!(!self.tuner);
        // SAFETY: created from the Hub's id.
        unsafe { self.device.wrapper() }
    }
    pub fn tune_wrapper(&self) -> &Wrapper<crate::tuning::plugin::HarmonigraphTune> {
        assert!(self.tuner);
        // SAFETY: created from the Tune's id.
        unsafe { self.device.wrapper() }
    }
    /// The instance's own tuning controls.
    pub fn shared(&self) -> Arc<setup::Shared> {
        if self.tuner {
            self.tune_wrapper().test_inspect_plugin(|plugin| plugin.shared.clone())
        } else {
            self.wrapper()
                .test_inspect_plugin(|plugin| plugin.aggregation.as_ref().unwrap().shared.clone())
        }
    }
    /// The Hub's tuning configuration, as the editor submits to and reads it.
    pub fn settings(&self) -> Arc<crate::configuration::Settings> {
        self.wrapper().test_inspect_plugin(|plugin| plugin.params.settings.clone())
    }
    /// The host's side of the editor, as the shell gave it to the plugin.
    pub fn editor_host(&self) -> Arc<dyn harmonigraph_clap::prelude::EditorHost> {
        self.wrapper().test_inspect_plugin(|plugin| plugin.editor_host.clone().expect("a host"))
    }
    /// The editor's state lock, which the window's size source logs under.
    pub fn editor_lock(&self) -> Arc<crate::editor::EditorLock> {
        self.wrapper().test_inspect_plugin(|plugin| plugin.editor_shared.clone())
    }

    /// Activate at 48 kHz with at most the device's max frames per callback.
    /// D is `multiplier x max_frames`, so the format a fixture activates at
    /// is also the delay a Tune plays against: 512 frames at the default 1x
    /// is the 512 samples this delay used to be fixed at. Callbacks stay
    /// whatever size the fixture drives; this is only the advertised maximum.
    pub fn activate(&mut self) {
        self.activate_format(48000.0, self.max_frames);
    }
    pub fn activate_format(&mut self, rate: f64, frames: u32) {
        self.device.activate(rate, frames);
    }
    pub fn deactivate(&mut self) {
        self.device.deactivate();
    }
    /// Record that a fixture activated or deactivated the plugin through
    /// its vtable, so the drop matches.
    pub fn set_active(&mut self, active: bool) {
        self.device.set_active(active);
    }
    pub fn reactivate_format(&mut self, rate: f64, frames: u32) {
        self.deactivate();
        self.activate_format(rate, frames);
    }
    /// `clap_plugin::reset`, which a host calls on one instance when it wants
    /// that instance's state discarded. It reaches the plugin as
    /// `Plugin::reset`.
    pub fn host_reset(&self) {
        self.device.reset();
    }
    pub fn main(&self) {
        self.device.main();
    }
    /// Run the host's main-thread callback, and count the times the project
    /// has been marked modified since the fixture began.
    pub fn service(&self) -> usize {
        self.main();
        self.host().dirty.load(std::sync::atomic::Ordering::Relaxed)
    }
    pub fn latency(&self) -> u32 {
        self.device.latency()
    }
    pub fn params(&self) -> &clap_plugin_params {
        self.device.params()
    }
    pub fn id(&self, key: ParamKey) -> u32 {
        self.id_named(key.host_name())
    }
    /// The host id of the parameter the host names `name`.
    pub fn id_named(&self, name: &str) -> u32 {
        for index in 0..unsafe { (self.params().count.unwrap())(self.plugin) } {
            let mut info: clap_sys::ext::params::clap_param_info = unsafe { std::mem::zeroed() };
            assert!(unsafe { (self.params().get_info.unwrap())(self.plugin, index, &mut info) });
            if unsafe { CStr::from_ptr(info.name.as_ptr()) }.to_bytes() == name.as_bytes() {
                return info.id;
            }
        }
        panic!("missing parameter");
    }
    pub fn load(&self, state: harmonigraph_clap::plugin::PluginState) {
        assert!(self.try_load(state));
    }
    pub fn try_load(&self, state: harmonigraph_clap::plugin::PluginState) -> bool {
        self.device.load(&serde_json::to_vec(&state).unwrap())
    }
    pub fn save(&self) -> harmonigraph_clap::plugin::PluginState {
        serde_json::from_slice(&self.device.save()).unwrap()
    }
    /// The JSON a save holds, as written.
    pub fn saved_json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.device.save()).unwrap()
    }

    /// One 64-frame callback with no transport, the host refusing what
    /// `reject_kind` names ([`testing::Sink::ALL`] for everything).
    pub fn run(&self, raw: i64, events: Vec<Input>, reject_kind: Option<u16>) -> Sink {
        self.run_select(raw, events, reject_kind, None)
    }
    pub fn run_select(
        &self,
        raw: i64,
        events: Vec<Input>,
        reject_kind: Option<u16>,
        reject_attempt: Option<usize>,
    ) -> Sink {
        self.run_format(raw, events, reject_kind, reject_attempt, 64)
    }
    pub fn run_format(
        &self,
        raw: i64,
        events: Vec<Input>,
        reject_kind: Option<u16>,
        reject_attempt: Option<usize>,
        frames: u32,
    ) -> Sink {
        self.run_status(raw, events, reject_kind, reject_attempt, frames, false)
    }
    pub fn run_status(
        &self,
        raw: i64,
        events: Vec<Input>,
        reject_kind: Option<u16>,
        reject_attempt: Option<usize>,
        frames: u32,
        expect_error: bool,
    ) -> Sink {
        self.run_callback(raw, events, (reject_kind, reject_attempt), (frames, expect_error), None)
    }
    /// A callback of `format`'s frames that fails exactly when it says, the
    /// host refusing what `rejection` names: a type, and an offer by its
    /// 1-based number.
    pub fn run_callback(
        &self,
        raw: i64,
        events: Vec<Input>,
        rejection: (Option<u16>, Option<usize>),
        format: (u32, bool),
        transport: Option<clap_event_transport>,
    ) -> Sink {
        let (frames, expect_error) = format;
        assert!(frames > 0 && frames <= 512);
        let sink = self.call(raw, events, rejection, frames, transport);
        assert_eq!(
            sink.status == CLAP_PROCESS_ERROR,
            expect_error,
            "tuner={} raw={raw} frames={frames}",
            self.tuner
        );
        sink
    }
    /// A 64-frame callback with the callback's own `transport`, the host
    /// refusing every offer if `reject`, else those of `reject_kind`.
    pub fn run_transport(
        &self,
        steady: i64,
        events: Vec<Input>,
        reject: bool,
        reject_kind: Option<u16>,
        transport: Option<clap_event_transport>,
    ) -> (clap_process_status, Sink) {
        let reject_kind = if reject { Some(testing::Sink::ALL) } else { reject_kind };
        let sink = self.call(steady, events, (reject_kind, None), 64, transport);
        (sink.status, sink)
    }
    fn call(
        &self,
        raw: i64,
        events: Vec<Input>,
        (refuse_kind, refuse_attempt): (Option<u16>, Option<usize>),
        frames: u32,
        transport: Option<clap_event_transport>,
    ) -> Sink {
        // Comfortably past EMIT_PER_CALLBACK, which is what bounds one
        // callback's note output; a sink too small to hold it aborts under
        // the allocation guard rather than reporting what overflowed.
        let mut offers = testing::Sink::with_capacity(1024);
        offers.refuse_kind = refuse_kind;
        offers.refuse_attempt = refuse_attempt;
        let [left, right] = self.sidechain.get();
        let audio = if self.tuner { Audio::None } else { Audio::Sidechain([&left, &right]) };
        let call = Call { steady: raw, frames, events: &events, transport, audio };
        // assert_process_allocs guards this entire exported callback, including
        // the wrapper. Nesting a second guard defeats its diagnostic permit.
        let status = self.device.process(&call, &mut offers);
        Sink::read(offers, status)
    }
}

std::thread_local! {
    static RECORDER: std::cell::RefCell<Option<harmonigraph_record::Recorder>> =
        const { std::cell::RefCell::new(None) };
}
/// The recorder [`recorded`] put in place for the next Hub this thread
/// constructs, which takes it.
pub(crate) fn injected_recorder() -> Option<harmonigraph_record::Recorder> {
    RECORDER.with(|r| r.borrow_mut().take())
}
pub(crate) fn inject_recorder(recorder: harmonigraph_record::Recorder) {
    RECORDER.with(|r| assert!(r.borrow_mut().replace(recorder).is_none()));
}
/// The Hub `make` constructs, recording into a capture rather than a take
/// file.
pub(crate) fn recorded(
    make: impl FnOnce() -> Device,
) -> (Device, harmonigraph_record::testing::Capture) {
    let (recorder, capture) = harmonigraph_record::testing::channel();
    inject_recorder(recorder);
    (make(), capture)
}
