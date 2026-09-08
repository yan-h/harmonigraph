//! Immutable original captures share the existing Source backing slabs.
//!
//! The only Source handles are constructed together, once, before registration.
//! Their non-Clone types own all mutable local fields. A published region is
//! never borrowed mutably or rewritten until exact remote retirement has been
//! acquired AND its local work has settled. In particular, no reference to a
//! whole Pending/Life/Work slot crosses this boundary: UnsafeCell regions are
//! addressed individually, and the ordinary Source snapshots are copies.
//!
//! rtrb Release publication transfers the unique token after the complete group
//! is initialized; the Hub's Acquire pop precedes every immutable read. Hub
//! permission bits are private to its serialized owner. It ends all views,
//! clears permissions and releases arena handles before publishing retirement
//! with Release. Source's Acquire pop and exact once-only identity check precede
//! reuse. Neither output ACK nor temporary-view Drop performs that transition.
//!
//! Source/Hub registry bridges pin the allocation before activation. Retired
//! owners and offered endpoints keep those bridges counted through detach;
//! final destruction is off audio. A dropped token grants no reuse authority.
use std::cell::UnsafeCell;
use std::mem::MaybeUninit;
use std::sync::Arc;

use super::event::Event;
use super::protocol::*;
use super::source::{channel, work, Life, Pending, ReleaseIndex, NONE};

#[derive(Clone, Copy)]
pub(super) struct Original {
    pub serial: u64,
    pub event: Event,
    pub life: u16,
    pub input: i64,
    pub generation: u64,
    pub work_head: u16,
    pub work_count: u8,
}
#[derive(Clone, Copy)]
pub(super) struct PendingLocal {
    pub channel: channel::Cell,
    pub staged: bool,
    pub cleanup_queued: bool,
    pub cleanup_next: u16,
    pub disposition: bool,
    pub work_tail: u16,
    pub work_remaining: u8,
    pub work_linked: u8,
    pub selected: u16,
    pub inline_done: bool,
    pub previous: u16,
    pub next: u16,
    pub phase: u8,
}
const VACANT: u8 = 0;
const BUILDING: u8 = 1;
const READY: u8 = 2;
const OFFERED: u8 = 3;
const RETIRED: u8 = 4;
const LOCAL_DONE: u8 = 8;
const PHASE: u8 = 7;
struct PendingSlot {
    original: UnsafeCell<MaybeUninit<Original>>,
    local: UnsafeCell<MaybeUninit<PendingLocal>>,
}
#[derive(Clone, Copy)]
pub(super) struct Birth {
    pub serial: u64,
    pub on_serial: u64,
    pub id: i32,
    pub channel: u8,
    pub key: u8,
    pub input: i64,
    pub generation: u64,
    pub midi: bool,
    pub adaptive: bool,
}
#[derive(Clone, Copy)]
pub(super) struct LifeLocal {
    pub assignment: Assignment,
    pub refs: u32,
    pub active: bool,
    pub reserved: bool,
    pub sounded: bool,
    pub canceled: bool,
    pub shift: i64,
    pub terminal: Option<(u64, i64, bool)>,
    pub release: Option<ReleaseIndex>,
    pub note_off_owed: bool,
    pub sound_off_refs: u16,
    pub ready_head: u16,
    pub ready_tail: u16,
    pub cleanup_next: u16,
    pub flags: u8,
}
struct LifeSlot {
    birth: UnsafeCell<MaybeUninit<Birth>>,
    local: UnsafeCell<Option<LifeLocal>>,
}

pub(crate) struct CaptureArena {
    pending: Box<[PendingSlot]>,
    lives: Box<[LifeSlot]>,
    pub work: Box<[work::Packed]>,
}
// SAFETY: the private construction and field ownership protocol above excludes
// concurrent mutation of every region the Hub may read. No public arena API
// exposes raw slots or mutable Source regions to a remote caller.
unsafe impl Sync for CaptureArena {}

pub(super) fn storage(shared: &super::setup::Shared) -> (PendingStore, Lives, work::Work) {
    let arena = Arc::new(CaptureArena {
        pending: (0..PENDING_EVENTS)
            .map(|_| PendingSlot {
                original: UnsafeCell::new(MaybeUninit::uninit()),
                local: UnsafeCell::new(MaybeUninit::uninit()),
            })
            .collect(),
        lives: (0..LIFETIMES)
            .map(|_| LifeSlot {
                birth: UnsafeCell::new(MaybeUninit::uninit()),
                local: UnsafeCell::new(None),
            })
            .collect(),
        work: work::backing(),
    });
    let pin = shared
        .source
        .as_ref()
        .map(|bridge| &bridge.arena)
        .or_else(|| shared.hub.as_ref().map(|bridge| &bridge.arena))
        .unwrap();
    assert!(pin.set(arena.clone()).is_ok(), "one Source owner per bridge");
    let pending = PendingStore {
        arena: arena.clone(),
        free: (0..PENDING_EVENTS as u16).rev().collect(),
        head: NONE,
        tail: NONE,
        len: 0,
    };
    // Before any callback exists, initialize only the Source-owned validity.
    for slot in &arena.pending {
        unsafe {
            (*slot.local.get()).write(PendingLocal {
                channel: channel::Cell::default(),
                staged: false,
                cleanup_queued: false,
                cleanup_next: NONE,
                disposition: false,
                work_tail: NONE,
                work_remaining: 0,
                work_linked: 0,
                selected: NONE,
                inline_done: false,
                previous: NONE,
                next: NONE,
                phase: VACANT,
            });
        }
    }
    (pending, Lives { arena: arena.clone() }, work::Work::new(arena))
}

pub(super) struct PendingStore {
    arena: Arc<CaptureArena>,
    free: Vec<u16>,
    head: u16,
    tail: u16,
    len: usize,
}
impl PendingStore {
    pub fn arena_identity(&self) -> usize {
        Arc::as_ptr(&self.arena) as usize
    }
    pub const BACKING_CELL_BYTES: usize = std::mem::size_of::<PendingSlot>();
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn free(&self) -> usize {
        self.free.len()
    }
    pub fn front_position(&self) -> Option<usize> {
        (self.head != NONE).then_some(self.head as usize)
    }
    pub fn back_position(&self) -> Option<usize> {
        (self.tail != NONE).then_some(self.tail as usize)
    }
    fn local(&self, index: usize) -> &PendingLocal {
        // SAFETY: this non-Clone handle exclusively owns this field region.
        unsafe { (*self.arena.pending[index].local.get()).assume_init_ref() }
    }
    fn local_mut(&mut self, index: usize) -> &mut PendingLocal {
        // SAFETY: no Hub API reads this Source-only region.
        unsafe { (*self.arena.pending[index].local.get()).assume_init_mut() }
    }
    pub fn next_position(&self, index: usize) -> Option<usize> {
        let next = self.local(index).next;
        (next != NONE).then_some(next as usize)
    }
    pub fn at(&self, index: usize) -> Option<Pending> {
        if index >= PENDING_EVENTS || self.local(index).phase == VACANT {
            return None;
        }
        let local = *self.local(index);
        // SAFETY: Source initialized the original before setting occupancy;
        // Hub can only read it while this Source also leaves it immutable.
        let original = unsafe { (*self.arena.pending[index].original.get()).assume_init_ref() };
        Some(Pending {
            channel: local.channel,
            serial: original.serial,
            event: original.event,
            life: original.life,
            input: original.input,
            generation: original.generation,
            staged: local.staged,
            cleanup_queued: local.cleanup_queued,
            cleanup_next: local.cleanup_next,
            disposition: local.disposition,
            work_head: original.work_head,
            work_tail: local.work_tail,
            work_count: original.work_count,
            work_remaining: local.work_remaining,
            work_linked: local.work_linked,
            selected: local.selected,
            inline_done: local.inline_done,
        })
    }
    pub fn set(&mut self, index: usize, value: Pending) {
        assert_ne!(self.local(index).phase, VACANT);
        if self.local(index).phase == BUILDING {
            // SAFETY: builder has not issued a token; only this Source can read.
            unsafe {
                (*self.arena.pending[index].original.get()).write(Original {
                    serial: value.serial,
                    event: value.event,
                    life: value.life,
                    input: value.input,
                    generation: value.generation,
                    work_head: value.work_head,
                    work_count: value.work_count,
                });
            }
        } else {
            let original = unsafe { (*self.arena.pending[index].original.get()).assume_init_ref() };
            assert_eq!(
                (original.serial, original.life, original.work_head, original.work_count),
                (value.serial, value.life, value.work_head, value.work_count)
            );
        }
        let local = self.local_mut(index);
        local.channel = value.channel;
        local.staged = value.staged;
        local.cleanup_queued = value.cleanup_queued;
        local.cleanup_next = value.cleanup_next;
        local.disposition = value.disposition;
        local.work_tail = value.work_tail;
        local.work_remaining = value.work_remaining;
        local.work_linked = value.work_linked;
        local.selected = value.selected;
        local.inline_done = value.inline_done;
    }
    pub fn push(&mut self, value: Pending) -> Result<(), Pending> {
        let Some(index) = self.free.pop() else {
            return Err(value);
        };
        let previous = self.tail;
        let local = self.local_mut(index as usize);
        local.phase = BUILDING;
        local.previous = previous;
        local.next = NONE;
        self.set(index as usize, value);
        if previous == NONE {
            self.head = index;
        } else {
            self.local_mut(previous as usize).next = index;
        }
        self.tail = index;
        self.len += 1;
        Ok(())
    }
    pub fn seal(&mut self, index: usize) {
        assert_eq!(self.local(index).phase, BUILDING);
        self.local_mut(index).phase = READY;
    }
    pub fn local_done(&self, index: usize) -> bool {
        self.local(index).phase & LOCAL_DONE != 0
    }
    pub fn finish_local(&mut self, index: usize) {
        self.local_mut(index).phase |= LOCAL_DONE;
    }
    pub fn remote_pending(&self, index: usize) -> bool {
        (self.local(index).phase & PHASE) == OFFERED
    }
    pub fn unpublished(&self, index: usize) -> bool {
        (self.local(index).phase & PHASE) == READY
    }
    pub fn remove(&mut self, index: usize) -> Option<Pending> {
        let value = self.at(index)?;
        assert_ne!(self.local(index).phase & PHASE, OFFERED, "exact remote retirement required");
        let local = *self.local(index);
        if local.previous == NONE {
            self.head = local.next;
        } else {
            self.local_mut(local.previous as usize).next = local.next;
        }
        if local.next == NONE {
            self.tail = local.previous;
        } else {
            self.local_mut(local.next as usize).previous = local.previous;
        }
        self.local_mut(index).phase = VACANT;
        self.free.push(index as u16);
        self.len -= 1;
        Some(value)
    }
    #[cfg(test)]
    pub fn test_layout(&self) -> [usize; 5] {
        [
            std::mem::size_of::<Pending>(),
            Self::BACKING_CELL_BYTES,
            PENDING_EVENTS,
            std::mem::size_of_val(&*self.arena.pending),
            self.free.capacity(),
        ]
    }
}

pub(super) struct Lives {
    arena: Arc<CaptureArena>,
}
impl Lives {
    pub fn at(&self, index: u16) -> Option<Life> {
        let slot = self.arena.lives.get(index as usize)?;
        // SAFETY: Source owns the local validity and holds every Birth pin.
        let local = unsafe { (*slot.local.get())? };
        let birth = unsafe { (*slot.birth.get()).assume_init_ref() };
        Some(Life {
            serial: birth.serial,
            on_serial: birth.on_serial,
            id: birth.id,
            channel: birth.channel,
            key: birth.key,
            input: birth.input,
            refs: local.refs,
            active: local.active,
            reserved: local.reserved,
            sounded: local.sounded,
            canceled: local.canceled,
            shift: (local.flags & super::source::SHIFT_VALID != 0).then_some(local.shift),
            terminal: local.terminal,
            release: local.release,
            generation: birth.generation,
            midi: birth.midi,
            adaptive: birth.adaptive,
            assignment: local.assignment,
            assignment_held: local.flags & super::source::ASSIGNMENT_HELD != 0,
            note_off_owed: local.note_off_owed,
            sound_off_refs: local.sound_off_refs,
            ready_head: local.ready_head,
            ready_tail: local.ready_tail,
            cleanup_next: local.cleanup_next,
            ready_queued: local.flags & super::source::READY_QUEUED != 0,
        })
    }
    pub fn local_mut(&mut self, index: u16) -> Option<&mut LifeLocal> {
        // SAFETY: unique Source handle; this region contains no shared Birth.
        unsafe { (*self.arena.lives[index as usize].local.get()).as_mut() }
    }
    pub fn insert(&mut self, index: u16, value: Life) {
        let slot = &self.arena.lives[index as usize];
        // SAFETY: free-list ownership requires all local and capture pins gone.
        unsafe {
            assert!((*slot.local.get()).is_none());
            (*slot.birth.get()).write(Birth {
                serial: value.serial,
                on_serial: value.on_serial,
                id: value.id,
                channel: value.channel,
                key: value.key,
                input: value.input,
                generation: value.generation,
                midi: value.midi,
                adaptive: value.adaptive,
            });
            *slot.local.get() = Some(LifeLocal {
                assignment: value.assignment,
                refs: value.refs,
                active: value.active,
                reserved: value.reserved,
                sounded: value.sounded,
                canceled: value.canceled,
                shift: value.shift.unwrap_or_default(),
                terminal: value.terminal,
                release: value.release,
                note_off_owed: value.note_off_owed,
                sound_off_refs: value.sound_off_refs,
                ready_head: value.ready_head,
                ready_tail: value.ready_tail,
                cleanup_next: value.cleanup_next,
                flags: (u8::from(value.ready_queued) * super::source::READY_QUEUED)
                    | (u8::from(value.assignment_held) * super::source::ASSIGNMENT_HELD)
                    | (u8::from(value.shift.is_some()) * super::source::SHIFT_VALID),
            });
        }
    }
    pub fn remove(&mut self, index: u16) {
        // SAFETY: the caller verified both local obligations and capture pins.
        let slot = &self.arena.lives[index as usize];
        unsafe {
            let local = (*slot.local.get()).as_ref().unwrap();
            assert_eq!(local.refs, 0);
            assert!(
                !local.active
                    && !local.reserved
                    && local.flags & (super::source::READY_QUEUED | super::source::ASSIGNMENT_HELD)
                        == 0
            );
            *slot.local.get() = None;
        }
    }
    #[cfg(test)]
    pub fn test_layout(&self) -> [usize; 3] {
        [std::mem::size_of::<LifeSlot>(), LIFETIMES, std::mem::size_of_val(&*self.arena.lives)]
    }
}
const _: () = assert!(std::mem::size_of::<PendingSlot>() <= 128);
const _: () = assert!(
    std::mem::size_of::<LifeSlot>()
        - std::mem::size_of::<harmonigraph_core::configuration::ResolvedConfig>()
        + 128
        <= 256
);

use harmonigraph_core::cohort::{self, FrozenInputId, TargetAccess};

/// Exact identity of one publication. An arena address is only compared while
/// registry/token pins keep that allocation alive; it is never dereferenced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Key {
    pub lease: Lease,
    pub epoch: u64,
    pub serial: u64,
    pub arena: usize,
    pub position: u16,
}

/// Unique authority moves Source -> ring -> retained Hub window -> retirement.
/// Deliberately neither Copy nor Clone. Dropping this never acknowledges reuse.
pub(crate) struct Token {
    arena: Arc<CaptureArena>,
    pub key: Key,
    pub sample: i64,
    pub frozen: Option<FrozenInputId>,
    pub status: Option<CaptureStatus>,
}
impl std::fmt::Debug for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CaptureToken").field("key", &self.key).finish()
    }
}
/// No shared reads remain in this phase. A full reply queue retains this unique
/// authority in the same ingress cell; retrying cannot recreate an input token.
#[derive(Debug)]
pub(crate) struct Retirement {
    pub key: Key,
}
impl PendingStore {
    pub fn offer(
        &mut self,
        position: usize,
        lease: Lease,
        epoch: u64,
        offset: i64,
    ) -> Option<Token> {
        assert_eq!(self.local(position).phase & PHASE, READY);
        let original = self.at(position).unwrap();
        let sample = original.input.checked_add(offset)?;
        self.local_mut(position).phase = (self.local(position).phase & LOCAL_DONE) | OFFERED;
        Some(Token {
            arena: self.arena.clone(),
            key: Key {
                lease,
                epoch,
                serial: original.serial,
                arena: Arc::as_ptr(&self.arena) as usize,
                position: position as u16,
            },
            sample,
            frozen: None,
            status: None,
        })
    }
    pub fn retire(&mut self, key: Key, lease: Lease, epoch: u64) -> Option<usize> {
        let position = usize::from(key.position);
        if key.lease != lease
            || key.epoch != epoch
            || key.arena != Arc::as_ptr(&self.arena) as usize
            || position >= PENDING_EVENTS
            || self.local(position).phase & PHASE != OFFERED
            || self.at(position)?.serial != key.serial
        {
            return None;
        }
        self.local_mut(position).phase = (self.local(position).phase & LOCAL_DONE) | RETIRED;
        Some(position)
    }
    pub fn owns_publication(&self, key: Key, lease: Lease, epoch: u64) -> bool {
        let position = usize::from(key.position);
        key.lease == lease
            && key.epoch == epoch
            && key.arena == Arc::as_ptr(&self.arena) as usize
            && position < PENDING_EVENTS
            && self.local(position).phase & PHASE == OFFERED
            && self.at(position).is_some_and(|original| original.serial == key.serial)
    }
}
impl Token {
    pub fn all_work(&self) -> u64 {
        u64::MAX.checked_shr(64 - u32::from(self.original().work_count)).unwrap_or(0)
    }
    pub fn completed(&self, received: u64) -> bool {
        self.status.is_some_and(|status| {
            status.inline_done
                && status.work_done == self.all_work()
                && received >= status.output_cut
        })
    }
    pub fn retain_status(&mut self, status: CaptureStatus) {
        if self.completed(u64::MAX) {
            return;
        }
        if self.status.is_none_or(|old| {
            old.work_done & !status.work_done == 0 && (!old.inline_done || status.inline_done)
        }) {
            self.status = Some(status);
        }
    }
    pub fn units(&self) -> usize {
        1 + usize::from(self.original().work_count)
    }
    pub fn retire_unread(self) -> Retirement {
        let key = self.key;
        drop(self);
        Retirement { key }
    }
    fn original(&self) -> &Original {
        // SAFETY: the unique token can only be minted for a sealed complete
        // group; Source cannot rewrite/recycle it before this token retires.
        unsafe {
            (*self.arena.pending[self.key.position as usize].original.get()).assume_init_ref()
        }
    }
    fn valid(&self, lease: Lease, epoch: u64) -> bool {
        self.key.lease == lease
            && self.key.epoch == epoch
            && self.key.arena == Arc::as_ptr(&self.arena) as usize
            && self.key.serial != 0
            && self.original().serial == self.key.serial
    }
    fn birth(&self, life: u16) -> Option<&Birth> {
        let slot = self.arena.lives.get(life as usize)?;
        // SAFETY: only reached through this capture's inline or Work pin.
        Some(unsafe { (*slot.birth.get()).assume_init_ref() })
    }
}

/// Hub-only permission planes. They authorize immutable reads, never ownership
/// or Source reuse. Exact token and frozen binding validation is also required.
pub(super) struct Permissions {
    work: Box<[u64]>,
    parents: Box<[u64]>,
}
impl Default for Permissions {
    fn default() -> Self {
        Self {
            work: vec![0; 32768 / 64].into_boxed_slice(),
            parents: vec![0; PENDING_EVENTS / 64].into_boxed_slice(),
        }
    }
}
impl Permissions {
    fn contains(bits: &[u64], index: usize) -> bool {
        bits.get(index / 64).is_some_and(|word| word & (1 << (index % 64)) != 0)
    }
    fn change(bits: &mut [u64], index: usize, value: bool) {
        let mask = 1 << (index % 64);
        if value {
            bits[index / 64] |= mask;
        } else {
            bits[index / 64] &= !mask;
        }
    }
    pub fn accept(&mut self, token: &Token, lease: Lease, epoch: u64) {
        assert!(token.valid(lease, epoch));
        let original = token.original();
        assert!(!Self::contains(&self.parents, token.key.position as usize));
        let mut next = original.work_head;
        for _ in 0..original.work_count {
            assert!(usize::from(next) < 32768);
            // SAFETY: initial installation walks the builder-certified complete
            // group under the unique token, before exposing any TargetAccess.
            let (serial, life, parent, successor, _) = token.arena.work[next as usize].original();
            assert_eq!(parent, token.key.position);
            assert_eq!(token.birth(life).unwrap().serial, serial);
            assert!(!Self::contains(&self.work, next as usize));
            Self::change(&mut self.work, next as usize, true);
            next = successor;
        }
        assert_eq!(next, NONE);
        if original.life != NONE {
            assert_ne!(token.birth(original.life).unwrap().serial, 0);
        }
        Self::change(&mut self.parents, token.key.position as usize, true);
    }
    pub fn retire(&mut self, token: Token) -> Retirement {
        assert!(token.frozen.is_none(), "persistent frozen owner still holds this capture");
        let original = token.original();
        let mut next = original.work_head;
        for _ in 0..original.work_count {
            assert!(Self::contains(&self.work, next as usize));
            let (_, _, _, successor, _) = token.arena.work[next as usize].original();
            Self::change(&mut self.work, next as usize, false);
            next = successor;
        }
        Self::change(&mut self.parents, token.key.position as usize, false);
        let key = token.key;
        // All views borrow the retained window; its exclusive removal is proof
        // no such borrow survives here. End the arena Arc before the ack Release.
        drop(token);
        Retirement { key }
    }
    /// Read only the authenticated Original-On identity while its unique token
    /// and parent permission remain owned. Terminal cut disposal needs no graph
    /// traversal or copied mutable Source state.
    pub fn original_on(&self, token: &Token) -> Option<(u16, u64)> {
        if !token.valid(token.key.lease, token.key.epoch)
            || !Self::contains(&self.parents, token.key.position as usize)
        {
            return None;
        }
        let original = token.original();
        original.event.attack()?;
        Some((original.life, token.birth(original.life)?.serial))
    }
    pub fn empty(&self) -> bool {
        self.work.iter().chain(self.parents.iter()).all(|word| *word == 0)
    }
}

/// A checked view of one retained source row, scoped to a persistent freeze.
/// Handles encode the stable ingress slot as well as the Work/inline domain.
/// This is the O(1) input-serial witness, without another per-Pending index slab.
pub(super) struct View<'a> {
    pub ingress: &'a super::queue::Window<Intent, INTENT_RING>,
    pub permissions: &'a Permissions,
    pub lease: Lease,
    pub epoch: u64,
    pub frozen: FrozenInputId,
}
const INLINE: u16 = 32768;
fn handle(slot: usize, reference: u16) -> u32 {
    ((slot as u32) << 16) | u32::from(reference)
}
impl View<'_> {
    fn token(&self, slot: usize) -> Option<&Token> {
        let Intent::Capture(token) = self.ingress.at_ref(slot)? else {
            return None;
        };
        (self.frozen.0 != 0
            && token.frozen == Some(self.frozen)
            && token.valid(self.lease, self.epoch)
            && Permissions::contains(&self.permissions.parents, token.key.position as usize))
        .then_some(token)
    }
    pub fn metadata(&self, slot: usize) -> Option<cohort::Event> {
        let token = self.token(slot)?;
        let original = token.original();
        let inline = if original.life == NONE {
            cohort::TargetSpan::default()
        } else {
            cohort::TargetSpan { first: handle(slot, INLINE | token.key.position), len: 1 }
        };
        let work = if original.work_count == 0 {
            cohort::TargetSpan::default()
        } else {
            cohort::TargetSpan { first: handle(slot, original.work_head), len: original.work_count }
        };
        let kind = if original.event.attack().is_some() {
            cohort::Kind::Onset
        } else if original.event.release() {
            cohort::Kind::Terminal
        } else {
            match original.event {
                Event::Participation(value) => cohort::Kind::Participation(value),
                Event::Expression { kind: 2, value, .. } if value.is_finite() => {
                    cohort::Kind::Tuning { value_bits: value.to_bits() }
                }
                Event::Expression { .. } => cohort::Kind::Expression,
                event if event.channel_control().is_some() => cohort::Kind::Channel {
                    channel: event.channel_control().unwrap(),
                    terminal: event.channel_termination().is_some(),
                },
                Event::Midi { data, .. } if data[0] & 0xf0 == 0xa0 => cohort::Kind::Expression,
                _ => cohort::Kind::Independent,
            }
        };
        let onset = kind == cohort::Kind::Onset;
        Some(cohort::Event {
            id: cohort::InputId { source: self.lease.slot, sequence: token.key.serial },
            sample: token.sample,
            kind,
            targets: if inline.len != 0 { inline } else { work },
            replaced: if onset { work } else { cohort::TargetSpan::default() },
        })
    }
    pub fn original(&self, slot: usize) -> Option<(Key, Original)> {
        let token = self.token(slot)?;
        Some((token.key, *token.original()))
    }
    pub fn onset(&self, slot: usize) -> Option<(u16, Birth)> {
        let token = self.token(slot)?;
        let original = token.original();
        original.event.attack()?;
        Some((original.life, *token.birth(original.life)?))
    }
    /// The caller froze this copied status and applied its output cut before
    /// replay. TargetAccess itself always exposes the immutable complete chain.
    pub fn effect_done(&self, address: u32, ordinal: u16) -> Option<bool> {
        let token = self.token((address >> 16) as usize)?;
        let status = token.status?;
        Some(if (address as u16) < INLINE {
            ordinal < u16::from(token.original().work_count)
                && status.work_done & (1u64 << ordinal) != 0
        } else {
            status.inline_done
        })
    }
}
impl TargetAccess for View<'_> {
    fn get(&self, source: u8, address: u32) -> Option<cohort::TargetLink> {
        if source != self.lease.slot {
            return None;
        }
        let slot = (address >> 16) as usize;
        let reference = address as u16;
        // Validate arena/lease/incarnation/epoch/input serial/FrozenInputId and
        // parent permission before touching Work, Birth or the inline target.
        let token = self.token(slot)?;
        let original = token.original();
        let (birth, next) = if reference < INLINE {
            if !Permissions::contains(&self.permissions.work, reference as usize) {
                return None;
            }
            let (serial, life, parent, next, _) = token.arena.work[reference as usize].original();
            if parent != token.key.position {
                return None;
            }
            let birth = token.birth(life)?;
            if birth.serial != serial {
                return None;
            }
            (birth, if next == NONE { cohort::NO_TARGET } else { handle(slot, next) })
        } else {
            if reference != (INLINE | token.key.position) || original.life == NONE {
                return None;
            }
            (token.birth(original.life)?, cohort::NO_TARGET)
        };
        Some(cohort::TargetLink {
            target: cohort::Target {
                lifetime: birth.serial,
                host_note_id: birth.id,
                channel: birth.channel,
                key: birth.key,
            },
            next,
        })
    }
}

const _: () = assert!(super::queue::Window::<Intent, INTENT_RING>::CELL_BYTES <= 128);

#[cfg(test)]
pub(super) fn print_test_memory_layout() {
    use std::mem::size_of;
    println!("LEDGER capture [arena,arc_allocation,pending,life,work,phase_bytes,permissions_owner,permission_bytes,frozen_owner,scratch,event,events_backing] {:?}",
        [size_of::<CaptureArena>(), size_of::<CaptureArena>() + 2 * size_of::<usize>(),
        size_of::<PendingSlot>(), size_of::<LifeSlot>(), size_of::<work::Packed>(),
        work::CAPACITY / 4, size_of::<Permissions>(), (work::CAPACITY + PENDING_EVENTS) / 8,
        size_of::<Frozen>(), size_of::<cohort::Scratch>(), size_of::<cohort::Event>(),
        cohort::COHORT_EVENTS * size_of::<cohort::Event>()]);
}

pub(super) struct Targets<'a> {
    pub rows: [Option<View<'a>>; TUNERS + 1],
}
impl TargetAccess for Targets<'_> {
    fn get(&self, source: u8, handle: u32) -> Option<cohort::TargetLink> {
        self.rows.get(source as usize)?.as_ref()?.get(source, handle)
    }
}

/// Persistent metadata and traversal owner. The caller must establish complete
/// membership/input/configuration frontiers before using this to schedule. This
/// ownership stage supplies the pin/cursor boundary, not that scheduling proof.
pub(super) struct Frozen {
    scratch: Box<cohort::Scratch>,
    inputs: Box<[MaybeUninit<cohort::Event>]>,
    pub id: FrozenInputId,
    pub sample: i64,
    len: usize,
    started: bool,
    pub active: bool,
    complete: bool,
    pub spent: usize,
}
impl Default for Frozen {
    fn default() -> Self {
        Self {
            scratch: Box::default(),
            inputs: vec![MaybeUninit::uninit(); cohort::COHORT_EVENTS].into_boxed_slice(),
            id: FrozenInputId(0),
            sample: 0,
            len: 0,
            started: false,
            active: false,
            complete: false,
            spent: 0,
        }
    }
}
impl Frozen {
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn event(&self, index: usize) -> Option<cohort::Event> {
        // SAFETY: push initializes every cell in this prefix.
        (index < self.len).then(|| unsafe { self.inputs[index].assume_init() })
    }
    pub fn begin(&mut self, sample: i64, count: usize) -> Result<FrozenInputId, cohort::Error> {
        if self.active {
            return Err(cohort::Error::CohortInProgress);
        }
        if count > cohort::COHORT_EVENTS {
            return Err(cohort::Error::TooManyEvents);
        }
        self.id = FrozenInputId(self.id.0.checked_add(1).ok_or(cohort::Error::InvalidBinding)?);
        self.sample = sample;
        self.len = 0;
        self.started = false;
        self.complete = false;
        self.active = true;
        Ok(self.id)
    }
    pub fn push(&mut self, event: cohort::Event) {
        assert!(self.active && !self.started && event.sample == self.sample);
        self.inputs[self.len].write(event);
        self.len += 1;
    }
    pub fn advance(
        &mut self,
        targets: &impl TargetAccess,
        units: usize,
    ) -> Result<cohort::Progress, cohort::Error> {
        if !self.active {
            return Err(cohort::Error::InvalidBinding);
        }
        // SAFETY: only push increments len; it initializes each prefix cell.
        let inputs = unsafe {
            std::slice::from_raw_parts(self.inputs.as_ptr().cast::<cohort::Event>(), self.len)
        };
        let mut view = if self.started {
            cohort::Cohort::resume(self.id, self.sample, inputs, targets, &mut self.scratch)?
        } else {
            self.started = true;
            cohort::Cohort::begin(self.id, self.sample, inputs, targets, &mut self.scratch)?
        };
        let before = view.work();
        let progress = view.advance(units)?;
        self.spent = (view.work().units() - before.units()) as usize;
        self.complete = matches!(progress, cohort::Progress::Complete { .. });
        Ok(progress)
    }
    pub fn commit(&mut self, targets: &impl TargetAccess) -> Result<(), cohort::Error> {
        if !self.active || !self.started {
            return Err(cohort::Error::InvalidBinding);
        }
        let inputs = unsafe {
            std::slice::from_raw_parts(self.inputs.as_ptr().cast::<cohort::Event>(), self.len)
        };
        cohort::Cohort::resume(self.id, self.sample, inputs, targets, &mut self.scratch)?.commit()
    }
    /// Caller has settled old external decisions and preserved all bindings.
    /// Keep capture permissions and the reserved checked generation intact.
    pub fn abandon(&mut self) {
        self.scratch.discard();
        self.active = false;
        self.started = false;
        self.complete = false;
        self.spent = 0;
    }
    pub fn end(&mut self, joined: bool) {
        assert!(!self.active || self.complete || joined, "offered phase still owned");
        self.active = false;
    }
}
