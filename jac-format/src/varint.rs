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
            return Err(crate::error::JacError::LimitExceeded("ULEB128 too long".to_string()));
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

