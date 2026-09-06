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

pub struct Slots<T> {
    cells: [Slot<T>; 2],
}
struct Slot<T> {
    state: AtomicU8,
    value: UnsafeCell<Option<T>>,
}
// An acquired state excludes every other access to the ordinary payload.
// Release/Acquire transfers ownership, including destruction responsibility.
unsafe impl<T: Send> Sync for Slot<T> {}

impl<T> Default for Slots<T> {
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

impl<T> Slots<T> {
    /// Off-thread preparation may outlive a borrow of its setup object. The
    /// owned reservation pins these ACTUAL slots until commit/abandonment.
    pub fn reserve_owned(self: &Arc<Self>) -> Option<OwnedReservation<T>> {
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

impl<T: Copy> Slots<T> {
    /// Keep the real slot in Reading until the audio owner has applied the
    /// prepared value. A copied pending value must not free preparation capacity.
    pub fn retain(self: &Arc<Self>) -> Option<Retained<T>> {
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

pub struct Retained<T: Copy> {
    slots: Arc<Slots<T>>,
    index: usize,
    pub value: T,
}
impl<T: Copy> Drop for Retained<T> {
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

pub struct OwnedReservation<T> {
    slots: Arc<Slots<T>>,
    index: usize,
    published: bool,
}
impl<T> OwnedReservation<T> {
    pub fn publish(mut self, value: T) {
        let slot = &self.slots.cells[self.index];
        // SAFETY: this owned reservation acquired Writing and pins the slot.
        unsafe {
            *slot.value.get() = Some(value);
        }
        self.published = true;
        slot.state.store(READY, Ordering::Release);
    }
}
impl<T> Drop for OwnedReservation<T> {
    fn drop(&mut self) {
        if !self.published {
            self.slots.cells[self.index].state.store(EMPTY, Ordering::Release);
        }
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
