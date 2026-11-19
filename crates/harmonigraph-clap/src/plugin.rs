//! The plugin a CLAP shell wraps: what it declares, and the calls the shell
//! makes into it, on the main thread and per process callback.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

mod state;
pub use state::*;

use crate::audio_setup::{Audio, BufferConfig, Ports};
use crate::editor::Editor;
use crate::events::{Input, InputValue, List, Output};
use crate::midi::MidiConfig;
use crate::params::Table;
use crate::wrapper::clap::EditorHost;
use crate::wrapper::clap::features::ClapFeature;
use crate::wrapper::clap::setup::Setup;
use clap_sys::events::clap_event_transport;

/// A CLAP plugin. The shell owns the host's side of every call; the plugin
/// owns everything musical.
///
/// Per process callback the shell calls, in this order:
///
/// 1. [`Plugin::begin`] with the callback's [`Boundary`]. The parameter table
///    still holds what it held before the callback, so a value read here is
///    the one in force at the callback's first sample.
/// 2. The shell writes the callback's parameter events into the table.
///    Automation therefore never reaches the table before `begin` has read it
///    (#1814's ordering rule): a note before a mid-block automation point is
///    shaped by the value before it.
/// 3. [`Plugin::process`] with the [`Block`]: the events, the audio and the
///    output list. Skipped for a boundary that is not [`Boundary::valid`].
/// 4. [`Plugin::end`].
///
/// All four run inside the process allocation guard.
#[allow(unused_variables)]
pub trait Plugin: Default + Send + 'static {
    const NAME: &'static str;
    const VENDOR: &'static str;
    const URL: &'static str;
    /// Semver compatible version string (e.g. `0.0.1`).
    const VERSION: &'static str;

    /// A unique id, in reverse domain name notation.
    const CLAP_ID: &'static str;
    const CLAP_DESCRIPTION: Option<&'static str>;
    const CLAP_MANUAL_URL: Option<&'static str>;
    const CLAP_SUPPORT_URL: Option<&'static str>;
    /// Keywords the host may sort the plugin by.
    const CLAP_FEATURES: &'static [ClapFeature];

    /// The audio ports.
    const PORTS: Ports;
    /// Whether the plugin has a note input and a note output port.
    const MIDI_INPUT: MidiConfig = MidiConfig::None;
    const MIDI_OUTPUT: MidiConfig = MidiConfig::None;
    /// Reported to the host's render extension.
    const HARD_REALTIME_ONLY: bool = false;

    /// The host parameters. Queried once, after creation.
    fn params(&self) -> Arc<Table>;
    /// Plugin-owned saved settings, prepared before and committed after a
    /// restore. Queried once, after creation, and used independently of the
    /// plugin's lock.
    fn setup(&self) -> Option<Arc<dyn Setup>> {
        None
    }
    /// Called once, before [`Plugin::editor`], with the host's side of the
    /// editor: the editor is the host's to destroy, so an editor with a close
    /// button of its own tells the host through this (#1607, #1609).
    fn editor_host(&mut self, host: Arc<dyn EditorHost>) {}
    /// The editor, if the plugin has one. Queried once, after creation.
    fn editor(&mut self) -> Option<Box<dyn Editor>> {
        None
    }

    // The main thread. Never during a process callback; do not call the host.

    fn init(&mut self) -> bool {
        true
    }
    /// The host activates the plugin at `config`. A latency asked for here is
    /// the one the shell tells the host, before the activation returns.
    fn activate(&mut self, config: &BufferConfig, latency: &Latency<'_>) -> bool {
        true
    }
    fn deactivate(&mut self) {}
    /// Main-thread service while deactivated, also just before activation.
    fn inactive(&mut self) {}
    fn destroy(&mut self) {}

    // The audio thread's lifecycle, inside the allocation guard.

    /// Processing starts, after a [`Plugin::reset`].
    fn start(&mut self) {}
    /// Processing stops.
    fn stop(&mut self) {}
    /// Discard what is sounding; also called when processing starts.
    fn reset(&mut self) {}

    // One process callback, inside the allocation guard.

    fn begin(&mut self, boundary: &Boundary) {}
    fn process(&mut self, block: &mut Block<'_>);
    fn end(&mut self) {}
}

/// What a process callback is, before its events are read.
#[derive(Clone, Copy, Debug)]
pub struct Boundary {
    /// The first sample on the host's steady clock.
    pub steady_time: i64,
    pub frames: u32,
    pub sample_rate: f64,
    /// The callback's own transport, as at its first sample.
    pub transport: Option<clap_event_transport>,
    /// The boundary itself can be used: a steady time from zero, and from 1
    /// to the activation's maximum frames. [`Plugin::process`] is called only
    /// for a valid one.
    pub valid: bool,
    /// The callback's input is lost: its list failed validation, or a flush
    /// since the last callback failed it. [`Plugin::process`] then reads no
    /// events and writes to an output that refuses everything, and the shell
    /// returns an error to the host.
    pub input_lost: bool,
}

/// One process callback, as [`Plugin::process`] reads and writes it.
pub struct Block<'a> {
    pub boundary: Boundary,
    pub(crate) events: List<'a>,
    pub output: Output<'a>,
    /// `None` when the input is lost, whose audio the host discards, and
    /// when there is no main output to write the audio to: the plugin
    /// declares none, or the host gave fewer channels than it declares.
    pub audio: Option<Audio<'a>>,
    pub(crate) latency: Latency<'a>,
}

impl<'a> Block<'a> {
    /// Every event of the callback in order: the callback's own transport
    /// first, at its first sample, then the host's list. As often as needed:
    /// each walk reads the host's list afresh. Empty when the input is lost.
    pub fn events(&self) -> impl Iterator<Item = Input> + use<'a> {
        let start = self.boundary.steady_time;
        let transport = self
            .boundary
            .transport
            .filter(|_| !self.boundary.input_lost)
            .map(|transport| (0, InputValue::Transport(transport.into())));
        transport
            .into_iter()
            .chain(self.events.values())
            .map(move |(offset, value)| Input { sample: start + i64::from(offset), value })
    }

    /// Ask the host for `samples` of latency. Published only across an
    /// activation (#615): while one runs, a change asks the host for the
    /// restart that adopts it.
    pub fn request_latency(&self, samples: u32) {
        self.latency.request(samples);
    }
}

/// The latency the host reads, and the one the plugin last asked for.
#[derive(Default)]
pub(crate) struct LatencyState {
    /// What the host reads: only an activation moves it.
    pub current: AtomicU32,
    pub pending: AtomicU32,
    /// A running activation was asked for a latency it does not report.
    pub changed: AtomicBool,
}

/// The plugin's handle for asking for a latency.
#[derive(Clone, Copy)]
pub struct Latency<'a> {
    pub(crate) state: &'a LatencyState,
    /// The plugin is active: a change waits for a restart, which the main
    /// thread asks for. Clear during activation, which adopts the request.
    pub(crate) active: bool,
    /// Asks the host for the main-thread call that asks for the restart.
    pub(crate) wake: Option<&'a AtomicBool>,
}

impl Latency<'_> {
    pub fn request(&self, samples: u32) {
        let previous = self.state.pending.swap(samples, Ordering::SeqCst);
        if self.active && previous != samples {
            self.state.changed.store(true, Ordering::Release);
            if let Some(wake) = self.wake {
                wake.store(true, Ordering::Release);
            }
        }
    }
}
