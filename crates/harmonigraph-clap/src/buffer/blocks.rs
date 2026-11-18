//! Per-block per-channel per-sample iterators.

use std::marker::PhantomData;

use super::SamplesIter;

/// An iterator over all samples in the buffer, slicing over the sample-dimension with a maximum
/// size of `max_block_size`. See [`Buffer::iter_blocks()`][super::Buffer::iter_blocks()]. Yields
/// both the block and the offset from the start of the buffer.
pub struct BlocksIter<'slice, 'sample: 'slice> {
    /// The raw output buffers.
    pub(super) buffers: *mut [&'sample mut [f32]],
    pub(super) max_block_size: usize,
    pub(super) current_block_start: usize,
    pub(super) _marker: PhantomData<&'slice mut [&'sample mut [f32]]>,
}

/// A block yielded by [`BlocksIter`]. Can be iterated over once or multiple times, and also
/// supports direct access to the block's samples if needed.
pub struct Block<'slice, 'sample: 'slice> {
    /// The raw output buffers.
    pub(self) buffers: *mut [&'sample mut [f32]],
    pub(self) current_block_start: usize,
    /// The index of the last sample in the block plus one.
    pub(self) current_block_end: usize,
    pub(self) _marker: PhantomData<&'slice mut [&'sample mut [f32]]>,
}

/// An iterator over all channels in a block yielded by [`Block`], returning an entire channel slice
/// at a time.
pub struct BlockChannelsIter<'slice, 'sample: 'slice> {
    /// The raw output buffers.
    pub(self) buffers: *mut [&'sample mut [f32]],
    pub(self) current_block_start: usize,
    pub(self) current_block_end: usize,
    pub(self) current_channel: usize,
    pub(self) _marker: PhantomData<&'slice mut [&'sample mut [f32]]>,
}

impl<'slice, 'sample> Iterator for BlocksIter<'slice, 'sample> {
    type Item = (usize, Block<'slice, 'sample>);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let buffer_len = unsafe { (*self.buffers).first().map(|b| b.len()).unwrap_or(0) };
        if self.current_block_start < buffer_len {
            let current_block_start = self.current_block_start;
            let current_block_end =
                (self.current_block_start + self.max_block_size).min(buffer_len);
            let block = Block {
                buffers: self.buffers,
                current_block_start,
                current_block_end,
                _marker: self._marker,
            };

            self.current_block_start += self.max_block_size;

            Some((current_block_start, block))
        } else {
            None
        }
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let buffer_len = unsafe { (*self.buffers).first().map(|b| b.len()).unwrap_or(0) };
        let remaining = (buffer_len as f32 / self.max_block_size as f32).ceil() as usize;

        (remaining, Some(remaining))
    }
}

impl<'slice, 'sample> IntoIterator for Block<'slice, 'sample> {
    type Item = &'sample mut [f32];
    type IntoIter = BlockChannelsIter<'slice, 'sample>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        BlockChannelsIter {
            buffers: self.buffers,
            current_block_start: self.current_block_start,
            current_block_end: self.current_block_end,
            current_channel: 0,
            _marker: self._marker,
        }
    }
}

impl<'slice, 'sample> Iterator for BlockChannelsIter<'slice, 'sample> {
    type Item = &'sample mut [f32];

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.current_channel < unsafe { (&(*self.buffers)).len() } {
            // SAFETY: These bounds have already been checked
            // SAFETY: It is also not possible to have multiple mutable references to the same
            //         sample at the same time
            let slice = unsafe {
                (&mut (*self.buffers))
                    .get_unchecked_mut(self.current_channel)
                    .get_unchecked_mut(self.current_block_start..self.current_block_end)
            };

            self.current_channel += 1;

            Some(slice)
        } else {
            None
        }
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = unsafe { (&(*self.buffers)).len() } - self.current_channel;

        (remaining, Some(remaining))
    }
}

impl ExactSizeIterator for BlocksIter<'_, '_> {}
impl ExactSizeIterator for BlockChannelsIter<'_, '_> {}

impl<'slice, 'sample> Block<'slice, 'sample> {
    /// Get the number of samples per channel in the block.
    #[inline]
    pub fn samples(&self) -> usize {
        self.current_block_end - self.current_block_start
    }

    /// Returns the number of channels in this buffer.
    #[inline]
    pub fn channels(&self) -> usize {
        unsafe { (&(*self.buffers)).len() }
    }

    /// A resetting iterator. This lets you iterate over the same block multiple times. Otherwise
    /// you don't need to use this function as [`Block`] already implements [`Iterator`]. You can
    /// also use the direct accessor functions on this block instead.
    #[inline]
    pub fn iter_mut(&mut self) -> BlockChannelsIter<'slice, 'sample> {
        BlockChannelsIter {
            buffers: self.buffers,
            current_block_start: self.current_block_start,
            current_block_end: self.current_block_end,
            current_channel: 0,
            _marker: self._marker,
        }
    }

    /// Iterate over this block on a per-sample per-channel basis. This is identical to
    /// [`Buffer::iter_samples()`][super::Buffer::iter_samples()] but for a smaller block instead of
    /// the entire buffer
    #[inline]
    pub fn iter_samples(&mut self) -> SamplesIter<'slice, 'sample> {
        SamplesIter {
            buffers: self.buffers,
            current_sample: self.current_block_start,
            samples_end: self.current_block_end,
            _marker: self._marker,
        }
    }

    /// Access a channel by index. Useful when you would otherwise iterate over this [`Block`]
    /// multiple times.
    #[inline]
    pub fn get(&self, channel_index: usize) -> Option<&[f32]> {
        // SAFETY: The block bound has already been checked
        unsafe {
            Some(
                (&(*self.buffers))
                    .get(channel_index)?
                    .get_unchecked(self.current_block_start..self.current_block_end),
            )
        }
    }

    /// The same as [`get()`][Self::get], but without any bounds checking.
    ///
    /// # Safety
    ///
    /// `channel_index` must be in the range `0..Self::len()`.
    #[inline]
    pub unsafe fn get_unchecked(&self, channel_index: usize) -> &[f32] {
        unsafe {
            (&(*self.buffers))
                .get_unchecked(channel_index)
                .get_unchecked(self.current_block_start..self.current_block_end)
        }
    }

    /// Access a mutable channel by index. Useful when you would otherwise iterate over this
    /// [`Block`] multiple times.
    #[inline]
    pub fn get_mut(&mut self, channel_index: usize) -> Option<&mut [f32]> {
        // SAFETY: The block bound has already been checked
        unsafe {
            Some(
                (&mut (*self.buffers))
                    .get_mut(channel_index)?
                    .get_unchecked_mut(self.current_block_start..self.current_block_end),
            )
        }
    }

    /// The same as [`get_mut()`][Self::get_mut], but without any bounds checking.
    ///
    /// # Safety
    ///
    /// `channel_index` must be in the range `0..Self::len()`.
    #[inline]
    pub unsafe fn get_unchecked_mut(&mut self, channel_index: usize) -> &mut [f32] {
        unsafe {
            (&mut (*self.buffers))
                .get_unchecked_mut(channel_index)
                .get_unchecked_mut(self.current_block_start..self.current_block_end)
        }
    }
}
