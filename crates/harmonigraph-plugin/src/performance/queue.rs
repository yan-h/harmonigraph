//! Callback-owned retained storage. Construction is off audio, operations move
//! owned values, and a full queue returns the caller's still-owned value.
pub struct Queue<T, const N: usize> {
    cells: Box<[Option<T>]>,
    head: usize,
    len: usize,
}
impl<T, const N: usize> Default for Queue<T, N> {
    fn default() -> Self {
        Self { cells: (0..N).map(|_| None).collect(), head: 0, len: 0 }
    }
}
impl<T, const N: usize> Queue<T, N> {
    pub fn position(&self, offset: usize) -> Option<usize> {
        (offset < self.len).then_some((self.head + offset) % N)
    }
    pub fn at(&self, position: usize) -> Option<T>
    where
        T: Copy,
    {
        self.cells.get(position).copied().flatten()
    }
    pub fn at_mut(&mut self, position: usize) -> Option<&mut T> {
        self.cells.get_mut(position)?.as_mut()
    }
    pub fn get(&self, offset: usize) -> Option<T>
    where
        T: Copy,
    {
        self.at(self.position(offset)?)
    }
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn free(&self) -> usize {
        N - self.len
    }
    pub fn clear(&mut self)
    where
        T: Copy,
    {
        while self.pop().is_some() {}
    }
    pub fn front(&self) -> Option<T>
    where
        T: Copy,
    {
        (self.len != 0).then(|| self.cells[self.head].unwrap())
    }
    pub fn push(&mut self, value: T) -> Result<(), T> {
        if self.len == N {
            return Err(value);
        }
        self.cells[(self.head + self.len) % N] = Some(value);
        self.len += 1;
        Ok(())
    }
    pub fn pop(&mut self) -> Option<T> {
        if self.len == 0 {
            return None;
        }
        let value = self.cells[self.head].take().unwrap();
        self.head = (self.head + 1) % N;
        self.len -= 1;
        Some(value)
    }
}

#[cfg(all(test, not(feature = "tuning-probe")))]
impl<T, const N: usize> Queue<T, N> {
    pub fn test_layout(&self) -> [usize; 3] {
        [std::mem::size_of::<Option<T>>(), self.cells.len(), std::mem::size_of_val(&*self.cells)]
    }
}

/// Retained ownership window with stable handles and independent reclamation.
/// A frozen capture does not prevent parsing later coverage or dispositions.
/// The free chain reuses the vacant cell's next link, adding no index slab.
pub struct Window<T, const N: usize> {
    cells: Box<[WindowCell<T>]>,
    head: u16,
    tail: u16,
    free: u16,
    len: usize,
}
struct WindowCell<T> {
    value: Option<T>,
    previous: u16,
    next: u16,
}
const NONE: u16 = u16::MAX;
impl<T, const N: usize> Default for Window<T, N> {
    fn default() -> Self {
        assert!(N > 0 && N < NONE as usize);
        Self {
            cells: (0..N)
                .map(|index| WindowCell {
                    value: None,
                    previous: NONE,
                    next: if index + 1 == N { NONE } else { (index + 1) as u16 },
                })
                .collect(),
            head: NONE,
            tail: NONE,
            free: 0,
            len: 0,
        }
    }
}
impl<T, const N: usize> Window<T, N> {
    pub const CELL_BYTES: usize = std::mem::size_of::<WindowCell<T>>();
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn free(&self) -> usize {
        N - self.len
    }
    pub fn front_position(&self) -> Option<usize> {
        (self.head != NONE).then_some(self.head as usize)
    }
    pub fn next_position(&self, index: usize) -> Option<usize> {
        (self.cells[index].next != NONE).then_some(self.cells[index].next as usize)
    }
    pub fn at_ref(&self, index: usize) -> Option<&T> {
        self.cells.get(index)?.value.as_ref()
    }
    pub fn at_mut(&mut self, index: usize) -> Option<&mut T> {
        self.cells.get_mut(index)?.value.as_mut()
    }
    pub fn push(&mut self, value: T) -> Result<usize, T> {
        if self.free == NONE {
            return Err(value);
        }
        let index = self.free as usize;
        self.free = self.cells[index].next;
        self.cells[index] = WindowCell { value: Some(value), previous: self.tail, next: NONE };
        if self.tail == NONE {
            self.head = index as u16;
        } else {
            self.cells[self.tail as usize].next = index as u16;
        }
        self.tail = index as u16;
        self.len += 1;
        Ok(index)
    }
    pub fn map_at(&mut self, index: usize, update: impl FnOnce(T) -> T) {
        let value = self.cells[index].value.take().unwrap();
        self.cells[index].value = Some(update(value));
    }
    pub fn remove(&mut self, index: usize) -> Option<T> {
        let value = self.cells[index].value.take()?;
        let previous = self.cells[index].previous;
        let next = self.cells[index].next;
        if previous == NONE {
            self.head = next;
        } else {
            self.cells[previous as usize].next = next;
        }
        if next == NONE {
            self.tail = previous;
        } else {
            self.cells[next as usize].previous = previous;
        }
        self.cells[index].next = self.free;
        self.free = index as u16;
        self.len -= 1;
        Some(value)
    }
    #[cfg(all(test, not(feature = "tuning-probe")))]
    pub fn test_layout(&self) -> [usize; 3] {
        [Self::CELL_BYTES, N, std::mem::size_of_val(&*self.cells)]
    }
}
