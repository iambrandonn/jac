use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use jac_io::{
    execute_compress, execute_decompress, execute_project, CompressOptions, CompressRequest,
    DecompressFormat, DecompressOptions, DecompressRequest, InputSource, JacInput, OutputSink,
    ProjectFormat, ProjectRequest,
};
use serde_json::json;
use std::io::{Cursor, Write};

fn generate_test_data(count: usize) -> Vec<u8> {
    let mut buf = Vec::new();
    let users = ["alice", "bob", "carol", "dave", "eve"];

    for i in 0..count {
        let record = json!({
            "id": i,
            "user": users[i % users.len()],
            "timestamp": 1600000000 + i,
            "value": i * 2,
            "level": if i % 2 == 0 { "info" } else { "warn" },
            "message": format!("Message {}", i % 100),
        });
        writeln!(&mut buf, "{}", record).unwrap();
    }

    buf
}

fn compress_data(data: &[u8]) -> Vec<u8> {
    let input = Cursor::new(data.to_vec());

    // Use a temporary file to capture compressed output
    let temp_dir = std::env::temp_dir();
    let temp_path = temp_dir.join(format!("jac_bench_{}.jac", std::process::id()));

    let request = CompressRequest {
        input: InputSource::NdjsonReader(Box::new(input)),
        output: OutputSink::Path(temp_path.clone()),
        options: CompressOptions::default(),
        container_hint: Some(jac_format::ContainerFormat::Ndjson),
        emit_index: false,
    };

    execute_compress(request).unwrap();

    let compressed = std::fs::read(&temp_path).unwrap();
    let _ = std::fs::remove_file(&temp_path); // Clean up
    compressed
}

fn bench_full_decompression(c: &mut Criterion) {
    let mut group = c.benchmark_group("full_decompression");

    let datasets = vec![
        ("10k_records", generate_test_data(10_000)),
        ("50k_records", generate_test_data(50_000)),
    ];

    for (name, data) in datasets {
        let compressed = compress_data(&data);
        let compressed_len = compressed.len();

        group.throughput(Throughput::Bytes(compressed_len as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(name),
            &compressed,
            |b, compressed| {
                b.iter(|| {
                    let input = Cursor::new(compressed.clone());
                    let output = Cursor::new(Vec::new());

                    let request = DecompressRequest {
                        input: JacInput::Reader(Box::new(input)),
                        output: OutputSink::Writer(Box::new(output)),
                        format: DecompressFormat::Ndjson,
                        options: DecompressOptions::default(),
                    };

                    black_box(execute_decompress(request).unwrap());
                });
            },
        );
    }

    group.finish();
}

fn bench_field_projection(c: &mut Criterion) {
    let mut group = c.benchmark_group("field_projection");

    let data = generate_test_data(50_000);
    let compressed = compress_data(&data);

    let projection_scenarios = vec![
        ("single_field", vec!["user"]),
        ("two_fields", vec!["user", "timestamp"]),
        ("four_fields", vec!["user", "timestamp", "value", "level"]),
    ];

    for (name, fields) in projection_scenarios {
        group.bench_with_input(BenchmarkId::from_parameter(name), &fields, |b, fields| {
            b.iter(|| {
                let input = Cursor::new(compressed.clone());
                let output = Cursor::new(Vec::new());

                let request = ProjectRequest {
                    input: JacInput::Reader(Box::new(input)),
                    output: OutputSink::Writer(Box::new(output)),
                    fields: fields.iter().map(|s| s.to_string()).collect(),
                    format: ProjectFormat::Ndjson,
                    options: DecompressOptions::default(),
                };

                black_box(execute_project(request).unwrap());
            });
        });
    }

    group.finish();
}

fn bench_projection_speedup(c: &mut Criterion) {
    let mut group = c.benchmark_group("projection_speedup");

    let data = generate_test_data(50_000);
    let compressed = compress_data(&data);
    let compressed_len = compressed.len();

    group.throughput(Throughput::Bytes(compressed_len as u64));

    // Full decompression baseline
    group.bench_function("full_decompress", |b| {
        b.iter(|| {
            let input = Cursor::new(compressed.clone());
            let output = Cursor::new(Vec::new());

            let request = DecompressRequest {
                input: JacInput::Reader(Box::new(input)),
                output: OutputSink::Writer(Box::new(output)),
                format: DecompressFormat::Ndjson,
                options: DecompressOptions::default(),
            };

            black_box(execute_decompress(request).unwrap());
        });
    });

    // Single field projection
    group.bench_function("project_one_field", |b| {
        b.iter(|| {
            let input = Cursor::new(compressed.clone());
            let output = Cursor::new(Vec::new());

            let request = ProjectRequest {
                input: JacInput::Reader(Box::new(input)),
                output: OutputSink::Writer(Box::new(output)),
                fields: vec!["user".to_string()],
                format: ProjectFormat::Ndjson,
                options: DecompressOptions::default(),
            };

            black_box(execute_project(request).unwrap());
        });
    });

    group.finish();
}

fn bench_block_scanning(c: &mut Criterion) {
    let mut group = c.benchmark_group("block_scanning");

    // Create data with multiple blocks
    let mut large_data = Vec::new();
    for _ in 0..5 {
        large_data.extend(generate_test_data(20_000));
    }

    let compressed = compress_data(&large_data);

    group.bench_function("scan_all_blocks", |b| {
        b.iter(|| {
            let input = Cursor::new(compressed.clone());
            let output = Cursor::new(Vec::new());

            let request = ProjectRequest {
                input: JacInput::Reader(Box::new(input)),
                output: OutputSink::Writer(Box::new(output)),
                fields: vec!["user".to_string()],
                format: ProjectFormat::Ndjson,
                options: DecompressOptions::default(),
            };

            black_box(execute_project(request).unwrap());
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_full_decompression,
    bench_field_projection,
    bench_projection_speedup,
    bench_block_scanning
);
criterion_main!(benches);
