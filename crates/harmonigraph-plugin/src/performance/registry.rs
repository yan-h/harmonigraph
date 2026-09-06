//! The only process-wide registry mutex. These entry points are main/off-thread
//! lifecycle/setup operations. They call no plugin or host while holding it.
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use super::{protocol::*, routing::SavedUuid, slots::Slots};

pub const REGISTRY_SESSIONS: usize = 4;
pub const REGISTRY_TUNER_LEASES: usize = 64;
pub const MISSING: u32 = 1;
pub const AMBIGUOUS: u32 = 2;
pub const OVERCAPACITY: u32 = 3;
pub const OFFERED: u32 = 4;
pub const ATTACHED: u32 = 5;

pub struct SourceOffer {
    pub generation: u64,
    pub lease: Lease,
    pub session: Arc<SessionControl>,
    pub endpoints: SourceEndpoints,
}
pub enum SourceReturn {
    Adopted { generation: u64, lease: Lease },
    Returned(SourceOffer),
}
pub struct HubOffer {
    pub session: Arc<SessionControl>,
    pub bank: Box<HubBank>,
}
pub struct SourceBridge {
    pub offers: Slots<SourceOffer>,
    pub returns: Slots<SourceReturn>,
    pub generation: AtomicU64,
    pub status: AtomicU32,
}
impl Default for SourceBridge {
    fn default() -> Self {
        Self {
            offers: Slots::default(),
            returns: Slots::default(),
            generation: AtomicU64::new(1),
            status: AtomicU32::new(MISSING),
        }
    }
}
pub struct HubBridge {
    pub offers: Slots<HubOffer>,
    pub returns: Slots<u64>,
    pub retired_pending: AtomicBool,
    pub wake: OnceLock<Arc<dyn Fn() + Send + Sync>>,
}
impl Default for HubBridge {
    fn default() -> Self {
        Self {
            offers: Slots::default(),
            returns: Slots::default(),
            retired_pending: AtomicBool::new(false),
            wake: OnceLock::new(),
        }
    }
}

struct HubEntry {
    id: u64,
    uuid: SavedUuid,
    bridge: Arc<HubBridge>,
    session: Arc<SessionControl>,
    sources: [Option<SourceEndpoints>; TUNERS],
    leases: [Option<u64>; TUNERS],
    returned: Option<HubOffer>,
    retired: bool,
    owner: Option<Box<super::hub::Hub>>,
    servicing: bool,
}
struct SourceEntry {
    id: u64,
    bridge: Arc<SourceBridge>,
    selected: Option<SavedUuid>,
    offered: Option<Lease>,
    pending: Option<SourceOffer>,
    returned: Option<SourceOffer>,
    retired: bool,
    owner: Option<Box<super::source::Source>>,
    servicing: bool,
}

pub struct Registry {
    hubs: [Option<HubEntry>; REGISTRY_SESSIONS],
    sources: [Option<SourceEntry>; REGISTRY_TUNER_LEASES],
    next: u64,
}
impl Default for Registry {
    fn default() -> Self {
        Self {
            hubs: std::array::from_fn(|_| None),
            sources: std::array::from_fn(|_| None),
            next: 0,
        }
    }
}

pub fn global() -> &'static Mutex<Registry> {
    static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(Registry::default()))
}

impl Registry {
    fn id(&mut self) -> Option<u64> {
        self.next = self.next.checked_add(1)?;
        Some(self.next)
    }

    pub fn register_hub(&mut self, uuid: SavedUuid, bridge: Arc<HubBridge>) -> Option<u64> {
        self.collect();
        let index = self.hubs.iter().position(Option::is_none)?;
        let id = self.id()?;
        let (bank, sources) = bank();
        let session = Arc::new(SessionControl {
            runtime: id,
            credits: std::sync::atomic::AtomicUsize::new(0),
            faults: AtomicU32::new(0),
            alive: std::sync::atomic::AtomicBool::new(true),
            epoch: AtomicU64::new(1),
            closing: AtomicU64::new(0),
            hub_through: std::sync::atomic::AtomicI64::new(i64::MIN),
            rows: std::array::from_fn(|_| Arc::new(SourceControl::default())),
        });
        // A newly constructed instance has no outstanding hub offer. Even a
        // defensive refusal retains the actual bank in this off-thread entry.
        let returned = bridge.offers.publish(HubOffer { session: session.clone(), bank }).err();
        self.hubs[index] = Some(HubEntry {
            id,
            uuid,
            bridge,
            session,
            sources,
            leases: [None; TUNERS],
            returned,
            retired: false,
            owner: None,
            servicing: false,
        });
        self.rematch();
        Some(id)
    }

    pub fn register_source(
        &mut self,
        selected: Option<SavedUuid>,
        bridge: Arc<SourceBridge>,
    ) -> Option<u64> {
        self.collect();
        let index = self.sources.iter().position(Option::is_none)?;
        let id = self.id()?;
        self.sources[index] = Some(SourceEntry {
            id,
            bridge,
            selected,
            offered: None,
            pending: None,
            returned: None,
            retired: false,
            owner: None,
            servicing: false,
        });
        self.rematch();
        Some(id)
    }

    pub fn pair(&mut self, id: u64, selected: Option<SavedUuid>, reinitialize: bool) {
        self.collect();
        if let Some(source) = self.sources.iter_mut().flatten().find(|s| s.id == id) {
            if source.selected != selected || reinitialize {
                source.selected = selected;
                if let Some(next) = source.bridge.generation.load(Ordering::Relaxed).checked_add(1)
                {
                    source.bridge.generation.store(next, Ordering::Release);
                } else {
                    source.bridge.status.store(OVERCAPACITY, Ordering::Release);
                    return;
                }
                if let Some(lease) = source.offered {
                    if let Some(hub) = self.hubs.iter().flatten().find(|h| h.id == lease.session) {
                        hub.session.rows[usize::from(lease.slot - 1)]
                            .withdrawn
                            .store(true, Ordering::Release);
                    }
                    // Cancel a still-Ready offer off audio. The exclusive slot
                    // claim loses harmlessly if the callback already took it;
                    // that callback then returns the whole stale generation.
                    if source.returned.is_none() {
                        source.returned =
                            source.pending.take().or_else(|| source.bridge.offers.take());
                    }
                }
            }
        }
        self.collect();
        self.rematch();
    }

    pub fn hub_uuid(&mut self, id: u64, uuid: SavedUuid) {
        if let Some(hub) = self.hubs.iter_mut().flatten().find(|h| h.id == id) {
            hub.uuid = uuid;
        }
        self.collect();
        self.rematch();
    }

    #[cfg(all(target_os = "macos", not(feature = "tuning-probe")))]
    pub fn candidates(&self) -> Vec<SavedUuid> {
        // Keep duplicates visible. Selecting their UUID cannot disambiguate two
        // restored copies of the same saved project.
        self.hubs.iter().flatten().filter(|h| !h.retired).map(|h| h.uuid).collect()
    }
    #[cfg(all(test, not(feature = "tuning-probe")))]
    pub fn test_session(&self, uuid: SavedUuid) -> Arc<SessionControl> {
        self.hubs.iter().flatten().find(|h| h.uuid == uuid && !h.retired).unwrap().session.clone()
    }
    #[cfg(test)]
    pub fn test_counts(&self) -> (usize, usize, usize) {
        (
            self.hubs.iter().flatten().count(),
            self.sources.iter().flatten().count(),
            self.sources.iter().flatten().filter(|s| s.retired).count(),
        )
    }
    #[cfg(all(test, not(feature = "tuning-probe")))]
    pub fn test_has_source(&self, id: u64) -> bool {
        self.sources.iter().flatten().any(|s| s.id == id)
    }
    #[cfg(all(test, not(feature = "tuning-probe")))]
    pub fn test_retained_hub(&self, id: u64) -> bool {
        self.hubs.iter().flatten().any(|h| h.id == id && h.retired && h.owner.is_some())
    }

    /// No audio caller waits for this bookkeeping. All sixteen audio adoptions
    /// can complete while every registry Adopted acknowledgement stays queued.
    pub fn collect(&mut self) {
        for source in self.sources.iter_mut().flatten() {
            for _ in 0..2 {
                match source.bridge.returns.take() {
                    Some(SourceReturn::Adopted { generation, lease }) => {
                        if source.offered == Some(lease)
                            && generation == source.bridge.generation.load(Ordering::Acquire)
                        {
                            source.bridge.status.store(ATTACHED, Ordering::Release);
                        }
                    }
                    Some(SourceReturn::Returned(offer)) => {
                        // The one-offer invariant excludes replacing an owned
                        // return. Keep a second whole value pinned if violated.
                        if source.returned.is_none() {
                            source.returned = Some(offer);
                        } else if source.pending.is_none() {
                            source.pending = Some(offer);
                        } else {
                            panic!("more returned endpoints than instance offers");
                        }
                    }
                    None => break,
                }
            }
            if let Some(offer) = source.returned.as_ref() {
                let row = &offer.session.rows[usize::from(offer.lease.slot - 1)];
                if row.source_detached.load(Ordering::Acquire)
                    && row.hub_detached.load(Ordering::Acquire)
                {
                    let offer = source.returned.take().unwrap();
                    if let Some(hub) =
                        self.hubs.iter_mut().flatten().find(|h| h.id == offer.lease.session)
                    {
                        let slot = usize::from(offer.lease.slot - 1);
                        assert!(hub.sources[slot].is_none());
                        hub.sources[slot] = Some(offer.endpoints);
                        hub.leases[slot] = None;
                        source.offered = None;
                    }
                }
            }
        }
        for hub in self.hubs.iter_mut().flatten() {
            for _ in 0..2 {
                match hub.bridge.returns.take() {
                    Some(id) => assert_eq!(id, hub.id),
                    None => break,
                }
            }
        }
        for source in &mut self.sources {
            if source.as_ref().is_some_and(|s| {
                s.retired
                    && !s.servicing
                    && s.owner.is_none()
                    && s.offered.is_none()
                    && s.pending.is_none()
                    && s.returned.is_none()
            }) {
                *source = None;
            }
        }
        for hub in &mut self.hubs {
            if hub.as_ref().is_some_and(|h| {
                h.retired
                    && !h.servicing
                    && h.owner.is_none()
                    && h.returned.is_some()
                    && h.leases.iter().all(Option::is_none)
                    && h.session.credits.load(Ordering::Acquire) == 0
            }) {
                *hub = None;
            }
        }
    }

    pub fn service(&mut self) {
        self.collect();
        self.rematch();
    }

    fn withdraw_candidate(&mut self, index: usize) {
        let source = self.sources[index].as_mut().unwrap();
        let Some(lease) = source.offered else {
            return;
        };
        let Some(hub) = self.hubs.iter().flatten().find(|h| h.id == lease.session) else {
            return;
        };
        let row = &hub.session.rows[usize::from(lease.slot - 1)];
        if !row.withdrawn.swap(true, Ordering::AcqRel) {
            if let Some(next) = source.bridge.generation.load(Ordering::Relaxed).checked_add(1) {
                source.bridge.generation.store(next, Ordering::Release);
            }
        }
        if source.returned.is_none() {
            source.returned = source.pending.take().or_else(|| source.bridge.offers.take());
        }
    }

    fn rematch(&mut self) {
        for index in 0..REGISTRY_TUNER_LEASES {
            let Some(source) = self.sources[index].as_ref().filter(|s| !s.retired) else {
                continue;
            };
            let mut candidate = None;
            let mut count = 0;
            for (i, hub) in self
                .hubs
                .iter()
                .enumerate()
                .filter_map(|(i, h)| h.as_ref().filter(|h| !h.retired).map(|h| (i, h)))
            {
                if source.selected.is_none_or(|uuid| uuid == hub.uuid) {
                    candidate = Some(i);
                    count += 1;
                }
            }
            if count != 1 {
                source
                    .bridge
                    .status
                    .store(if count == 0 { MISSING } else { AMBIGUOUS }, Ordering::Release);
                self.withdraw_candidate(index);
                self.collect();
                continue;
            }
            let hub_index = candidate.unwrap();
            if self.hubs[hub_index].as_ref().unwrap().session.closing.load(Ordering::Acquire) != 0 {
                continue;
            }
            if let Some(lease) = source.offered {
                if lease.session != self.hubs[hub_index].as_ref().unwrap().id {
                    self.withdraw_candidate(index);
                    self.collect();
                }
                if self.sources[index].as_ref().unwrap().offered.is_some() {
                    continue;
                }
            }
            let Some(slot) =
                self.hubs[hub_index].as_ref().unwrap().sources.iter().position(Option::is_some)
            else {
                self.sources[index]
                    .as_ref()
                    .unwrap()
                    .bridge
                    .status
                    .store(OVERCAPACITY, Ordering::Release);
                continue;
            };
            let Some(incarnation) = self.id() else {
                continue;
            };
            let hub = self.hubs[hub_index].as_mut().unwrap();
            let source = self.sources[index].as_mut().unwrap();
            let lease = Lease {
                session: hub.id,
                source: harmonigraph_core::SourceId(incarnation),
                incarnation,
                slot: (slot + 1) as u8,
            };
            let row = &hub.session.rows[slot];
            row.expected_incarnation.store(incarnation, Ordering::Release);
            row.withdrawn.store(false, Ordering::Release);
            row.faults.store(0, Ordering::Release);
            row.emission_gate.store(super::source::CLOSED, Ordering::Release);
            // An unadopted stale offer has never become a callback user.
            row.source_detached.store(true, Ordering::Release);
            row.hub_detached.store(true, Ordering::Release);
            hub.leases[slot] = Some(source.id);
            let offer = SourceOffer {
                generation: source.bridge.generation.load(Ordering::Acquire),
                lease,
                session: hub.session.clone(),
                endpoints: hub.sources[slot].take().unwrap(),
            };
            source.offered = Some(lease);
            source.pending = source.bridge.offers.publish(offer).err();
            source.bridge.status.store(OFFERED, Ordering::Release);
        }
        for source in self.sources.iter_mut().flatten() {
            if let Some(offer) = source.pending.take() {
                source.pending = source.bridge.offers.publish(offer).err();
            }
        }
    }
}

/// Move the actual joined owner into its EXISTING counted entry. Registry
/// bookkeeping never borrows a live plugin and never calls a host under lock.
#[cfg(not(feature = "tuning-probe"))]
pub fn retire_source(mut owner: Box<super::source::Source>) {
    let Some(id) = owner.shared.registration() else {
        return;
    };
    owner.stop();
    owner.join_producer();
    {
        let mut registry = global().lock().unwrap();
        let entry = registry.sources.iter_mut().flatten().find(|s| s.id == id).unwrap();
        entry.retired = true;
        assert!(entry.owner.is_none());
        entry.owner = Some(owner);
        if entry.returned.is_none() {
            entry.returned = entry.pending.take().or_else(|| entry.bridge.offers.take());
        }
        if let Some(lease) = entry.offered {
            if let Some(hub) = registry.hubs.iter().flatten().find(|h| h.id == lease.session) {
                hub.bridge.retired_pending.store(true, Ordering::Release);
                hub.session.rows[usize::from(lease.slot - 1)]
                    .withdrawn
                    .store(true, Ordering::Release);
            }
        }
    }
    service_retired();
}
pub fn retire_hub(owner: Box<super::hub::Hub>) {
    let Some(id) = owner.shared.registration() else {
        return;
    };
    {
        let mut registry = global().lock().unwrap();
        let entry = registry.hubs.iter_mut().flatten().find(|h| h.id == id).unwrap();
        entry.retired = true;
        entry.session.alive.store(false, Ordering::Release);
        entry.session.closing.store(u64::MAX, Ordering::Release);
        for row in &entry.session.rows {
            row.withdrawn.store(true, Ordering::Release);
            for state in [super::source::OPEN, super::source::CLOSED] {
                let _ = row.emission_gate.compare_exchange(
                    state,
                    super::source::FENCED,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                );
            }
        }
        if owner.offer.is_none() {
            assert!(entry.returned.is_none());
            entry.returned = entry.bridge.offers.take();
        }
        assert!(entry.owner.is_none());
        entry.owner = Some(owner);
        registry.rematch();
    }
    service_retired();
}

pub fn service_retired() {
    // All-retired peers have no live callback to request. Complete locally
    // ready handshakes while they actually change retained ownership; stop at
    // an absent/unknown proof. The selected manifest/pending pools bound work.
    // Conservative serialized phase bound with8192 input envelopes AND32768
    // child references: ceil((8192+32768+1024+1024)/64)=672 disposition
    // rounds;128 traversal/cleanup rounds;400 source-output collection;132
    // aggregate merge (including observed DIRECT);8 journal retirement;2 reply
    // tail;80 fixed handshakes. The traversal allowance rounds up
    // ceil(8192/256)+ceil((2*(8192+32768)+65*672)/(2048-2*65))=98,
    // including blocked-parent restarts and indivisible cleanup tails.
    // Every outer round services all four sessions independently.2048 rounds
    // exceed the1438-round sum, including16 serialized joined-producer control
    // consumptions. Mailbox retries wait only for the reports/output phases
    // already counted above; joined publication/consumption bump revision and
    // add no acknowledgement lane. Missing external proof still exits immediately.
    for _ in 0..2048 {
        if !service_retired_once() {
            break;
        }
    }
}
fn service_retired_once() -> bool {
    let mut progressed = false;
    // A servicing marker keeps the slot counted while its owned Box is outside
    // the mutex. Concurrent lifecycle calls cannot reclaim or reuse that entry.
    let mut more = false;
    for index in 0..REGISTRY_TUNER_LEASES {
        let item = {
            let mut registry = global().lock().unwrap();
            registry.sources[index].as_mut().and_then(|entry| {
                let owner = entry.owner.take()?;
                entry.servicing = true;
                Some((entry.id, owner))
            })
        };
        let Some((id, mut owner)) = item else {
            continue;
        };
        let before = owner.service_position();
        let mut ready = owner.retired_pump();
        if owner.offer.is_none() {
            // A quiesced unpaired owner has no musical peer to wake it. This
            // is one bounded local service batch, not a complete drain bound:
            // child references and deferred cleanup can require later outer
            // rounds, whose continuation follows actual ownership progress.
            for _ in 0..(PENDING_EVENTS / 256 + 1) {
                if !ready {
                    break;
                }
                ready = owner.retired_pump();
            }
        }
        more |= ready;
        let finished = owner.settled() && owner.offer.is_none();
        progressed |= finished || owner.service_position() != before;
        let mut registry = global().lock().unwrap();
        let entry = registry.sources[index].as_mut().unwrap();
        assert_eq!(entry.id, id);
        entry.servicing = false;
        if !finished {
            entry.owner = Some(owner);
        } else {
            drop(registry);
            drop(owner);
        }
        // Otherwise destruction is here, on this off-audio service thread.
    }
    for index in 0..REGISTRY_SESSIONS {
        let item = {
            let mut registry = global().lock().unwrap();
            registry.hubs[index].as_mut().and_then(|entry| {
                let owner = entry.owner.take()?;
                entry.servicing = true;
                Some((entry.id, owner))
            })
        };
        let Some((id, mut owner)) = item else {
            continue;
        };
        let before = (owner.service_position(), owner.direct.service_position());
        owner.retired_pump();
        let finished = owner.retired_settled();
        progressed |=
            finished || (owner.service_position(), owner.direct.service_position()) != before;
        if finished {
            if let Some(offer) = owner.offer.take() {
                // Main owns reclamation and may collect acknowledgements first.
                let mut registry = global().lock().unwrap();
                registry.collect();
                let entry = registry.hubs[index].as_mut().unwrap();
                entry.returned = Some(offer);
            }
        }
        let mut registry = global().lock().unwrap();
        let entry = registry.hubs[index].as_mut().unwrap();
        assert_eq!(entry.id, id);
        entry.servicing = false;
        if !finished {
            entry.owner = Some(owner);
        } else {
            drop(registry);
            drop(owner);
        }
    }
    let wakes = {
        let mut registry = global().lock().unwrap();
        registry.service();
        for hub in registry.hubs.iter().flatten() {
            let pending = registry
                .sources
                .iter()
                .flatten()
                .any(|s| s.retired && s.offered.is_some_and(|lease| lease.session == hub.id));
            hub.bridge.retired_pending.store(pending, Ordering::Release);
        }
        registry
            .hubs
            .iter()
            .flatten()
            .filter(|h| !h.retired && h.bridge.retired_pending.load(Ordering::Acquire))
            .filter_map(|h| h.bridge.wake.get().cloned())
            .collect::<Vec<_>>()
    };
    if more {
        for wake in wakes {
            wake();
        }
    }
    progressed
}
