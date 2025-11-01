//! Shared utilities for wrapper processing

use super::error::WrapperError;
use serde_json::Value;

/// Parse and validate a JSON Pointer according to RFC 6901
///
/// Returns a vector of unescaped tokens. Empty string returns empty vec (root).
pub fn parse_pointer(
    pointer: &str,
    max_length: usize,
    max_depth: usize,
) -> Result<Vec<String>, WrapperError> {
    // Validate length
    if pointer.len() > max_length {
        return Err(WrapperError::PointerTooLong {
            pointer: pointer.to_string(),
            length: pointer.len(),
            max_length,
        });
    }

    // Empty string is root
    if pointer.is_empty() {
        return Ok(Vec::new());
    }

    // Must start with '/'
    if !pointer.starts_with('/') {
        return Err(WrapperError::InvalidPointer {
            pointer: pointer.to_string(),
            reason: "Pointer must start with '/' (or be empty for root)".to_string(),
        });
    }

    // Split and unescape tokens
    let tokens: Vec<String> = pointer
        .split('/')
        .skip(1) // Skip empty first element
        .map(unescape_pointer_token)
        .collect();

    // Validate depth
    if tokens.len() > max_depth {
        return Err(WrapperError::DepthLimitExceeded {
            pointer: pointer.to_string(),
            depth: tokens.len(),
            max_depth,
            suggested_depth: tokens.len().min(10),
        });
    }

    // Validate escape sequences
    for (idx, _token) in tokens.iter().enumerate() {
        if let Err(e) = validate_escape_sequences(&pointer.split('/').nth(idx + 1).unwrap_or("")) {
            return Err(WrapperError::InvalidPointer {
                pointer: pointer.to_string(),
                reason: e,
            });
        }
    }

    Ok(tokens)
}

/// Unescape a JSON Pointer token according to RFC 6901
///
/// - '~1' → '/'
/// - '~0' → '~'
pub fn unescape_pointer_token(token: &str) -> String {
    // Must process ~1 before ~0 to avoid double-unescaping
    token.replace("~1", "/").replace("~0", "~")
}

/// Escape a string for use in a JSON Pointer
pub fn escape_pointer_token(token: &str) -> String {
    // Must escape ~ before / to avoid incorrect escaping
    token.replace('~', "~0").replace('/', "~1")
}

/// Validate escape sequences in a token (before unescaping)
fn validate_escape_sequences(token: &str) -> Result<(), String> {
    let mut chars = token.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '~' {
            match chars.peek() {
                Some('0') | Some('1') => {
                    chars.next(); // consume the digit
                }
                Some(other) => {
                    return Err(format!(
                        "Invalid escape sequence '~{}'. Use '~0' for '~' and '~1' for '/'.",
                        other
                    ));
                }
                None => {
                    return Err(
                        "Incomplete escape sequence at end of token. Use '~0' for '~' and '~1' for '/'.".to_string()
                    );
                }
            }
        }
    }
    Ok(())
}

/// Navigate to a JSON value using a pointer path
///
/// Returns the value at the pointer location, or an error if not found.
pub fn navigate_pointer<'a>(
    value: &'a Value,
    tokens: &[String],
    original_pointer: &str,
) -> Result<&'a Value, WrapperError> {
    let mut current = value;
    let mut path_so_far = String::new();

    for (idx, token) in tokens.iter().enumerate() {
        path_so_far.push('/');
        path_so_far.push_str(&escape_pointer_token(token));

        current = match current {
            Value::Object(map) => map.get(token).ok_or_else(|| {
                let available: Vec<&str> = map.keys().take(10).map(|s| s.as_str()).collect();
                let available_display = if map.len() > 10 {
                    format!("{}, ... ({} total)", available.join(", "), map.len())
                } else if available.is_empty() {
                    "<empty object>".to_string()
                } else {
                    available.join(", ")
                };

                WrapperError::PointerNotFound {
                    pointer: original_pointer.to_string(),
                    reached_path: path_so_far.clone(),
                    available_keys: available_display,
                }
            })?,
            Value::Array(arr) => {
                // Try to parse as array index
                if let Ok(index) = token.parse::<usize>() {
                    arr.get(index)
                        .ok_or_else(|| WrapperError::PointerNotFound {
                            pointer: original_pointer.to_string(),
                            reached_path: path_so_far.clone(),
                            available_keys: format!("<array with {} elements>", arr.len()),
                        })?
                } else {
                    return Err(WrapperError::PointerNotFound {
                        pointer: original_pointer.to_string(),
                        reached_path: if idx == 0 {
                            "/".to_string()
                        } else {
                            path_so_far
                        },
                        available_keys: format!(
                            "<array with {} elements, expected numeric index>",
                            arr.len()
                        ),
                    });
                }
            }
            other => {
                return Err(WrapperError::PointerNotFound {
                    pointer: original_pointer.to_string(),
                    reached_path: if idx == 0 {
                        "/".to_string()
                    } else {
                        path_so_far
                    },
                    available_keys: format!("<{}, cannot traverse further>", type_name(other)),
                });
            }
        };
    }

    Ok(current)
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
    fn parse_pointer_validates_length() {
        let long_pointer = format!("/{}", "a".repeat(300));
        let result = parse_pointer(&long_pointer, 256, 10);
        assert!(matches!(result, Err(WrapperError::PointerTooLong { .. })));
    }

    #[test]
    fn parse_pointer_validates_depth() {
        let deep_pointer = "/a/b/c/d/e/f/g/h/i/j/k";
        let result = parse_pointer(deep_pointer, 256, 10);
        assert!(matches!(
            result,
            Err(WrapperError::DepthLimitExceeded { .. })
        ));
    }

    #[test]
    fn parse_pointer_rejects_invalid_start() {
        let result = parse_pointer("data/field", 256, 10);
        assert!(matches!(result, Err(WrapperError::InvalidPointer { .. })));
    }

    #[test]
    fn parse_pointer_handles_root() {
        let result = parse_pointer("", 256, 10).unwrap();
        assert_eq!(result, Vec::<String>::new());
    }

    #[test]
    fn parse_pointer_unescapes_tokens() {
        let result = parse_pointer("/a~1b/c~0d", 256, 10).unwrap();
        assert_eq!(result, vec!["a/b".to_string(), "c~d".to_string()]);
    }

    #[test]
    fn parse_pointer_validates_escape_sequences() {
        let result = parse_pointer("/invalid~2", 256, 10);
        assert!(matches!(result, Err(WrapperError::InvalidPointer { .. })));

        let result = parse_pointer("/trailing~", 256, 10);
        assert!(matches!(result, Err(WrapperError::InvalidPointer { .. })));
    }

    #[test]
    fn escape_and_unescape_roundtrip() {
        let original = "hello/world~test";
        let escaped = escape_pointer_token(original);
        let unescaped = unescape_pointer_token(&escaped);
        assert_eq!(unescaped, original);
    }

    #[test]
    fn navigate_pointer_finds_nested_object() {
        let doc = json!({
            "data": {
                "users": [
                    {"name": "alice"},
                    {"name": "bob"}
                ]
            }
        });

        let tokens = parse_pointer("/data/users", 256, 10).unwrap();
        let result = navigate_pointer(&doc, &tokens, "/data/users").unwrap();
        assert!(result.is_array());
        assert_eq!(result.as_array().unwrap().len(), 2);
    }

    #[test]
    fn navigate_pointer_handles_array_index() {
        let doc = json!({
            "items": ["a", "b", "c"]
        });

        let tokens = parse_pointer("/items/1", 256, 10).unwrap();
        let result = navigate_pointer(&doc, &tokens, "/items/1").unwrap();
        assert_eq!(result, &json!("b"));
    }

    #[test]
    fn navigate_pointer_errors_on_missing_key() {
        let doc = json!({"data": {}});
        let tokens = parse_pointer("/data/users", 256, 10).unwrap();
        let result = navigate_pointer(&doc, &tokens, "/data/users");
        assert!(matches!(result, Err(WrapperError::PointerNotFound { .. })));
    }

    #[test]
    fn navigate_pointer_errors_on_wrong_type() {
        let doc = json!({"data": "string"});
        let tokens = parse_pointer("/data/field", 256, 10).unwrap();
        let result = navigate_pointer(&doc, &tokens, "/data/field");
        assert!(matches!(result, Err(WrapperError::PointerNotFound { .. })));
    }

    #[test]
    fn navigate_pointer_with_escaped_keys() {
        let doc = json!({
            "field/name": {"value": 42}
        });

        let tokens = parse_pointer("/field~1name/value", 256, 10).unwrap();
        let result = navigate_pointer(&doc, &tokens, "/field~1name/value").unwrap();
        assert_eq!(result, &json!(42));
    }
}
