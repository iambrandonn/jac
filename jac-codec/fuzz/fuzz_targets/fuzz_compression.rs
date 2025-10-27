#![no_main]

use jac_codec::{block_builder::BlockBuilder, column::ColumnBuilder, Codec, TryAddRecordOutcome};
use jac_format::{CompressOpts, Limits, TypeTag};
use libfuzzer_sys::fuzz_target;
use serde_json::Value;

fuzz_target!(|data: &[u8]| {
    // Only process if we have enough data
    if data.len() < 4 {
        return;
    }

    let mut opts = CompressOpts::default();
    opts.block_target_records = 128;
    opts.default_codec = Codec::None;
    opts.limits = Limits::default();

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
    let mut builder = BlockBuilder::new(opts.clone());

    for value in records {
        if let Value::Object(map) = value {
            match builder.try_add_record(map) {
                Ok(TryAddRecordOutcome::Added) => {}
                Ok(TryAddRecordOutcome::BlockFull { record }) => {
                    let _ = builder.finalize();
                    builder = BlockBuilder::new(opts.clone());
                    let _ = builder.try_add_record(record);
                }
                Err(_) => {}
            }
        }
    }

    let _ = builder.finalize();
});
