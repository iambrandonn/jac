//! Bit packing utilities for presence bitmaps and type tags

use bitvec::prelude::*;

/// Presence bitmap for tracking absent vs present fields
#[derive(Debug, Clone)]
pub struct PresenceBitmap {
    bits: BitVec<u8, Lsb0>,
}

impl PresenceBitmap {
    /// Create a new presence bitmap for the given number of records
    pub fn new(record_count: usize) -> Self {
        Self {
            bits: BitVec::repeat(false, record_count),
        }
    }

    /// Set presence for a record
    pub fn set_present(&mut self, record_idx: usize, present: bool) {
        if record_idx < self.bits.len() {
            self.bits.set(record_idx, present);
        }
    }

    /// Check if a record is present
    pub fn is_present(&self, record_idx: usize) -> bool {
        self.bits.get(record_idx).map(|b| *b).unwrap_or(false)
    }

    /// Convert to bytes
    pub fn to_bytes(&self) -> Vec<u8> {
        self.bits.as_raw_slice().to_vec()
    }

    /// Create from bytes
    pub fn from_bytes(bytes: &[u8], count: usize) -> Self {
        let mut bits = BitVec::<u8, Lsb0>::from_slice(bytes);
        bits.truncate(count);
        Self { bits }
    }
}

/// 3-bit type tag packer
#[derive(Debug, Clone)]
pub struct TagPacker {
    bits: BitVec<u8, Lsb0>,
}

impl TagPacker {
    /// Create a new tag packer
    pub fn new() -> Self {
        Self {
            bits: BitVec::new(),
        }
    }

    /// Push a 3-bit tag
    pub fn push(&mut self, tag: u8) {
        assert!(tag < 8, "Tag must be < 8");
        self.bits.extend_from_bitslice(&tag.view_bits::<Lsb0>()[..3]);
    }

    /// Finish packing and return bytes
    pub fn finish(self) -> Vec<u8> {
        self.bits.into_vec()
    }
}

/// 3-bit type tag unpacker
#[derive(Debug, Clone)]
pub struct TagUnpacker {
    bits: BitVec<u8, Lsb0>,
    pos: usize,
}

impl TagUnpacker {
    /// Create a new tag unpacker
    pub fn new(bytes: &[u8], count: usize) -> Self {
        let mut bits = BitVec::<u8, Lsb0>::from_slice(bytes);
        bits.truncate(count * 3);
        Self { bits, pos: 0 }
    }
}

impl Iterator for TagUnpacker {
    type Item = u8;

    fn next(&mut self) -> Option<Self::Item> {
        if self.pos + 3 > self.bits.len() {
            return None;
        }

        let tag = self.bits[self.pos..self.pos + 3].load::<u8>();
        self.pos += 3;
        Some(tag)
    }
}

