// SPDX-License-Identifier: GPL-2.0-only
// Allocation strategy adapted from os-findmy-e macOS-ADI-emulator/harness.py.
use std::{collections::BTreeMap, fmt};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum MemoryError {
    InvalidRange,
    InvalidAlignment,
    Overflow,
    Exhausted,
    InvalidRelease,
}

impl MemoryError {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidRange => "invalid heap range",
            Self::InvalidAlignment => "invalid allocation alignment",
            Self::Overflow => "allocation overflow",
            Self::Exhausted => "emulated heap exhausted",
            Self::InvalidRelease => "invalid heap release",
        }
    }
}

impl fmt::Display for MemoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::error::Error for MemoryError {}

#[derive(Debug)]
pub struct Heap {
    free: BTreeMap<u64, u64>,
    used: BTreeMap<u64, u64>,
}
impl Heap {
    /// Creates an allocator over a nonzero, nonempty guest address range.
    ///
    /// # Errors
    /// Returns an error if the start is zero, the size is zero, or the range overflows `u64`.
    pub fn new(start: u64, size: u64) -> Result<Self, MemoryError> {
        if start == 0 || size == 0 || start.checked_add(size).is_none() {
            return Err(MemoryError::InvalidRange);
        }
        Ok(Self {
            free: BTreeMap::from([(start, size)]),
            used: BTreeMap::new(),
        })
    }
    /// Allocates at least 32 bytes with the requested power-of-two alignment.
    ///
    /// # Errors
    /// Returns an error for invalid alignment, size overflow, or insufficient free space.
    pub fn allocate(&mut self, size: u64, alignment: u64) -> Result<u64, MemoryError> {
        if !alignment.is_power_of_two() {
            return Err(MemoryError::InvalidAlignment);
        }
        let size = size
            .max(32)
            .checked_add(alignment - 1)
            .ok_or(MemoryError::Overflow)?
            & !(alignment - 1);
        let chosen = self
            .free
            .iter()
            .find_map(|(&start, &len)| {
                let address = start.checked_add(alignment - 1)? & !(alignment - 1);
                let end = address.checked_add(size)?;
                (end <= start.checked_add(len)?).then_some((start, len, address, end))
            })
            .ok_or(MemoryError::Exhausted)?;
        let (start, len, address, end) = chosen;
        self.free.remove(&start);
        if address > start {
            self.free.insert(start, address - start);
        }
        if end < start + len {
            self.free.insert(end, start + len - end);
        }
        self.used.insert(address, size);
        Ok(address)
    }
    /// Releases an allocation and coalesces adjacent free ranges; zero is a no-op.
    ///
    /// # Errors
    /// Returns an error if a nonzero address is not the start of a live allocation.
    pub fn release(&mut self, address: u64) -> Result<(), MemoryError> {
        if address == 0 {
            return Ok(());
        }
        let mut len = self
            .used
            .remove(&address)
            .ok_or(MemoryError::InvalidRelease)?;
        let mut start = address;
        if let Some((&a, &n)) = self.free.range(..address).next_back()
            && a + n == address
        {
            self.free.remove(&a);
            start = a;
            len += n;
        }
        if let Some(&n) = self.free.get(&(start + len)) {
            self.free.remove(&(start + len));
            len += n;
        }
        self.free.insert(start, len);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::Heap;
    #[test]
    fn rejects_null_empty_and_overflowing_ranges() {
        for (start, size) in [(0, 32), (1, 0), (u64::MAX, 1)] {
            assert!(
                Heap::new(start, size).is_err(),
                "range {start:#x} + {size:#x}"
            );
        }
        let mut heap = Heap::new(1, 64).unwrap();
        let address = heap.allocate(32, 1).unwrap();
        assert_ne!(address, 0);
        heap.release(0).unwrap();
        heap.release(address).unwrap();
        assert_eq!(
            heap.allocate(64, 1).unwrap(),
            1,
            "released memory is reusable"
        );
    }
    #[test]
    fn releases_coalesce_and_alignment_does_not_lose_space() {
        let mut h = Heap::new(0x1000, 0x2000).unwrap();
        let a = h.allocate(64, 16).unwrap();
        let b = h.allocate(1, 4096).unwrap();
        assert_eq!(b, 0x2000);
        assert!(h.allocate(u64::MAX, 16).is_err());
        h.release(a).unwrap();
        h.release(b).unwrap();
        assert_eq!(h.allocate(0x2000, 4096).unwrap(), 0x1000);
        assert!(h.allocate(1, 16).is_err());
        assert!(h.release(0x1001).is_err());
        h.release(0x1000).unwrap();
        assert!(h.release(0x1000).is_err());
    }
}
