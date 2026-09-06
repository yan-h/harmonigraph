//! Exclusive owned handoff, including non-Copy endpoint bundles. Taking an
//! attachment on audio requires reserving its return slot first. Failed writes
//! return the entire value; no error arm can implicitly destroy an endpoint.
use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

const EMPTY: u8 = 0;
const WRITING: u8 = 1;
const READY: u8 = 2;
const READING: u8 = 3;

pub struct Slots<T, const N: usize = 2> {
    cells: [Slot<T>; N],
}
struct Slot<T> {
    state: AtomicU8,
    value: UnsafeCell<Option<T>>,
}
// An acquired state excludes every other access to the ordinary payload.
// Release/Acquire transfers ownership, including destruction responsibility.
unsafe impl<T: Send> Sync for Slot<T> {}

impl<T, const N: usize> Default for Slots<T, N> {
    fn default() -> Self {
        Self {
            cells: std::array::from_fn(|_| Slot {
                state: AtomicU8::new(EMPTY),
                value: UnsafeCell::new(None),
            }),
        }
    }
}

pub struct Reservation<'a, T> {
    slot: &'a Slot<T>,
    published: bool,
}

impl<T, const N: usize> Slots<T, N> {
    /// Retain the actual shared payload without copying a large chunk into a
    /// second owner. The guard keeps Reading until bounded application ends.
    pub fn read_owned(self: &Arc<Self>) -> Option<ReadGuard<T, N>> {
        for (index, slot) in self.cells.iter().enumerate() {
            if slot
                .state
                .compare_exchange(READY, READING, Ordering::Acquire, Ordering::Relaxed)
                .is_ok()
            {
                return Some(ReadGuard {
                    slots: self.clone(),
                    index,
                    value: std::marker::PhantomData,
                });
            }
        }
        None
    }
    pub(super) fn reserve_at(&self, index: usize) -> Option<Reservation<'_, T>> {
        let slot = &self.cells[index];
        slot.state
            .compare_exchange(EMPTY, WRITING, Ordering::Acquire, Ordering::Relaxed)
            .ok()
            .map(|_| Reservation { slot, published: false })
    }
    pub(super) fn take_at(&self, index: usize) -> Option<T> {
        let slot = &self.cells[index];
        slot.state.compare_exchange(READY, READING, Ordering::Acquire, Ordering::Relaxed).ok()?;
        // SAFETY: Reading exclusively owns the complete published payload.
        let value = unsafe { (*slot.value.get()).take() }.expect("published slot");
        slot.state.store(EMPTY, Ordering::Release);
        Some(value)
    }
    /// Off-thread preparation may outlive a borrow of its setup object. The
    /// owned reservation pins these ACTUAL slots until commit/abandonment.
    pub fn reserve_owned(self: &Arc<Self>) -> Option<OwnedReservation<T, N>> {
        for (index, slot) in self.cells.iter().enumerate() {
            if slot
                .state
                .compare_exchange(EMPTY, WRITING, Ordering::Acquire, Ordering::Relaxed)
                .is_ok()
            {
                return Some(OwnedReservation { slots: self.clone(), index, published: false });
            }
        }
        None
    }
    pub fn reserve(&self) -> Option<Reservation<'_, T>> {
        for slot in &self.cells {
            if slot
                .state
                .compare_exchange(EMPTY, WRITING, Ordering::Acquire, Ordering::Relaxed)
                .is_ok()
            {
                return Some(Reservation { slot, published: false });
            }
        }
        None
    }

    pub fn publish(&self, value: T) -> Result<(), T> {
        match self.reserve() {
            Some(reservation) => {
                reservation.publish(value);
                Ok(())
            }
            None => Err(value),
        }
    }

    pub fn take(&self) -> Option<T> {
        for slot in &self.cells {
            if slot
                .state
                .compare_exchange(READY, READING, Ordering::Acquire, Ordering::Relaxed)
                .is_ok()
            {
                // SAFETY: Reading owns the payload, and the published slot
                // always contains one whole value. Empty follows its move out.
                let value = unsafe { (*slot.value.get()).take() }.expect("published slot");
                slot.state.store(EMPTY, Ordering::Release);
                return Some(value);
            }
        }
        None
    }
}

impl<T: Copy, const N: usize> Slots<T, N> {
    pub(super) fn take_at_if(&self, index: usize, accept: impl FnOnce(T) -> bool) -> Option<T> {
        let slot = &self.cells[index];
        slot.state.compare_exchange(READY, READING, Ordering::Acquire, Ordering::Relaxed).ok()?;
        // SAFETY: Reading excludes any concurrent access to this payload.
        let value = unsafe { (*slot.value.get()).unwrap() };
        if !accept(value) {
            slot.state.store(READY, Ordering::Release);
            return None;
        }
        unsafe {
            *slot.value.get() = None;
        }
        slot.state.store(EMPTY, Ordering::Release);
        Some(value)
    }
    /// Keep the real slot in Reading until the audio owner has applied the
    /// prepared value. A copied pending value must not free preparation capacity.
    pub fn retain(self: &Arc<Self>) -> Option<Retained<T, N>> {
        for (index, slot) in self.cells.iter().enumerate() {
            if slot
                .state
                .compare_exchange(READY, READING, Ordering::Acquire, Ordering::Relaxed)
                .is_ok()
            {
                // SAFETY: Reading excludes both a writer and another reader.
                return Some(Retained {
                    slots: self.clone(),
                    index,
                    value: unsafe { (*slot.value.get()).unwrap() },
                });
            }
        }
        None
    }
}

pub struct Retained<T: Copy, const N: usize = 2> {
    slots: Arc<Slots<T, N>>,
    index: usize,
    pub value: T,
}
impl<T: Copy, const N: usize> Drop for Retained<T, N> {
    fn drop(&mut self) {
        let slot = &self.slots.cells[self.index];
        assert_eq!(slot.state.load(Ordering::Relaxed), READING);
        // SAFETY: this unique guard owns Reading and pins the actual slot.
        unsafe {
            *slot.value.get() = None;
        }
        slot.state.store(EMPTY, Ordering::Release);
    }
}

pub struct OwnedReservation<T, const N: usize = 2> {
    slots: Arc<Slots<T, N>>,
    index: usize,
    published: bool,
}
impl<T, const N: usize> OwnedReservation<T, N> {
    pub fn initialize(&mut self, value: T) {
        // SAFETY: Writing excludes every reader and other writer.
        let cell = unsafe { &mut *self.slots.cells[self.index].value.get() };
        assert!(cell.is_none());
        *cell = Some(value);
    }
    pub fn value_mut(&mut self) -> &mut T {
        // SAFETY: this unique guard owns Writing for the whole borrow.
        unsafe { (*self.slots.cells[self.index].value.get()).as_mut().expect("initialized writer") }
    }
    pub fn publish_initialized(mut self) {
        assert!(unsafe { (*self.slots.cells[self.index].value.get()).is_some() });
        self.published = true;
        self.slots.cells[self.index].state.store(READY, Ordering::Release);
    }
    pub fn publish(mut self, value: T) {
        let slot = &self.slots.cells[self.index];
        // SAFETY: this owned reservation acquired Writing and pins the slot.
        unsafe {
            assert!((*slot.value.get()).is_none(), "use publish_initialized for a retained writer");
            *slot.value.get() = Some(value);
        }
        self.published = true;
        slot.state.store(READY, Ordering::Release);
    }
}
impl<T, const N: usize> Drop for OwnedReservation<T, N> {
    fn drop(&mut self) {
        if !self.published {
            // SAFETY: an abandoned partial writer still exclusively owns it.
            unsafe {
                *self.slots.cells[self.index].value.get() = None;
            }
            self.slots.cells[self.index].state.store(EMPTY, Ordering::Release);
        }
    }
}

pub struct ReadGuard<T, const N: usize = 2> {
    slots: Arc<Slots<T, N>>,
    index: usize,
    // value() exposes &T, so sharing this guard requires T: Sync even though
    // the exclusive slot itself may safely transport a merely Send payload.
    value: std::marker::PhantomData<T>,
}
impl<T, const N: usize> ReadGuard<T, N> {
    pub fn value(&self) -> &T {
        // SAFETY: Reading excludes writes until this pinned guard is dropped.
        unsafe { (*self.slots.cells[self.index].value.get()).as_ref().expect("published reader") }
    }
}
impl<T, const N: usize> Drop for ReadGuard<T, N> {
    fn drop(&mut self) {
        let slot = &self.slots.cells[self.index];
        // SAFETY: this guard exclusively owns Reading, including destruction.
        unsafe {
            *slot.value.get() = None;
        }
        slot.state.store(EMPTY, Ordering::Release);
    }
}

impl<T> Reservation<'_, T> {
    pub fn publish(mut self, value: T) {
        // SAFETY: this reservation exclusively owns Writing and the previous
        // reader moved the old value before releasing Empty. Never replace/drop.
        unsafe {
            *self.slot.value.get() = Some(value);
        }
        self.published = true;
        self.slot.state.store(READY, Ordering::Release);
    }
}
impl<T> Drop for Reservation<'_, T> {
    fn drop(&mut self) {
        if !self.published {
            self.slot.state.store(EMPTY, Ordering::Release);
        }
    }
}
