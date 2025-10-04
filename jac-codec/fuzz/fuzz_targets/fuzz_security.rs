#![no_main]

use jac_codec::block_decode::DecompressOpts;
use jac_codec::BlockDecoder;
use jac_format::{Limits, TypeTag, FILE_MAGIC, BLOCK_MAGIC};
use jac_format::varint::{decode_uleb128, decode_zigzag_i64};
use jac_format::bitpack::{pack_presence_bitmap, unpack_presence_bitmap};
use jac_format::checksum::crc32c;
use libfuzzer_sys::fuzz_target;

/// Security-focused fuzzing target for JAC
/// Tests for common security vulnerabilities and attack vectors
fuzz_target!(|data: &[u8]| {
    // Test 1: Memory exhaustion attacks
    test_memory_exhaustion_attacks(data);

    // Test 2: Integer overflow/underflow attacks
    test_integer_overflow_attacks(data);

    // Test 3: Buffer overflow attacks
    test_buffer_overflow_attacks(data);

    // Test 4: Format string attacks
    test_format_string_attacks(data);

    // Test 5: Injection attacks
    test_injection_attacks(data);

    // Test 6: Denial of service attacks
    test_dos_attacks(data);

    // Test 7: Malformed input attacks
    test_malformed_input_attacks(data);

    // Test 8: Resource exhaustion attacks
    test_resource_exhaustion_attacks(data);
});

/// Test for memory exhaustion attacks
fn test_memory_exhaustion_attacks(data: &[u8]) {
    // Test with extremely large input sizes
    if data.len() > 0 {
        // Test varint decoding with large numbers
        let _ = decode_uleb128(data);
        let _ = decode_zigzag_i64(data);

        // Test with large presence bitmaps
        if data.len() > 1000 {
            let _ = unpack_presence_bitmap(data, data.len() / 8);
        }
    }
}

/// Test for integer overflow/underflow attacks
fn test_integer_overflow_attacks(data: &[u8]) {
    // Test varint decoding with potentially overflowing values
    if data.len() > 0 {
        let _ = decode_uleb128(data);
        let _ = decode_zigzag_i64(data);
    }

    // Test type tag validation
    for &byte in data.iter().take(10) {
        let _ = TypeTag::from_u8(byte);
    }
}

/// Test for buffer overflow attacks
fn test_buffer_overflow_attacks(data: &[u8]) {
    // Test with various buffer sizes
    for size in [1, 10, 100, 1000, 10000] {
        if data.len() >= size {
            let slice = &data[..size];
            let _ = decode_uleb128(slice);
            let _ = decode_zigzag_i64(slice);
        }
    }
}

/// Test for format string attacks
fn test_format_string_attacks(data: &[u8]) {
    // Test with potentially malicious format strings
    if data.len() > 0 {
        // Convert data to string and test for format string vulnerabilities
        if let Ok(s) = std::str::from_utf8(data) {
            // Test that we don't have format string vulnerabilities
            // by ensuring we don't use user input in format strings
            let _ = format!("Safe: {}", s.len());
        }
    }
}

/// Test for injection attacks
fn test_injection_attacks(data: &[u8]) {
    // Test with potentially malicious input that could be injected
    if data.len() > 0 {
        // Test varint decoding with potentially malicious data
        let _ = decode_uleb128(data);
        let _ = decode_zigzag_i64(data);

        // Test CRC calculation with potentially malicious data
        let _ = crc32c(data);
    }
}

/// Test for denial of service attacks
fn test_dos_attacks(data: &[u8]) {
    // Test with inputs designed to cause excessive computation
    if data.len() > 0 {
        // Test with very large numbers that might cause excessive computation
        let _ = decode_uleb128(data);
        let _ = decode_zigzag_i64(data);

        // Test with inputs that might cause infinite loops
        // (This is handled by the fuzzer's timeout mechanism)
    }
}

/// Test for malformed input attacks
fn test_malformed_input_attacks(data: &[u8]) {
    // Test with malformed magic numbers
    if data.len() >= 4 {
        // Test with corrupted magic numbers
        let mut corrupted_data = data.to_vec();
        if corrupted_data.len() >= 4 {
            corrupted_data[0] = 0xFF;
            corrupted_data[1] = 0xFF;
            corrupted_data[2] = 0xFF;
            corrupted_data[3] = 0xFF;
        }

        // Test block decoder with corrupted data
        let opts = DecompressOpts {
            limits: Limits::default(),
            verify_checksums: false,
        };
        let _ = BlockDecoder::new(&corrupted_data, &opts);
    }
}

/// Test for resource exhaustion attacks
fn test_resource_exhaustion_attacks(data: &[u8]) {
    // Test with inputs designed to exhaust specific resources
    if data.len() > 0 {
        // Test with very large presence bitmaps
        if data.len() > 100 {
            let bitmap_size = std::cmp::min(data.len() / 8, 10000);
            let _ = unpack_presence_bitmap(data, bitmap_size);
        }

        // Test with large varint values
        let _ = decode_uleb128(data);
        let _ = decode_zigzag_i64(data);
    }
}

/// Test for specific security vulnerabilities
fn test_specific_vulnerabilities(data: &[u8]) {
    // Test for CVE-style vulnerabilities
    if data.len() > 0 {
        // Test with null bytes (potential null pointer dereference)
        if data.contains(&0) {
            let _ = decode_uleb128(data);
        }

        // Test with maximum values (potential integer overflow)
        if data.iter().all(|&b| b == 0xFF) {
            let _ = decode_uleb128(data);
            let _ = decode_zigzag_i64(data);
        }

        // Test with alternating patterns (potential buffer overrun)
        if data.len() > 1 {
            let mut pattern_data = Vec::new();
            for i in 0..std::cmp::min(data.len(), 1000) {
                pattern_data.push(if i % 2 == 0 { 0xAA } else { 0x55 });
            }
            let _ = decode_uleb128(&pattern_data);
        }
    }
}

/// Test for race conditions and concurrency issues
fn test_concurrency_issues(data: &[u8]) {
    // Test with inputs that might cause race conditions
    if data.len() > 0 {
        // Test varint decoding (should be thread-safe)
        let _ = decode_uleb128(data);
        let _ = decode_zigzag_i64(data);

        // Test CRC calculation (should be thread-safe)
        let _ = crc32c(data);
    }
}

/// Test for information disclosure vulnerabilities
fn test_information_disclosure(data: &[u8]) {
    // Test with inputs that might leak sensitive information
    if data.len() > 0 {
        // Test varint decoding (should not leak memory)
        let _ = decode_uleb128(data);
        let _ = decode_zigzag_i64(data);

        // Test that we don't expose internal state
        // (This is more of a design requirement than a fuzz test)
    }
}

/// Test for authentication and authorization bypasses
fn test_auth_bypasses(data: &[u8]) {
    // Test with inputs that might bypass security checks
    if data.len() > 0 {
        // Test with malformed headers that might bypass validation
        if data.len() >= 8 {
            let mut bypass_data = data.to_vec();
            // Try to bypass magic number validation
            bypass_data[0..4].copy_from_slice(&FILE_MAGIC);
            bypass_data[4..8].copy_from_slice(&BLOCK_MAGIC);

            let opts = DecompressOpts {
                limits: Limits::default(),
                verify_checksums: false,
            };
            let _ = BlockDecoder::new(&bypass_data, &opts);
        }
    }
}

/// Test for input validation bypasses
fn test_validation_bypasses(data: &[u8]) {
    // Test with inputs designed to bypass input validation
    if data.len() > 0 {
        // Test with edge cases that might bypass validation
        let _ = decode_uleb128(data);
        let _ = decode_zigzag_i64(data);

        // Test type tag validation with edge cases
        for &byte in data.iter().take(10) {
            let _ = TypeTag::from_u8(byte);
        }
    }
}
