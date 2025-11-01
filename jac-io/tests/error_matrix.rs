//! Comprehensive error test matrix covering all JacError variants

use jac_codec::{
    block_builder::BlockData, BlockBuilder, Codec, CompressOpts, DecompressOpts,
    TryAddRecordOutcome,
};
use jac_format::varint::{decode_uleb128, encode_uleb128};
use jac_format::{
    constants::FLAG_NESTED_OPAQUE, BlockHeader, FieldDirectoryEntry, FileHeader, JacError, Limits,
};
use jac_io::{
    execute_compress, execute_decompress, execute_project, CompressOptions, CompressRequest,
    ContainerFormat, DecompressFormat, DecompressOptions, DecompressRequest, InputSource, JacInput,
    JacReader, OutputSink, ProjectFormat, ProjectRequest, WrapperConfig,
};
use serde_json::{json, Map, Value};
use std::io::Cursor;
use std::io::{self, Write};
use std::path::PathBuf;

/// Error test matrix mapping each JacError variant to test scenarios
///
/// This matrix ensures comprehensive coverage of all error paths in the JAC implementation.
/// Each test verifies that the correct error variant is returned with appropriate context.
pub struct ErrorTestMatrix;

impl ErrorTestMatrix {
    /// Test InvalidMagic error
    /// Triggered when file doesn't start with expected magic bytes
    pub fn test_invalid_magic() {
        let mut bytes = vec![0x4A, 0x41, 0x43, 0x01]; // Valid magic
        bytes[0] = 0xFF; // Corrupt first byte
                         // Add enough bytes to pass the minimum length check
        bytes.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);

        match JacReader::new(Cursor::new(bytes), DecompressOpts::default()) {
            Err(JacError::InvalidMagic) => {}
            Err(err) => panic!("expected InvalidMagic, got {err:?}"),
            Ok(_) => panic!("expected InvalidMagic, got Ok"),
        }
    }

    /// Test UnsupportedVersion error
    /// Triggered when file version is not supported
    pub fn test_unsupported_version() {
        let mut bytes = vec![0x4A, 0x41, 0x43, 0x01]; // Valid magic
        bytes[3] = 0xFF; // Invalid version
                         // Add enough bytes to pass the minimum length check
        bytes.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);

        match JacReader::new(Cursor::new(bytes), DecompressOpts::default()) {
            Err(JacError::UnsupportedVersion(v)) => assert_eq!(v, 0xFF),
            Err(err) => panic!("expected UnsupportedVersion, got {err:?}"),
            Ok(_) => panic!("expected UnsupportedVersion, got Ok"),
        }
    }

    /// Test CorruptHeader error
    /// Triggered when file header metadata is all zeros but not empty
    pub fn test_corrupt_header() {
        // Create a valid header with non-zero metadata length but all-zero metadata
        let mut bytes = vec![0x4A, 0x41, 0x43, 0x01]; // Valid magic
        bytes.extend_from_slice(&0u32.to_le_bytes()); // flags
        bytes.push(1); // compressor
        bytes.push(15); // level
        bytes.extend_from_slice(&encode_uleb128(100_000)); // block size hint
        bytes.extend_from_slice(&encode_uleb128(4)); // metadata length = 4
        bytes.extend_from_slice(&[0, 0, 0, 0]); // all-zero metadata (triggers CorruptHeader)

        match JacReader::new(Cursor::new(bytes), DecompressOpts::default()) {
            Err(JacError::CorruptHeader) => {}
            Err(err) => panic!("expected CorruptHeader, got {err:?}"),
            Ok(_) => panic!("expected CorruptHeader, got Ok"),
        }
    }

    /// Test CorruptBlock error
    /// Triggered when block header is malformed
    pub fn test_corrupt_block() {
        let (file_header, block_header, segments) = build_test_block(&[json!({"id": 1})]);
        let mut bytes = encode_test_file(&file_header, &block_header, &segments, false);

        // Corrupt block magic
        let header_len = file_header.encode().expect("encode header").len();
        if let Some(byte) = bytes.get_mut(header_len) {
            *byte ^= 0xFF;
        }

        let mut reader =
            JacReader::new(Cursor::new(bytes), DecompressOpts::default()).expect("reader");
        let mut stream = reader.record_stream().expect("record stream");

        match stream.next() {
            Some(Err(JacError::CorruptBlock)) => {}
            other => panic!("expected CorruptBlock, got {:?}", other),
        }
    }

    /// Test ChecksumMismatch error
    /// Triggered when CRC32C verification fails
    pub fn test_checksum_mismatch() {
        let (file_header, block_header, segments) = build_test_block(&[json!({"id": 42})]);
        let bytes = encode_test_file(&file_header, &block_header, &segments, true); // Tamper checksum

        let mut reader =
            JacReader::new(Cursor::new(bytes), DecompressOpts::default()).expect("reader");
        let mut stream = reader.record_stream().expect("record stream");

        match stream.next() {
            Some(Err(JacError::ChecksumMismatch)) => {}
            other => panic!("expected ChecksumMismatch, got {:?}", other),
        }
    }

    /// Test UnexpectedEof error
    /// Triggered when input is truncated
    pub fn test_unexpected_eof() {
        let (file_header, block_header, segments) = build_test_block(&[json!({"id": 1})]);
        let mut bytes = encode_test_file(&file_header, &block_header, &segments, false);
        bytes.truncate(bytes.len() - 3); // Truncate

        let mut reader =
            JacReader::new(Cursor::new(bytes), DecompressOpts::default()).expect("reader");
        let mut stream = reader.record_stream().expect("record stream");

        match stream.next() {
            Some(Err(JacError::UnexpectedEof)) => {}
            other => panic!("expected UnexpectedEof, got {:?}", other),
        }
    }

    /// Test DecompressError error
    /// Triggered when decompression fails
    pub fn test_decompress_error() {
        let (file_header, mut block_header, segments) = build_test_block(&[json!({"msg": "hi"})]);

        // Mark as compressed but don't actually compress
        for entry in block_header.fields.iter_mut() {
            entry.compressor = 1; // Zstd
        }

        let bytes = encode_test_file(&file_header, &block_header, &segments, false);
        let mut reader =
            JacReader::new(Cursor::new(bytes), DecompressOpts::default()).expect("reader");
        let mut stream = reader.record_stream().expect("record stream");

        match stream.next() {
            Some(Err(JacError::DecompressError(message))) => {
                assert!(message.to_lowercase().contains("zstd"));
            }
            other => panic!("expected DecompressError, got {:?}", other),
        }
    }

    /// Test LimitExceeded error for various limits
    pub fn test_limit_exceeded() {
        // Test string length limit
        let (file_header, block_header, segments) = build_test_block(&[json!({"msg": "exceeds"})]);
        let bytes = encode_test_file(&file_header, &block_header, &segments, false);

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

    /// Test TypeMismatch error
    /// Triggered when JSON type doesn't match expected format
    pub fn test_type_mismatch() {
        let request = CompressRequest {
            input: InputSource::JsonArrayReader(Box::new(Cursor::new(Vec::from(
                b"invalid json content\n" as &[u8],
            )))),
            output: OutputSink::Writer(Box::new(Cursor::new(Vec::new()))),
            options: CompressOptions::default(),
            container_hint: Some(ContainerFormat::JsonArray),
            emit_index: true,
            wrapper_config: WrapperConfig::None,
        };

        match execute_compress(request) {
            Err(JacError::TypeMismatch) => {}
            Err(err) => panic!("expected TypeMismatch, got {err:?}"),
            Ok(_) => panic!("expected TypeMismatch, got Ok"),
        }
    }

    /// Test DictionaryError error
    /// Triggered when dictionary index is out of range
    pub fn test_dictionary_error() {
        let (file_header, block_header, mut segments) = build_test_block(&[
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

        // Corrupt dictionary index
        let mut segment = segments[field_index].clone();
        let mut cursor = entry.presence_bytes + entry.tag_bytes;
        for _ in 0..entry.dict_entry_count {
            let (len, len_bytes) = decode_uleb128(&segment[cursor..]).expect("dict entry length");
            cursor += len_bytes + len as usize;
        }

        let (_orig_index, index_bytes) = decode_uleb128(&segment[cursor..]).expect("string index");
        let bad_index = entry.dict_entry_count as u64 + 1;
        let bad_encoded = encode_uleb128(bad_index);
        segment[cursor..cursor + index_bytes].copy_from_slice(&bad_encoded);

        segments[field_index] = segment;
        let bytes = encode_test_file(&file_header, &block_header, &segments, false);

        let mut reader =
            JacReader::new(Cursor::new(bytes), DecompressOpts::default()).expect("reader");
        let mut stream = reader.record_stream().expect("record stream");

        match stream.next() {
            Some(Err(JacError::DictionaryError)) => {}
            other => panic!("expected DictionaryError, got {:?}", other),
        }
    }

    /// Test UnsupportedFeature error
    /// Triggered when encountering reserved or unsupported features
    pub fn test_unsupported_feature() {
        let (file_header, block_header, mut segments) =
            build_test_block(&[json!({"flag": true}), json!({"flag": false})]);

        let field_index = block_header
            .fields
            .iter()
            .position(|entry| entry.field_name == "flag")
            .expect("flag field present");

        let FieldDirectoryEntry { presence_bytes, .. } = block_header.fields[field_index].clone();

        // Overwrite tag stream with reserved tag = 7
        let tag_offset = presence_bytes;
        segments[field_index][tag_offset] = 0b0000_0111;

        let bytes = encode_test_file(&file_header, &block_header, &segments, false);
        let mut reader =
            JacReader::new(Cursor::new(bytes), DecompressOpts::default()).expect("reader");
        let mut stream = reader.record_stream().expect("record stream");

        match stream.next() {
            Some(Err(JacError::UnsupportedFeature(message))) => {
                assert!(message.contains("Reserved type tag 7"));
            }
            other => panic!("expected UnsupportedFeature, got {:?}", other),
        }
    }

    /// Test UnsupportedCompression error
    /// Triggered when encountering unknown compression codec
    pub fn test_unsupported_compression() {
        let (file_header, mut block_header, segments) =
            build_test_block(&[json!({"id": 1}), json!({"id": 2})]);

        for entry in block_header.fields.iter_mut() {
            entry.compressor = 99; // Unknown compressor
        }

        let bytes = encode_test_file(&file_header, &block_header, &segments, false);
        let mut reader =
            JacReader::new(Cursor::new(bytes), DecompressOpts::default()).expect("reader");
        let mut stream = reader.record_stream().expect("record stream");

        match stream.next() {
            Some(Err(JacError::UnsupportedCompression(code))) => assert_eq!(code, 99),
            other => panic!("expected UnsupportedCompression, got {:?}", other),
        }
    }

    /// Test Io error for input failures
    pub fn test_io_input_error() {
        let request = CompressRequest {
            input: InputSource::NdjsonPath(PathBuf::from("/definitely/missing.ndjson")),
            output: OutputSink::Writer(Box::new(Cursor::new(Vec::new()))),
            options: CompressOptions::default(),
            container_hint: None,
            emit_index: false,
            wrapper_config: WrapperConfig::None,
        };

        match execute_compress(request) {
            Err(JacError::Io(err)) => {
                assert_eq!(err.kind(), io::ErrorKind::NotFound);
            }
            Err(err) => panic!("expected Io error, got {err:?}"),
            Ok(_) => panic!("expected Io error, got Ok"),
        }
    }

    /// Test Io error for output failures
    pub fn test_io_output_error() {
        let (file_header, block_header, segments) = build_test_block(&[json!({"v": 1})]);
        let bytes = encode_test_file(&file_header, &block_header, &segments, false);

        let request = DecompressRequest {
            input: JacInput::Reader(Box::new(Cursor::new(bytes))),
            output: OutputSink::Writer(Box::new(FailingWriter)),
            format: DecompressFormat::Ndjson,
            options: DecompressOptions::default(),
        };

        match execute_decompress(request) {
            Err(JacError::Io(err)) => {
                assert_eq!(err.kind(), io::ErrorKind::Other);
            }
            Err(err) => panic!("expected Io error, got {err:?}"),
            Ok(_) => panic!("expected Io error, got Ok"),
        }
    }

    /// Test Json error for malformed JSON input
    pub fn test_json_error() {
        let request = CompressRequest {
            input: InputSource::NdjsonReader(Box::new(Cursor::new(b"{invalid}\n".to_vec()))),
            output: OutputSink::Writer(Box::new(Cursor::new(Vec::new()))),
            options: CompressOptions::default(),
            container_hint: Some(ContainerFormat::Ndjson),
            emit_index: false,
            wrapper_config: WrapperConfig::None,
        };

        match execute_compress(request) {
            Err(JacError::Json(_)) => {}
            Err(err) => panic!("expected Json error, got {err:?}"),
            Ok(_) => panic!("expected Json error, got Ok"),
        }
    }

    /// Test Internal error for invalid internal state
    pub fn test_internal_error() {
        let (file_header, block_header, segments) = build_test_block(&[json!({"value": 42})]);
        let bytes = encode_test_file(&file_header, &block_header, &segments, false);

        let request = ProjectRequest {
            input: JacInput::Reader(Box::new(Cursor::new(bytes))),
            output: OutputSink::Writer(Box::new(Cursor::new(Vec::new()))),
            fields: Vec::new(), // Empty fields should trigger internal error
            format: ProjectFormat::Ndjson,
            options: DecompressOptions::default(),
        };

        match execute_project(request) {
            Err(JacError::Internal(message)) => {
                assert!(message.contains("project request"));
            }
            Err(err) => panic!("expected Internal error, got {err:?}"),
            Ok(_) => panic!("expected Internal error, got Ok"),
        }
    }
}

// Helper functions for building test data

fn map_from(value: Value) -> Map<String, Value> {
    value.as_object().expect("object").clone()
}

fn build_test_block(records: &[Value]) -> (FileHeader, BlockHeader, Vec<Vec<u8>>) {
    let mut opts = CompressOpts::default();
    opts.block_target_records = records.len().max(1);
    opts.default_codec = Codec::None;

    let mut builder = BlockBuilder::new(opts.clone());
    for record in records {
        match builder
            .try_add_record(map_from(record.clone()))
            .expect("try add record")
        {
            TryAddRecordOutcome::Added => {}
            TryAddRecordOutcome::BlockFull { .. } => {
                panic!("unexpected block flush in error matrix setup")
            }
        }
    }

    let BlockData {
        header, segments, ..
    } = builder.finalize().expect("finalize block").data;

    let file_header = FileHeader {
        flags: FLAG_NESTED_OPAQUE,
        default_compressor: opts.default_codec.compressor_id(),
        default_compression_level: opts.default_codec.level(),
        block_size_hint_records: opts.block_target_records,
        user_metadata: Vec::new(),
    };

    (file_header, header, segments)
}

fn encode_test_file(
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

struct FailingWriter;

impl Write for FailingWriter {
    fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
        Err(io::Error::new(io::ErrorKind::Other, "sink failure"))
    }

    fn flush(&mut self) -> io::Result<()> {
        Err(io::Error::new(io::ErrorKind::Other, "flush failure"))
    }
}

// Test cases for each error variant

#[test]
fn test_invalid_magic() {
    ErrorTestMatrix::test_invalid_magic();
}

#[test]
fn test_unsupported_version() {
    ErrorTestMatrix::test_unsupported_version();
}

#[test]
fn test_corrupt_header() {
    ErrorTestMatrix::test_corrupt_header();
}

#[test]
fn test_corrupt_block() {
    ErrorTestMatrix::test_corrupt_block();
}

#[test]
fn test_checksum_mismatch() {
    ErrorTestMatrix::test_checksum_mismatch();
}

#[test]
fn test_unexpected_eof() {
    ErrorTestMatrix::test_unexpected_eof();
}

#[test]
fn test_decompress_error() {
    ErrorTestMatrix::test_decompress_error();
}

#[test]
fn test_limit_exceeded() {
    ErrorTestMatrix::test_limit_exceeded();
}

#[test]
fn test_type_mismatch() {
    ErrorTestMatrix::test_type_mismatch();
}

#[test]
fn test_dictionary_error() {
    ErrorTestMatrix::test_dictionary_error();
}

#[test]
fn test_unsupported_feature() {
    ErrorTestMatrix::test_unsupported_feature();
}

#[test]
fn test_unsupported_compression() {
    ErrorTestMatrix::test_unsupported_compression();
}

#[test]
fn test_io_input_error() {
    ErrorTestMatrix::test_io_input_error();
}

#[test]
fn test_io_output_error() {
    ErrorTestMatrix::test_io_output_error();
}

#[test]
fn test_json_error() {
    ErrorTestMatrix::test_json_error();
}

#[test]
fn test_internal_error() {
    ErrorTestMatrix::test_internal_error();
}
