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
pub fn diagnostics_text(status: u32, extra_delay: u64) -> String {
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
    lines.push(format!("Current extra delay: {extra_delay} samples (above fixed tuner latency)"));
    lines.join("\n")
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
    flags: AtomicU32,
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
        self.flags.store(
            u32::from(value.calibration.validated) | (u32::from(value.valid) << 1),
            Ordering::SeqCst,
        );
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
        let flags = self.flags.load(Ordering::SeqCst);
        (self.sequence.load(Ordering::SeqCst) == sequence).then_some(Adopted {
            generation,
            calibration: Calibration { offset, validated: flags & 1 != 0 },
            sample_rate,
            max_frames,
            valid: flags & 2 != 0,
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
    pub extra_delay: AtomicU64,
}
impl Shared {
    pub fn hub() -> Arc<Self> {
        Self::new(Routing::Hub(HubSetup::default()))
    }
    #[cfg(not(feature = "tuning-probe"))]
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
pub struct Adapter(pub Arc<Shared>);
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
        assert!(self.0.wake.set(wake).is_ok());
    }
    fn service(&self) -> bool {
        registry::service_retired();
        registry::global().lock().unwrap().service();
        self.0.diagnostics.log(&self.0);
        self.0.dirty.swap(false, Ordering::AcqRel)
    }
}
