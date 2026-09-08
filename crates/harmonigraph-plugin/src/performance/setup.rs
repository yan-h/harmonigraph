//! Instance-owned nonautomatable setup. Parsing, UUID selection, rematching,
//! dirty notifications and allocation stay outside the serialized audio owner.
use nice_plug::plugin::{ParamValue, PluginState};
use nice_plug::wrapper::clap::setup::{Prepared, Setup};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use super::{
    clock::Calibration,
    registry::{self, HubBridge, SourceBridge},
    routing::{HubSetup, SourceSetup},
    slots::{OwnedReservation, Slots},
};

pub const HUB_FIELD: &str = "harmonigraph-session";
pub const SOURCE_FIELD: &str = "harmonigraph-tune-pairing";
pub const PARTICIPATING: &str = "participating";
/// Noncanceling timing summary, separate from the emergency fault bits.
pub const TIMING_FAILURE: u32 = 1 << 5;

/// Cosmetic UI text only. Reads published values without clearing them or
/// interpreting a setup request as acknowledged recovery. Never called on audio.
pub fn diagnostics_text(status: u32) -> String {
    use super::source::{CLOCK_FAULT, INPUT_FAULT, OUTPUT_FAULT, REFERENCE_FAULT, STORAGE_FAULT};
    let mut lines = Vec::new();
    if status == 0 {
        lines.push("No timing or emergency fault reported".to_owned());
    }
    if status & TIMING_FAILURE != 0 {
        // The latch can outlive the delayed attack, so do not claim it is pending.
        lines.push("Timing failure: notes delayed".to_owned());
    }
    let faults = [
        (STORAGE_FAULT, "retention capacity reached"),
        (OUTPUT_FAULT, "output failure"),
        (CLOCK_FAULT, "clock failure"),
        (INPUT_FAULT, "input failure"),
        (REFERENCE_FAULT, "reference/session capacity reached"),
    ];
    let reasons: Vec<_> =
        faults.iter().filter(|(bit, _)| status & bit != 0).map(|(_, reason)| *reason).collect();
    if !reasons.is_empty() {
        lines.push(format!("Emergency: {}. Reset / valid recovery required.", reasons.join(", ")));
    }
    let known = faults.iter().fold(TIMING_FAILURE, |mask, (bit, _)| mask | bit);
    if status & !known != 0 {
        lines.push("Unrecognized recovery status".to_owned());
    }
    lines.join("\n")
}

/// The Tuning delay setting, as multiplier, samples and milliseconds together.
/// `active` is what this activation adopted and `requested` what the parameter
/// now says; they differ exactly while a latency change waits for the host to
/// reactivate, which is the only moment the two numbers are worth showing at
/// once. Zero frames means no activation has been seen yet.
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

/// The missed-deadline status, and the three states that are not one. Raising
/// the multiplier is the remedy for exactly one of them; a session that has
/// not started, a Hub that is missing or ambiguous, and a latched output or
/// resource fault would each keep notes late at any multiplier, so they are
/// named here instead of being counted as evidence that D is too small.
///
/// `pairing` is the Tune's registry status, absent for a Hub, which has no
/// pairing of its own to be missing.
pub fn deadline_text(
    status: u32,
    pairing: Option<u32>,
    starting: bool,
    misses: u64,
    worst: u64,
    rate: f64,
) -> String {
    use super::source::{CLOCK_FAULT, INPUT_FAULT, OUTPUT_FAULT, REFERENCE_FAULT, STORAGE_FAULT};
    let faults = STORAGE_FAULT | OUTPUT_FAULT | CLOCK_FAULT | INPUT_FAULT | REFERENCE_FAULT;
    if status & faults != 0 {
        return "Deadline status unavailable: an output or resource fault is latched, which a \
                larger delay does not address"
            .to_owned();
    }
    if let Some(reason) = pairing.and_then(|status| match status {
        registry::MISSING => Some("no matching hub"),
        registry::AMBIGUOUS => Some("ambiguous hub UUID"),
        registry::OVERCAPACITY => Some("session capacity reached"),
        _ => None,
    }) {
        return format!("Notes are not being assigned ({reason}) - a larger delay is not a remedy");
    }
    if starting {
        return "Waiting for the session to start - startup is not a missed deadline".to_owned();
    }
    if misses == 0 {
        return "No missed deadlines".to_owned();
    }
    let lateness = if rate > 0.0 {
        format!("{:.2} ms", worst as f64 * 1000.0 / rate)
    } else {
        format!("{worst} samples")
    };
    let notes = if misses == 1 { "1 note missed its deadline" } else {
        &format!("{misses} notes missed their deadline")
    };
    format!("{notes} - worst lateness {lateness}")
}

#[cfg(test)]
#[derive(Default)]
pub struct TestPause {
    pub enabled: AtomicBool,
    pub entered: AtomicBool,
}
#[cfg(test)]
impl TestPause {
    pub fn reach(&self) {
        if self.enabled.load(Ordering::Acquire) {
            self.entered.store(true, Ordering::Release);
            while self.enabled.load(Ordering::Acquire) {
                std::thread::yield_now();
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Routing {
    Hub(HubSetup),
    Source(SourceSetup),
}
#[derive(Clone, Copy, Debug)]
pub struct Update {
    pub generation: u64,
    pub pairing_generation: u64,
    pub routing: Routing,
    pub participating: Option<bool>,
    pub reset: bool,
}
impl Routing {
    pub fn calibration(self) -> Calibration {
        match self {
            Self::Hub(h) => h.calibration,
            Self::Source(s) => s.calibration,
        }
    }
}
struct Main {
    value: Update,
    next: u64,
    registration: Option<u64>,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Adopted {
    pub generation: u64,
    pub calibration: Calibration,
    pub sample_rate: f64,
    pub max_frames: u32,
    pub valid: bool,
}
#[derive(Default)]
struct Adoption {
    sequence: AtomicU64,
    generation: AtomicU64,
    offset: AtomicI64,
    rate: AtomicU64,
    frames: AtomicU32,
    valid: AtomicBool,
}
impl Adoption {
    // The serialized performance owner is the only writer. All fields use the
    // same sequentially consistent order: a GUI read that overlaps a write
    // either observes one whole immutable calibration or skips this refresh.
    fn publish(&self, value: Adopted) {
        self.sequence.fetch_add(1, Ordering::SeqCst);
        self.generation.store(value.generation, Ordering::SeqCst);
        self.offset.store(value.calibration.offset, Ordering::SeqCst);
        self.rate.store(value.sample_rate.to_bits(), Ordering::SeqCst);
        self.frames.store(value.max_frames, Ordering::SeqCst);
        self.valid.store(value.valid, Ordering::SeqCst);
        self.sequence.fetch_add(1, Ordering::SeqCst);
    }
    fn read(&self) -> Option<Adopted> {
        let sequence = self.sequence.load(Ordering::SeqCst);
        if sequence == 0 || sequence & 1 != 0 {
            return None;
        }
        let generation = self.generation.load(Ordering::SeqCst);
        let offset = self.offset.load(Ordering::SeqCst);
        let sample_rate = f64::from_bits(self.rate.load(Ordering::SeqCst));
        let max_frames = self.frames.load(Ordering::SeqCst);
        let valid = self.valid.load(Ordering::SeqCst);
        (self.sequence.load(Ordering::SeqCst) == sequence).then_some(Adopted {
            generation,
            calibration: Calibration { offset },
            sample_rate,
            max_frames,
            valid,
        })
    }
}
pub struct Shared {
    pub(super) diagnostics: super::diagnostics::Shared,
    #[cfg(test)]
    pub before_transfer: TestPause,
    #[cfg(test)]
    pub after_offer_take: TestPause,
    #[cfg(test)]
    pub before_direct_repair: TestPause,
    main: Mutex<Main>,
    pub updates: Arc<Slots<Update>>,
    pub hub: Option<Arc<HubBridge>>,
    pub source: Option<Arc<SourceBridge>>,
    wake: OnceLock<Arc<dyn Fn() + Send + Sync>>,
    dirty: AtomicBool,
    preparing: AtomicBool,
    pub applied: AtomicU64,
    adopted: Adoption,
    pub status: AtomicU32,
    /// Worst lateness in samples and notes counted once each, over the
    /// interval since the delay setting was last applied.
    pub extra_delay: AtomicU64,
    pub deadline_misses: AtomicU64,
    /// The multiplier this activation adopted; zero until the first one. The
    /// parameter holds what is requested, so the pair is what the display
    /// compares while a latency change waits for the host.
    pub active_multiplier: AtomicU32,
}
impl Shared {
    pub fn hub() -> Arc<Self> {
        Self::new(Routing::Hub(HubSetup::default()))
    }
    pub fn source() -> Arc<Self> {
        Self::new(Routing::Source(SourceSetup::default()))
    }
    fn new(routing: Routing) -> Arc<Self> {
        Arc::new(Self {
            diagnostics: super::diagnostics::Shared::new(matches!(routing, Routing::Hub(_))),
            #[cfg(test)]
            before_transfer: TestPause::default(),
            #[cfg(test)]
            after_offer_take: TestPause::default(),
            #[cfg(test)]
            before_direct_repair: TestPause::default(),
            main: Mutex::new(Main {
                value: Update {
                    generation: 0,
                    pairing_generation: 1,
                    routing,
                    participating: None,
                    reset: false,
                },
                next: 0,
                registration: None,
            }),
            updates: Arc::new(Slots::default()),
            hub: matches!(routing, Routing::Hub(_)).then(|| Arc::new(HubBridge::default())),
            source: matches!(routing, Routing::Source(_))
                .then(|| Arc::new(SourceBridge::default())),
            wake: OnceLock::new(),
            dirty: AtomicBool::new(false),
            preparing: AtomicBool::new(false),
            applied: AtomicU64::new(0),
            adopted: Adoption::default(),
            status: AtomicU32::new(0),
            extra_delay: AtomicU64::new(0),
            deadline_misses: AtomicU64::new(0),
            active_multiplier: AtomicU32::new(0),
        })
    }
    pub fn value(&self) -> Update {
        self.main.lock().unwrap().value
    }
    pub fn adopted(&self) -> Option<Adopted> {
        self.adopted.read()
    }
    pub fn publish_clock(&self, clock: &super::clock::Clock) {
        self.adopted.publish(Adopted {
            generation: self.applied.load(Ordering::Acquire),
            calibration: clock.calibration,
            sample_rate: clock.sample_rate,
            max_frames: clock.max_frames,
            valid: clock.valid,
        });
    }
    pub fn registration(&self) -> Option<u64> {
        self.main.lock().unwrap().registration
    }

    pub fn register(&self) -> bool {
        let mut main = self.main.lock().unwrap();
        if main.registration.is_some() {
            return true;
        }
        let mut registry = registry::global().lock().unwrap();
        main.registration = match main.value.routing {
            Routing::Hub(hub) => {
                registry.register_hub(hub.uuid, self.hub.as_ref().unwrap().clone())
            }
            Routing::Source(source) => {
                registry.register_source(source.selected, self.source.as_ref().unwrap().clone())
            }
        };
        if main.registration.is_none() {
            if let Some(source) = &self.source {
                source.status.store(registry::OVERCAPACITY, Ordering::Release);
            }
            self.status.store(16, Ordering::Release);
        }
        main.registration.is_some()
    }
    pub fn request_main(&self) {
        if let Some(wake) = self.wake.get() {
            wake();
        }
    }

    fn prepare_value(
        self: &Arc<Self>,
        routing: Routing,
        participating: Option<bool>,
        reset: bool,
    ) -> Result<Box<dyn Prepared>, &'static str> {
        if self
            .preparing
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err("a setup restore is already being prepared");
        }
        let Some(slot) = self.updates.reserve_owned() else {
            self.preparing.store(false, Ordering::Release);
            return Err("both setup slots are retained");
        };
        let mut main = self.main.lock().unwrap();
        let Some(next) = main.next.checked_add(1) else {
            self.preparing.store(false, Ordering::Release);
            return Err("setup generation exhausted");
        };
        main.next = next;
        Ok(Box::new(Pending {
            shared: self.clone(),
            slot: Some(slot),
            value: Update {
                generation: main.next,
                pairing_generation: main.value.pairing_generation,
                routing,
                participating,
                reset,
            },
        }))
    }

    pub fn apply(self: &Arc<Self>, routing: Routing, reset: bool) -> Result<(), &'static str> {
        self.prepare_value(routing, None, reset)?.commit();
        Ok(())
    }
}

struct Pending {
    shared: Arc<Shared>,
    slot: Option<OwnedReservation<Update>>,
    value: Update,
}
impl Drop for Pending {
    fn drop(&mut self) {
        self.shared.preparing.store(false, Ordering::Release);
    }
}
impl Prepared for Pending {
    fn commit(mut self: Box<Self>) {
        {
            let mut main = self.shared.main.lock().unwrap();
            if let Some(id) = main.registration {
                let mut registry = registry::global().lock().unwrap();
                match self.value.routing {
                    Routing::Hub(hub) => registry.hub_uuid(id, hub.uuid),
                    Routing::Source(source) => registry.pair(id, source.selected, self.value.reset),
                }
            }
            if let Some(source) = &self.shared.source {
                self.value.pairing_generation = source.generation.load(Ordering::Acquire);
            }
            main.value = self.value;
            self.slot.take().unwrap().publish(self.value);
        }
        nice_plug::nice_log!(
            "HG-TUNING-SETUP pid={} instance={:?} accepted_gen={} pairing_gen={} reset={} routing={:?} previous_adopted={:?}",
            std::process::id(), self.shared.registration(), self.value.generation,
            self.value.pairing_generation, self.value.reset, self.value.routing, self.shared.adopted(),
        );
        self.shared.dirty.store(true, Ordering::Release);
        self.shared.request_main();
    }
}

/// Arc adapter lets a prepared value retain the instance's actual owned setup
/// slot across wrapper field restoration, without any borrowed self-reference.
/// A Tune also hands over its parameters, because the only value a Hub can ask
/// it to change is one this instance owns and must write itself.
pub struct Adapter(pub Arc<Shared>, pub Option<Arc<super::tune::TuneParams>>);
impl Setup for Adapter {
    fn prepare(&self, state: &PluginState) -> Result<Box<dyn Prepared>, &'static str> {
        let routing = if self.0.hub.is_some() {
            Routing::Hub(match state.fields.get(HUB_FIELD) {
                Some(json) => serde_json::from_str(json).map_err(|_| "invalid hub setup")?,
                None => HubSetup::default(),
            })
        } else {
            Routing::Source(match state.fields.get(SOURCE_FIELD) {
                Some(json) => serde_json::from_str(json).map_err(|_| "invalid tuner setup")?,
                None => SourceSetup::default(),
            })
        };
        let participating = if self.0.source.is_some() {
            match state.params.get(PARTICIPATING) {
                Some(ParamValue::Bool(value)) => Some(*value),
                None => None,
                _ => return Err("invalid participation value"),
            }
        } else {
            None
        };
        let reinitialize = self.0.value().routing != routing;
        self.0.prepare_value(routing, participating, reinitialize)
    }
    fn save(&self, state: &mut PluginState) {
        match self.0.value().routing {
            Routing::Hub(hub) => {
                state.fields.insert(HUB_FIELD.into(), serde_json::to_string(&hub).unwrap());
            }
            Routing::Source(source) => {
                state.fields.insert(SOURCE_FIELD.into(), serde_json::to_string(&source).unwrap());
            }
        }
    }
    fn install_wakeup(&self, wake: Box<dyn Fn() + Send + Sync>) {
        let wake: Arc<dyn Fn() + Send + Sync> = wake.into();
        if let Some(hub) = &self.0.hub {
            assert!(hub.wake.set(wake.clone()).is_ok());
        }
        if let Some(source) = &self.0.source {
            assert!(source.wake.set(wake.clone()).is_ok());
        }
        assert!(self.0.wake.set(wake).is_ok());
    }
    fn service(&self) -> bool {
        registry::service_retired();
        registry::global().lock().unwrap().service();
        self.0.diagnostics.log(&self.0);
        let adopted = self.adopt_requested_delay();
        self.0.dirty.swap(false, Ordering::AcqRel) || adopted
    }
}
impl Adapter {
    /// Take a Hub's "apply to all" request, if there is one. Writing the
    /// parameter here and reporting true is exactly what this trait's `true`
    /// means -- a host parameter rescan and a dirty state -- so the value the
    /// project saves is the value on screen. The latency that follows is
    /// reported from this Tune's own process callback, which is what asks the
    /// host to reactivate it.
    fn adopt_requested_delay(&self) -> bool {
        let (Some(params), Some(bridge)) = (&self.1, &self.0.source) else {
            return false;
        };
        let requested = bridge.requested_delay.swap(0, Ordering::AcqRel);
        let plain = (requested as i32).clamp(1, super::protocol::DELAY_MULTIPLIER_MAX);
        if requested == 0 || plain == params.delay.value() {
            return false;
        }
        // SAFETY: the pointer is into `params`, which this adapter keeps alive
        // for as long as the plugin instance, and the write is the same
        // atomic store the wrapper makes for a host parameter event.
        use nice_plug::prelude::Param;
        unsafe {
            params.delay.as_ptr()._internal_set_normalized_value(
                Param::preview_normalized(&params.delay, plain),
            );
        }
        true
    }
}
