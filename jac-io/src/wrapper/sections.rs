//! Multi-section array concatenation wrapper
//!
//! This module implements the Sections wrapper mode, which allows concatenating
//! multiple named arrays from a single top-level JSON object into a unified
//! record stream. Each section can optionally be labeled to preserve provenance.

use super::error::WrapperError;
use super::utils::{navigate_pointer, parse_pointer};
use crate::{MissingSectionBehavior, SectionSpec, WrapperLimits};
use serde_json::{Map, Value};
use std::io::Read;
use std::time::{Duration, Instant};

/// Metrics tracked during sections processing
#[derive(Debug, Clone)]
pub struct SectionsStreamMetrics {
    /// Peak buffer bytes used to hold the parsed document
    pub peak_buffer_bytes: usize,
    /// Records emitted across all sections
    pub records_emitted: usize,
    /// Per-section record counts (section name, count)
    pub section_counts: Vec<(String, usize)>,
    /// Time spent processing
    pub processing_duration: Duration,
}

/// Iterator that streams records from multiple sections of a JSON document
pub struct SectionsStream {
    /// The parsed sections with their records
    sections: Vec<(String, Vec<Map<String, Value>>)>,
    /// Current section index
    current_section_idx: usize,
    /// Current record index within the section
    current_record_idx: usize,
    /// Metrics
    metrics: SectionsStreamMetrics,
}

impl SectionsStream {
    /// Create a new SectionsStream from a reader
    ///
    /// This will:
    /// 1. Read and parse the entire input as a JSON object
    /// 2. Navigate to each section using its pointer
    /// 3. Extract array elements (with optional label injection)
    /// 4. Prepare for streaming
    pub fn new<R: Read>(
        reader: R,
        sections: Vec<SectionSpec>,
        limits: WrapperLimits,
        label_field: Option<String>,
        inject_label: bool,
        missing_behavior: MissingSectionBehavior,
    ) -> Result<Self, WrapperError> {
        let start_time = Instant::now();

        // Validate limits
        limits.validate()?;

        // Read entire input into buffer (with size limit enforcement)
        let mut buffer = Vec::new();
        let mut limited_reader = reader.take(limits.max_buffer_bytes as u64 + 1);
        limited_reader
            .read_to_end(&mut buffer)
            .map_err(|e| WrapperError::Io(e))?;

        // Check if we exceeded the buffer limit
        if buffer.len() > limits.max_buffer_bytes {
            return Err(WrapperError::BufferLimitExceeded {
                limit_bytes: limits.max_buffer_bytes,
                buffered_bytes: buffer.len(),
                pointer: "<sections>".to_string(),
                suggested_size: WrapperError::suggest_buffer_size(buffer.len()),
                jq_expr: "jq '.section1, .section2' input.json".to_string(),
            });
        }

        let peak_buffer_bytes = buffer.len();

        // Parse as JSON object
        let document: Value =
            serde_json::from_slice(&buffer).map_err(|e| WrapperError::JsonParse {
                context: "parsing top-level document for sections".to_string(),
                source: e,
            })?;

        // Ensure top-level is an object
        let _root_obj =
            document
                .as_object()
                .ok_or_else(|| WrapperError::PointerTargetWrongType {
                    pointer: "/".to_string(),
                    expected_type: "object".to_string(),
                    found_type: type_name(&document).to_string(),
                })?;

        // Extract sections
        let mut extracted_sections = Vec::new();
        let mut section_counts = Vec::new();

        for section_spec in sections {
            // Parse pointer
            let tokens = parse_pointer(
                &section_spec.pointer,
                limits.max_pointer_length,
                limits.max_depth,
            )?;

            // Navigate to section
            let section_value = match navigate_pointer(&document, &tokens, &section_spec.pointer) {
                Ok(val) => val,
                Err(WrapperError::PointerNotFound {
                    pointer,
                    reached_path: _,
                    available_keys,
                }) => {
                    match missing_behavior {
                        MissingSectionBehavior::Skip => {
                            // Record zero count and continue
                            section_counts.push((section_spec.name.clone(), 0));
                            continue;
                        }
                        MissingSectionBehavior::Error => {
                            return Err(WrapperError::SectionNotFound {
                                section: section_spec.name,
                                pointer,
                                available_keys,
                            });
                        }
                    }
                }
                Err(e) => return Err(e),
            };

            // Ensure section is an array
            let section_array =
                section_value
                    .as_array()
                    .ok_or_else(|| WrapperError::PointerTargetWrongType {
                        pointer: section_spec.pointer.clone(),
                        expected_type: "array".to_string(),
                        found_type: type_name(section_value).to_string(),
                    })?;

            // Extract records from section
            let mut section_records = Vec::new();
            let label_to_inject = if inject_label {
                Some(
                    section_spec
                        .label
                        .clone()
                        .unwrap_or_else(|| section_spec.name.clone()),
                )
            } else {
                None
            };

            for element in section_array {
                // Each element must be an object
                let mut record = element
                    .as_object()
                    .ok_or_else(|| WrapperError::PointerTargetWrongType {
                        pointer: format!("{}[element]", section_spec.pointer),
                        expected_type: "object".to_string(),
                        found_type: type_name(element).to_string(),
                    })?
                    .clone();

                // Inject label if requested
                if let Some(ref label) = label_to_inject {
                    let default_field = "_section".to_string();
                    let field = label_field.as_ref().unwrap_or(&default_field);

                    // Check for collision
                    if record.contains_key(field) {
                        return Err(WrapperError::SectionLabelCollision {
                            field: field.clone(),
                            section: section_spec.name.clone(),
                        });
                    }

                    record.insert(field.clone(), Value::String(label.clone()));
                }

                section_records.push(record);
            }

            let count = section_records.len();
            section_counts.push((section_spec.name.clone(), count));
            extracted_sections.push((section_spec.name.clone(), section_records));
        }

        let processing_duration = start_time.elapsed();

        Ok(Self {
            sections: extracted_sections,
            current_section_idx: 0,
            current_record_idx: 0,
            metrics: SectionsStreamMetrics {
                peak_buffer_bytes,
                records_emitted: 0,
                section_counts,
                processing_duration,
            },
        })
    }

    /// Get metrics for this stream
    pub fn metrics(&self) -> &SectionsStreamMetrics {
        &self.metrics
    }
}

impl Iterator for SectionsStream {
    type Item = Result<Map<String, Value>, WrapperError>;

    fn next(&mut self) -> Option<Self::Item> {
        // Find next non-empty section
        while self.current_section_idx < self.sections.len() {
            let (_section_name, records) = &self.sections[self.current_section_idx];

            if self.current_record_idx < records.len() {
                let record = records[self.current_record_idx].clone();
                self.current_record_idx += 1;
                self.metrics.records_emitted += 1;
                return Some(Ok(record));
            }

            // Move to next section
            self.current_section_idx += 1;
            self.current_record_idx = 0;
        }

        None
    }
}

/// Get a human-readable type name for a JSON value
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

    #[test]
    fn sections_stream_concatenates_multiple_arrays() {
        let input = json!({
            "users": [
                {"name": "alice", "role": "admin"},
                {"name": "bob", "role": "user"}
            ],
            "guests": [
                {"name": "charlie", "role": "guest"}
            ]
        });

        let input_bytes = serde_json::to_vec(&input).unwrap();
        let sections = vec![
            SectionSpec {
                name: "users".to_string(),
                pointer: "/users".to_string(),
                label: None,
            },
            SectionSpec {
                name: "guests".to_string(),
                pointer: "/guests".to_string(),
                label: None,
            },
        ];

        let stream = SectionsStream::new(
            input_bytes.as_slice(),
            sections,
            WrapperLimits::default(),
            None,
            false,
            MissingSectionBehavior::Skip,
        )
        .unwrap();

        let records: Result<Vec<_>, _> = stream.collect();
        let records = records.unwrap();
        assert_eq!(records.len(), 3);
        assert_eq!(records[0].get("name").unwrap(), "alice");
        assert_eq!(records[1].get("name").unwrap(), "bob");
        assert_eq!(records[2].get("name").unwrap(), "charlie");
    }

    #[test]
    fn sections_stream_injects_labels() {
        let input = json!({
            "users": [{"name": "alice"}],
            "admins": [{"name": "bob"}]
        });

        let input_bytes = serde_json::to_vec(&input).unwrap();
        let sections = vec![
            SectionSpec {
                name: "users".to_string(),
                pointer: "/users".to_string(),
                label: Some("user".to_string()),
            },
            SectionSpec {
                name: "admins".to_string(),
                pointer: "/admins".to_string(),
                label: Some("admin".to_string()),
            },
        ];

        let stream = SectionsStream::new(
            input_bytes.as_slice(),
            sections,
            WrapperLimits::default(),
            Some("_section".to_string()),
            true,
            MissingSectionBehavior::Skip,
        )
        .unwrap();

        let records: Result<Vec<_>, _> = stream.collect();
        let records = records.unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].get("_section").unwrap(), "user");
        assert_eq!(records[1].get("_section").unwrap(), "admin");
    }

    #[test]
    fn sections_stream_skips_missing_sections() {
        let input = json!({
            "users": [{"name": "alice"}]
        });

        let input_bytes = serde_json::to_vec(&input).unwrap();
        let sections = vec![
            SectionSpec {
                name: "users".to_string(),
                pointer: "/users".to_string(),
                label: None,
            },
            SectionSpec {
                name: "admins".to_string(),
                pointer: "/admins".to_string(),
                label: None,
            },
        ];

        let stream = SectionsStream::new(
            input_bytes.as_slice(),
            sections,
            WrapperLimits::default(),
            None,
            false,
            MissingSectionBehavior::Skip,
        )
        .unwrap();

        let mut stream = stream;
        let records: Result<Vec<_>, _> = stream.by_ref().collect();
        let records = records.unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].get("name").unwrap(), "alice");

        // Check metrics show zero count for missing section
        assert_eq!(stream.metrics().section_counts.len(), 2);
        assert_eq!(stream.metrics().section_counts[0].1, 1);
        assert_eq!(stream.metrics().section_counts[1].1, 0);
    }

    #[test]
    fn sections_stream_errors_on_missing_when_configured() {
        let input = json!({
            "users": [{"name": "alice"}]
        });

        let input_bytes = serde_json::to_vec(&input).unwrap();
        let sections = vec![SectionSpec {
            name: "admins".to_string(),
            pointer: "/admins".to_string(),
            label: None,
        }];

        let result = SectionsStream::new(
            input_bytes.as_slice(),
            sections,
            WrapperLimits::default(),
            None,
            false,
            MissingSectionBehavior::Error,
        );

        assert!(matches!(result, Err(WrapperError::SectionNotFound { .. })));
    }

    #[test]
    fn sections_stream_detects_label_collision() {
        let input = json!({
            "users": [{"name": "alice", "_section": "existing"}]
        });

        let input_bytes = serde_json::to_vec(&input).unwrap();
        let sections = vec![SectionSpec {
            name: "users".to_string(),
            pointer: "/users".to_string(),
            label: None,
        }];

        let result = SectionsStream::new(
            input_bytes.as_slice(),
            sections,
            WrapperLimits::default(),
            Some("_section".to_string()),
            true,
            MissingSectionBehavior::Skip,
        );

        assert!(matches!(
            result,
            Err(WrapperError::SectionLabelCollision { .. })
        ));
    }

    #[test]
    fn sections_stream_enforces_buffer_limit() {
        let input = json!({
            "users": [{"name": "alice"}]
        });

        let input_bytes = serde_json::to_vec(&input).unwrap();
        let sections = vec![SectionSpec {
            name: "users".to_string(),
            pointer: "/users".to_string(),
            label: None,
        }];

        let mut small_limits = WrapperLimits::default();
        small_limits.max_buffer_bytes = 10; // Very small

        let result = SectionsStream::new(
            input_bytes.as_slice(),
            sections,
            small_limits,
            None,
            false,
            MissingSectionBehavior::Skip,
        );

        assert!(matches!(
            result,
            Err(WrapperError::BufferLimitExceeded { .. })
        ));
    }

    #[test]
    fn sections_stream_handles_empty_arrays() {
        let input = json!({
            "users": [],
            "admins": [{"name": "alice"}]
        });

        let input_bytes = serde_json::to_vec(&input).unwrap();
        let sections = vec![
            SectionSpec {
                name: "users".to_string(),
                pointer: "/users".to_string(),
                label: None,
            },
            SectionSpec {
                name: "admins".to_string(),
                pointer: "/admins".to_string(),
                label: None,
            },
        ];

        let stream = SectionsStream::new(
            input_bytes.as_slice(),
            sections,
            WrapperLimits::default(),
            None,
            false,
            MissingSectionBehavior::Skip,
        )
        .unwrap();

        let records: Result<Vec<_>, _> = stream.collect();
        let records = records.unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].get("name").unwrap(), "alice");
    }

    #[test]
    fn sections_stream_rejects_non_object_root() {
        let input = json!([{"name": "alice"}]);
        let input_bytes = serde_json::to_vec(&input).unwrap();
        let sections = vec![SectionSpec {
            name: "users".to_string(),
            pointer: "/users".to_string(),
            label: None,
        }];

        let result = SectionsStream::new(
            input_bytes.as_slice(),
            sections,
            WrapperLimits::default(),
            None,
            false,
            MissingSectionBehavior::Skip,
        );

        assert!(matches!(
            result,
            Err(WrapperError::PointerTargetWrongType { .. })
        ));
    }

    #[test]
    fn sections_stream_rejects_non_array_section() {
        let input = json!({
            "users": {"name": "alice"}
        });

        let input_bytes = serde_json::to_vec(&input).unwrap();
        let sections = vec![SectionSpec {
            name: "users".to_string(),
            pointer: "/users".to_string(),
            label: None,
        }];

        let result = SectionsStream::new(
            input_bytes.as_slice(),
            sections,
            WrapperLimits::default(),
            None,
            false,
            MissingSectionBehavior::Skip,
        );

        assert!(matches!(
            result,
            Err(WrapperError::PointerTargetWrongType { .. })
        ));
    }
}
