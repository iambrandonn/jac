//! Array-with-headers wrapper for CSV-like JSON data
//!
//! This module provides support for JSON arrays where the first element is a header
//! row (array of strings) and subsequent elements are data rows (arrays of values).
//!
//! # Format
//!
//! ```json
//! [
//!   ["id", "name", "age"],     // Header row
//!   [1, "Alice", 30],           // Data row
//!   [2, "Bob", 25]              // Data row
//! ]
//! ```
//!
//! Each data row is converted to a JSON object using the header as field names.

use super::error::WrapperError;
use crate::WrapperLimits;
use serde_json::{Map, Value};
use std::io::Read;
use std::time::Instant;

/// Metrics for array-with-headers processing
#[derive(Debug, Clone)]
pub struct ArrayHeadersMetrics {
    /// Peak buffer bytes used
    pub peak_buffer_bytes: usize,
    /// Number of records emitted
    pub records_emitted: usize,
    /// Processing duration
    pub processing_duration: std::time::Duration,
    /// Number of header fields
    pub header_field_count: usize,
}

/// Stream that converts array-with-headers format to records
pub struct ArrayHeadersStream {
    records: Vec<Map<String, Value>>,
    index: usize,
    metrics: ArrayHeadersMetrics,
}

impl ArrayHeadersStream {
    /// Create a new ArrayHeadersStream
    ///
    /// # Arguments
    ///
    /// * `reader` - JSON input containing an array with header row
    /// * `limits` - Wrapper limits to enforce
    ///
    /// # Format
    ///
    /// The input must be a JSON array where:
    /// - First element is an array of strings (header row)
    /// - Subsequent elements are arrays of values (data rows)
    /// - All rows must have the same length as the header
    ///
    /// # Example
    ///
    /// ```json
    /// [
    ///   ["id", "name", "active"],
    ///   [1, "Alice", true],
    ///   [2, "Bob", false]
    /// ]
    /// ```
    ///
    /// Produces:
    /// ```json
    /// {"id": 1, "name": "Alice", "active": true}
    /// {"id": 2, "name": "Bob", "active": false}
    /// ```
    pub fn new(
        mut reader: Box<dyn Read + Send>,
        limits: WrapperLimits,
    ) -> Result<Self, WrapperError> {
        let start = Instant::now();

        // Read entire input (must fit in buffer)
        let mut buffer = Vec::new();
        reader.read_to_end(&mut buffer)?;

        if buffer.len() > limits.max_buffer_bytes {
            return Err(WrapperError::BufferLimitExceeded {
                limit_bytes: limits.max_buffer_bytes,
                buffered_bytes: buffer.len(),
                pointer: "(array-with-headers)".to_string(),
                suggested_size: WrapperError::suggest_buffer_size(buffer.len()),
                jq_expr: "jq '[.[0] as $h | .[1:][] | [. as $row | $h | to_entries | map(.value = $row[.key]) | from_entries]] | .[]' input.json".to_string(),
            });
        }

        let peak_buffer_bytes = buffer.len();

        // Parse JSON array
        let arr: Vec<Value> = serde_json::from_slice(&buffer).map_err(|e| {
            WrapperError::JsonParse {
                context: "Failed to parse array-with-headers input".to_string(),
                source: e,
            }
        })?;

        if arr.is_empty() {
            return Err(WrapperError::InvalidHeaderRow {
                reason: "Array is empty, expected at least a header row".to_string(),
            });
        }

        // Extract and validate header
        let header = arr[0].as_array().ok_or_else(|| WrapperError::InvalidHeaderRow {
            reason: "First element is not an array".to_string(),
        })?;

        let header_strings: Result<Vec<String>, _> = header
            .iter()
            .enumerate()
            .map(|(i, v)| {
                v.as_str().ok_or_else(|| WrapperError::InvalidHeaderRow {
                    reason: format!("Header element {} is not a string: {:?}", i, v),
                })
                .map(|s| s.to_string())
            })
            .collect();

        let header_strings = header_strings?;
        let header_field_count = header_strings.len();

        if header_field_count == 0 {
            return Err(WrapperError::InvalidHeaderRow {
                reason: "Header row is empty".to_string(),
            });
        }

        // Convert data rows to records
        let mut records = Vec::with_capacity(arr.len() - 1);

        for (row_index, row_value) in arr.iter().skip(1).enumerate() {
            let row = row_value.as_array().ok_or_else(|| WrapperError::InvalidHeaderRow {
                reason: format!("Row {} is not an array: {:?}", row_index + 1, row_value),
            })?;

            if row.len() != header_field_count {
                return Err(WrapperError::ArrayRowLengthMismatch {
                    row_index: row_index + 1,
                    actual: row.len(),
                    expected: header_field_count,
                });
            }

            let mut record = Map::new();
            for (field_name, field_value) in header_strings.iter().zip(row.iter()) {
                record.insert(field_name.clone(), field_value.clone());
            }

            records.push(record);
        }

        let records_emitted = records.len();
        let processing_duration = start.elapsed();

        Ok(Self {
            records,
            index: 0,
            metrics: ArrayHeadersMetrics {
                peak_buffer_bytes,
                records_emitted,
                processing_duration,
                header_field_count,
            },
        })
    }

    /// Get metrics for this stream
    pub fn metrics(&self) -> &ArrayHeadersMetrics {
        &self.metrics
    }
}

impl Iterator for ArrayHeadersStream {
    type Item = Result<Map<String, Value>, WrapperError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index < self.records.len() {
            let record = self.records[self.index].clone();
            self.index += 1;
            Some(Ok(record))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn test_limits() -> WrapperLimits {
        WrapperLimits::default()
    }

    #[test]
    fn test_basic_array_with_headers() {
        let input = r#"[
            ["id", "name", "age"],
            [1, "Alice", 30],
            [2, "Bob", 25]
        ]"#;

        let reader = Box::new(Cursor::new(input));
        let stream = ArrayHeadersStream::new(reader, test_limits()).unwrap();

        let records: Result<Vec<_>, _> = stream.collect();
        let records = records.unwrap();
        assert_eq!(records.len(), 2);

        assert_eq!(records[0].get("id").and_then(|v| v.as_i64()), Some(1));
        assert_eq!(
            records[0].get("name").and_then(|v| v.as_str()),
            Some("Alice")
        );
        assert_eq!(records[0].get("age").and_then(|v| v.as_i64()), Some(30));

        assert_eq!(records[1].get("id").and_then(|v| v.as_i64()), Some(2));
        assert_eq!(
            records[1].get("name").and_then(|v| v.as_str()),
            Some("Bob")
        );
        assert_eq!(records[1].get("age").and_then(|v| v.as_i64()), Some(25));
    }

    #[test]
    fn test_mixed_types() {
        let input = r#"[
            ["id", "name", "active", "score"],
            [1, "Alice", true, 95.5],
            [2, "Bob", false, null]
        ]"#;

        let reader = Box::new(Cursor::new(input));
        let stream = ArrayHeadersStream::new(reader, test_limits()).unwrap();

        let records: Result<Vec<_>, _> = stream.collect();
        let records = records.unwrap();
        assert_eq!(records.len(), 2);

        assert_eq!(
            records[0].get("active").and_then(|v| v.as_bool()),
            Some(true)
        );
        assert_eq!(
            records[0].get("score").and_then(|v| v.as_f64()),
            Some(95.5)
        );
        assert!(records[1].get("score").unwrap().is_null());
    }

    #[test]
    fn test_empty_array_error() {
        let input = "[]";
        let reader = Box::new(Cursor::new(input));
        let result = ArrayHeadersStream::new(reader, test_limits());
        assert!(matches!(result, Err(WrapperError::InvalidHeaderRow { .. })));
    }

    #[test]
    fn test_non_array_header_error() {
        let input = r#"[
            "not an array",
            [1, 2, 3]
        ]"#;
        let reader = Box::new(Cursor::new(input));
        let result = ArrayHeadersStream::new(reader, test_limits());
        assert!(matches!(result, Err(WrapperError::InvalidHeaderRow { .. })));
    }

    #[test]
    fn test_non_string_header_element_error() {
        let input = r#"[
            ["id", 123, "name"],
            [1, 2, 3]
        ]"#;
        let reader = Box::new(Cursor::new(input));
        let result = ArrayHeadersStream::new(reader, test_limits());
        assert!(matches!(result, Err(WrapperError::InvalidHeaderRow { .. })));
    }

    #[test]
    fn test_row_length_mismatch_error() {
        let input = r#"[
            ["id", "name", "age"],
            [1, "Alice", 30],
            [2, "Bob"]
        ]"#;
        let reader = Box::new(Cursor::new(input));
        let result = ArrayHeadersStream::new(reader, test_limits());
        assert!(matches!(
            result,
            Err(WrapperError::ArrayRowLengthMismatch { .. })
        ));
    }

    #[test]
    fn test_metrics() {
        let input = r#"[
            ["id", "name"],
            [1, "Alice"],
            [2, "Bob"],
            [3, "Carol"]
        ]"#;

        let reader = Box::new(Cursor::new(input));
        let stream = ArrayHeadersStream::new(reader, test_limits()).unwrap();

        let metrics = stream.metrics();
        assert_eq!(metrics.records_emitted, 3);
        assert_eq!(metrics.header_field_count, 2);
        assert!(metrics.peak_buffer_bytes > 0);
        assert!(metrics.processing_duration.as_nanos() > 0);
    }

    #[test]
    fn test_buffer_limit() {
        let input = r#"[
            ["id", "name"],
            [1, "Alice"]
        ]"#;

        let reader = Box::new(Cursor::new(input));
        let limits = WrapperLimits {
            max_buffer_bytes: 10, // Very small limit
            ..Default::default()
        };

        let result = ArrayHeadersStream::new(reader, limits);
        assert!(matches!(
            result,
            Err(WrapperError::BufferLimitExceeded { .. })
        ));
    }

    #[test]
    fn test_single_row() {
        let input = r#"[
            ["id", "name"],
            [1, "Alice"]
        ]"#;

        let reader = Box::new(Cursor::new(input));
        let stream = ArrayHeadersStream::new(reader, test_limits()).unwrap();

        let records: Result<Vec<_>, _> = stream.collect();
        let records = records.unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].get("id").and_then(|v| v.as_i64()), Some(1));
    }

    #[test]
    fn test_empty_header_error() {
        let input = r#"[
            [],
            [1, 2, 3]
        ]"#;
        let reader = Box::new(Cursor::new(input));
        let result = ArrayHeadersStream::new(reader, test_limits());
        assert!(matches!(result, Err(WrapperError::InvalidHeaderRow { .. })));
    }
}
