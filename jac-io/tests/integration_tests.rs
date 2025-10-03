//! Integration tests for the JAC I/O layer

use jac_codec::{Codec, CompressOpts, DecompressOpts};
use jac_format::{
    constants::{FILE_MAGIC, INDEX_MAGIC},
    FileHeader, IndexFooter, JacError, Limits,
};
use jac_io::{
    execute_project, DecompressOptions, JacInput, JacReader, JacWriter, OutputSink, ProjectFormat,
    ProjectRequest,
};
use serde_json::{json, Map, Value};
use std::fs;
use std::io::Cursor;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn default_compress_opts(block_target_records: usize) -> (FileHeader, CompressOpts) {
    let mut opts = CompressOpts::default();
    opts.block_target_records = block_target_records;
    opts.default_codec = Codec::None; // keep tests deterministic and fast
    opts.limits = Limits::default();

    let header = FileHeader {
        flags: jac_format::constants::FLAG_NESTED_OPAQUE,
        default_compressor: opts.default_codec.compressor_id(),
        default_compression_level: opts.default_codec.level(),
        block_size_hint_records: block_target_records,
        user_metadata: Vec::new(),
    };

    (header, opts)
}

fn default_decompress_opts() -> DecompressOpts {
    DecompressOpts {
        limits: Limits::default(),
        verify_checksums: true,
    }
}

fn map_from(value: Value) -> Map<String, Value> {
    value.as_object().expect("object expected").clone()
}

fn finish_writer(writer: JacWriter<Cursor<Vec<u8>>>, with_index: bool) -> Vec<u8> {
    writer
        .finish(with_index)
        .expect("finish should succeed")
        .into_inner()
}

fn sample_projection_file() -> Vec<u8> {
    let (header, opts) = default_compress_opts(4);
    let buffer = Cursor::new(Vec::<u8>::new());
    let mut writer = JacWriter::new(buffer, header, opts).unwrap();

    let records = [
        json!({"user": "alice", "active": true}),
        json!({"user": "bob"}),
        json!({"user": "carol", "active": null}),
    ];

    for record in &records {
        writer
            .write_record(&map_from(record.clone()))
            .expect("write record");
    }

    finish_writer(writer, true)
}

#[test]
fn million_record_roundtrip_and_projection() {
    const RECORDS: usize = 1_000_000;
    let (header, mut opts) = default_compress_opts(100_000);
    opts.default_codec = Codec::None;

    let buffer = Cursor::new(Vec::<u8>::new());
    let mut writer = JacWriter::new(buffer, header, opts).expect("writer");

    for i in 0..RECORDS {
        let mut record = Map::new();
        record.insert("id".to_string(), Value::from(i as i64));
        record.insert("even".to_string(), Value::Bool(i % 2 == 0));
        writer.write_record(&record).expect("write record");
    }

    let bytes = finish_writer(writer, false);

    // Ensure file materializes multiple blocks and total record count matches.
    let opts = default_decompress_opts();
    let mut reader = JacReader::new(Cursor::new(bytes.clone()), opts).expect("reader");
    let blocks = reader
        .blocks()
        .collect::<jac_format::Result<Vec<_>>>()
        .expect("collect blocks");
    assert!(blocks.len() > 1, "expected multi-block corpus");
    let block_total: usize = blocks.iter().map(|block| block.record_count).sum();
    assert_eq!(block_total, RECORDS, "block record totals");

    // Verify semantic round-trip via streaming record iterator.
    let opts = default_decompress_opts();
    let mut reader = JacReader::new(Cursor::new(bytes.clone()), opts).expect("reader");
    let mut stream = reader.record_stream().expect("record stream");
    let mut processed = 0usize;
    while let Some(result) = stream.next() {
        let record = result.expect("record decode");
        let id = record.get("id").and_then(|v| v.as_i64()).expect("id");
        assert_eq!(id as usize, processed);
        let even = record.get("even").and_then(|v| v.as_bool()).expect("even");
        assert_eq!(even, processed % 2 == 0);
        processed += 1;
    }
    assert_eq!(processed, RECORDS, "record stream produced all records");

    // Validate projection semantics for the `id` field across the entire corpus.
    let opts = default_decompress_opts();
    let mut reader = JacReader::new(Cursor::new(bytes), opts).expect("reader");
    let mut projection = reader
        .projection_stream("id".to_string())
        .expect("projection stream");
    for expected in 0..RECORDS {
        match projection.next() {
            Some(Ok(Some(Value::Number(num)))) => {
                let id = num.as_i64().expect("numeric id");
                assert_eq!(id as usize, expected);
            }
            other => panic!("unexpected projection result: {:?}", other),
        }
    }
    assert!(projection.next().is_none(), "projection exhausted exactly");
}

#[test]
fn writer_writes_index_footer_and_pointer() {
    let (header, opts) = default_compress_opts(4);
    let buffer = Cursor::new(Vec::<u8>::new());
    let mut writer = JacWriter::new(buffer, header, opts).expect("writer");

    for id in 0..3 {
        let record = map_from(json!({"id": id, "name": format!("user_{id}")}));
        writer.write_record(&record).expect("record write");
    }

    let bytes = finish_writer(writer, true);
    assert!(bytes.starts_with(&FILE_MAGIC), "file magic");

    // Read pointer
    assert!(bytes.len() >= 8, "file should include index pointer");
    let pointer_pos = bytes.len() - 8;
    let index_offset = u64::from_le_bytes(bytes[pointer_pos..].try_into().unwrap()) as usize;
    assert!(
        index_offset < pointer_pos,
        "index offset should precede pointer"
    );
    assert_eq!(
        &bytes[index_offset..index_offset + 4],
        &INDEX_MAGIC.to_le_bytes()
    );

    let footer =
        IndexFooter::decode(&bytes[index_offset..pointer_pos]).expect("decode index footer");
    assert_eq!(footer.blocks.len(), 1, "single block in index");
    let entry = &footer.blocks[0];
    assert_eq!(entry.record_count, 3);
    assert!(entry.block_offset > FILE_MAGIC.len() as u64);
}

#[test]
fn writer_flush_emits_partial_blocks() {
    let (header, opts) = default_compress_opts(8);
    let buffer = Cursor::new(Vec::<u8>::new());
    let mut writer = JacWriter::new(buffer, header, opts).expect("writer");

    let rec_one = map_from(json!({"id": 1, "payload": "first"}));
    writer.write_record(&rec_one).unwrap();
    writer.flush().unwrap();

    let rec_two = map_from(json!({"id": 2, "payload": "second"}));
    writer.write_record(&rec_two).unwrap();
    writer.flush().unwrap();

    let bytes = finish_writer(writer, true);

    let opts = default_decompress_opts();
    let mut reader = JacReader::new(Cursor::new(bytes), opts).expect("reader");
    let blocks = reader
        .blocks()
        .collect::<jac_format::Result<Vec<_>>>()
        .expect("blocks iteration");
    assert_eq!(blocks.len(), 2, "two blocks flushed manually");
    assert_eq!(blocks[0].record_count, 1);
    assert_eq!(blocks[1].record_count, 1);
}

#[test]
fn reader_blocks_without_index() {
    let (header, opts) = default_compress_opts(2);
    let buffer = Cursor::new(Vec::<u8>::new());
    let mut writer = JacWriter::new(buffer, header, opts).unwrap();

    for id in 0..3 {
        let record = map_from(json!({"id": id}));
        writer.write_record(&record).unwrap();
    }

    let bytes = finish_writer(writer, false);
    let opts = default_decompress_opts();
    let mut reader = JacReader::new(Cursor::new(bytes), opts).unwrap();

    let blocks = reader
        .blocks()
        .collect::<jac_format::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(blocks.len(), 2, "streaming mode should expose two blocks");
    assert_eq!(blocks[0].record_count, 2);
    assert_eq!(blocks[1].record_count, 1);
}

#[test]
fn reader_blocks_with_index_uses_footer() {
    let (header, opts) = default_compress_opts(1);
    let buffer = Cursor::new(Vec::<u8>::new());
    let mut writer = JacWriter::new(buffer, header, opts).unwrap();

    for id in 0..3 {
        let record = map_from(json!({"id": id}));
        writer.write_record(&record).unwrap();
    }

    let bytes = finish_writer(writer, true);
    let opts = default_decompress_opts();
    let mut reader = JacReader::new(Cursor::new(bytes.clone()), opts).unwrap();

    let blocks = reader
        .blocks()
        .collect::<jac_format::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(blocks.len(), 3);
    for handle in &blocks {
        assert_eq!(
            handle.record_count, 1,
            "each block should contain one record"
        );
        assert_eq!(handle.offset, handle.offset, "offset stable");
        assert!(handle.size > 0);
        assert!(handle.header.fields.len() >= 1);
    }

    // Ensure index footer still decodes to the same entries
    let pointer_pos = bytes.len() - 8;
    let index_offset = u64::from_le_bytes(bytes[pointer_pos..].try_into().unwrap()) as usize;
    let footer = IndexFooter::decode(&bytes[index_offset..pointer_pos]).unwrap();
    assert_eq!(footer.blocks.len(), 3);
}

#[test]
fn projection_handles_absent_and_null() {
    let (header, opts) = default_compress_opts(4);
    let buffer = Cursor::new(Vec::<u8>::new());
    let mut writer = JacWriter::new(buffer, header, opts).unwrap();

    let records = [
        json!({"user": "alice"}),
        json!({"user": "bob", "active": null}),
        json!({"user": "carol", "active": true}),
    ];

    for record in records.iter() {
        writer.write_record(&map_from(record.clone())).unwrap();
    }

    let bytes = finish_writer(writer, true);
    let opts = default_decompress_opts();
    let mut reader = JacReader::new(Cursor::new(bytes), opts).unwrap();
    let block = reader.blocks().next().unwrap().unwrap();

    let iter = reader.project_field(&block, "active").unwrap();
    let values = iter.collect::<jac_format::Result<Vec<_>>>().unwrap();
    assert_eq!(values.len(), 3);
    assert!(values[0].is_none(), "field absent should yield None");
    assert_eq!(values[1], Some(Value::Null));
    assert_eq!(values[2], Some(Value::Bool(true)));
}

#[test]
fn resync_skips_corrupt_block_when_not_strict() {
    let (header, opts) = default_compress_opts(1);
    let buffer = Cursor::new(Vec::<u8>::new());
    let mut writer = JacWriter::new(buffer, header, opts).unwrap();

    let rec_one = map_from(json!({"id": 1}));
    let rec_two = map_from(json!({"id": 2}));
    writer.write_record(&rec_one).unwrap();
    writer.flush().unwrap();
    writer.write_record(&rec_two).unwrap();

    let mut bytes = finish_writer(writer, false);

    // Corrupt the first block magic to trigger resync
    let header_len = FileHeader::decode(&bytes).unwrap().1;
    let block_magic_pos = header_len;
    bytes[block_magic_pos..block_magic_pos + 4].copy_from_slice(&[0u8; 4]);

    let opts = default_decompress_opts();
    let mut strict_reader = JacReader::new(Cursor::new(bytes.clone()), opts.clone()).unwrap();
    let first = strict_reader.blocks().next().unwrap();
    assert!(matches!(first, Err(JacError::CorruptBlock)));

    let mut lenient_reader = JacReader::with_strict_mode(Cursor::new(bytes), opts, false).unwrap();
    let blocks = lenient_reader
        .blocks()
        .collect::<jac_format::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(blocks.len(), 1, "corrupted block should be skipped");
    assert_eq!(blocks[0].record_count, 1);
    assert_eq!(blocks[0].header.fields.len(), 1);
}

#[test]
fn decode_block_detects_crc_mismatch() {
    let (header, opts) = default_compress_opts(2);
    let buffer = Cursor::new(Vec::<u8>::new());
    let mut writer = JacWriter::new(buffer, header, opts).unwrap();

    writer.write_record(&map_from(json!({"id": 42}))).unwrap();

    let mut bytes = finish_writer(writer, true);

    // Locate pointer and corrupt CRC (last 4 bytes before pointer or end when no index)
    let pointer_pos = bytes.len() - 8;
    let index_offset = u64::from_le_bytes(bytes[pointer_pos..].try_into().unwrap()) as usize;
    let crc_pos = index_offset - 4; // CRC directly precedes footer when index present
    bytes[crc_pos..crc_pos + 4].copy_from_slice(&[0u8; 4]);

    let opts = default_decompress_opts();
    let mut reader = JacReader::new(Cursor::new(bytes), opts).unwrap();
    let block = reader.blocks().next().unwrap().unwrap();
    let result = reader.decode_block(&block);
    assert!(matches!(result, Err(JacError::ChecksumMismatch)));
}

#[test]
fn project_outputs_ndjson_objects() {
    let bytes = sample_projection_file();

    let output_str = run_projection(&bytes, ProjectFormat::Ndjson, &["user", "active"]);
    let expected = concat!(
        "{\"active\":true,\"user\":\"alice\"}\n",
        "{\"user\":\"bob\"}\n",
        "{\"active\":null,\"user\":\"carol\"}\n"
    );
    assert_eq!(output_str, expected);
}

#[test]
fn project_outputs_json_array() {
    let bytes = sample_projection_file();

    let output_str = run_projection(&bytes, ProjectFormat::JsonArray, &["user", "active"]);
    assert_eq!(
        output_str,
        "[{\"active\":true,\"user\":\"alice\"},{\"user\":\"bob\"},{\"active\":null,\"user\":\"carol\"}]"
    );
}

fn run_projection(bytes: &[u8], format: ProjectFormat, fields: &[&str]) -> String {
    let path = temp_output_path(match format {
        ProjectFormat::Ndjson => "ndjson",
        ProjectFormat::JsonArray => "json",
        ProjectFormat::Csv { .. } => "csv",
    });

    let request = ProjectRequest {
        input: JacInput::Reader(Box::new(Cursor::new(bytes.to_vec()))),
        output: OutputSink::Path(path.clone()),
        fields: fields.iter().map(|field| field.to_string()).collect(),
        format,
        options: DecompressOptions::default(),
    };

    execute_project(request).expect("projection");

    let content = fs::read_to_string(&path).expect("read projection output");
    let _ = fs::remove_file(&path);
    content
}

fn temp_output_path(label: &str) -> PathBuf {
    let base = std::env::temp_dir();
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    base.join(format!("jac_integration_{}_{}.out", label, unique))
}
