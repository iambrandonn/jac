//! Error types for wrapper preprocessing

use thiserror::Error;

/// Errors that can occur during wrapper preprocessing
#[derive(Debug, Error)]
pub enum WrapperError {
    /// Buffer limit exceeded while traversing to target
    #[error(
        "Buffer limit exceeded: buffered {buffered_bytes} bytes (limit: {limit_bytes} bytes) \
         while navigating to '{pointer}'.\n\
         \n\
         Suggested fixes:\n\
         1. Increase buffer: --wrapper-pointer-buffer {suggested_size}\n\
         2. Preprocess with jq: {jq_expr}\n\
         3. Restructure input to minimize envelope size"
    )]
    BufferLimitExceeded {
        /// Maximum buffer size allowed (bytes)
        limit_bytes: usize,
        /// Actual bytes buffered when limit was exceeded
        buffered_bytes: usize,
        /// JSON Pointer path being processed
        pointer: String,
        /// Suggested buffer size for this input (formatted string)
        suggested_size: String,
        /// Equivalent jq command for preprocessing
        jq_expr: String,
    },

    /// Pointer depth limit exceeded
    #[error(
        "Pointer depth limit exceeded: '{pointer}' has depth {depth} (max: {max_depth}).\n\
         \n\
         Suggested fixes:\n\
         1. Increase depth: --wrapper-pointer-depth {suggested_depth}\n\
         2. Flatten structure before compression"
    )]
    DepthLimitExceeded {
        /// JSON Pointer path being processed
        pointer: String,
        /// Actual depth of the pointer
        depth: usize,
        /// Maximum depth allowed
        max_depth: usize,
        /// Suggested depth for this pointer
        suggested_depth: usize,
    },

    /// Pointer string too long
    #[error(
        "Pointer too long: {length} characters (max: {max_length}).\n\
         \n\
         This limit prevents malicious inputs. If you have a legitimate use case,\n\
         consider restructuring your data or preprocessing externally."
    )]
    PointerTooLong {
        /// The pointer string that exceeded the limit
        pointer: String,
        /// Actual length of the pointer string (characters)
        length: usize,
        /// Maximum pointer length allowed (characters)
        max_length: usize,
    },

    /// Pointer path not found in document
    #[error(
        "Pointer not found: '{pointer}' does not exist.\n\
         \n\
         Reached: '{reached_path}'\n\
         Available keys at this level: {available_keys}"
    )]
    PointerNotFound {
        /// JSON Pointer path that was not found
        pointer: String,
        /// Path segment successfully navigated before failure
        reached_path: String,
        /// Comma-separated list of keys available at the failure point
        available_keys: String,
    },

    /// Pointer target has wrong type
    #[error(
        "Pointer target type mismatch: '{pointer}' points to {found_type}, expected {expected_type}.\n\
         \n\
         JAC wrappers can only extract arrays or objects (which emit a single record).\n\
         Scalar and null values are not supported as wrapper targets."
    )]
    PointerTargetWrongType {
        /// JSON Pointer path to the target with wrong type
        pointer: String,
        /// Expected type (e.g., "array" or "object")
        expected_type: String,
        /// Actual type found (e.g., "null", "string", "number")
        found_type: String,
    },

    /// Invalid pointer syntax
    #[error(
        "Invalid JSON Pointer syntax: '{pointer}' - {reason}\n\
         \n\
         JSON Pointers must:\n\
         - Start with '/' (or be empty string for root)\n\
         - Use '~0' to escape '~' and '~1' to escape '/'\n\
         - Be valid UTF-8\n\
         \n\
         See RFC 6901 for details."
    )]
    InvalidPointer {
        /// The invalid pointer string
        pointer: String,
        /// Explanation of why the pointer is invalid
        reason: String,
    },

    /// Configuration exceeds hard limits
    #[error(
        "Configuration exceeds hard limits: {reason}\n\
         \n\
         Hard limits:\n\
         - max_depth: {max_depth}\n\
         - max_buffer_bytes: {max_buffer} bytes\n\
         - max_pointer_length: {max_ptr_len} characters"
    )]
    ConfigurationExceedsHardLimits {
        /// Description of which limit was exceeded
        reason: String,
        /// Hard maximum depth
        max_depth: usize,
        /// Hard maximum buffer size (bytes)
        max_buffer: usize,
        /// Hard maximum pointer length (characters)
        max_ptr_len: usize,
    },

    /// Section not found in input document
    #[error(
        "Section not found: '{section}' at pointer '{pointer}' does not exist.\n\
         \n\
         Available keys at this level: {available_keys}\n\
         \n\
         Suggested fixes:\n\
         1. Check section name spelling\n\
         2. Verify input structure matches expected format\n\
         3. Use --wrapper-sections-missing-skip to allow missing sections"
    )]
    SectionNotFound {
        /// Name of the section that was not found
        section: String,
        /// JSON Pointer path that was used
        pointer: String,
        /// Comma-separated list of keys available at the target location
        available_keys: String,
    },

    /// Section label field collision
    #[error(
        "Section label collision: field '{field}' already exists in record from section '{section}'.\n\
         \n\
         The wrapper tried to inject section label into field '{field}', but this field\n\
         already exists in the source record.\n\
         \n\
         Suggested fixes:\n\
         1. Choose a different label field: --wrapper-section-label-field <field>\n\
         2. Disable label injection: --wrapper-section-no-label"
    )]
    SectionLabelCollision {
        /// Field name that collided
        field: String,
        /// Section that caused the collision
        section: String,
    },

    /// Map key too long
    #[error(
        "Map key too long: key '{key}' has length {length} bytes (max: {max_length} bytes).\n\
         \n\
         This limit prevents malicious inputs and aligns with JAC's max_string_len_per_value.\n\
         If you have a legitimate use case, consider restructuring your data or preprocessing externally."
    )]
    MapKeyTooLong {
        /// The key that exceeded the limit
        key: String,
        /// Actual length of the key (bytes)
        length: usize,
        /// Maximum key length allowed (bytes)
        max_length: usize,
    },

    /// Key field collision in map mode
    #[error(
        "Key field collision: field '{field}' already exists in record with map key '{map_key}'.\n\
         \n\
         The wrapper tried to inject the map key into field '{field}', but this field\n\
         already exists in the source record.\n\
         \n\
         Suggested fixes:\n\
         1. Choose a different key field: --wrapper-map-key-field <field>\n\
         2. Enable overwrite mode: --wrapper-map-overwrite-key"
    )]
    KeyFieldCollision {
        /// Field name that collided
        field: String,
        /// Map key that caused the collision
        map_key: String,
    },

    /// Map value is not an object
    #[error(
        "Map value not an object: key '{key}' points to {found_type}, expected object.\n\
         \n\
         JAC map wrappers require all values to be objects (records).\n\
         Scalar, null, and array values cannot be processed."
    )]
    MapValueNotObject {
        /// The map key with wrong value type
        key: String,
        /// Actual type found
        found_type: String,
    },

    /// JSON parsing error during wrapper traversal
    #[error("JSON parse error while processing wrapper: {context} - {source}")]
    JsonParse {
        /// Context describing where parsing failed
        context: String,
        /// Underlying serde_json error
        source: serde_json::Error,
    },

    /// I/O error during wrapper processing
    #[error("I/O error during wrapper processing: {0}")]
    Io(#[from] std::io::Error),

    /// Underlying JAC error
    #[error("JAC error: {0}")]
    Jac(#[from] jac_format::JacError),

    /// Plugin already registered with same name
    #[error(
        "Plugin already registered: a plugin named '{name}' is already registered.\n\
         \n\
         Plugin names must be unique. Either:\n\
         1. Unregister the existing plugin first\n\
         2. Choose a different name for your plugin"
    )]
    PluginAlreadyRegistered {
        /// Name of the plugin that was already registered
        name: String,
    },

    /// Plugin not found
    #[error("Plugin not found: no plugin named '{name}' is registered")]
    PluginNotFound {
        /// Name of the plugin that was not found
        name: String,
    },

    /// Plugin configuration invalid
    #[error(
        "Invalid plugin configuration for '{plugin}': {reason}\n\
         \n\
         Check the plugin documentation for required configuration format."
    )]
    InvalidPluginConfig {
        /// Name of the plugin
        plugin: String,
        /// Reason why the configuration is invalid
        reason: String,
    },

    /// Plugin execution error
    #[error("Plugin '{plugin}' execution failed: {reason}")]
    PluginExecutionFailed {
        /// Name of the plugin that failed
        plugin: String,
        /// Error message from the plugin
        reason: String,
    },

    /// Array header row error
    #[error(
        "Invalid header row: {reason}\n\
         \n\
         Array-with-headers wrapper requires the first element to be an array of strings.\n\
         Example: [[\"id\", \"name\"], [1, \"Alice\"], [2, \"Bob\"]]"
    )]
    InvalidHeaderRow {
        /// Reason why the header is invalid
        reason: String,
    },

    /// Array row length mismatch
    #[error(
        "Row length mismatch: row {row_index} has {actual} elements, expected {expected} (from header).\n\
         \n\
         All rows must have the same number of elements as the header row."
    )]
    ArrayRowLengthMismatch {
        /// Index of the row with wrong length
        row_index: usize,
        /// Actual number of elements
        actual: usize,
        /// Expected number of elements
        expected: usize,
    },
}

impl WrapperError {
    /// Suggest a buffer size with safety margin
    pub fn suggest_buffer_size(current_bytes: usize) -> String {
        let suggested = (current_bytes as f64 * 1.5).ceil() as usize;
        format_bytes(suggested)
    }

    /// Convert pointer to equivalent jq expression
    pub fn pointer_to_jq(pointer: &str) -> String {
        if pointer.is_empty() {
            return "jq '.' input.json".to_string();
        }

        let tokens: Vec<&str> = pointer.split('/').skip(1).collect();
        let mut jq_path = String::new();

        for token in tokens {
            let unescaped = unescape_pointer_token(token);
            // Check if it's a valid identifier (no special chars)
            if unescaped
                .chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
            {
                jq_path.push('.');
                jq_path.push_str(&unescaped);
            } else {
                jq_path.push('[');
                jq_path.push('"');
                jq_path.push_str(&unescaped.replace('\\', "\\\\").replace('"', "\\\""));
                jq_path.push('"');
                jq_path.push(']');
            }
        }

        format!("jq '{} | .[]' input.json", jq_path)
    }
}

/// Format bytes as human-readable string (using decimal MB/GB for user-facing suggestions)
fn format_bytes(bytes: usize) -> String {
    const KB: f64 = 1000.0;
    const MB: f64 = 1_000_000.0;
    const GB: f64 = 1_000_000_000.0;

    let b = bytes as f64;
    if b >= GB {
        format!("{:.1}G", b / GB)
    } else if b >= MB {
        format!("{:.1}M", b / MB)
    } else if b >= KB {
        format!("{:.1}K", b / KB)
    } else {
        format!("{}", bytes)
    }
}

/// Unescape a JSON Pointer token according to RFC 6901
fn unescape_pointer_token(token: &str) -> String {
    token.replace("~1", "/").replace("~0", "~")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_bytes_displays_human_readable() {
        assert_eq!(format_bytes(512), "512");
        assert_eq!(format_bytes(1000), "1.0K");
        assert_eq!(format_bytes(1500), "1.5K");
        assert_eq!(format_bytes(1_000_000), "1.0M");
        assert_eq!(format_bytes(16_000_000), "16.0M");
        assert_eq!(format_bytes(1_000_000_000), "1.0G");
    }

    #[test]
    fn pointer_to_jq_handles_simple_paths() {
        assert_eq!(WrapperError::pointer_to_jq(""), "jq '.' input.json");
        assert_eq!(
            WrapperError::pointer_to_jq("/data"),
            "jq '.data | .[]' input.json"
        );
        assert_eq!(
            WrapperError::pointer_to_jq("/api/v1/results"),
            "jq '.api.v1.results | .[]' input.json"
        );
    }

    #[test]
    fn pointer_to_jq_handles_special_characters() {
        assert_eq!(
            WrapperError::pointer_to_jq("/field with spaces"),
            "jq '[\"field with spaces\"] | .[]' input.json"
        );
        assert_eq!(
            WrapperError::pointer_to_jq("/data/user~1name"),
            "jq '.data[\"user/name\"] | .[]' input.json"
        );
    }

    #[test]
    fn suggest_buffer_size_adds_margin() {
        assert!(WrapperError::suggest_buffer_size(10_000_000).contains("15"));
    }
}
