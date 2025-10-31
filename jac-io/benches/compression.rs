use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use jac_io::{
    execute_compress, parallel::ParallelConfig, Codec, CompressOptions, CompressRequest,
    InputSource, OutputSink,
};
use serde_json::json;
use std::io::{Cursor, Write};
use std::sync::Arc;

/// Generate logs with low cardinality (good compression)
fn generate_low_cardinality_logs(count: usize) -> Vec<u8> {
    let users = ["alice", "bob", "carol", "dave", "eve"];
    let levels = ["info", "warn", "error", "debug"];
    let mut buf = Vec::new();

    for i in 0..count {
        let record = json!({
            "timestamp": 1600000000 + i,
            "user": users[i % users.len()],
            "level": levels[i % levels.len()],
            "message": format!("Log message {}", i % 100),
        });
        writeln!(&mut buf, "{}", record).unwrap();
    }

    buf
}

/// Generate events with high cardinality (poor compression)
fn generate_high_cardinality_events(count: usize) -> Vec<u8> {
    let mut buf = Vec::new();

    for i in 0..count {
        let record = json!({
            "event_id": format!("evt-{:08x}", i),
            "user_id": format!("user-{:08x}", i),
            "timestamp": 1600000000 + i,
            "data": format!("Event data for {}", i),
        });
        writeln!(&mut buf, "{}", record).unwrap();
    }

    buf
}

/// Generate records with nested objects
fn generate_nested_objects(count: usize) -> Vec<u8> {
    let mut buf = Vec::new();

    for i in 0..count {
        let record = json!({
            "id": i,
            "metadata": {
                "created": 1600000000 + i,
                "updated": 1600000000 + i + 3600,
                "tags": ["tag1", "tag2", "tag3"],
                "properties": {
                    "key1": format!("value{}", i % 10),
                    "key2": i * 2,
                    "key3": i % 2 == 0,
                }
            }
        });
        writeln!(&mut buf, "{}", record).unwrap();
    }

    buf
}

fn bench_compression_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("compression_throughput");

    let datasets = vec![
        ("low_card_10k", generate_low_cardinality_logs(10_000)),
        ("high_card_10k", generate_high_cardinality_events(10_000)),
        ("nested_10k", generate_nested_objects(10_000)),
    ];

    for (name, data) in datasets {
        let data_len = data.len();
        group.throughput(Throughput::Bytes(data_len as u64));

        group.bench_with_input(BenchmarkId::from_parameter(name), &data, |b, data| {
            b.iter(|| {
                let input = Cursor::new(data.clone());
                let output = Cursor::new(Vec::new());

                let request = CompressRequest {
                    input: InputSource::NdjsonReader(Box::new(input)),
                    output: OutputSink::Writer(Box::new(output)),
                    options: CompressOptions::default(),
                    container_hint: Some(jac_format::ContainerFormat::Ndjson),
                    emit_index: false,
                };

                black_box(execute_compress(request).unwrap());
            });
        });
    }

    group.finish();
}

fn bench_block_size_impact(c: &mut Criterion) {
    let mut group = c.benchmark_group("block_size_impact");

    let data = generate_low_cardinality_logs(50_000);
    let block_sizes = vec![10_000, 50_000, 100_000];

    for block_size in block_sizes {
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{}records", block_size)),
            &data,
            |b, data| {
                b.iter(|| {
                    let input = Cursor::new(data.clone());
                    let output = Cursor::new(Vec::new());

                    let mut options = CompressOptions::default();
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

fn bench_zstd_levels(c: &mut Criterion) {
    let mut group = c.benchmark_group("zstd_levels");

    let data = generate_low_cardinality_logs(10_000);
    let zstd_levels = vec![1, 3, 6, 9];

    for level in zstd_levels {
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("level{}", level)),
            &data,
            |b, data| {
                b.iter(|| {
                    let input = Cursor::new(data.clone());
                    let output = Cursor::new(Vec::new());

                    let mut options = CompressOptions::default();
                    options.default_codec = Codec::Zstd(level);

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

fn bench_parallel_speedup(c: &mut Criterion) {
    benchmark_parallel_config(c, "parallel_speedup_default_zstd", Codec::Zstd(6));
    benchmark_parallel_config(
        c,
        "parallel_speedup_zstd_single_thread",
        Codec::ZstdWithThreads {
            level: 6,
            threads: 1,
        },
    );
}

fn benchmark_parallel_config(c: &mut Criterion, group_name: &str, codec: Codec) {
    let data = Arc::new(generate_low_cardinality_logs(80_000));
    let mut group = c.benchmark_group(group_name);
    let thread_counts = [1usize, 2, 4, 8];

    for &threads in &thread_counts {
        let dataset = Arc::clone(&data);

        group.bench_with_input(
            BenchmarkId::from_parameter(threads),
            &threads,
            move |b, &thread_count| {
                let codec = codec;
                b.iter(|| {
                    let input = Cursor::new((*dataset).clone());
                    let output = Cursor::new(Vec::new());

                    let mut options = CompressOptions::default();
                    options.block_target_records = 20_000;
                    options.default_codec = codec;
                    options.parallel_config = ParallelConfig {
                        memory_reservation_factor: 1.0,
                        max_threads: Some(thread_count),
                    };
                    options.limits.max_block_uncompressed_total = 32 * 1024 * 1024;

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
    bench_compression_throughput,
    bench_block_size_impact,
    bench_zstd_levels,
    bench_parallel_speedup
);
criterion_main!(benches);
