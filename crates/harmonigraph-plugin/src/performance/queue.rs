//! Callback-owned retained storage. Construction is off audio, operations move
//! only Copy values, and a full queue returns the caller's still-owned value.
pub struct Queue<T: Copy, const N: usize> {
    cells: Box<[Option<T>]>,
    head: usize,
    len: usize,
}
impl<T: Copy, const N: usize> Default for Queue<T, N> {
    fn default() -> Self {
        Self { cells: vec![None; N].into_boxed_slice(), head: 0, len: 0 }
    }
}
impl<T: Copy, const N: usize> Queue<T, N> {
    pub fn position(&self, offset: usize) -> Option<usize> {
        (offset < self.len).then_some((self.head + offset) % N)
    }
    pub fn at(&self, position: usize) -> Option<T> {
        self.cells.get(position).copied().flatten()
    }
    pub fn get(&self, offset: usize) -> Option<T> {
        self.at(self.position(offset)?)
    }
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn free(&self) -> usize {
        N - self.len
    }
    pub fn clear(&mut self) {
        self.head = 0;
        self.len = 0;
    }
    pub fn front(&self) -> Option<T> {
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
        let value = self.front()?;
        self.cells[self.head] = None;
        self.head = (self.head + 1) % N;
        self.len -= 1;
        Some(value)
    }
}

/// Intrusive FIFO with independently reclaimable cells. An accepted established
/// release frees its event slot even while an older unsounded attack is blocked.
/// Stable cell indices are paired with caller serials across staged completion.
pub struct Indexed<T: Copy, const N: usize> {
    cells: Box<[Linked<T>]>,
    free: Vec<u16>,
    head: Option<u16>,
    tail: Option<u16>,
    len: usize,
}
#[derive(Clone, Copy)]
struct Linked<T> {
    value: Option<T>,
    previous: Option<u16>,
    next: Option<u16>,
}
impl<T: Copy, const N: usize> Default for Indexed<T, N> {
    fn default() -> Self {
        assert!(N < usize::from(u16::MAX));
        Self {
            cells: vec![Linked { value: None, previous: None, next: None }; N].into_boxed_slice(),
            free: (0..N as u16).rev().collect(),
            head: None,
            tail: None,
            len: 0,
        }
    }
}
impl<T: Copy, const N: usize> Indexed<T, N> {
    pub const BACKING_CELL_BYTES: usize = std::mem::size_of::<Linked<T>>();
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn free(&self) -> usize {
        self.free.len()
    }
    pub fn front_position(&self) -> Option<usize> {
        self.head.map(usize::from)
    }
    pub fn back_position(&self) -> Option<usize> {
        self.tail.map(usize::from)
    }
    pub fn next_position(&self, index: usize) -> Option<usize> {
        self.cells[index].next.map(usize::from)
    }
    pub fn at(&self, index: usize) -> Option<T> {
        self.cells.get(index)?.value
    }
    pub fn set(&mut self, index: usize, value: T) {
        assert!(self.cells[index].value.is_some());
        self.cells[index].value = Some(value);
    }
    pub fn push(&mut self, value: T) -> Result<(), T> {
        let Some(index) = self.free.pop() else {
            return Err(value);
        };
        self.cells[usize::from(index)] =
            Linked { value: Some(value), previous: self.tail, next: None };
        if let Some(tail) = self.tail {
            self.cells[usize::from(tail)].next = Some(index);
        } else {
            self.head = Some(index);
        }
        self.tail = Some(index);
        self.len += 1;
        Ok(())
    }
    pub fn remove(&mut self, index: usize) -> Option<T> {
        let cell = self.cells[index];
        let value = cell.value?;
        if let Some(previous) = cell.previous {
            self.cells[usize::from(previous)].next = cell.next;
        } else {
            self.head = cell.next;
        }
        if let Some(next) = cell.next {
            self.cells[usize::from(next)].previous = cell.previous;
        } else {
            self.tail = cell.previous;
        }
        self.cells[index].value = None;
        self.free.push(index as u16);
        self.len -= 1;
        Some(value)
    }
}

#[cfg(test)]
impl<T: Copy, const N: usize> Queue<T, N> {
    pub fn test_layout(&self) -> [usize; 3] {
        [std::mem::size_of::<Option<T>>(), self.cells.len(), std::mem::size_of_val(&*self.cells)]
    }
}
#[cfg(test)]
impl<T: Copy, const N: usize> Indexed<T, N> {
    pub fn test_layout(&self) -> [usize; 5] {
        [
            std::mem::size_of::<Option<T>>(),
            std::mem::size_of::<Linked<T>>(),
            self.cells.len(),
            std::mem::size_of_val(&*self.cells),
            self.free.capacity(),
        ]
    }
}
