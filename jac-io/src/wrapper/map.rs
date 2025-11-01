//! Keyed map object flattening wrapper
//!
//! This module implements the KeyedMap wrapper mode, which allows flattening
//! object-of-objects structures into a record stream. Each object key is injected
//! as a field in the corresponding record, preserving the original key-value mapping.

use super::error::WrapperError;
use super::utils::{navigate_pointer, parse_pointer};
use crate::{KeyCollisionMode, WrapperLimits};
use jac_format::Limits;
use serde_json::{Map, Value};
use std::io::Read;
use std::time::{Duration, Instant};

/// Metrics tracked during keyed map processing
#[derive(Debug, Clone)]
pub struct KeyedMapStreamMetrics {
    /// Peak buffer bytes used to hold the parsed document
    pub peak_buffer_bytes: usize,
    /// Records emitted (equals number of map entries)
    pub records_emitted: usize,
    /// Number of entries in the map
    pub map_entry_count: usize,
    /// Time spent processing
    pub processing_duration: Duration,
}

/// Iterator that streams records from a keyed map object
pub struct KeyedMapStream {
    /// The extracted records with injected keys
    records: Vec<Map<String, Value>>,
    /// Current record index
    current_idx: usize,
    /// Metrics
    metrics: KeyedMapStreamMetrics,
}

impl KeyedMapStream {
    /// Create a new KeyedMapStream from a reader
    ///
    /// This will:
    /// 1. Read and parse the entire input
    /// 2. Navigate to the target map object using the pointer
    /// 3. Extract map entries and inject keys into each record
    /// 4. Prepare for streaming
    pub fn new<R: Read>(
        reader: R,
        pointer: String,
        key_field: String,
        limits: WrapperLimits,
        collision_mode: KeyCollisionMode,
    ) -> Result<Self, WrapperError> {
        let start_time = Instant::now();

        // Validate limits
        limits.validate()?;

        // Read entire input into buffer (with size limit enforcement)
        let mut buffer = Vec::new();
        let mut limited_reader = reader.take(limits.max_buffer_bytes as u64 + 1);
        limited_reader
            .read_to_end(&mut buffer)
            .map_err(WrapperError::Io)?;

        // Check if we exceeded the buffer limit
        if buffer.len() > limits.max_buffer_bytes {
            return Err(WrapperError::BufferLimitExceeded {
                limit_bytes: limits.max_buffer_bytes,
                buffered_bytes: buffer.len(),
                pointer: pointer.clone(),
                suggested_size: WrapperError::suggest_buffer_size(buffer.len()),
                jq_expr: if pointer.is_empty() {
                    "jq 'to_entries | map(.value + {_key: .key})' input.json".to_string()
                } else {
                    format!(
                        "jq '{} | to_entries | map(.value + {{_key: .key}})' input.json",
                        WrapperError::pointer_to_jq(&pointer).replace(" | .[]", "")
                    )
                },
            });
        }

        let peak_buffer_bytes = buffer.len();

        // Parse as JSON
        let document: Value =
            serde_json::from_slice(&buffer).map_err(|e| WrapperError::JsonParse {
                context: "parsing document for keyed map".to_string(),
                source: e,
            })?;

        // Navigate to target map object
        let map_value = if pointer.is_empty() {
            &document
        } else {
            let tokens = parse_pointer(&pointer, limits.max_pointer_length, limits.max_depth)?;
            navigate_pointer(&document, &tokens, &pointer)?
        };

        // Ensure target is an object
        let map_object =
            map_value
                .as_object()
                .ok_or_else(|| WrapperError::PointerTargetWrongType {
                    pointer: pointer.clone(),
                    expected_type: "object".to_string(),
                    found_type: type_name(map_value).to_string(),
                })?;

        // Extract records from map
        let mut records = Vec::new();
        let max_key_length = Limits::default().max_string_len_per_value;

        for (key, value) in map_object {
            // Validate key length
            if key.len() > max_key_length {
                return Err(WrapperError::MapKeyTooLong {
                    key: key.clone(),
                    length: key.len(),
                    max_length: max_key_length,
                });
            }

            // Ensure value is an object
            let mut record = value
                .as_object()
                .ok_or_else(|| WrapperError::MapValueNotObject {
                    key: key.clone(),
                    found_type: type_name(value).to_string(),
                })?
                .clone();

            // Inject key field
            if record.contains_key(&key_field) {
                match collision_mode {
                    KeyCollisionMode::Error => {
                        return Err(WrapperError::KeyFieldCollision {
                            field: key_field.clone(),
                            map_key: key.clone(),
                        });
                    }
                    KeyCollisionMode::Overwrite => {
                        // Overwrite existing field
                        record.insert(key_field.clone(), Value::String(key.clone()));
                    }
                }
            } else {
                record.insert(key_field.clone(), Value::String(key.clone()));
            }

            records.push(record);
        }

        let entry_count = records.len();
        let processing_duration = start_time.elapsed();

        Ok(Self {
            records,
            current_idx: 0,
            metrics: KeyedMapStreamMetrics {
                peak_buffer_bytes,
                records_emitted: 0, // Will be updated as we iterate
                map_entry_count: entry_count,
                processing_duration,
            },
        })
    }

    /// Get processing metrics
    pub fn metrics(&self) -> &KeyedMapStreamMetrics {
        &self.metrics
    }

    /// Consume the stream and return final metrics
    pub fn into_metrics(mut self) -> KeyedMapStreamMetrics {
        self.metrics.records_emitted = self.current_idx;
        self.metrics
    }
}

impl Iterator for KeyedMapStream {
    type Item = Result<Map<String, Value>, WrapperError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.current_idx >= self.records.len() {
            return None;
        }

        let record = self.records[self.current_idx].clone();
        self.current_idx += 1;
        self.metrics.records_emitted = self.current_idx;

        Some(Ok(record))
    }
}

/// Get a human-readable type name
fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Cursor;

    fn default_limits() -> WrapperLimits {
        WrapperLimits::default()
    }

    #[test]
    fn basic_map_flattening() {
        let input = json!({
            "alice": {"age": 30, "role": "admin"},
            "bob": {"age": 25, "role": "user"}
        });

        let reader = Cursor::new(serde_json::to_vec(&input).unwrap());
        let stream = KeyedMapStream::new(
            reader,
            "".to_string(),
            "_key".to_string(),
            default_limits(),
            KeyCollisionMode::Error,
        )
        .unwrap();

        let records: Vec<_> = stream.collect::<Result<Vec<_>, _>>().unwrap();
        assert_eq!(records.len(), 2);

        // Verify keys are injected
        let keys: Vec<String> = records
            .iter()
            .map(|r| r.get("_key").unwrap().as_str().unwrap().to_string())
            .collect();
        assert!(keys.contains(&"alice".to_string()));
        assert!(keys.contains(&"bob".to_string()));

        // Verify original fields are preserved
        for record in &records {
            assert!(record.contains_key("age"));
            assert!(record.contains_key("role"));
        }
    }

    #[test]
    fn nested_pointer_map() {
        let input = json!({
            "data": {
                "users": {
                    "alice": {"score": 100},
                    "bob": {"score": 200}
                }
            }
        });

        let reader = Cursor::new(serde_json::to_vec(&input).unwrap());
        let stream = KeyedMapStream::new(
            reader,
            "/data/users".to_string(),
            "_key".to_string(),
            default_limits(),
            KeyCollisionMode::Error,
        )
        .unwrap();

        // Check metrics before consuming the stream
        let map_entry_count = stream.metrics().map_entry_count;
        let records: Vec<_> = stream.collect::<Result<Vec<_>, _>>().unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(map_entry_count, 2);
    }

    #[test]
    fn custom_key_field() {
        let input = json!({
            "alice": {"age": 30},
            "bob": {"age": 25}
        });

        let reader = Cursor::new(serde_json::to_vec(&input).unwrap());
        let stream = KeyedMapStream::new(
            reader,
            "".to_string(),
            "user_id".to_string(),
            default_limits(),
            KeyCollisionMode::Error,
        )
        .unwrap();

        let records: Vec<_> = stream.collect::<Result<Vec<_>, _>>().unwrap();
        for record in &records {
            assert!(record.contains_key("user_id"));
            assert!(!record.contains_key("_key"));
        }
    }

    #[test]
    fn key_collision_error_mode() {
        let input = json!({
            "alice": {"_key": "existing", "age": 30}
        });

        let reader = Cursor::new(serde_json::to_vec(&input).unwrap());
        let result = KeyedMapStream::new(
            reader,
            "".to_string(),
            "_key".to_string(),
            default_limits(),
            KeyCollisionMode::Error,
        );

        assert!(matches!(
            result,
            Err(WrapperError::KeyFieldCollision { .. })
        ));
    }

    #[test]
    fn key_collision_overwrite_mode() {
        let input = json!({
            "alice": {"_key": "old_value", "age": 30}
        });

        let reader = Cursor::new(serde_json::to_vec(&input).unwrap());
        let stream = KeyedMapStream::new(
            reader,
            "".to_string(),
            "_key".to_string(),
            default_limits(),
            KeyCollisionMode::Overwrite,
        )
        .unwrap();

        let records: Vec<_> = stream.collect::<Result<Vec<_>, _>>().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].get("_key").unwrap().as_str().unwrap(), "alice");
    }

    #[test]
    fn rejects_non_object_values() {
        let input = json!({
            "alice": {"age": 30},
            "bob": "not an object"
        });

        let reader = Cursor::new(serde_json::to_vec(&input).unwrap());
        let result = KeyedMapStream::new(
            reader,
            "".to_string(),
            "_key".to_string(),
            default_limits(),
            KeyCollisionMode::Error,
        );

        assert!(matches!(
            result,
            Err(WrapperError::MapValueNotObject { .. })
        ));
    }

    #[test]
    fn rejects_non_object_target() {
        let input = json!({
            "data": ["not", "an", "object"]
        });

        let reader = Cursor::new(serde_json::to_vec(&input).unwrap());
        let result = KeyedMapStream::new(
            reader,
            "/data".to_string(),
            "_key".to_string(),
            default_limits(),
            KeyCollisionMode::Error,
        );

        assert!(matches!(
            result,
            Err(WrapperError::PointerTargetWrongType { .. })
        ));
    }

    #[test]
    fn empty_map_produces_no_records() {
        let input = json!({});

        let reader = Cursor::new(serde_json::to_vec(&input).unwrap());
        let stream = KeyedMapStream::new(
            reader,
            "".to_string(),
            "_key".to_string(),
            default_limits(),
            KeyCollisionMode::Error,
        )
        .unwrap();

        // Check metrics before consuming the stream
        let map_entry_count = stream.metrics().map_entry_count;
        let records: Vec<_> = stream.collect::<Result<Vec<_>, _>>().unwrap();
        assert_eq!(records.len(), 0);
        assert_eq!(map_entry_count, 0);
    }

    #[test]
    fn enforces_buffer_limit() {
        let input = json!({
            "key": {"data": "x".repeat(100)}
        });

        let reader = Cursor::new(serde_json::to_vec(&input).unwrap());
        let mut limits = default_limits();
        limits.max_buffer_bytes = 50; // Very small limit

        let result = KeyedMapStream::new(
            reader,
            "".to_string(),
            "_key".to_string(),
            limits,
            KeyCollisionMode::Error,
        );

        assert!(matches!(
            result,
            Err(WrapperError::BufferLimitExceeded { .. })
        ));
    }

    #[test]
    fn validates_key_length() {
        // Create a key that exceeds max_string_len_per_value (16 MiB default)
        // but doesn't exceed wrapper buffer limit. The serialized JSON will be
        // larger than the key, so we use a smaller key to stay under buffer limits.
        let long_key = "a".repeat(17 * 1024 * 1024); // 17 MiB - exceeds default max_string_len
        let mut map = serde_json::Map::new();
        map.insert(long_key, json!({"data": "value"}));
        let input = Value::Object(map);

        // Use larger buffer limit to ensure we can read the JSON, but still check key length
        let mut limits = default_limits();
        limits.max_buffer_bytes = 128 * 1024 * 1024; // Allow large buffer for this test

        let reader = Cursor::new(serde_json::to_vec(&input).unwrap());
        let result = KeyedMapStream::new(
            reader,
            "".to_string(),
            "_key".to_string(),
            limits,
            KeyCollisionMode::Error,
        );

        assert!(matches!(result, Err(WrapperError::MapKeyTooLong { .. })));
    }

    #[test]
    fn metrics_track_correctly() {
        let input = json!({
            "alice": {"age": 30},
            "bob": {"age": 25},
            "carol": {"age": 35}
        });

        let reader = Cursor::new(serde_json::to_vec(&input).unwrap());
        let stream = KeyedMapStream::new(
            reader,
            "".to_string(),
            "_key".to_string(),
            default_limits(),
            KeyCollisionMode::Error,
        )
        .unwrap();

        let initial_metrics = stream.metrics().clone();
        assert_eq!(initial_metrics.map_entry_count, 3);
        assert_eq!(initial_metrics.records_emitted, 0);

        let _records: Vec<_> = stream.collect::<Result<Vec<_>, _>>().unwrap();
        // Note: We can't check final metrics here as stream is consumed
    }
}
