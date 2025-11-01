//! JSON Pointer-based array extraction

use super::error::WrapperError;
use super::utils::{navigate_pointer, parse_pointer};
use serde_json::{Map, Value};
use std::io::{BufReader, Read};
use std::time::Instant;

/// Configuration limits for pointer-based wrappers
#[derive(Debug, Clone)]
pub struct PointerLimits {
    /// Maximum pointer depth (number of levels)
    pub max_depth: usize,
    /// Maximum bytes to buffer before reaching target
    pub max_buffer_bytes: usize,
    /// Maximum pointer string length
    pub max_pointer_length: usize,
}

impl Default for PointerLimits {
    fn default() -> Self {
        Self {
            max_depth: 3,
            max_buffer_bytes: 16 * 1024 * 1024, // 16 MiB
            max_pointer_length: 256,
        }
    }
}

impl PointerLimits {
    /// Hard maximum limits that cannot be exceeded
    pub fn hard_maximums() -> Self {
        Self {
            max_depth: 10,
            max_buffer_bytes: 128 * 1024 * 1024, // 128 MiB
            max_pointer_length: 2048,
        }
    }

    /// Validate limits against hard maximums
    pub fn validate(&self) -> Result<(), WrapperError> {
        let hard = Self::hard_maximums();

        if self.max_depth > hard.max_depth {
            return Err(WrapperError::ConfigurationExceedsHardLimits {
                reason: format!(
                    "max_depth {} exceeds hard limit {}",
                    self.max_depth, hard.max_depth
                ),
                max_depth: hard.max_depth,
                max_buffer: hard.max_buffer_bytes,
                max_ptr_len: hard.max_pointer_length,
            });
        }

        if self.max_buffer_bytes > hard.max_buffer_bytes {
            return Err(WrapperError::ConfigurationExceedsHardLimits {
                reason: format!(
                    "max_buffer_bytes {} exceeds hard limit {}",
                    self.max_buffer_bytes, hard.max_buffer_bytes
                ),
                max_depth: hard.max_depth,
                max_buffer: hard.max_buffer_bytes,
                max_ptr_len: hard.max_pointer_length,
            });
        }

        if self.max_pointer_length > hard.max_pointer_length {
            return Err(WrapperError::ConfigurationExceedsHardLimits {
                reason: format!(
                    "max_pointer_length {} exceeds hard limit {}",
                    self.max_pointer_length, hard.max_pointer_length
                ),
                max_depth: hard.max_depth,
                max_buffer: hard.max_buffer_bytes,
                max_ptr_len: hard.max_pointer_length,
            });
        }

        Ok(())
    }
}

/// Metrics collected during pointer stream processing
#[derive(Debug, Clone)]
pub struct PointerMetrics {
    /// Pointer path used
    pub pointer: String,
    /// Peak bytes buffered
    pub peak_buffer_bytes: usize,
    /// Number of records emitted
    pub records_emitted: usize,
    /// Time spent traversing and extracting
    pub processing_duration: std::time::Duration,
}

/// Iterator that navigates to a JSON Pointer target and streams records
///
/// Supports extracting arrays (streams elements) or objects (emits single record).
/// Enforces depth and buffer limits to prevent resource exhaustion.
pub struct PointerArrayStream {
    inner: Box<dyn Iterator<Item = Result<Map<String, Value>, WrapperError>> + Send>,
    metrics: PointerMetrics,
}

impl PointerArrayStream {
    /// Create a new pointer stream from a reader
    ///
    /// # Arguments
    /// * `reader` - Input JSON source
    /// * `pointer` - RFC 6901 JSON Pointer string
    /// * `limits` - Configuration limits
    ///
    /// # Returns
    /// A new stream or an error if configuration is invalid
    pub fn new<R: Read + Send + 'static>(
        reader: R,
        pointer: String,
        limits: PointerLimits,
    ) -> Result<Self, WrapperError> {
        limits.validate()?;

        let start_time = Instant::now();
        let buf_reader = BufReader::new(reader);

        // Parse pointer and validate
        let tokens = parse_pointer(&pointer, limits.max_pointer_length, limits.max_depth)?;

        // Parse the entire input to navigate (bounded by buffer limit)
        let mut limited_reader = LimitedReader::new(buf_reader, limits.max_buffer_bytes);
        let document: Value = serde_json::from_reader(&mut limited_reader).map_err(|e| {
            if limited_reader.limit_exceeded() {
                let buffered = limited_reader.bytes_read();
                WrapperError::BufferLimitExceeded {
                    limit_bytes: limits.max_buffer_bytes,
                    buffered_bytes: buffered,
                    pointer: pointer.clone(),
                    suggested_size: WrapperError::suggest_buffer_size(buffered),
                    jq_expr: WrapperError::pointer_to_jq(&pointer),
                }
            } else {
                WrapperError::JsonParse {
                    context: "parsing input document for pointer navigation".to_string(),
                    source: e,
                }
            }
        })?;

        let peak_buffer_bytes = limited_reader.bytes_read();

        // Navigate to target
        let target = if tokens.is_empty() {
            &document
        } else {
            navigate_pointer(&document, &tokens, &pointer)?
        };

        // Create iterator based on target type
        let (iter, count): (
            Box<dyn Iterator<Item = Result<Map<String, Value>, WrapperError>> + Send>,
            usize,
        ) = match target {
            Value::Array(arr) => {
                let count = arr.len();
                let items = arr.clone();
                let pointer_clone = pointer.clone();

                let iter = items.into_iter().map(move |v| match v {
                    Value::Object(map) => Ok(map),
                    other => Err(WrapperError::PointerTargetWrongType {
                        pointer: pointer_clone.clone(),
                        expected_type: "array of objects".to_string(),
                        found_type: format!("array containing {}", type_name(&other)),
                    }),
                });
                (Box::new(iter), count)
            }
            Value::Object(obj) => {
                let map = obj.clone();
                let iter = std::iter::once(Ok(map));
                (Box::new(iter), 1)
            }
            Value::Null => {
                return Err(WrapperError::PointerTargetWrongType {
                    pointer,
                    expected_type: "array or object".to_string(),
                    found_type: "null".to_string(),
                });
            }
            other => {
                return Err(WrapperError::PointerTargetWrongType {
                    pointer,
                    expected_type: "array or object".to_string(),
                    found_type: type_name(other).to_string(),
                });
            }
        };

        let processing_duration = start_time.elapsed();

        Ok(Self {
            inner: iter,
            metrics: PointerMetrics {
                pointer,
                peak_buffer_bytes,
                records_emitted: count,
                processing_duration,
            },
        })
    }

    /// Get processing metrics
    pub fn metrics(&self) -> &PointerMetrics {
        &self.metrics
    }

    /// Consume the stream and return final metrics
    pub fn into_metrics(self) -> PointerMetrics {
        self.metrics
    }
}

impl Iterator for PointerArrayStream {
    type Item = Result<Map<String, Value>, WrapperError>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
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

/// Reader wrapper that enforces a byte limit
struct LimitedReader<R> {
    inner: R,
    bytes_read: usize,
    limit: usize,
    exceeded: bool,
}

impl<R> LimitedReader<R> {
    fn new(inner: R, limit: usize) -> Self {
        Self {
            inner,
            bytes_read: 0,
            limit,
            exceeded: false,
        }
    }

    fn bytes_read(&self) -> usize {
        self.bytes_read
    }

    fn limit_exceeded(&self) -> bool {
        self.exceeded
    }
}

impl<R: Read> Read for LimitedReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.exceeded {
            return Ok(0);
        }

        let remaining = self.limit.saturating_sub(self.bytes_read);
        if remaining == 0 {
            self.exceeded = true;
            return Ok(0);
        }

        let max_read = buf.len().min(remaining);
        let n = self.inner.read(&mut buf[..max_read])?;
        self.bytes_read += n;

        if self.bytes_read >= self.limit {
            self.exceeded = true;
        }

        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Cursor;

    #[test]
    fn pointer_limits_default_within_hard_max() {
        let limits = PointerLimits::default();
        assert!(limits.validate().is_ok());
    }

    #[test]
    fn pointer_limits_rejects_excessive_depth() {
        let mut limits = PointerLimits::default();
        limits.max_depth = 20;
        assert!(limits.validate().is_err());
    }

    #[test]
    fn pointer_limits_rejects_excessive_buffer() {
        let mut limits = PointerLimits::default();
        limits.max_buffer_bytes = 256 * 1024 * 1024; // 256 MiB
        assert!(limits.validate().is_err());
    }

    #[test]
    fn pointer_stream_extracts_nested_array() {
        let doc = json!({
            "api": {
                "data": [
                    {"id": 1, "name": "alice"},
                    {"id": 2, "name": "bob"}
                ]
            }
        });

        let input = serde_json::to_vec(&doc).unwrap();
        let reader = Cursor::new(input);

        let mut stream =
            PointerArrayStream::new(reader, "/api/data".to_string(), PointerLimits::default())
                .unwrap();

        let first = stream.next().unwrap().unwrap();
        assert_eq!(first.get("id").unwrap(), &json!(1));

        let second = stream.next().unwrap().unwrap();
        assert_eq!(second.get("id").unwrap(), &json!(2));

        assert!(stream.next().is_none());

        let metrics = stream.metrics();
        assert_eq!(metrics.pointer, "/api/data");
        assert_eq!(metrics.records_emitted, 2);
    }

    #[test]
    fn pointer_stream_emits_single_object() {
        let doc = json!({"name": "test", "value": 42});
        let input = serde_json::to_vec(&doc).unwrap();
        let reader = Cursor::new(input);

        let mut stream = PointerArrayStream::new(
            reader,
            "".to_string(), // root
            PointerLimits::default(),
        )
        .unwrap();

        let record = stream.next().unwrap().unwrap();
        assert_eq!(record.get("name").unwrap(), &json!("test"));
        assert!(stream.next().is_none());
    }

    #[test]
    fn pointer_stream_errors_on_missing_pointer() {
        let doc = json!({"data": []});
        let input = serde_json::to_vec(&doc).unwrap();
        let reader = Cursor::new(input);

        let result = PointerArrayStream::new(
            reader,
            "/missing/path".to_string(),
            PointerLimits::default(),
        );

        assert!(matches!(result, Err(WrapperError::PointerNotFound { .. })));
    }

    #[test]
    fn pointer_stream_errors_on_scalar_target() {
        let doc = json!({"value": 123});
        let input = serde_json::to_vec(&doc).unwrap();
        let reader = Cursor::new(input);

        let result =
            PointerArrayStream::new(reader, "/value".to_string(), PointerLimits::default());

        assert!(matches!(
            result,
            Err(WrapperError::PointerTargetWrongType { .. })
        ));
    }

    #[test]
    fn pointer_stream_errors_on_null_target() {
        let doc = json!({"data": null});
        let input = serde_json::to_vec(&doc).unwrap();
        let reader = Cursor::new(input);

        let result = PointerArrayStream::new(reader, "/data".to_string(), PointerLimits::default());

        assert!(matches!(
            result,
            Err(WrapperError::PointerTargetWrongType { .. })
        ));
    }

    #[test]
    fn pointer_stream_enforces_buffer_limit() {
        let large_doc = json!({
            "prefix": "x".repeat(1000),
            "data": [{"id": 1}]
        });
        let input = serde_json::to_vec(&large_doc).unwrap();
        let reader = Cursor::new(input);

        let mut limits = PointerLimits::default();
        limits.max_buffer_bytes = 100; // Very small limit

        let result = PointerArrayStream::new(reader, "/data".to_string(), limits);

        assert!(matches!(
            result,
            Err(WrapperError::BufferLimitExceeded { .. })
        ));
    }

    #[test]
    fn pointer_stream_errors_on_non_object_array_elements() {
        let doc = json!({"data": [1, 2, 3]});
        let input = serde_json::to_vec(&doc).unwrap();
        let reader = Cursor::new(input);

        let mut stream =
            PointerArrayStream::new(reader, "/data".to_string(), PointerLimits::default()).unwrap();

        let result = stream.next().unwrap();
        assert!(matches!(
            result,
            Err(WrapperError::PointerTargetWrongType { .. })
        ));
    }

    #[test]
    fn pointer_stream_handles_empty_array() {
        let doc = json!({"data": []});
        let input = serde_json::to_vec(&doc).unwrap();
        let reader = Cursor::new(input);

        let mut stream =
            PointerArrayStream::new(reader, "/data".to_string(), PointerLimits::default()).unwrap();

        assert!(stream.next().is_none());
        assert_eq!(stream.metrics().records_emitted, 0);
    }

    #[test]
    fn pointer_stream_tracks_peak_buffer() {
        let doc = json!({"data": [{"test": "value"}]});
        let input = serde_json::to_vec(&doc).unwrap();
        let expected_size = input.len();
        let reader = Cursor::new(input);

        let stream =
            PointerArrayStream::new(reader, "/data".to_string(), PointerLimits::default()).unwrap();

        assert_eq!(stream.metrics().peak_buffer_bytes, expected_size);
    }
}
