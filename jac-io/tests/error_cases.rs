//! Negative decoding tests covering key `JacError` variants

use jac_codec::{block_builder::BlockData, BlockBuilder, Codec, CompressOpts, DecompressOpts};
use jac_format::varint::{decode_uleb128, encode_uleb128};
use jac_format::{
    constants::FLAG_NESTED_OPAQUE, BlockHeader, FieldDirectoryEntry, FileHeader, JacError, Limits,
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

fn encode_file(
    header: &FileHeader,
    block_header: &BlockHeader,
    segments: &[Vec<u8>],
    tamper_checksum: bool,
) -> Vec<u8> {
    let mut bytes = header.encode().expect("encode file header");

    let mut block_bytes = block_header.encode().expect("encode block header");
    for segment in segments {
        block_bytes.extend_from_slice(segment);
    }
    let mut crc = jac_format::checksum::compute_crc32c(&block_bytes);
    if tamper_checksum {
        crc ^= 0xFFFF_FFFF;
    }
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
    let (file_header, mut block_header, segments) =
        build_block(&[json!({"id": 1}), json!({"id": 2})]);

    for entry in block_header.fields.iter_mut() {
        entry.compressor = 99;
    }

    let bytes = encode_file(&file_header, &block_header, &segments, false);
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

    let FieldDirectoryEntry {
        presence_bytes,
        tag_bytes,
        ..
    } = block_header.fields[field_index].clone();
    assert!(tag_bytes >= 1);

    // Overwrite tag stream with reserved tag = 7
    let tag_offset = presence_bytes;
    segments[field_index][tag_offset] = 0b0000_0111;

    let bytes = encode_file(&file_header, &block_header, &segments, false);
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
    let (file_header, block_header, segments) = build_block(&[json!({"id": 1})]);

    let mut bytes = encode_file(&file_header, &block_header, &segments, false);
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

    let bytes = encode_file(&file_header, &block_header, &segments, false);
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

#[test]
fn reader_reports_checksum_mismatch() {
    let (file_header, block_header, segments) = build_block(&[json!({"id": 42})]);

    let bytes = encode_file(&file_header, &block_header, &segments, true);
    let mut reader = JacReader::new(Cursor::new(bytes), default_decode_opts()).expect("reader");
    let mut stream = reader.record_stream().expect("record stream");

    match stream.next() {
        Some(Err(JacError::ChecksumMismatch)) => {}
        other => panic!("expected ChecksumMismatch, got {:?}", other),
    }
}

#[test]
fn reader_reports_dictionary_index_error() {
    let (file_header, block_header, mut segments) = build_block(&[
        json!({"user": "alice"}),
        json!({"user": "bob"}),
        json!({"user": "alice"}),
    ]);

    let field_index = block_header
        .fields
        .iter()
        .position(|entry| entry.field_name == "user")
        .expect("user field present");
    let entry = &block_header.fields[field_index];
    assert!(entry.dict_entry_count >= 2, "dictionary expected");

    let mut segment = segments[field_index].clone();
    let mut cursor = entry.presence_bytes + entry.tag_bytes;
    for _ in 0..entry.dict_entry_count {
        let (len, len_bytes) = decode_uleb128(&segment[cursor..]).expect("dict entry length");
        cursor += len_bytes + len as usize;
    }

    let (_orig_index, index_bytes) = decode_uleb128(&segment[cursor..]).expect("string index");
    let bad_index = entry.dict_entry_count as u64 + 1;
    let bad_encoded = encode_uleb128(bad_index);
    assert_eq!(index_bytes, bad_encoded.len(), "index byte width stable");
    segment[cursor..cursor + index_bytes].copy_from_slice(&bad_encoded);

    segments[field_index] = segment;
    let bytes = encode_file(&file_header, &block_header, &segments, false);

    let mut reader = JacReader::new(Cursor::new(bytes), default_decode_opts()).expect("reader");
    let mut stream = reader.record_stream().expect("record stream");
    match stream.next() {
        Some(Err(JacError::DictionaryError)) => {}
        other => panic!("expected DictionaryError, got {:?}", other),
    }
}

#[test]
fn reader_rejects_invalid_magic() {
    let (file_header, block_header, segments) = build_block(&[json!({"id": 1})]);
    let mut bytes = encode_file(&file_header, &block_header, &segments, false);
    assert!(bytes.len() >= 4);
    bytes[0..4].copy_from_slice(b"BADC");

    match JacReader::new(Cursor::new(bytes), default_decode_opts()) {
        Err(JacError::InvalidMagic) => {}
        Err(err) => panic!("expected InvalidMagic, got {err:?}"),
        Ok(_) => panic!("expected InvalidMagic, got Ok"),
    }
}
