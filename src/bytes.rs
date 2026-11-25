//! Bounds-checked access: little-endian reads for the binary data parsers, and
//! element access that cannot panic in a release build.
#![deny(clippy::unwrap_used, clippy::indexing_slicing)]

/// The `N` bytes at `offset`, if they are all there.
pub(crate) fn array_at<const N: usize>(data: &[u8], offset: usize) -> Option<[u8; N]> {
    data.get(offset..)?.first_chunk().copied()
}

pub(crate) fn u16_at(data: &[u8], offset: usize) -> Option<u16> {
    array_at(data, offset).map(u16::from_le_bytes)
}

pub(crate) fn u32_at(data: &[u8], offset: usize) -> Option<u32> {
    array_at(data, offset).map(u32::from_le_bytes)
}

/// Element access for indices the code has already bounded.
///
/// A wrong index is a bug, so a debug build (and so the test suite and the
/// fuzzers) asserts on it; a release build reads a default or drops the write
/// instead of taking the caller's process down.
pub(crate) trait Bounded<T> {
    fn at(&self, i: usize) -> T
    where
        T: Copy + Default;
    fn put(&mut self, i: usize, value: T);
}

impl<T> Bounded<T> for [T] {
    #[inline]
    #[track_caller]
    fn at(&self, i: usize) -> T
    where
        T: Copy + Default,
    {
        debug_assert!(i < self.len(), "index {i} of {}", self.len());
        self.get(i).copied().unwrap_or_default()
    }

    #[inline]
    #[track_caller]
    fn put(&mut self, i: usize, value: T) {
        debug_assert!(i < self.len(), "index {i} of {}", self.len());
        if let Some(slot) = self.get_mut(i) {
            *slot = value;
        }
    }
}
