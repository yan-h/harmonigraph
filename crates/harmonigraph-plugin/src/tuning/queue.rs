//! Callback-owned retained storage. Construction is off audio, operations move
//! owned values, and a full queue returns the caller's still-owned value. The
//! backing is allocated once at `N` cells and never resized, so capacity is
//! `N` and a queue is never anything but its own fixed size.
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
    const fn capacity(&self) -> usize {
        N
    }
    pub fn position(&self, offset: usize) -> Option<usize> {
        (offset < self.len).then(|| (self.head + offset) % self.capacity())
    }
    pub fn at(&self, position: usize) -> Option<T>
    where
        T: Copy,
    {
        self.cells.get(position).copied().flatten()
    }
    /// Replace a retained value in place. A reply reaching its onset changes
    /// what that cell will emit without changing where it sits in the line.
    pub fn set(&mut self, position: usize, value: T) {
        self.cells[position] = Some(value);
    }
    pub fn len(&self) -> usize {
        self.len
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
        if self.len == self.capacity() {
            return Err(value);
        }
        self.cells[(self.head + self.len) % self.capacity()] = Some(value);
        self.len += 1;
        Ok(())
    }
    pub fn pop(&mut self) -> Option<T> {
        if self.len == 0 {
            return None;
        }
        let value = self.cells[self.head].take().unwrap();
        self.head = (self.head + 1) % self.capacity();
        self.len -= 1;
        Some(value)
    }
}
