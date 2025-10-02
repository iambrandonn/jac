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
