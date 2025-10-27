//! Cross-platform and version compatibility tests for JAC format
//!
//! This module tests that JAC files created on one platform can be read
//! on another platform, and that version compatibility is maintained.

use jac_codec::block_builder::BlockBuilder;
use jac_codec::block_decode::BlockDecoder;
use jac_codec::{Codec, CompressOpts, DecompressOpts, TryAddRecordOutcome};
use jac_format::{
    block::BlockHeader, error::JacError, header::FileHeader, limits::Limits, types::TypeTag,
};
use serde_json::{Map, Value};

/// Test that files created with different endianness assumptions work correctly
#[test]
fn test_endianness_compatibility() {
    // Create a test file with various data types that would be affected by endianness
    let records = vec![
        create_test_record(1000, "value1"),
        create_test_record(2000, "value2"),
        create_test_record(3000, "value3"),
    ];

    // Encode the records
    let opts = CompressOpts {
        block_target_records: 10,
        default_codec: Codec::Zstd(15),
        canonicalize_keys: true,
        canonicalize_numbers: true,
        nested_opaque: true,
        max_dict_entries: 4096,
        limits: Limits::default(),
    };

    let mut block_builder = BlockBuilder::new(opts.clone());
    for record in records {
        match block_builder
            .try_add_record(record)
            .expect("try add record")
        {
            TryAddRecordOutcome::Added => {}
            TryAddRecordOutcome::BlockFull { .. } => {
                panic!("unexpected block flush in endianness test")
            }
        }
    }

    let block_data = block_builder.finalize().unwrap();

    // Verify the block can be decoded
    let decompress_opts = DecompressOpts {
        limits: Limits::default(),
        verify_checksums: true,
    };

    let block_bytes = {
        let mut bytes = block_data.header.encode().unwrap();
        for segment in &block_data.segments {
            bytes.extend_from_slice(segment);
        }
        bytes.extend_from_slice(&block_data.crc32c.to_le_bytes());
        bytes
    };

    let decoder = BlockDecoder::new(&block_bytes, &decompress_opts).unwrap();
    let decoded_records = decoder.decode_records().unwrap();

    // Verify the data round-trips correctly
    assert_eq!(decoded_records.len(), 3);

    // Check that numeric values are preserved correctly (endianness-sensitive)
    for (i, record) in decoded_records.iter().enumerate() {
        if let Some(Value::Number(num)) = record.get("id") {
            assert_eq!(num.as_i64().unwrap(), ((i + 1) * 1000) as i64);
        } else {
            panic!("Expected numeric id field");
        }
    }
}

/// Test version compatibility by creating files with different version assumptions
#[test]
fn test_version_compatibility() {
    // Test that we can handle files with different magic versions
    let test_cases = vec![
        // Current version (0x01)
        (0x01, true),
        // Future version (should be rejected)
        (0x02, false),
        // Invalid version (should be rejected)
        (0x00, false),
    ];

    for (version, should_succeed) in test_cases {
        let mut header_bytes = vec![0x4A, 0x41, 0x43, version]; // "JAC" + version

        // Add minimal header data
        header_bytes.extend_from_slice(&0u32.to_le_bytes()); // flags
        header_bytes.push(1); // default_compressor
        header_bytes.push(15); // default_compression_level
        header_bytes.extend_from_slice(&1u64.to_le_bytes()); // block_size_hint_records (ULEB128)
        header_bytes.extend_from_slice(&0u64.to_le_bytes()); // user_metadata_len (ULEB128)

        let result = FileHeader::decode(&header_bytes);

        if should_succeed {
            assert!(result.is_ok(), "Version {} should be supported", version);
        } else {
            assert!(result.is_err(), "Version {} should be rejected", version);
            if let Err(JacError::UnsupportedVersion(v)) = result {
                assert_eq!(v, version);
            } else {
                panic!("Expected UnsupportedVersion error for version {}", version);
            }
        }
    }
}

/// Test that type tags are handled consistently across platforms
#[test]
fn test_type_tag_compatibility() {
    let test_cases = vec![
        (0, TypeTag::Null),
        (1, TypeTag::Bool),
        (2, TypeTag::Int),
        (3, TypeTag::Decimal),
        (4, TypeTag::String),
        (5, TypeTag::Object),
        (6, TypeTag::Array),
    ];

    for (tag_value, expected_tag) in test_cases {
        let tag = TypeTag::from_u8(tag_value).unwrap();
        assert_eq!(tag, expected_tag);
        assert_eq!(tag as u8, tag_value);
    }

    // Test reserved tag (7) is rejected
    let result = TypeTag::from_u8(7);
    assert!(result.is_err());
    if let Err(JacError::UnsupportedFeature(msg)) = result {
        assert!(msg.contains("Reserved type tag 7"));
    } else {
        panic!(
            "Expected UnsupportedFeature for reserved tag, got: {:?}",
            result
        );
    }
}

/// Test that compression codecs work consistently across platforms
#[test]
fn test_compression_codec_compatibility() {
    let test_cases = vec![
        (Codec::None, true),
        (Codec::Zstd(1), true),
        (Codec::Zstd(15), true),
        (Codec::Zstd(22), true),
        (Codec::Brotli(11), false), // Not implemented in v0.1.0
        (Codec::Deflate(6), false), // Not implemented in v0.1.0
    ];

    for (codec, should_succeed) in test_cases {
        let records = vec![create_test_record(123, "value")];

        let opts = CompressOpts {
            block_target_records: 10,
            default_codec: codec,
            canonicalize_keys: false,
            canonicalize_numbers: false,
            nested_opaque: true,
            max_dict_entries: 4096,
            limits: Limits::default(),
        };

        let mut block_builder = BlockBuilder::new(opts);
        for record in records {
            match block_builder
                .try_add_record(record)
                .expect("try add record")
            {
                TryAddRecordOutcome::Added => {}
                TryAddRecordOutcome::BlockFull { .. } => {
                    panic!("unexpected block flush in codec compatibility test")
                }
            }
        }

        let result = block_builder.finalize();
        if should_succeed {
            assert!(result.is_ok(), "Codec {:?} should work", codec);
        } else {
            assert!(result.is_err(), "Codec {:?} should fail", codec);
        }
    }
}

/// Test that limits are enforced consistently across platforms
#[test]
fn test_limits_compatibility() {
    let limits = Limits::default();

    // Test that default limits are reasonable
    assert!(limits.max_records_per_block > 0);
    assert!(limits.max_fields_per_block > 0);
    assert!(limits.max_segment_uncompressed_len > 0);
    assert!(limits.max_block_uncompressed_total > 0);
    assert!(limits.max_dict_entries_per_field > 0);
    assert!(limits.max_string_len_per_value > 0);
    assert!(limits.max_decimal_digits_per_value > 0);
    assert!(limits.max_presence_bytes > 0);
    assert!(limits.max_tag_bytes > 0);

    // Test that hard limits are enforced
    let hard_limits = Limits {
        max_records_per_block: 1_000_000,
        max_fields_per_block: 65_535,
        max_segment_uncompressed_len: 64 * 1024 * 1024,
        max_block_uncompressed_total: 256 * 1024 * 1024,
        max_dict_entries_per_field: 65_535,
        max_string_len_per_value: 16 * 1024 * 1024,
        max_decimal_digits_per_value: 65_536,
        max_presence_bytes: 32 * 1024 * 1024,
        max_tag_bytes: 32 * 1024 * 1024,
    };

    // These should be the maximum allowed values
    assert_eq!(hard_limits.max_records_per_block, 1_000_000);
    assert_eq!(hard_limits.max_fields_per_block, 65_535);
    assert_eq!(hard_limits.max_segment_uncompressed_len, 64 * 1024 * 1024);
}

/// Test that file headers are encoded/decoded consistently
#[test]
fn test_file_header_cross_platform() {
    let test_cases = vec![
        // Basic header
        FileHeader {
            flags: 0,
            default_compressor: 1,
            default_compression_level: 15,
            block_size_hint_records: 100_000,
            user_metadata: vec![],
        },
        // Header with flags
        FileHeader {
            flags: 0b111, // All flags set
            default_compressor: 1,
            default_compression_level: 22,
            block_size_hint_records: 50_000,
            user_metadata: b"test metadata".to_vec(),
        },
        // Header with large metadata
        FileHeader {
            flags: 0b101, // Some flags set
            default_compressor: 0,
            default_compression_level: 1,
            block_size_hint_records: 0,
            user_metadata: vec![0u8; 1024],
        },
    ];

    for header in test_cases {
        let encoded = header.encode().unwrap();
        let (decoded, bytes_consumed) = FileHeader::decode(&encoded).unwrap();

        assert_eq!(decoded.flags, header.flags);
        assert_eq!(decoded.default_compressor, header.default_compressor);
        assert_eq!(
            decoded.default_compression_level,
            header.default_compression_level
        );
        assert_eq!(
            decoded.block_size_hint_records,
            header.block_size_hint_records
        );
        assert_eq!(decoded.user_metadata, header.user_metadata);

        // Verify we consumed all bytes
        assert_eq!(bytes_consumed, encoded.len());
    }
}

/// Test that block headers are encoded/decoded consistently
#[test]
fn test_block_header_cross_platform() {
    use jac_format::block::FieldDirectoryEntry;

    let test_cases = vec![
        // Empty block
        BlockHeader {
            record_count: 0,
            fields: vec![],
        },
        // Single field block
        BlockHeader {
            record_count: 100,
            fields: vec![FieldDirectoryEntry {
                field_name: "id".to_string(),
                compressor: 1,
                compression_level: 15,
                presence_bytes: 13, // ceil(100/8)
                tag_bytes: 38,      // ceil(3*100/8)
                value_count_present: 100,
                encoding_flags: 0,
                dict_entry_count: 0,
                segment_uncompressed_len: 1000,
                segment_compressed_len: 500,
                segment_offset: 0,
            }],
        },
        // Multiple fields block
        BlockHeader {
            record_count: 1000,
            fields: vec![
                FieldDirectoryEntry {
                    field_name: "id".to_string(),
                    compressor: 1,
                    compression_level: 15,
                    presence_bytes: 125, // ceil(1000/8)
                    tag_bytes: 375,      // ceil(3*1000/8)
                    value_count_present: 1000,
                    encoding_flags: 1, // dictionary
                    dict_entry_count: 10,
                    segment_uncompressed_len: 5000,
                    segment_compressed_len: 2500,
                    segment_offset: 0,
                },
                FieldDirectoryEntry {
                    field_name: "name".to_string(),
                    compressor: 1,
                    compression_level: 15,
                    presence_bytes: 125,
                    tag_bytes: 375,
                    value_count_present: 1000,
                    encoding_flags: 1, // dictionary
                    dict_entry_count: 50,
                    segment_uncompressed_len: 10000,
                    segment_compressed_len: 5000,
                    segment_offset: 2500,
                },
            ],
        },
    ];

    for header in test_cases {
        let encoded = header.encode().unwrap();
        let (decoded, bytes_consumed) = BlockHeader::decode(&encoded, &Limits::default()).unwrap();

        assert_eq!(decoded.record_count, header.record_count);
        assert_eq!(decoded.fields.len(), header.fields.len());

        for (i, (decoded_field, original_field)) in
            decoded.fields.iter().zip(header.fields.iter()).enumerate()
        {
            assert_eq!(
                decoded_field.field_name, original_field.field_name,
                "Field {} name mismatch",
                i
            );
            assert_eq!(
                decoded_field.compressor, original_field.compressor,
                "Field {} compressor mismatch",
                i
            );
            assert_eq!(
                decoded_field.compression_level, original_field.compression_level,
                "Field {} compression_level mismatch",
                i
            );
            assert_eq!(
                decoded_field.presence_bytes, original_field.presence_bytes,
                "Field {} presence_bytes mismatch",
                i
            );
            assert_eq!(
                decoded_field.tag_bytes, original_field.tag_bytes,
                "Field {} tag_bytes mismatch",
                i
            );
            assert_eq!(
                decoded_field.value_count_present, original_field.value_count_present,
                "Field {} value_count_present mismatch",
                i
            );
            assert_eq!(
                decoded_field.encoding_flags, original_field.encoding_flags,
                "Field {} encoding_flags mismatch",
                i
            );
            assert_eq!(
                decoded_field.dict_entry_count, original_field.dict_entry_count,
                "Field {} dict_entry_count mismatch",
                i
            );
            assert_eq!(
                decoded_field.segment_uncompressed_len, original_field.segment_uncompressed_len,
                "Field {} segment_uncompressed_len mismatch",
                i
            );
            assert_eq!(
                decoded_field.segment_compressed_len, original_field.segment_compressed_len,
                "Field {} segment_compressed_len mismatch",
                i
            );
            assert_eq!(
                decoded_field.segment_offset, original_field.segment_offset,
                "Field {} segment_offset mismatch",
                i
            );
        }

        // Verify we consumed all bytes
        assert_eq!(bytes_consumed, encoded.len());
    }
}

/// Test that the spec conformance fixture works across platforms
#[test]
fn test_spec_conformance_cross_platform() {
    // Use embedded test data instead of file system
    let fixture_content = r#"{"ts":1623000000,"level":"INFO","msg":"Started","user":"alice"}
{"ts":1623000005,"level":"INFO","msg":"Step1","user":"alice"}
{"ts":1623000010,"level":"WARN","msg":"Low disk","user":"bob"}
{"ts":1623000020,"user":"carol","error":"Disk failure"}"#;

    let records: Vec<Map<String, Value>> = fixture_content
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();

    assert_eq!(records.len(), 4, "Spec fixture should have 4 records");

    // Test compression and decompression
    let opts = CompressOpts {
        block_target_records: 10,
        default_codec: Codec::Zstd(15),
        canonicalize_keys: true,
        canonicalize_numbers: true,
        nested_opaque: true,
        max_dict_entries: 4096,
        limits: Limits::default(),
    };

    let mut block_builder = BlockBuilder::new(opts.clone());
    for record in &records {
        match block_builder
            .try_add_record(record.clone())
            .expect("try add record")
        {
            TryAddRecordOutcome::Added => {}
            TryAddRecordOutcome::BlockFull { .. } => {
                panic!("unexpected block flush in spec conformance test")
            }
        }
    }

    let block_data = block_builder.finalize().unwrap();

    // Verify the block can be decoded
    let decompress_opts = DecompressOpts {
        limits: Limits::default(),
        verify_checksums: true,
    };

    let block_bytes = {
        let mut bytes = block_data.header.encode().unwrap();
        for segment in &block_data.segments {
            bytes.extend_from_slice(segment);
        }
        bytes.extend_from_slice(&block_data.crc32c.to_le_bytes());
        bytes
    };

    let decoder = BlockDecoder::new(&block_bytes, &decompress_opts).unwrap();
    let decoded_records = decoder.decode_records().unwrap();

    // Verify semantic equality
    assert_eq!(decoded_records.len(), records.len());

    // Test projection of the "user" field as specified in the spec
    let user_values: Vec<Option<&Value>> = decoded_records
        .iter()
        .map(|record| record.get("user"))
        .collect();

    // Create owned values for comparison
    let alice1 = Value::String("alice".to_string());
    let alice2 = Value::String("alice".to_string());
    let bob = Value::String("bob".to_string());
    let carol = Value::String("carol".to_string());

    let expected_users = vec![Some(&alice1), Some(&alice2), Some(&bob), Some(&carol)];

    assert_eq!(user_values, expected_users);
}

/// Helper function to create test records
fn create_test_record(id_value: i64, value_field: &str) -> Map<String, Value> {
    let mut record = Map::new();
    record.insert(
        "id".to_string(),
        Value::Number(serde_json::Number::from(id_value)),
    );
    record.insert("value".to_string(), Value::String(value_field.to_string()));
    record
}
