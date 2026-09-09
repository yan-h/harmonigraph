//! Instance-owned off-audio state: the diagnostics snapshot, the published
//! next-attack context, the status bits and the one button left.
//!
//! Nothing here is persisted. The Tune's pairing UUID, its routing offset and
//! its Participating flag are gone, and so is the Hub's session UUID: pairing
//! is a load of one process-wide slot, so there is no saved choice for a
//! project to restore and no calibration for it to carry.
use nice_plug::plugin::PluginState;
use nice_plug::wrapper::clap::setup::{Prepared, Setup};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

use super::session;

/// The Tuning delay setting, as multiplier, samples and milliseconds together.
/// `active` is what this activation adopted and `requested` what the parameter
/// now says; they differ exactly while a latency change waits for the host to
/// reactivate, which is the only moment the two numbers are worth showing.
pub fn delay_text(requested: u32, active: u32, frames: u32, rate: f64) -> String {
    let describe = |multiplier: u32| {
        let samples = u64::from(multiplier) * u64::from(frames);
        if frames == 0 || rate <= 0.0 {
            return format!("{multiplier}x buffer");
        }
        format!(
            "{multiplier}x buffer - {samples} samples / {:.2} ms",
            samples as f64 * 1000.0 / rate
        )
    };
    if active == 0 {
        return format!("Tuning delay {} - waiting for the host format", describe(requested));
    }
    if requested == active {
        return format!("Tuning delay {}", describe(active));
    }
    format!(
        "Tuning delay {} · requested {requested}x, waiting for host reactivation",
        describe(active)
    )
}

/// The missed-correction status. Raising the multiplier is the remedy for
/// exactly one shape of miss, so the two states that a larger delay would not
/// help — no Hub in the process, and no free row — are named rather than
/// counted as evidence that D is too small.
pub fn deadline_text(status: u32, misses: u64) -> String {
    if status & (session::NO_HUB | session::NO_ROW) != 0 {
        return "Notes are passing through uncorrected; a larger delay is not the remedy"
            .to_owned();
    }
    if misses == 0 {
        return "No missed correction deadlines".to_owned();
    }
    if misses == 1 {
        return "1 note sounded uncorrected - raise the delay and play again".to_owned();
    }
    format!("{misses} notes sounded uncorrected - raise the delay and play again")
}

pub struct Shared {
    pub(super) diagnostics: super::diagnostics::Shared,
    pub(super) neighbourhood: super::neighbourhood::Published,
    /// The whole of this instance's setup: a Reset counter the editor bumps
    /// and the audio owner turns into one cut for the session.
    reset: AtomicU64,
    wake: OnceLock<Arc<dyn Fn() + Send + Sync>>,
    dirty: AtomicBool,
    pub status: AtomicU32,
    /// The multiplier this activation adopted; zero until the first one. The
    /// parameter holds what is requested, so the pair is what the display
    /// compares while a latency change waits for the host.
    pub active_multiplier: AtomicU32,
    /// The host format this activation advertised, for the delay display. Zero
    /// frames means no activation has been seen yet.
    pub active_frames: AtomicU32,
    active_rate: AtomicU64,
    /// Onsets this instance emitted without a correction, since its last cut.
    pub misses: AtomicU64,
    /// The session's "apply to all" request this instance has already adopted.
    /// A generation rather than a consumed value, so every Tune sees the same
    /// request and none of them races another for it.
    adopted_delay_request: AtomicU64,
    hub: bool,
}

impl Shared {
    pub fn hub() -> Arc<Self> {
        Self::new(true)
    }
    pub fn source() -> Arc<Self> {
        Self::new(false)
    }
    fn new(hub: bool) -> Arc<Self> {
        Arc::new(Self {
            diagnostics: super::diagnostics::Shared::new(hub),
            neighbourhood: Default::default(),
            reset: AtomicU64::new(0),
            wake: OnceLock::new(),
            dirty: AtomicBool::new(false),
            status: AtomicU32::new(0),
            active_multiplier: AtomicU32::new(0),
            active_frames: AtomicU32::new(0),
            active_rate: AtomicU64::new(0),
            misses: AtomicU64::new(0),
            adopted_delay_request: AtomicU64::new(0),
            hub,
        })
    }
    pub fn publish_format(&self, rate: f64, frames: u32) {
        self.active_frames.store(frames, Ordering::Release);
        self.active_rate.store(rate.to_bits(), Ordering::Release);
    }
    /// Sample rate and maximum callback size, as this activation advertised.
    pub fn format(&self) -> (f64, u32) {
        (
            f64::from_bits(self.active_rate.load(Ordering::Acquire)),
            self.active_frames.load(Ordering::Acquire),
        )
    }
    pub fn is_hub(&self) -> bool {
        self.hub
    }
    pub fn neighbourhood(&self) -> Option<harmonigraph_core::policy::reach::Snapshot> {
        self.neighbourhood.read()
    }
    /// The editor's Reset. Clear the musical context and re-pair, which the
    /// audio owner performs as one cut across every row.
    pub fn request_reset(&self) {
        self.reset.fetch_add(1, Ordering::AcqRel);
        self.dirty.store(true, Ordering::Release);
        self.request_main();
    }
    /// The generation an audio owner compares against its own. A change is a
    /// Reset; there is no acknowledgement and nothing to wait for.
    pub fn reset_generation(&self) -> u64 {
        self.reset.load(Ordering::Acquire)
    }
    pub fn status(&self) -> u32 {
        self.status.load(Ordering::Acquire)
    }
    pub fn request_main(&self) {
        if let Some(wake) = self.wake.get() {
            wake();
        }
    }
    /// Ask every Tune in the process to adopt one multiplier. The Hub owns the
    /// request; each Tune owns the parameter it then writes.
    pub fn request_delay_for_all(multiplier: u32) {
        session::session().request_delay(multiplier);
    }
    /// The multiplier this instance still owes the session, or none.
    fn pending_delay(&self) -> Option<u32> {
        let (generation, multiplier) = session::session().delay_request();
        (generation != 0
            && self.adopted_delay_request.swap(generation, Ordering::AcqRel) != generation)
            .then_some(multiplier)
    }
}

/// Arc adapter, so the wrapper's main-thread service reaches this instance. A
/// Tune also hands over its parameters, because the only value the Hub can ask
/// it to change is one this instance owns and must write itself.
pub struct Adapter(pub Arc<Shared>, pub Option<Arc<super::plugin::TuneParams>>);

/// Nothing is restored, so preparation cannot fail and commit has nothing to
/// publish. The two field keys the old design saved are simply absent now; a
/// project that still carries them is read without them.
struct Nothing;
impl Prepared for Nothing {
    fn commit(self: Box<Self>) {}
}

impl Setup for Adapter {
    fn prepare(&self, _: &PluginState) -> Result<Box<dyn Prepared>, &'static str> {
        Ok(Box::new(Nothing))
    }
    fn save(&self, _: &mut PluginState) {}
    fn install_wakeup(&self, wake: Box<dyn Fn() + Send + Sync>) {
        assert!(self.0.wake.set(wake.into()).is_ok());
    }
    fn service(&self) -> bool {
        self.0.diagnostics.log(&self.0);
        let adopted = self.adopt_requested_delay();
        self.0.dirty.swap(false, Ordering::AcqRel) || adopted
    }
}

impl Adapter {
    /// Take a Hub's "apply to all" request, if there is one. Writing the
    /// parameter here and reporting true is exactly what this trait's `true`
    /// means — a host parameter rescan and a dirty state — so the value the
    /// project saves is the value on screen. The latency that follows is
    /// reported from this Tune's own process callback, which is what asks the
    /// host to reactivate it.
    fn adopt_requested_delay(&self) -> bool {
        let Some(params) = &self.1 else { return false };
        let Some(requested) = self.0.pending_delay() else { return false };
        let plain = (requested as i32).clamp(1, super::DELAY_MULTIPLIER_MAX);
        if plain == params.delay.value() {
            return false;
        }
        // SAFETY: the pointer is into `params`, which this adapter keeps alive
        // for as long as the plugin instance, and the write is the same atomic
        // store the wrapper makes for a host parameter event.
        use nice_plug::prelude::Param;
        unsafe {
            params
                .delay
                .as_ptr()
                ._internal_set_normalized_value(Param::preview_normalized(&params.delay, plain));
        }
        true
    }
}
