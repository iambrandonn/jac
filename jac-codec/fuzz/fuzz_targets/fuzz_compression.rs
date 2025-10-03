#![no_main]

use jac_codec::block_builder::BlockBuilder;
use jac_codec::column::ColumnBuilder;
use jac_format::{CompressOpts, Limits, TypeTag};
use libfuzzer_sys::fuzz_target;
use serde_json::Value;

fuzz_target!(|data: &[u8]| {
    // Only process if we have enough data
    if data.len() < 4 {
        return;
    }

    let opts = CompressOpts {
        limits: Limits::default(),
        compression_level: 1, // Fast compression for fuzzing
    };

    // Create a simple test record from the fuzz data
    let mut records = Vec::new();

    // Try to create a valid JSON record from the fuzz data
    if let Ok(json_str) = std::str::from_utf8(data) {
        if let Ok(record) = serde_json::from_str::<Value>(json_str) {
            records.push(record);
        }
    }

    // If JSON parsing failed, create a simple record with the raw data
    if records.is_empty() {
        let mut record = serde_json::Map::new();
        record.insert("data".to_string(), Value::String(
            data.iter().map(|b| format!("{:02x}", b)).collect::<Vec<_>>().join("")
        ));
        records.push(Value::Object(record));
    }

    // Try to build a block with the records
    let mut builder = BlockBuilder::new(&opts);

    for record in records {
        let _ = builder.add_record(&record);
    }

    let _ = builder.finalize();
});
