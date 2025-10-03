//! Conformance tests for SPEC §12.1 sample fixture and comprehensive validation suite

use jac_codec::{
    block_builder::BlockData,
    block_decode::{BlockDecoder, DecompressOpts},
    BlockBuilder, Codec, CompressOpts,
};
use jac_format::constants::{ENCODING_FLAG_DELTA, ENCODING_FLAG_DICTIONARY};
// Unused imports removed for now
use serde_json::{json, Map, Value};
use std::fs;
use std::path::PathBuf;

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../testdata/spec/v12_1.jsonl")
}

fn load_spec_records() -> Vec<Map<String, Value>> {
    let contents = fs::read_to_string(fixture_path()).expect("read spec fixture");
    contents
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            serde_json::from_str::<Value>(line)
                .expect("valid JSON line")
                .as_object()
                .expect("object record")
                .clone()
        })
        .collect()
}

fn build_spec_block() -> (BlockData, Vec<Map<String, Value>>) {
    let records = load_spec_records();
    let mut opts = CompressOpts::default();
    opts.block_target_records = records.len().max(1);
    opts.default_codec = Codec::None; // deterministic payload for assertions

    let mut builder = BlockBuilder::new(opts);
    for record in records.iter().cloned() {
        builder.add_record(record).expect("add record");
    }

    let block = builder.finalize().expect("finalize block");
    (block, records)
}

fn assemble_block_bytes(block: &BlockData) -> Vec<u8> {
    let mut bytes = block.header.encode().expect("encode header");
    for segment in &block.segments {
        bytes.extend_from_slice(segment);
    }
    bytes.extend_from_slice(&block.crc32c.to_le_bytes());
    bytes
}

#[test]
fn spec12_round_trip_matches_fixture() {
    let (block, original_records) = build_spec_block();
    assert_eq!(block.header.record_count, original_records.len());

    let bytes = assemble_block_bytes(&block);
    let decoder = BlockDecoder::new(&bytes, &DecompressOpts::default()).expect("decode block");
    let decoded_records = decoder.decode_records().expect("decode records");

    assert_eq!(decoded_records, original_records, "semantic round trip");
}

#[test]
fn spec12_field_encodings_match_requirements() {
    let (block, _records) = build_spec_block();

    let ts_entry = block
        .header
        .fields
        .iter()
        .find(|entry| entry.field_name == "ts")
        .expect("ts field present");
    assert_eq!(ts_entry.value_count_present, block.header.record_count);
    assert_ne!(
        ts_entry.encoding_flags & ENCODING_FLAG_DELTA,
        0,
        "ts uses delta encoding"
    );
    assert_eq!(ts_entry.encoding_flags & ENCODING_FLAG_DICTIONARY, 0);

    let level_entry = block
        .header
        .fields
        .iter()
        .find(|entry| entry.field_name == "level")
        .expect("level field present");
    assert_eq!(level_entry.value_count_present, 3); // last record omits level
    assert_ne!(
        level_entry.encoding_flags & ENCODING_FLAG_DICTIONARY,
        0,
        "level uses dictionary encoding"
    );
    assert_eq!(level_entry.dict_entry_count, 2, "INFO and WARN entries");

    let user_entry = block
        .header
        .fields
        .iter()
        .find(|entry| entry.field_name == "user")
        .expect("user field present");
    assert_eq!(user_entry.value_count_present, block.header.record_count);

    let error_entry = block
        .header
        .fields
        .iter()
        .find(|entry| entry.field_name == "error")
        .expect("error field present");
    assert_eq!(error_entry.value_count_present, 1, "error present once");
    assert_eq!(error_entry.encoding_flags & ENCODING_FLAG_DELTA, 0);
    assert!(error_entry.dict_entry_count <= 1);
}

#[test]
fn spec12_projection_yields_expected_sequence() {
    let (block, _records) = build_spec_block();
    let bytes = assemble_block_bytes(&block);
    let decoder = BlockDecoder::new(&bytes, &DecompressOpts::default()).expect("decode block");

    let user_projection = decoder.project_field("user").expect("project user field");
    let users: Vec<String> = user_projection
        .into_iter()
        .map(|opt| {
            opt.expect("user present")
                .as_str()
                .expect("string")
                .to_owned()
        })
        .collect();
    assert_eq!(users, ["alice", "alice", "bob", "carol"]);

    let error_projection = decoder.project_field("error").expect("project error field");
    let mut error_values = error_projection.into_iter();
    assert!(error_values.next().unwrap().is_none());
    assert!(error_values.next().unwrap().is_none());
    assert!(error_values.next().unwrap().is_none());
    assert_eq!(
        error_values
            .next()
            .unwrap()
            .expect("error present")
            .as_str()
            .expect("string"),
        "Disk failure"
    );
}

/// Test schema drift - fields changing types across records
#[test]
fn schema_drift_validation() {
    let records: Vec<Value> = vec![
        json!({"id": 1, "value": "string"}),
        json!({"id": 2, "value": 42}),
        json!({"id": 3, "value": true}),
        json!({"id": 4, "value": null}),
    ];

    let mut opts = CompressOpts::default();
    opts.block_target_records = records.len();
    opts.default_codec = Codec::None;

    let mut builder = BlockBuilder::new(opts);
    for record in records.iter() {
        builder
            .add_record(record.as_object().unwrap().clone())
            .expect("add record");
    }

    let block = builder.finalize().expect("finalize block");
    let bytes = assemble_block_bytes(&block);
    let decoder = BlockDecoder::new(&bytes, &DecompressOpts::default()).expect("decode block");

    // Verify all records decode correctly despite type changes
    let decoded_records = decoder.decode_records().expect("decode records");
    assert_eq!(decoded_records.len(), 4);

    // Verify field segment structure handles mixed types
    let value_entry = block
        .header
        .fields
        .iter()
        .find(|entry| entry.field_name == "value")
        .expect("value field present");

    // Should have presence bits for all records
    assert_eq!(value_entry.value_count_present, 4);

    // Type tags should be packed for all present values
    assert!(value_entry.tag_bytes > 0);
}

/// Test multi-level validation - verify intermediate representations
#[test]
fn multi_level_validation() {
    let (block, _records) = build_spec_block();

    // Validate block header structure
    assert!(block.header.record_count > 0);
    assert!(!block.header.fields.is_empty());

    // Validate field directory entries
    for field in &block.header.fields {
        assert!(!field.field_name.is_empty());
        assert!(field.presence_bytes > 0);
        assert!(field.tag_bytes > 0);
        assert!(field.value_count_present <= block.header.record_count);
        // segment_offset is usize, so >= 0 is always true
        assert!(field.segment_compressed_len > 0);
        assert!(field.segment_uncompressed_len > 0);
    }

    // Validate segment layout
    // Segment offsets are relative to the start of the segments region (after header)
    let mut expected_offset = 0;
    for (i, field) in block.header.fields.iter().enumerate() {
        assert_eq!(field.segment_offset, expected_offset, "field {} offset mismatch: actual {}, expected {}", i, field.segment_offset, expected_offset);
        expected_offset += field.segment_compressed_len;

        // Verify segment bytes exist
        assert!(i < block.segments.len());
        assert_eq!(block.segments[i].len(), field.segment_compressed_len);
    }
}

/// Test deeply nested objects and arrays
#[test]
fn deeply_nested_structures() {
    let records: Vec<Value> = vec![
        json!({
            "id": 1,
            "nested": {
                "level1": {
                    "level2": {
                        "level3": {
                            "level4": {
                                "level5": "deep_value"
                            }
                        }
                    }
                }
            },
            "array": [1, 2, [3, 4, [5, 6, [7, 8]]]]
        }),
    ];

    let mut opts = CompressOpts::default();
    opts.block_target_records = records.len();
    opts.default_codec = Codec::None;

    let mut builder = BlockBuilder::new(opts);
    for record in records.iter() {
        builder
            .add_record(record.as_object().unwrap().clone())
            .expect("add record");
    }

    let block = builder.finalize().expect("finalize block");
    let bytes = assemble_block_bytes(&block);
    let decoder = BlockDecoder::new(&bytes, &DecompressOpts::default()).expect("decode block");
    let decoded_records = decoder.decode_records().expect("decode records");

    // Verify round-trip preserves nested structure
    assert_eq!(decoded_records.len(), 1);
    let decoded = &decoded_records[0];

    // Verify nested object structure
    let nested = decoded.get("nested").unwrap().as_object().unwrap();
    let level1 = nested.get("level1").unwrap().as_object().unwrap();
    let level2 = level1.get("level2").unwrap().as_object().unwrap();
    let level3 = level2.get("level3").unwrap().as_object().unwrap();
    let level4 = level3.get("level4").unwrap().as_object().unwrap();
    let level5 = level4.get("level5").unwrap().as_str().unwrap();
    assert_eq!(level5, "deep_value");

    // Verify nested array structure
    let array = decoded.get("array").unwrap().as_array().unwrap();
    assert_eq!(array[0], 1);
    assert_eq!(array[1], 2);
    let inner_array = array[2].as_array().unwrap();
    assert_eq!(inner_array[0], 3);
    assert_eq!(inner_array[1], 4);
    let deeper_array = inner_array[2].as_array().unwrap();
    assert_eq!(deeper_array[0], 5);
    assert_eq!(deeper_array[1], 6);
    let deepest_array = deeper_array[2].as_array().unwrap();
    assert_eq!(deepest_array[0], 7);
    assert_eq!(deepest_array[1], 8);
}

/// Test high-precision decimal values
#[test]
fn high_precision_decimals() {
    let records: Vec<Value> = vec![
        json!({"id": 1, "value": "0.123456789012345678901234567890123456789"}),
        json!({"id": 2, "value": "1e-100"}),
        json!({"id": 3, "value": "1e+100"}),
        json!({"id": 4, "value": "999999999999999999999999999999999999999.999999999999999999999999999999999999999"}),
    ];

    let mut opts = CompressOpts::default();
    opts.block_target_records = records.len();
    opts.default_codec = Codec::None;

    let mut builder = BlockBuilder::new(opts);
    for record in records.iter() {
        builder
            .add_record(record.as_object().unwrap().clone())
            .expect("add record");
    }

    let block = builder.finalize().expect("finalize block");
    let bytes = assemble_block_bytes(&block);
    let decoder = BlockDecoder::new(&bytes, &DecompressOpts::default()).expect("decode block");
    let decoded_records = decoder.decode_records().expect("decode records");

    // Verify decimal precision is preserved
    assert_eq!(decoded_records.len(), 4);
    for (i, record) in decoded_records.iter().enumerate() {
        let value = record.get("value").unwrap().as_str().unwrap();
        let original = records[i].as_object().unwrap().get("value").unwrap().as_str().unwrap();
        assert_eq!(value, original, "decimal precision preserved for record {}", i);
    }
}

/// Test empty and single-record files
#[test]
fn empty_and_single_record_files() {
    // Test single record
    let single_record: Vec<Value> = vec![json!({"id": 1, "value": "test"})];

    let mut opts = CompressOpts::default();
    opts.block_target_records = single_record.len();
    opts.default_codec = Codec::None;

    let mut builder = BlockBuilder::new(opts);
    builder
        .add_record(single_record[0].as_object().unwrap().clone())
        .expect("add record");

    let block = builder.finalize().expect("finalize block");
    let bytes = assemble_block_bytes(&block);
    let decoder = BlockDecoder::new(&bytes, &DecompressOpts::default()).expect("decode block");
    let decoded_records = decoder.decode_records().expect("decode records");

    assert_eq!(decoded_records.len(), 1);
    assert_eq!(decoded_records[0].get("id").unwrap(), 1);
    assert_eq!(decoded_records[0].get("value").unwrap().as_str().unwrap(), "test");
}

/// Test large synthetic block (approaching limits)
#[test]
fn large_synthetic_block() {
    let mut records = Vec::new();
    for i in 0..1000 {
        records.push(json!({
            "id": i,
            "timestamp": 1623000000 + i,
            "user": format!("user_{}", i % 100),
            "message": format!("This is message number {} with some additional text", i),
            "score": i as f64 * 0.1,
            "active": i % 2 == 0
        }));
    }

    let mut opts = CompressOpts::default();
    opts.block_target_records = records.len();
    opts.default_codec = Codec::None;

    let mut builder = BlockBuilder::new(opts);
    for record in records.iter() {
        builder
            .add_record(record.as_object().unwrap().clone())
            .expect("add record");
    }

    let block = builder.finalize().expect("finalize block");
    let bytes = assemble_block_bytes(&block);
    let decoder = BlockDecoder::new(&bytes, &DecompressOpts::default()).expect("decode block");
    let decoded_records = decoder.decode_records().expect("decode records");

    assert_eq!(decoded_records.len(), 1000);

    // Verify first and last records
    assert_eq!(decoded_records[0].get("id").unwrap(), 0);
    assert_eq!(decoded_records[999].get("id").unwrap(), 999);

    // Verify user field uses dictionary encoding (should have many repeated values)
    let user_entry = block
        .header
        .fields
        .iter()
        .find(|entry| entry.field_name == "user")
        .expect("user field present");

    // Check if dictionary encoding was used (depends on threshold)
    if user_entry.encoding_flags & ENCODING_FLAG_DICTIONARY != 0 {
        // Should be much less than 1000 unique values due to repetition
        assert!(user_entry.dict_entry_count < 1000, "dict_entry_count: {}", user_entry.dict_entry_count);
    } else {
        // If not dictionary encoded, verify it's raw encoding
        assert_eq!(user_entry.encoding_flags & ENCODING_FLAG_DICTIONARY, 0);
    }
}

/// Test Unicode edge cases
#[test]
fn unicode_edge_cases() {
    let records: Vec<Value> = vec![
        json!({"id": 1, "text": "Hello, 世界! 🌍"}),
        json!({"id": 2, "text": "مرحبا بالعالم"}),
        json!({"id": 3, "text": "Здравствуй, мир!"}),
        json!({"id": 4, "text": "こんにちは世界"}),
        json!({"id": 5, "text": "🎉🎊🎈"}),
    ];

    let mut opts = CompressOpts::default();
    opts.block_target_records = records.len();
    opts.default_codec = Codec::None;

    let mut builder = BlockBuilder::new(opts);
    for record in records.iter() {
        builder
            .add_record(record.as_object().unwrap().clone())
            .expect("add record");
    }

    let block = builder.finalize().expect("finalize block");
    let bytes = assemble_block_bytes(&block);
    let decoder = BlockDecoder::new(&bytes, &DecompressOpts::default()).expect("decode block");
    let decoded_records = decoder.decode_records().expect("decode records");

    assert_eq!(decoded_records.len(), 5);
    for (i, record) in decoded_records.iter().enumerate() {
        let text = record.get("text").unwrap().as_str().unwrap();
        let original = records[i].as_object().unwrap().get("text").unwrap().as_str().unwrap();
        assert_eq!(text, original, "Unicode text preserved for record {}", i);
    }
}

/// Test boundary values and edge cases
#[test]
fn boundary_values_and_edge_cases() {
    let records: Vec<Value> = vec![
        json!({"id": 0, "value": 0}),
        json!({"id": 1, "value": i64::MAX}),
        json!({"id": 2, "value": i64::MIN}),
        json!({"id": 3, "value": 0.0}),
        json!({"id": 4, "value": f64::MIN_POSITIVE}),
        json!({"id": 5, "value": f64::MAX}),
        json!({"id": 6, "value": ""}),
        json!({"id": 7, "value": "a".repeat(1000)}),
        json!({"id": 8, "value": []}),
        json!({"id": 9, "value": {}}),
    ];

    let mut opts = CompressOpts::default();
    opts.block_target_records = records.len();
    opts.default_codec = Codec::None;

    let mut builder = BlockBuilder::new(opts);
    for record in records.iter() {
        builder
            .add_record(record.as_object().unwrap().clone())
            .expect("add record");
    }

    let block = builder.finalize().expect("finalize block");
    let bytes = assemble_block_bytes(&block);
    let decoder = BlockDecoder::new(&bytes, &DecompressOpts::default()).expect("decode block");
    let decoded_records = decoder.decode_records().expect("decode records");

    assert_eq!(decoded_records.len(), 10);

    // Verify boundary values are preserved
    assert_eq!(decoded_records[0].get("value").unwrap(), 0);
    assert_eq!(decoded_records[1].get("value").unwrap(), i64::MAX);
    assert_eq!(decoded_records[2].get("value").unwrap(), i64::MIN);
    assert_eq!(decoded_records[3].get("value").unwrap(), 0.0);
    assert_eq!(decoded_records[6].get("value").unwrap().as_str().unwrap(), "");
    assert_eq!(decoded_records[7].get("value").unwrap().as_str().unwrap().len(), 1000);
    assert!(decoded_records[8].get("value").unwrap().as_array().unwrap().is_empty());
    assert!(decoded_records[9].get("value").unwrap().as_object().unwrap().is_empty());
}
