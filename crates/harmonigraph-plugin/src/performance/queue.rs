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

#[cfg(test)]
impl<T, const N: usize> Queue<T, N> {
    pub fn test_layout(&self) -> [usize; 3] {
        [std::mem::size_of::<Option<T>>(), self.cells.len(), std::mem::size_of_val(&*self.cells)]
    }
}
