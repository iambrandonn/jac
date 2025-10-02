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

    /// Count the number of present (true) bits
    pub fn count_present(&self) -> usize {
        self.bits.count_ones()
    }

    /// Create from boolean values
    pub fn from_bools(bools: &[bool]) -> Self {
        let mut bits = BitVec::<u8, Lsb0>::new();
        for &b in bools {
            bits.push(b);
        }
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
        self.bits
            .extend_from_bitslice(&tag.view_bits::<Lsb0>()[..3]);
    }

    /// Finish packing and return bytes
    pub fn finish(self) -> Vec<u8> {
        self.bits.into_vec()
    }
}

impl Default for TagPacker {
    fn default() -> Self {
        Self::new()
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

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn test_presence_bitmap_basic() {
        let mut bitmap = PresenceBitmap::new(10);

        // Initially all absent
        for i in 0..10 {
            assert!(!bitmap.is_present(i));
        }

        // Set some present
        bitmap.set_present(0, true);
        bitmap.set_present(5, true);
        bitmap.set_present(9, true);

        assert!(bitmap.is_present(0));
        assert!(!bitmap.is_present(1));
        assert!(!bitmap.is_present(4));
        assert!(bitmap.is_present(5));
        assert!(!bitmap.is_present(8));
        assert!(bitmap.is_present(9));
    }

    #[test]
    fn test_presence_bitmap_roundtrip() {
        let mut bitmap = PresenceBitmap::new(100);

        // Set every 3rd bit
        for i in (0..100).step_by(3) {
            bitmap.set_present(i, true);
        }

        let bytes = bitmap.to_bytes();
        let restored = PresenceBitmap::from_bytes(&bytes, 100);

        for i in 0..100 {
            assert_eq!(bitmap.is_present(i), restored.is_present(i));
        }
    }

    #[test]
    fn test_presence_bitmap_edge_cases() {
        // Test with 1 record
        let mut bitmap = PresenceBitmap::new(1);
        bitmap.set_present(0, true);
        let bytes = bitmap.to_bytes();
        assert_eq!(bytes.len(), 1);

        // Test with 7 records (fits in 1 byte)
        let mut bitmap = PresenceBitmap::new(7);
        for i in 0..7 {
            bitmap.set_present(i, true);
        }
        let bytes = bitmap.to_bytes();
        assert_eq!(bytes.len(), 1);

        // Test with 8 records (needs 1 byte)
        let mut bitmap = PresenceBitmap::new(8);
        for i in 0..8 {
            bitmap.set_present(i, true);
        }
        let bytes = bitmap.to_bytes();
        assert_eq!(bytes.len(), 1);

        // Test with 9 records (needs 2 bytes)
        let mut bitmap = PresenceBitmap::new(9);
        for i in 0..9 {
            bitmap.set_present(i, true);
        }
        let bytes = bitmap.to_bytes();
        assert_eq!(bytes.len(), 2);
    }

    #[test]
    fn test_tag_packer_basic() {
        let mut packer = TagPacker::new();
        packer.push(0); // null
        packer.push(1); // bool
        packer.push(2); // int
        packer.push(3); // decimal
        packer.push(4); // string
        packer.push(5); // object
        packer.push(6); // array

        let bytes = packer.finish();
        assert_eq!(bytes.len(), 3); // 7 * 3 = 21 bits = 3 bytes

        // The bit pattern should be: 000 001 010 011 100 101 110
        // In LSB-first order, this becomes: 110 101 100 011 010 001 000
        // Packed into bytes: 10001000 11000110 00011010
        assert_eq!(bytes[0], 0b10001000);
        assert_eq!(bytes[1], 0b11000110);
        assert_eq!(bytes[2], 0b00011010);
    }

    #[test]
    fn test_tag_packer_unpacker_roundtrip() {
        let tags = vec![0, 1, 2, 3, 4, 5, 6, 7];

        let mut packer = TagPacker::new();
        for &tag in &tags {
            packer.push(tag);
        }
        let bytes = packer.finish();

        let unpacker = TagUnpacker::new(&bytes, tags.len());
        let unpacked: Vec<u8> = unpacker.collect();

        assert_eq!(tags, unpacked);
    }

    #[test]
    fn test_tag_packer_padding() {
        // Test with 3 tags (9 bits, needs 2 bytes with padding)
        let mut packer = TagPacker::new();
        packer.push(4);
        packer.push(5);
        packer.push(6);

        let bytes = packer.finish();
        assert_eq!(bytes.len(), 2);

        // Last 5 bits should be zero (padding)
        assert_eq!(bytes[1] & 0b11111000, 0); // Check padding bits are zero
    }

    #[test]
    #[should_panic]
    fn test_tag_packer_invalid_tag() {
        let mut packer = TagPacker::new();
        packer.push(8); // Should panic
    }

    proptest! {
        #[test]
        fn prop_presence_roundtrip(bools in proptest::collection::vec(any::<bool>(), 0..512)) {
            let bitmap = PresenceBitmap::from_bools(&bools);
            let bytes = bitmap.to_bytes();
            let restored = PresenceBitmap::from_bytes(&bytes, bools.len());

            for (idx, expected) in bools.iter().enumerate() {
                prop_assert_eq!(restored.is_present(idx), *expected);
            }
        }

        #[test]
        fn prop_tag_roundtrip(tags in proptest::collection::vec(0u8..=6, 0..256)) {
            let mut packer = TagPacker::new();
            for tag in &tags {
                packer.push(*tag);
            }
            let bytes = packer.finish();
            let unpacker = TagUnpacker::new(&bytes, tags.len());
            let unpacked: Vec<u8> = unpacker.collect();

            prop_assert_eq!(unpacked, tags);
        }

        #[test]
        fn prop_boolean_bitpacking_roundtrip(bools in proptest::collection::vec(any::<bool>(), 0..512)) {
            let bitmap = PresenceBitmap::from_bools(&bools);
            let bytes = bitmap.to_bytes();
            let mut bitvec = bitvec::prelude::BitVec::<u8, Lsb0>::from_slice(&bytes);
            bitvec.truncate(bools.len());
            let restored: Vec<bool> = bitvec.iter().map(|bit| *bit).collect();

            prop_assert_eq!(restored, bools);
        }
    }
}
