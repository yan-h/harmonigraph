//! Instance settings and diagnostics. Saved participation and visibility are
//! independent; pairing remains automatic and has no persisted identity.
use nice_plug::plugin::PluginState;
use nice_plug::wrapper::clap::setup::{Prepared, Setup};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

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
    pub instance_id: AtomicU64,
    pub slot: AtomicU32,
    /// Low bit is enabled; upper bits change on every participation edit so
    /// even an off/on between callbacks forgets this source's old context.
    retune: AtomicU64,
    pub show: AtomicBool,
    pub name: Mutex<String>,
    pub held: AtomicU64,
    pub notes_in: AtomicU64,
    pub notes_out: AtomicU64,
    pub last_input: AtomicI64,
    pub last_output: AtomicI64,
    pub requested_multiplier: AtomicU32,
    pending_multiplier: AtomicU32,
    pub(super) diagnostics: super::diagnostics::Shared,
    pub(super) neighbourhood: super::neighbourhood::Published,
    /// The Reset counter the editor bumps
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
            instance_id: AtomicU64::new(0),
            slot: AtomicU32::new(u32::MAX),
            retune: AtomicU64::new(1),
            show: AtomicBool::new(true),
            name: Mutex::new(String::new()),
            held: AtomicU64::new(0),
            notes_in: AtomicU64::new(0),
            notes_out: AtomicU64::new(0),
            last_input: AtomicI64::new(i64::MIN),
            last_output: AtomicI64::new(i64::MIN),
            requested_multiplier: AtomicU32::new(1),
            pending_multiplier: AtomicU32::new(0),
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
            hub,
        })
    }
    pub fn publish_format(&self, rate: f64, frames: u32) {
        self.active_frames.store(frames, Ordering::Release);
        self.active_rate.store(rate.to_bits(), Ordering::Release);
    }
    pub fn retuning(&self) -> u64 {
        self.retune.load(Ordering::Acquire)
    }
    fn changed(&self) {
        self.dirty.store(true, Ordering::Release);
        self.request_main();
    }
    pub fn set_retune(&self, value: bool) {
        let _ = self.retune.fetch_update(Ordering::AcqRel, Ordering::Acquire, |old| {
            ((old & 1 != 0) != value).then_some(((old & !1) + 2) | u64::from(value))
        });
        self.publish_controls();
        self.changed();
    }
    pub fn set_show(&self, value: bool) {
        self.show.store(value, Ordering::Release);
        self.publish_controls();
        self.changed();
    }
    /// UI/main thread only. A stopped tuner still has a visible held set, so
    /// control edits must reach the Hub without waiting for that tuner's audio.
    fn publish_controls(&self) {
        let slot = self.slot.load(Ordering::Acquire);
        if slot < super::TUNERS as u32 {
            session::session().row(slot as u8).update_controls(self);
        }
    }
    pub fn display_name(&self) -> String {
        let name = self.name.lock().unwrap();
        if !name.is_empty() {
            return name.clone();
        }
        if self.hub {
            "Harmonigraph input".to_owned()
        } else {
            format!("Tune {}", self.instance_id.load(Ordering::Acquire))
        }
    }
    pub fn set_name(&self, value: String) {
        *self.name.lock().unwrap() = value.chars().take(80).collect();
        self.changed();
    }
    pub fn request_delay(&self, value: u32) {
        if !self.hub {
            let value = value.clamp(1, super::DELAY_MULTIPLIER_MAX as u32);
            self.requested_multiplier.store(value, Ordering::Release);
            self.pending_multiplier.store(value, Ordering::Release);
            self.request_main();
        }
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
    fn pending_delay(&self) -> Option<u32> {
        let requested = self.pending_multiplier.swap(0, Ordering::AcqRel);
        (requested != 0).then_some(requested)
    }
}

/// Arc adapter, so the wrapper's main-thread service reaches this instance. A
/// Tune also hands over its delay parameter so a central timing edit is saved
/// and reported to that instance's own host.
pub struct Adapter(pub Arc<Shared>, pub Option<Arc<super::plugin::TuneParams>>);

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(default)]
struct InstanceSettings {
    retune: bool,
    show: bool,
    name: String,
}
impl Default for InstanceSettings {
    fn default() -> Self {
        Self { retune: true, show: true, name: String::new() }
    }
}
struct Restore(Arc<Shared>, InstanceSettings);
impl Prepared for Restore {
    fn commit(self: Box<Self>) {
        self.0.set_retune(self.1.retune);
        self.0.set_show(self.1.show);
        self.0.set_name(self.1.name);
    }
}

impl Setup for Adapter {
    fn prepare(&self, state: &PluginState) -> Result<Box<dyn Prepared>, &'static str> {
        let settings = match state.fields.get("tuning-instance") {
            Some(json) => serde_json::from_str(json).map_err(|error| {
                nice_plug::nice_error!("Tuning instance settings refused: {error}");
                "invalid tuning instance settings"
            })?,
            None => InstanceSettings::default(),
        };
        Ok(Box::new(Restore(self.0.clone(), settings)))
    }
    fn save(&self, state: &mut PluginState) {
        let settings = InstanceSettings {
            retune: self.0.retuning() & 1 != 0,
            show: self.0.show.load(Ordering::Acquire),
            name: self.0.name.lock().unwrap().clone(),
        };
        state
            .fields
            .insert("tuning-instance".to_owned(), serde_json::to_string(&settings).unwrap());
    }
    fn install_wakeup(&self, wake: Box<dyn Fn() + Send + Sync>) {
        assert!(self.0.wake.set(wake.into()).is_ok());
    }
    fn service(&self) -> bool {
        self.0.diagnostics.log(&self.0);
        let adopted = self.adopt_requested_delay();
        if let Some(params) = &self.1 {
            self.0
                .requested_multiplier
                .store(params.delay.value().max(1) as u32, Ordering::Release);
        }
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
