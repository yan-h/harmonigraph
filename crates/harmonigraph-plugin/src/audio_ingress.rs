//! One SPSC publication boundary for live analysis. Payload is committed before
//! its descriptor; the consumer never reads undescribed samples. Both queues
//! are preallocated. Exhaustion retains a whole-frame prefix (or nothing), and
//! the next descriptor's source position exposes the gap. No loss recovery.
//!
//! Source frames count from an epoch's first callback, including dropped frames.
//! An explicit plugin reset or format/input change starts a new epoch. `origin`
//! is that epoch's first frame in the producer's continuous presentation seconds,
//! not seekable project transport time. Loops/stops/seeks do not rewind it.
//! LiveInput converts it to GUI seconds with the current shared ClockMapper
//! offset once per drain; no separate analyzer clock estimate is maintained.

/// Enough descriptors for 4096 callbacks, even when samples would fit more.
/// One-frame callbacks can exhaust this queue first; that is an ordinary gap.
const DESCRIPTORS: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Format {
    pub channels: usize,
    pub sample_rate: f32,
    pub sidechain: bool,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Block {
    pub epoch: u64,
    pub origin: f64,
    pub first_frame: u64,
    pub frames: usize,
    pub format: Format,
}

pub(crate) struct Producer {
    samples: rtrb::Producer<f32>,
    blocks: rtrb::Producer<Block>,
    epoch: u64,
    origin: f64,
    frame: u64,
    format: Option<Format>,
}

pub(crate) struct Consumer {
    samples: rtrb::Consumer<f32>,
    blocks: rtrb::Consumer<Block>,
    /// Reused, bounded scratch also joins ring wraps that split a channel frame.
    scratch: Vec<f32>,
}

pub(crate) fn channel(samples: usize) -> (Producer, Consumer) {
    let (sample_tx, sample_rx) = rtrb::RingBuffer::new(samples);
    let (block_tx, block_rx) = rtrb::RingBuffer::new(DESCRIPTORS);
    (
        Producer {
            samples: sample_tx,
            blocks: block_tx,
            epoch: 0,
            origin: 0.0,
            frame: 0,
            format: None,
        },
        Consumer { samples: sample_rx, blocks: block_rx, scratch: Vec::with_capacity(samples) },
    )
}

impl Producer {
    pub fn reset(&mut self) {
        self.epoch = self.epoch.wrapping_add(1);
        self.frame = 0;
        self.format = None;
    }

    /// `samples` yields the callback's interleaved complete frames. Only the
    /// retained prefix is visited; no callback allocation or per-sample atomics.
    pub fn publish(
        &mut self,
        frames: usize,
        format: Format,
        presentation: f64,
        samples: impl Iterator<Item = f32>,
    ) {
        if self.format != Some(format) {
            self.reset();
            self.format = Some(format);
            self.origin = presentation;
        }
        let first_frame = self.frame;
        self.frame += frames as u64;
        if frames == 0 || format.channels == 0 || self.blocks.slots() == 0 {
            return;
        }
        let retained = frames.min(self.samples.slots() / format.channels);
        if retained == 0 {
            return;
        }
        let count = retained * format.channels;
        // Only this producer can take slots, so both reservations remain valid.
        self.samples.write_chunk_uninit(count).unwrap().fill_from_iter(samples.take(count));
        self.blocks
            .push(Block {
                epoch: self.epoch,
                origin: self.origin,
                first_frame,
                frames: retained,
                format,
            })
            .unwrap();
    }
}

impl Consumer {
    pub fn drain(&mut self, mut feed: impl FnMut(Block, &[f32])) {
        // Bound a tick to its initial descriptor snapshot even if callbacks
        // continue publishing while analysis is catching up.
        let available = self.blocks.slots();
        for _ in 0..available {
            let block = self.blocks.pop().unwrap();
            let chunk = self.samples.read_chunk(block.frames * block.format.channels).unwrap();
            let (a, b) = chunk.as_slices();
            self.scratch.clear();
            self.scratch.extend_from_slice(a);
            self.scratch.extend_from_slice(b);
            chunk.commit_all();
            feed(block, &self.scratch);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MONO: Format = Format { channels: 1, sample_rate: 48_000.0, sidechain: false };

    #[test]
    fn bounded_publication_keeps_format_frames_and_gaps_coherent() {
        let (mut tx, mut rx) = channel(7);
        let stereo = Format { channels: 2, sample_rate: 96_000.0, sidechain: true };
        tx.publish(2, MONO, 10.0, [1.0, 2.0].into_iter());
        // Only two of four stereo frames fit. The spare sample is not a frame.
        tx.publish(4, stereo, 11.0, (10..18).map(|v| v as f32));
        tx.publish(3, stereo, 12.0, std::iter::repeat(99.0));
        let mut blocks = Vec::new();
        rx.drain(|b, s| blocks.push((b, s.to_vec())));
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].1, [1.0, 2.0]);
        assert_eq!(blocks[0].0.format, MONO);
        assert_eq!(blocks[1].1, [10.0, 11.0, 12.0, 13.0]);
        assert_eq!(blocks[1].0.format, stereo);
        assert_eq!(blocks[1].0.frames, 2);
        assert_ne!(blocks[0].0.epoch, blocks[1].0.epoch);

        // Wrap the sample ring through the middle of a stereo frame. Both the
        // partial tail and entirely dropped callback still advanced source time.
        tx.publish(3, stereo, 13.0, (20..26).map(|v| v as f32));
        rx.drain(|b, s| {
            assert_eq!(b.first_frame, 7);
            assert_eq!(b.origin, 11.0);
            assert_eq!(s, [20.0, 21.0, 22.0, 23.0, 24.0, 25.0]);
        });
        tx.reset();
        tx.publish(1, stereo, 14.0, [30.0, 31.0].into_iter());
        rx.drain(|b, _| {
            assert_ne!(b.epoch, blocks[1].0.epoch);
            assert_eq!(b.first_frame, 0);
            assert_eq!(b.origin, 14.0);
        });

        // Descriptor exhaustion first must not leave undescribed payload.
        let (mut tx, mut rx) = channel(DESCRIPTORS + 10);
        for i in 0..DESCRIPTORS + 2 {
            tx.publish(1, MONO, i as f64 / 48_000.0, std::iter::once(i as f32));
        }
        let mut count = 0;
        rx.drain(|b, s| {
            assert_eq!(b.first_frame, count);
            assert_eq!(s, [count as f32]);
            count += 1;
        });
        assert_eq!(count, DESCRIPTORS as u64);
        tx.publish(1, MONO, 1.0, std::iter::once(42.0));
        rx.drain(|b, s| {
            assert_eq!(b.first_frame, DESCRIPTORS as u64 + 2);
            assert_eq!(s, [42.0]);
        });
    }
}
