use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId};
use jac_format::Limits;
use jac_io::{execute_compress, CompressOptions, CompressRequest, InputSource, OutputSink};
use serde_json::json;
use std::io::{Cursor, Write};

/// Generate records with growing field sizes to trigger early flushes
fn generate_segment_pressure_data(count: usize, field_size_kb: usize) -> Vec<u8> {
    let mut buf = Vec::new();

    for i in 0..count {
        // Create a string that grows with the record index
        let size = (field_size_kb * 1024 * (i + 1)) / count;
        let large_field = "x".repeat(size);

        let record = json!({
            "id": i,
            "growing_field": large_field,
            "timestamp": 1600000000 + i,
        });
        writeln!(&mut buf, "{}", record).unwrap();
    }

    buf
}

/// Generate records with consistently large fields
fn generate_large_field_data(count: usize, field_size_kb: usize) -> Vec<u8> {
    let mut buf = Vec::new();
    let large_field = "x".repeat(field_size_kb * 1024);

    for i in 0..count {
        let record = json!({
            "id": i,
            "large_field": large_field,
            "timestamp": 1600000000 + i,
        });
        writeln!(&mut buf, "{}", record).unwrap();
    }

    buf
}

fn bench_early_flush_impact(c: &mut Criterion) {
    let mut group = c.benchmark_group("early_flush_impact");

    // Generate data that will trigger early flushes with small limits
    let data = generate_large_field_data(1000, 100); // 100KB per record

    let scenarios = vec![
        ("no_flush_64MiB", 64 * 1024 * 1024),      // Default - should not flush
        ("frequent_flush_8MiB", 8 * 1024 * 1024),  // Forces early flushes
        ("very_frequent_flush_2MiB", 2 * 1024 * 1024), // Very frequent flushes
    ];

    for (name, segment_limit) in scenarios {
        group.bench_with_input(
            BenchmarkId::from_parameter(name),
            &data,
            |b, data| {
                b.iter(|| {
                    let input = Cursor::new(data.clone());
                    let output = Cursor::new(Vec::new());

                    let mut limits = Limits::default();
                    limits.max_segment_uncompressed_len = segment_limit;

                    let mut options = CompressOptions::default();
                    options.limits = limits;
                    options.block_target_records = 10000; // High target to force segment limit

                    let request = CompressRequest {
                        input: InputSource::NdjsonReader(Box::new(input)),
                        output: OutputSink::Writer(Box::new(output)),
                        options,
                        container_hint: Some(jac_format::ContainerFormat::Ndjson),
                        emit_index: false,
                    };

                    let summary = execute_compress(request).unwrap();
                    black_box((summary.metrics.segment_limit_flushes, summary.metrics.blocks_written));
                });
            },
        );
    }

    group.finish();
}

fn bench_block_size_vs_segment_limit(c: &mut Criterion) {
    let mut group = c.benchmark_group("block_size_vs_segment_limit");

    let data = generate_large_field_data(2000, 50); // 50KB per record

    // With 50KB records, we can fit:
    // - At 64 MiB: ~1310 records
    // - At 32 MiB: ~655 records
    // - At 16 MiB: ~327 records

    let block_sizes = vec![
        ("block_100_limit_64M", 100, 64 * 1024 * 1024),   // Should not flush
        ("block_500_limit_64M", 500, 64 * 1024 * 1024),   // Should not flush
        ("block_1000_limit_64M", 1000, 64 * 1024 * 1024), // Should flush
        ("block_1000_limit_32M", 1000, 32 * 1024 * 1024), // Should flush more
    ];

    for (name, block_size, segment_limit) in block_sizes {
        group.bench_with_input(
            BenchmarkId::from_parameter(name),
            &data,
            |b, data| {
                b.iter(|| {
                    let input = Cursor::new(data.clone());
                    let output = Cursor::new(Vec::new());

                    let mut limits = Limits::default();
                    limits.max_segment_uncompressed_len = segment_limit;

                    let mut options = CompressOptions::default();
                    options.limits = limits;
                    options.block_target_records = block_size;

                    let request = CompressRequest {
                        input: InputSource::NdjsonReader(Box::new(input)),
                        output: OutputSink::Writer(Box::new(output)),
                        options,
                        container_hint: Some(jac_format::ContainerFormat::Ndjson),
                        emit_index: false,
                    };

                    black_box(execute_compress(request).unwrap());
                });
            },
        );
    }

    group.finish();
}

fn bench_growing_field_impact(c: &mut Criterion) {
    let mut group = c.benchmark_group("growing_field_impact");

    // Test different growth rates
    let datasets = vec![
        ("grow_to_1MB", generate_segment_pressure_data(1000, 1024)),
        ("grow_to_5MB", generate_segment_pressure_data(1000, 5 * 1024)),
        ("grow_to_10MB", generate_segment_pressure_data(1000, 10 * 1024)),
    ];

    for (name, data) in datasets {
        group.bench_with_input(
            BenchmarkId::from_parameter(name),
            &data,
            |b, data| {
                b.iter(|| {
                    let input = Cursor::new(data.clone());
                    let output = Cursor::new(Vec::new());

                    let mut options = CompressOptions::default();
                    options.block_target_records = 10000;

                    let request = CompressRequest {
                        input: InputSource::NdjsonReader(Box::new(input)),
                        output: OutputSink::Writer(Box::new(output)),
                        options,
                        container_hint: Some(jac_format::ContainerFormat::Ndjson),
                        emit_index: false,
                    };

                    black_box(execute_compress(request).unwrap());
                });
            },
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_early_flush_impact,
    bench_block_size_vs_segment_limit,
    bench_growing_field_impact
);
criterion_main!(benches);
