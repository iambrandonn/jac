//! Conformance tests for SPEC §12.1 sample fixture

use jac_codec::{
    block_builder::BlockData,
    block_decode::{BlockDecoder, DecompressOpts},
    BlockBuilder, Codec, CompressOpts,
};
use jac_format::constants::{ENCODING_FLAG_DELTA, ENCODING_FLAG_DICTIONARY};
use serde_json::{Map, Value};
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
