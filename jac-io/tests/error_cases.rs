//! Negative decoding tests covering key `JacError` variants

use jac_codec::{block_builder::BlockData, BlockBuilder, Codec, CompressOpts, DecompressOpts};
use jac_format::{
    checksum::compute_crc32c, constants::FLAG_NESTED_OPAQUE, BlockHeader, FileHeader, JacError,
    Limits,
};
use jac_io::JacReader;
use serde_json::{json, Map, Value};
use std::io::Cursor;

fn map_from(value: Value) -> Map<String, Value> {
    value.as_object().expect("object").clone()
}

fn build_block(records: &[Value]) -> (FileHeader, BlockHeader, Vec<Vec<u8>>) {
    let mut opts = CompressOpts::default();
    opts.block_target_records = records.len().max(1);
    opts.default_codec = Codec::None;

    let mut builder = BlockBuilder::new(opts.clone());
    for record in records {
        builder
            .add_record(map_from(record.clone()))
            .expect("add record");
    }

    let BlockData {
        header, segments, ..
    } = builder.finalize().expect("finalize block");

    let file_header = FileHeader {
        flags: FLAG_NESTED_OPAQUE,
        default_compressor: opts.default_codec.compressor_id(),
        default_compression_level: opts.default_codec.level(),
        block_size_hint_records: opts.block_target_records,
        user_metadata: Vec::new(),
    };

    (file_header, header, segments)
}

fn encode_file(header: &FileHeader, block_header: &BlockHeader, segments: &[Vec<u8>]) -> Vec<u8> {
    let mut bytes = header.encode().expect("encode file header");

    let mut block_bytes = block_header.encode().expect("encode block header");
    for segment in segments {
        block_bytes.extend_from_slice(segment);
    }
    let crc = compute_crc32c(&block_bytes);
    block_bytes.extend_from_slice(&crc.to_le_bytes());
    bytes.extend_from_slice(&block_bytes);

    bytes
}

fn default_decode_opts() -> DecompressOpts {
    DecompressOpts {
        limits: Limits::default(),
        verify_checksums: true,
    }
}

#[test]
fn reader_reports_unsupported_compression() {
    let (file_header, mut block_header, segments) = build_block(&[
        json!({"id": 1, "msg": "hello"}),
        json!({"id": 2, "msg": "world"}),
    ]);

    for entry in block_header.fields.iter_mut() {
        entry.compressor = 99; // invalid compressor id
    }

    let bytes = encode_file(&file_header, &block_header, &segments);
    let mut reader = JacReader::new(Cursor::new(bytes), default_decode_opts()).expect("reader");
    let mut stream = reader.record_stream().expect("record stream");

    match stream.next() {
        Some(Err(JacError::UnsupportedCompression(code))) => assert_eq!(code, 99),
        other => panic!("expected UnsupportedCompression, got {:?}", other),
    }
}

#[test]
fn reader_reports_reserved_type_tag() {
    let (file_header, block_header, mut segments) =
        build_block(&[json!({"flag": true}), json!({"flag": false})]);

    let field_index = block_header
        .fields
        .iter()
        .position(|entry| entry.field_name == "flag")
        .expect("flag field present");

    let presence_bytes = block_header.fields[field_index].presence_bytes;
    let tag_bytes = block_header.fields[field_index].tag_bytes;
    assert!(tag_bytes >= 1, "expected tag bytes for flag field");

    // Overwrite the first 3-bit tag with value 7 (reserved)
    let tag_offset = presence_bytes;
    segments[field_index][tag_offset] = 0b0000_0111;

    let bytes = encode_file(&file_header, &block_header, &segments);
    let mut reader = JacReader::new(Cursor::new(bytes), default_decode_opts()).expect("reader");
    let mut stream = reader.record_stream().expect("record stream");

    match stream.next() {
        Some(Err(JacError::UnsupportedFeature(message))) => {
            assert!(message.contains("Reserved type tag 7"))
        }
        other => panic!("expected UnsupportedFeature, got {:?}", other),
    }
}

#[test]
fn reader_reports_unexpected_eof_for_truncated_block() {
    let (file_header, block_header, segments) = build_block(&[json!({"id": 1, "msg": "hello"})]);

    let mut bytes = encode_file(&file_header, &block_header, &segments);
    assert!(bytes.len() > 8);
    bytes.truncate(bytes.len() - 3);

    let mut reader = JacReader::new(Cursor::new(bytes), default_decode_opts()).expect("reader");
    let mut stream = reader.record_stream().expect("record stream");

    match stream.next() {
        Some(Err(JacError::UnexpectedEof)) => {}
        other => panic!("expected UnexpectedEof, got {:?}", other),
    }
}

#[test]
fn reader_enforces_string_length_limit() {
    let (file_header, block_header, segments) = build_block(&[json!({"msg": "exceeds"})]);

    let bytes = encode_file(&file_header, &block_header, &segments);
    let mut limits = Limits::default();
    limits.max_string_len_per_value = 4;
    let opts = DecompressOpts {
        limits,
        verify_checksums: true,
    };

    let mut reader = JacReader::new(Cursor::new(bytes), opts).expect("reader");
    let mut stream = reader.record_stream().expect("record stream");

    match stream.next() {
        Some(Err(JacError::LimitExceeded(message))) => {
            assert!(message.to_lowercase().contains("string"));
        }
        other => panic!("expected LimitExceeded, got {:?}", other),
    }
}
