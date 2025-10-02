//! Variable-length integer encoding (ULEB128 / ZigZag)

use smallvec::SmallVec;

/// Encode a u64 as ULEB128
pub fn encode_uleb128(val: u64) -> SmallVec<[u8; 10]> {
    let mut result = SmallVec::new();
    let mut x = val;

    while x >= 0x80 {
        result.push((x & 0x7F) as u8 | 0x80);
        x >>= 7;
    }
    result.push((x & 0x7F) as u8);

    result
}

/// Decode ULEB128 from bytes
pub fn decode_uleb128(bytes: &[u8]) -> Result<(u64, usize), crate::error::JacError> {
    let mut result = 0u64;
    let mut shift = 0;

    for (i, &byte) in bytes.iter().enumerate() {
        if i >= 10 {
            return Err(crate::error::JacError::LimitExceeded(
                "ULEB128 too long".to_string(),
            ));
        }

        result |= ((byte & 0x7F) as u64) << shift;

        if (byte & 0x80) == 0 {
            return Ok((result, i + 1));
        }

        shift += 7;
    }

    Err(crate::error::JacError::UnexpectedEof)
}

/// ZigZag encode a signed integer
pub fn zigzag_encode(v: i64) -> u64 {
    ((v << 1) ^ (v >> 63)) as u64
}

/// ZigZag decode to signed integer
pub fn zigzag_decode(u: u64) -> i64 {
    ((u >> 1) as i64) ^ -((u & 1) as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn test_uleb128_roundtrip() {
        let test_cases = vec![0u64, 1, 127, 128, 16383, 16384, u64::MAX];

        for val in test_cases {
            let encoded = encode_uleb128(val);
            let (decoded, bytes_consumed) = decode_uleb128(&encoded).unwrap();
            assert_eq!(val, decoded);
            assert_eq!(bytes_consumed, encoded.len());
        }
    }

    proptest! {
        #[test]
        fn prop_uleb128_roundtrip(value in any::<u64>()) {
            let encoded = encode_uleb128(value);
            let (decoded, consumed) = decode_uleb128(&encoded).unwrap();
            prop_assert_eq!(decoded, value);
            prop_assert!(consumed <= 10, "encoded length should be <= 10 bytes");
        }

        #[test]
        fn prop_zigzag_roundtrip(value in any::<i64>()) {
            let encoded = zigzag_encode(value);
            let decoded = zigzag_decode(encoded);
            prop_assert_eq!(decoded, value);
        }
    }

    #[test]
    fn test_uleb128_decode_truncated() {
        let encoded = encode_uleb128(1000);
        let truncated = &encoded[..encoded.len() - 1];
        assert!(decode_uleb128(truncated).is_err());
    }

    #[test]
    fn test_uleb128_decode_too_long() {
        let mut long_bytes = vec![0x80; 11]; // 11 bytes, all with continuation bit
        long_bytes.push(0x00); // final byte
        assert!(decode_uleb128(&long_bytes).is_err());
    }

    #[test]
    fn test_zigzag_roundtrip() {
        let test_cases = vec![0i64, 1, -1, 127, -127, 128, -128, i64::MAX, i64::MIN];

        for val in test_cases {
            let encoded = zigzag_encode(val);
            let decoded = zigzag_decode(encoded);
            assert_eq!(val, decoded);
        }
    }

    #[test]
    fn test_zigzag_encoding_values() {
        assert_eq!(zigzag_encode(0), 0);
        assert_eq!(zigzag_encode(-1), 1);
        assert_eq!(zigzag_encode(1), 2);
        assert_eq!(zigzag_encode(-2), 3);
        assert_eq!(zigzag_encode(2), 4);
    }
}
