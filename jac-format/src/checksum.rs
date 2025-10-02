//! CRC32C checksum utilities

/// Compute CRC32C for data
pub fn compute_crc32c(data: &[u8]) -> u32 {
    crc32c::crc32c(data)
}

/// Verify CRC32C for data
pub fn verify_crc32c(data: &[u8], expected: u32) -> Result<(), crate::error::JacError> {
    let actual = compute_crc32c(data);
    if actual == expected {
        Ok(())
    } else {
        Err(crate::error::JacError::ChecksumMismatch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_crc32c_basic() {
        let data = b"Hello, world!";
        let crc = compute_crc32c(data);
        assert_ne!(crc, 0);
    }

    #[test]
    fn test_crc32c_empty() {
        let data = b"";
        let crc = compute_crc32c(data);
        assert_eq!(crc, 0);
    }

    #[test]
    fn test_crc32c_verify_match() {
        let data = b"Test data";
        let crc = compute_crc32c(data);
        assert!(verify_crc32c(data, crc).is_ok());
    }

    #[test]
    fn test_crc32c_verify_mismatch() {
        let data = b"Test data";
        let wrong_crc = 0x12345678;
        assert!(verify_crc32c(data, wrong_crc).is_err());
    }

    #[test]
    fn test_crc32c_deterministic() {
        let data = b"Same data";
        let crc1 = compute_crc32c(data);
        let crc2 = compute_crc32c(data);
        assert_eq!(crc1, crc2);
    }

    #[test]
    fn test_crc32c_different_data() {
        let data1 = b"Data 1";
        let data2 = b"Data 2";
        let crc1 = compute_crc32c(data1);
        let crc2 = compute_crc32c(data2);
        assert_ne!(crc1, crc2);
    }
}
