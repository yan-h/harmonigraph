//! Instance settings and diagnostics. Saved participation and visibility are
//! independent; pairing remains automatic and has no persisted identity.
use nice_plug::plugin::PluginState;
use nice_plug::wrapper::clap::setup::{Prepared, Serviced, Setup};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use super::session;

/// The Tuning delay setting, as multiplier, samples and milliseconds together.
/// `active` is what this activation adopted and `requested` what the setting
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
    track_name: Mutex<String>,
    pub held: AtomicU64,
    /// Cosmetic voices: channel * 128 + key in the upper word, f32 MIDI pitch
    /// bits in the lower word; MAX is empty. Identity and pitch are atomic
    /// together. Readers can span callbacks without blocking the audio owner.
    pub voices: [AtomicU64; super::HELD_PER_SOURCE],
    pub notes_in: AtomicU64,
    pub notes_out: AtomicU64,
    pub last_input: AtomicI64,
    pub last_output: AtomicI64,
    /// The saved Tuning delay, in buffers. A Tune adopts it at activation and
    /// reports the latency it implies; the Hub's own input has no delay.
    pub requested_multiplier: AtomicU32,
    pub(super) diagnostics: super::diagnostics::Shared,
    wake: OnceLock<Arc<dyn Fn() + Send + Sync>>,
    dirty: AtomicBool,
    pub status: AtomicU32,
    /// The multiplier this activation adopted; zero until the first one. The
    /// setting holds what is requested, so the pair is what the display
    /// compares while a latency change waits for the host.
    pub active_multiplier: AtomicU32,
    /// The host format this activation advertised, for the delay display. Zero
    /// frames means no activation has been seen yet.
    pub active_frames: AtomicU32,
    active_rate: AtomicU64,
    /// Onsets this instance emitted without a correction, since the last cut
    /// that was not a transport Stop's.
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
            retune: AtomicU64::new(u64::from(!hub)),
            show: AtomicBool::new(true),
            name: Mutex::new(String::new()),
            track_name: Mutex::new(String::new()),
            held: AtomicU64::new(0),
            voices: std::array::from_fn(|_| AtomicU64::new(u64::MAX)),
            notes_in: AtomicU64::new(0),
            notes_out: AtomicU64::new(0),
            last_input: AtomicI64::new(i64::MIN),
            last_output: AtomicI64::new(i64::MIN),
            requested_multiplier: AtomicU32::new(1),
            diagnostics: super::diagnostics::Shared::new(hub),
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
    fn store_retune(&self, value: bool) {
        let _ = self.retune.try_update(Ordering::AcqRel, Ordering::Acquire, |old| {
            ((old & 1 != 0) != value).then_some(((old & !1) + 2) | u64::from(value))
        });
    }
    pub fn set_retune(&self, value: bool) {
        self.store_retune(value);
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
        let track_name = self.track_name.lock().unwrap();
        if !track_name.is_empty() {
            return track_name.clone();
        }
        if self.hub {
            "Harmonigraph input".to_owned()
        } else {
            format!("Tune {}", self.instance_id.load(Ordering::Acquire))
        }
    }
    fn store_name(&self, value: String) {
        *self.name.lock().unwrap() = value.chars().take(80).collect();
    }
    pub fn set_name(&self, value: String) {
        self.store_name(value);
        self.changed();
    }
    /// A saved setting, so the project is marked modified. The latency it
    /// implies is reported from the Tune's own process callback, which is what
    /// asks the host to reactivate it.
    pub fn request_delay(&self, value: u32) {
        if !self.hub {
            self.requested_multiplier.store(clamp_delay(value), Ordering::Release);
            self.changed();
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
    pub fn status(&self) -> u32 {
        self.status.load(Ordering::Acquire)
    }
    pub fn request_main(&self) {
        if let Some(wake) = self.wake.get() {
            wake();
        }
    }
}

pub(super) fn clamp_delay(value: u32) -> u32 {
    value.clamp(1, super::DELAY_MULTIPLIER_MAX as u32)
}

/// Arc adapter, so the wrapper's main-thread service reaches this instance.
pub struct Adapter(pub Arc<Shared>);

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(default)]
struct InstanceSettings {
    retune: bool,
    show: bool,
    name: String,
    /// A Tune's Tuning delay, in buffers. The Hub's own input has none, and
    /// saves none.
    #[serde(skip_serializing_if = "Option::is_none")]
    delay: Option<u32>,
}
impl Default for InstanceSettings {
    fn default() -> Self {
        Self { retune: true, show: true, name: String::new(), delay: None }
    }
}
struct Restore(Arc<Shared>, InstanceSettings);
impl Prepared for Restore {
    /// What the project holds, stored without the edit setters' `changed()`:
    /// opening a project is not a change to it.
    fn commit(self: Box<Self>) {
        let Self(shared, settings) = *self;
        shared.store_retune(settings.retune);
        shared.show.store(settings.show, Ordering::Release);
        shared.store_name(settings.name);
        if !shared.hub {
            let delay = clamp_delay(settings.delay.unwrap_or(1));
            shared.requested_multiplier.store(delay, Ordering::Release);
        }
        shared.publish_controls();
    }
}

impl Setup for Adapter {
    fn track_name_changed(&self, name: Option<&str>) {
        *self.0.track_name.lock().unwrap() = name.unwrap_or_default().to_owned();
    }
    fn prepare(&self, state: &PluginState) -> Result<Box<dyn Prepared>, &'static str> {
        let settings = match state.fields.get("tuning-instance") {
            Some(json) => {
                let settings: InstanceSettings = serde_json::from_str(json).map_err(|error| {
                    nice_plug::nice_error!("Tuning instance settings refused: {error}");
                    "invalid tuning instance settings"
                })?;
                if !self.0.is_hub() && settings.delay.is_none() {
                    nice_plug::nice_warn!(
                        "This Tune's settings hold no Tuning delay; it opens at 1x"
                    );
                }
                settings
            }
            // Nothing saved: the Hub's own input starts with Retune off, a Tune on.
            None => InstanceSettings { retune: !self.0.is_hub(), ..Default::default() },
        };
        Ok(Box::new(Restore(self.0.clone(), settings)))
    }
    fn save(&self, state: &mut PluginState) {
        let settings = InstanceSettings {
            retune: self.0.retuning() & 1 != 0,
            show: self.0.show.load(Ordering::Acquire),
            name: self.0.name.lock().unwrap().clone(),
            delay: (!self.0.is_hub()).then(|| self.0.requested_multiplier.load(Ordering::Acquire)),
        };
        state
            .fields
            .insert("tuning-instance".to_owned(), serde_json::to_string(&settings).unwrap());
    }
    fn install_wakeup(&self, wake: Box<dyn Fn() + Send + Sync>) {
        assert!(self.0.wake.set(wake.into()).is_ok());
    }
    fn service(&self) -> Serviced {
        self.0.diagnostics.log(&self.0);
        let modified = self.0.dirty.swap(false, Ordering::AcqRel);
        Serviced { modified, values: false, text: false }
    }
}
