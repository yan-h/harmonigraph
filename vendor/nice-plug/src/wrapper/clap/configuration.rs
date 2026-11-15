//! The configuration walk's input: every owned host input of a callback, in
//! input order, with its exact sample. The plugin owns its musical
//! configuration itself; this module only retains the input it walks.

pub const INPUT_SCAN: usize = 2048;

#[derive(Clone, Copy, Debug)]
pub struct ConfigurationBoundary {
    pub steady_time: i64,
    pub frames: u32,
    pub sample_rate: f32,
    pub transport_seconds: Option<f64>,
    pub playing: bool,
}

/// The owned input pool has no free cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Full;

/// Exact owned CLAP input values, shared with the later performance adapter.
/// Signed wildcard addresses, event flags and f64 tuning survive capture.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum InputValue {
    Parameter {
        id: u32,
        value: f64,
        modulation: bool,
    },
    Note {
        kind: u16,
        note_id: i32,
        port: i16,
        channel: i16,
        key: i16,
        velocity: f64,
        flags: u32,
    },
    Expression {
        expression: i32,
        note_id: i32,
        port: i16,
        channel: i16,
        key: i16,
        value: f64,
        flags: u32,
    },
    Midi {
        port: u16,
        data: [u8; 3],
        flags: u32,
    },
    /// Full raw transport payload, including flags and fixed-point timelines.
    Transport(super::performance::Transport),
    /// Unsupported header: never a claim that its payload was retained.
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OwnedInput {
    /// None only for untimed params.flush. Bound once at the next process boundary.
    pub sample: Option<i64>,
    pub event_index: u32,
    /// Original enclosing offset; never rewritten when retained or bound.
    pub offset: u32,
    pub enclosing_start: Option<i64>,
    pub enclosing_frames: u32,
    pub flush: bool,
    pub batch: u64,
    pub value: InputValue,
}

/// The single budgeted persistent input allocation. It is not rebuilt on Rust
/// sub-blocks and never retains a borrowed host pointer across a callback.
pub(crate) struct InputStorage {
    cells: Box<[Option<OwnedInput>; INPUT_SCAN]>,
    head: usize,
    len: usize,
}
impl Default for InputStorage {
    fn default() -> Self {
        Self { cells: vec![None; INPUT_SCAN].into_boxed_slice().try_into().unwrap(), head: 0, len: 0 }
    }
}
impl InputStorage {
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn available(&self) -> usize {
        INPUT_SCAN - self.len
    }
    pub fn push(&mut self, input: OwnedInput) -> Result<(), Full> {
        if self.len == INPUT_SCAN {
            return Err(Full);
        }
        self.cells[(self.head + self.len) % INPUT_SCAN] = Some(input);
        self.len += 1;
        Ok(())
    }
    pub fn get(&self, index: usize) -> Option<OwnedInput> {
        if index >= self.len {
            return None;
        }
        self.cells[(self.head + index) % INPUT_SCAN]
    }
    pub fn pop(&mut self) -> Option<OwnedInput> {
        if self.len == 0 {
            return None;
        }
        let value = self.cells[self.head].take();
        self.head = (self.head + 1) % INPUT_SCAN;
        self.len -= 1;
        value
    }
    pub fn truncate(&mut self, len: usize) {
        while self.len > len {
            self.len -= 1;
            self.cells[(self.head + self.len) % INPUT_SCAN] = None;
        }
    }
    pub fn reset_timed(&mut self) {
        let count = self.len;
        for _ in 0..count {
            let input = self.pop().unwrap();
            if input.sample.is_none() {
                self.push(input).expect("retained untimed flush");
            }
        }
    }

    pub fn bind_untimed(&mut self, boundary: i64) {
        for i in 0..self.len {
            let input = self.cells[(self.head + i) % INPUT_SCAN].as_mut().unwrap();
            if input.sample.is_none() {
                input.sample = Some(boundary);
            }
        }
    }
}

const _: () = assert!(std::mem::size_of::<Option<OwnedInput>>() <= 192);
const _: () = assert!(std::mem::align_of::<OwnedInput>() <= 8);
