use jac_format::Result;
use jac_io::{
    execute_compress, execute_decompress, parallel::ParallelConfig, CompressOptions,
    CompressRequest, CompressSummary, ContainerFormat, DecompressFormat, DecompressOptions,
    DecompressRequest, InputSource, JacInput, OutputSink,
};
use serde_json::{Map, Value};
use std::fs;
use tempfile::TempDir;

fn build_records(count: usize) -> Vec<Map<String, Value>> {
    let levels = ["debug", "info", "warn", "error"];
    let mut records = Vec::with_capacity(count);

    for i in 0..count {
        let mut map = Map::new();
        map.insert(
            "ts".to_string(),
            Value::Number(serde_json::Number::from(1_700_000_000_i64 + i as i64)),
        );
        map.insert(
            "user".to_string(),
            Value::String(format!("user_{:04}", i % 1024)),
        );
        map.insert(
            "level".to_string(),
            Value::String(levels[i % levels.len()].to_string()),
        );
        map.insert(
            "message".to_string(),
            Value::String(format!("payload for record {}", i)),
        );
        map.insert(
            "session".to_string(),
            Value::String(format!("session-{}", i % 256)),
        );
        records.push(map);
    }

    records
}

fn compress_with_threads(
    records: &[Map<String, Value>],
    threads: usize,
) -> Result<(Vec<u8>, CompressSummary)> {
    let temp_dir = TempDir::new()?;
    let output_path = temp_dir.path().join(format!("compression-{}.jac", threads));

    let mut options = CompressOptions::default();
    options.block_target_records = 2_000;
    options.default_codec = jac_codec::Codec::Zstd(6);
    options.parallel_config = ParallelConfig {
        memory_reservation_factor: 1.0,
        max_threads: Some(threads),
    };
    options.limits.max_block_uncompressed_total = 32 * 1024 * 1024;

    let iterator_records = records.to_vec();
    let request = CompressRequest {
        input: InputSource::Iterator(Box::new(iterator_records.into_iter())),
        output: OutputSink::Path(output_path.clone()),
        options,
        container_hint: Some(ContainerFormat::Ndjson),
        emit_index: false,
    };

    let summary = execute_compress(request)?;
    let bytes = fs::read(&output_path)?;
    Ok((bytes, summary))
}

fn decompress_bytes(bytes: &[u8]) -> Result<Vec<Value>> {
    let temp_dir = TempDir::new()?;
    let input_path = temp_dir.path().join("input.jac");
    let output_path = temp_dir.path().join("output.ndjson");

    fs::write(&input_path, bytes)?;

    let request = DecompressRequest {
        input: JacInput::Path(input_path),
        output: OutputSink::Path(output_path.clone()),
        format: DecompressFormat::Ndjson,
        options: DecompressOptions::default(),
    };

    execute_decompress(request)?;
    let contents = fs::read_to_string(&output_path)?;
    let mut values = Vec::new();
    for line in contents.lines() {
        if line.trim().is_empty() {
            continue;
        }
        values.push(serde_json::from_str::<Value>(line)?);
    }
    Ok(values)
}

#[test]
fn parallel_output_matches_sequential_bytes() -> Result<()> {
    let records = build_records(12_000);

    let (seq_bytes, seq_summary) = compress_with_threads(&records, 1)?;
    let (par_bytes, par_summary) = compress_with_threads(&records, 8)?;

    assert_eq!(
        seq_summary.metrics.records_written,
        par_summary.metrics.records_written
    );
    assert_eq!(seq_bytes, par_bytes);

    let seq_decision = seq_summary
        .parallel_decision
        .expect("sequential decision present");
    assert!(!seq_decision.use_parallel);
    assert_eq!(seq_decision.thread_count, 1);

    let par_decision = par_summary
        .parallel_decision
        .expect("parallel decision present");
    assert!(par_decision.use_parallel);
    assert!(par_decision.thread_count >= 2);

    Ok(())
}

#[test]
fn parallel_decompression_matches_original_data() -> Result<()> {
    let records = build_records(6_000);
    let (bytes, summary) = compress_with_threads(&records, 8)?;
    assert!(summary
        .parallel_decision
        .as_ref()
        .map(|decision| decision.use_parallel)
        .unwrap_or(false));

    let decoded = decompress_bytes(&bytes)?;
    let original: Vec<Value> = records.iter().cloned().map(Value::Object).collect();
    assert_eq!(decoded, original);
    Ok(())
}
