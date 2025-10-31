use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use jac_codec::{compress_block_segments, BlockBuilder, CompressOpts, TryAddRecordOutcome};
use serde_json::{json, Map, Value};

fn create_test_records(count: usize, cardinality: usize) -> Vec<Map<String, Value>> {
    let users: Vec<String> = (0..cardinality).map(|i| format!("user{}", i)).collect();

    (0..count)
        .map(|i| {
            serde_json::from_value(json!({
                "id": i,
                "user": users[i % cardinality],
                "timestamp": 1600000000 + i,
                "value": i * 2,
                "level": if i % 3 == 0 { "info" } else if i % 3 == 1 { "warn" } else { "error" }
            }))
            .unwrap()
        })
        .collect()
}

fn bench_block_building(c: &mut Criterion) {
    let mut group = c.benchmark_group("block_building");

    for cardinality in [10, 100, 1000] {
        for record_count in [1000, 10000] {
            let records = create_test_records(record_count, cardinality);

            group.bench_with_input(
                BenchmarkId::from_parameter(format!("{}rec_{}card", record_count, cardinality)),
                &records,
                |b, records| {
                    b.iter(|| {
                        let mut opts = CompressOpts::default();
                        opts.block_target_records = record_count + 100;
                        let mut builder = BlockBuilder::new(opts);

                        for record in records {
                            match builder.try_add_record(black_box(record.clone())).unwrap() {
                                TryAddRecordOutcome::Added => {}
                                TryAddRecordOutcome::BlockFull { .. } => {
                                    panic!("unexpected block full");
                                }
                            }
                        }

                        black_box(builder.finalize().unwrap());
                    });
                },
            );
        }
    }

    group.finish();
}

fn bench_dictionary_effectiveness(c: &mut Criterion) {
    let mut group = c.benchmark_group("dictionary_encoding");

    // Low cardinality - should compress well with dictionary
    let low_card_records = create_test_records(10000, 10);
    group.bench_function("low_cardinality", |b| {
        b.iter(|| {
            let mut opts = CompressOpts::default();
            opts.block_target_records = 10000;
            let mut builder = BlockBuilder::new(opts);

            for record in &low_card_records {
                builder.try_add_record(black_box(record.clone())).unwrap();
            }

            black_box(builder.finalize().unwrap());
        });
    });

    // High cardinality - dictionary less effective
    let high_card_records = create_test_records(10000, 5000);
    group.bench_function("high_cardinality", |b| {
        b.iter(|| {
            let mut opts = CompressOpts::default();
            opts.block_target_records = 10000;
            let mut builder = BlockBuilder::new(opts);

            for record in &high_card_records {
                builder.try_add_record(black_box(record.clone())).unwrap();
            }

            black_box(builder.finalize().unwrap());
        });
    });

    group.finish();
}

fn bench_finalize_vs_prepare(c: &mut Criterion) {
    let records = create_test_records(5_000, 250);

    let mut group = c.benchmark_group("block_finalize_vs_prepare");

    group.bench_function("finalize_sequential", |b| {
        b.iter(|| {
            let mut opts = CompressOpts::default();
            opts.block_target_records = 6_000;
            let mut builder = BlockBuilder::new(opts);

            for record in &records {
                match builder
                    .try_add_record(black_box(record.clone()))
                    .expect("add record")
                {
                    TryAddRecordOutcome::Added => {}
                    TryAddRecordOutcome::BlockFull { .. } => {
                        panic!("unexpected block fullness during benchmark");
                    }
                }
            }

            black_box(builder.finalize().expect("finalize block"));
        });
    });

    group.bench_function("prepare_then_compress", |b| {
        b.iter(|| {
            let mut opts = CompressOpts::default();
            opts.block_target_records = 6_000;
            let codec = opts.default_codec;
            let mut builder = BlockBuilder::new(opts);

            for record in &records {
                match builder
                    .try_add_record(black_box(record.clone()))
                    .expect("add record")
                {
                    TryAddRecordOutcome::Added => {}
                    TryAddRecordOutcome::BlockFull { .. } => {
                        panic!("unexpected block fullness during benchmark");
                    }
                }
            }

            let uncompressed = builder.prepare_segments().expect("prepare segments");
            black_box(compress_block_segments(uncompressed, codec).expect("compress segments"));
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_block_building,
    bench_dictionary_effectiveness,
    bench_finalize_vs_prepare
);
criterion_main!(benches);
