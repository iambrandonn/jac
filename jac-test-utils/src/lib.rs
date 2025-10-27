//! JAC Test Utilities
//!
//! This crate provides shared testing utilities and helpers for the JAC project.

use serde_json::Value;
use std::collections::HashMap;

pub mod debug_tools;
pub mod profiler;
pub mod test_categories;
pub mod test_config;
pub mod test_debugger;
pub mod visualization;

/// Builder for creating test records with common patterns
pub struct RecordBuilder {
    fields: HashMap<String, Value>,
}

impl RecordBuilder {
    /// Create a new record builder
    pub fn new() -> Self {
        Self {
            fields: HashMap::new(),
        }
    }

    /// Add a field with a string value
    pub fn string(mut self, key: &str, value: &str) -> Self {
        self.fields
            .insert(key.to_string(), Value::String(value.to_string()));
        self
    }

    /// Add a field with an integer value
    pub fn int(mut self, key: &str, value: i64) -> Self {
        self.fields
            .insert(key.to_string(), Value::Number(value.into()));
        self
    }

    /// Add a field with a boolean value
    pub fn bool(mut self, key: &str, value: bool) -> Self {
        self.fields.insert(key.to_string(), Value::Bool(value));
        self
    }

    /// Add a field with a null value
    pub fn null(mut self, key: &str) -> Self {
        self.fields.insert(key.to_string(), Value::Null);
        self
    }

    /// Add a field with a decimal value (as string to preserve precision)
    pub fn decimal(mut self, key: &str, value: &str) -> Self {
        self.fields
            .insert(key.to_string(), Value::String(value.to_string()));
        self
    }

    /// Add a field with an object value
    pub fn object(mut self, key: &str, value: Value) -> Self {
        self.fields.insert(key.to_string(), value);
        self
    }

    /// Add a field with an array value
    pub fn array(mut self, key: &str, value: Vec<Value>) -> Self {
        self.fields.insert(key.to_string(), Value::Array(value));
        self
    }

    /// Build the record
    pub fn build(self) -> Value {
        Value::Object(self.fields.into_iter().collect())
    }
}

impl Default for RecordBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Generate test data with various patterns
pub struct TestDataGenerator;

impl TestDataGenerator {
    /// Generate records with schema drift (field types change across records)
    pub fn schema_drift_records() -> Vec<Value> {
        vec![
            RecordBuilder::new()
                .string("id", "1")
                .int("value", 42)
                .build(),
            RecordBuilder::new()
                .string("id", "2")
                .string("value", "hello") // Same field, different type
                .build(),
            RecordBuilder::new()
                .string("id", "3")
                .bool("value", true) // Same field, different type
                .build(),
        ]
    }

    /// Generate records with deeply nested structures
    pub fn deeply_nested_records() -> Vec<Value> {
        let mut nested = Value::Object(serde_json::Map::new());
        for i in 0..10 {
            let mut level = Value::Object(serde_json::Map::new());
            level.as_object_mut().unwrap().insert(
                format!("level_{}", i),
                Value::String(format!("value_{}", i)),
            );
            nested = level;
        }

        vec![RecordBuilder::new()
            .string("id", "deep")
            .object("nested", nested)
            .build()]
    }

    /// Generate records with high-precision decimal values
    pub fn high_precision_decimal_records() -> Vec<Value> {
        vec![
            RecordBuilder::new()
                .string("id", "1")
                .decimal("pi", "3.1415926535897932384626433832795028841971693993751058209749445923078164062862089986280348253421170679")
                .decimal("e", "2.7182818284590452353602874713526624977572470936999595749669676277240766303535475945713821785251664274")
                .build(),
            RecordBuilder::new()
                .string("id", "2")
                .decimal("small", "0.0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000001")
                .decimal("large", "9999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999.9999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999")
                .build(),
        ]
    }

    /// Generate records with Unicode edge cases
    pub fn unicode_edge_records() -> Vec<Value> {
        vec![
            RecordBuilder::new()
                .string("id", "1")
                .string("ascii", "Hello, World!")
                .build(),
            RecordBuilder::new()
                .string("id", "2")
                .string("unicode", "Hello, 世界! 🌍")
                .build(),
            RecordBuilder::new()
                .string("id", "3")
                .string("emoji", "🚀🎉💯🔥⭐")
                .build(),
            RecordBuilder::new()
                .string("id", "4")
                .string("mixed", "ASCII + 中文 + 🎯 + العربية")
                .build(),
        ]
    }

    /// Generate records with boundary values
    pub fn boundary_value_records() -> Vec<Value> {
        vec![
            RecordBuilder::new()
                .string("id", "int_max")
                .int("value", i64::MAX)
                .build(),
            RecordBuilder::new()
                .string("id", "int_min")
                .int("value", i64::MIN)
                .build(),
            RecordBuilder::new()
                .string("id", "empty_string")
                .string("value", "")
                .build(),
            RecordBuilder::new()
                .string("id", "empty_array")
                .array("value", vec![])
                .build(),
            RecordBuilder::new()
                .string("id", "empty_object")
                .object("value", Value::Object(serde_json::Map::new()))
                .build(),
        ]
    }

    /// Generate a large number of records for stress testing
    pub fn large_record_set(count: usize) -> Vec<Value> {
        let mut records = Vec::with_capacity(count);

        for i in 0..count {
            let user_id = i % 100; // 100 unique users
            let level = match i % 4 {
                0 => "DEBUG",
                1 => "INFO",
                2 => "WARN",
                _ => "ERROR",
            };

            records.push(
                RecordBuilder::new()
                    .int("id", i as i64)
                    .int("timestamp", 1609459200 + i as i64)
                    .string("level", level)
                    .string("user", &format!("user_{}", user_id))
                    .string("message", &format!("Test message number {}", i))
                    .build(),
            );
        }

        records
    }
}

/// Utility functions for test assertions
pub mod assertions {
    use serde_json::Value;

    /// Assert that two JSON values are semantically equal (ignoring formatting)
    pub fn assert_json_equal(actual: &Value, expected: &Value, context: &str) {
        if actual != expected {
            panic!(
                "JSON assertion failed in {}:\nExpected: {}\nActual: {}",
                context,
                serde_json::to_string_pretty(expected).unwrap(),
                serde_json::to_string_pretty(actual).unwrap()
            );
        }
    }

    /// Assert that a field projection contains expected values
    pub fn assert_field_projection(actual: &[Value], expected: &[Value], field_name: &str) {
        if actual != expected {
            panic!(
                "Field projection assertion failed for field '{}':\nExpected: {:?}\nActual: {:?}",
                field_name, expected, actual
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_builder() {
        let record = RecordBuilder::new()
            .string("name", "test")
            .int("age", 25)
            .bool("active", true)
            .build();

        assert!(record.is_object());
        let obj = record.as_object().unwrap();
        assert_eq!(obj.get("name").unwrap().as_str().unwrap(), "test");
        assert_eq!(obj.get("age").unwrap().as_i64().unwrap(), 25);
        assert_eq!(obj.get("active").unwrap().as_bool().unwrap(), true);
    }

    #[test]
    fn test_schema_drift_records() {
        let records = TestDataGenerator::schema_drift_records();
        assert_eq!(records.len(), 3);

        // Check that the "value" field has different types
        assert!(records[0]["value"].is_number());
        assert!(records[1]["value"].is_string());
        assert!(records[2]["value"].is_boolean());
    }

    #[test]
    fn test_large_record_set() {
        let records = TestDataGenerator::large_record_set(1000);
        assert_eq!(records.len(), 1000);

        // Check that user field cycles through 100 unique values
        let mut users = std::collections::HashSet::new();
        for record in &records {
            if let Some(user) = record.get("user") {
                users.insert(user.as_str().unwrap());
            }
        }
        assert_eq!(users.len(), 100);
    }
}
