//! Column builder for converting records to columnar format

use crate::CompressOpts;
use jac_format::{
    TypeTag, Decimal, Result, JacError,
    varint::{encode_uleb128, zigzag_encode},
    bitpack::{PresenceBitmap, TagPacker},
};
use serde_json;
use std::collections::HashMap;

/// Column builder for a single field across records
#[derive(Clone)]
pub struct ColumnBuilder {
    /// Total number of records in the block
    record_count: usize,
    /// Presence bitmap (1 = present, 0 = absent)
    presence: PresenceBitmap,
    /// Type tags for present values only
    tags: Vec<TypeTag>,
    /// Boolean values (bit-packed)
    bools: Vec<bool>,
    /// Integer values (varint/delta encoded)
    ints: Vec<i64>,
    /// Decimal values (exact representation)
    decimals: Vec<Decimal>,
    /// String values (for dictionary or raw)
    strings: Vec<String>,
    /// Object values (minified JSON)
    objects: Vec<Vec<u8>>,
    /// Array values (minified JSON)
    arrays: Vec<Vec<u8>>,
    /// String dictionary (if using dictionary encoding)
    string_dict: Option<HashMap<String, usize>>,
    /// Current position in present values
    present_idx: usize,
}

impl ColumnBuilder {
    /// Create new column builder
    pub fn new(record_count: usize) -> Self {
        Self {
            record_count,
            presence: PresenceBitmap::new(record_count),
            tags: Vec::new(),
            bools: Vec::new(),
            ints: Vec::new(),
            decimals: Vec::new(),
            strings: Vec::new(),
            objects: Vec::new(),
            arrays: Vec::new(),
            string_dict: None,
            present_idx: 0,
        }
    }

    /// Add value to column
    pub fn add_value(&mut self, record_idx: usize, value: &serde_json::Value) -> Result<()> {
        if record_idx >= self.record_count {
            return Err(JacError::Internal("Record index out of bounds".to_string()));
        }

        match value {
            serde_json::Value::Null => {
                self.presence.set_present(record_idx, true);
                self.tags.push(TypeTag::Null);
                self.present_idx += 1;
            }
            serde_json::Value::Bool(b) => {
                self.presence.set_present(record_idx, true);
                self.tags.push(TypeTag::Bool);
                self.bools.push(*b);
                self.present_idx += 1;
            }
            serde_json::Value::Number(n) => {
                self.presence.set_present(record_idx, true);

                // Integer detection logic per spec
                if let Some(i) = n.as_i64() {
                    self.tags.push(TypeTag::Int);
                    self.ints.push(i);
                } else if let Some(u) = n.as_u64() {
                    if u <= i64::MAX as u64 {
                        self.tags.push(TypeTag::Int);
                        self.ints.push(u as i64);
                    } else {
                        // u64 > i64::MAX, must use decimal
                        let decimal = Decimal::from_str_exact(&n.to_string())?;
                        self.tags.push(TypeTag::Decimal);
                        self.decimals.push(decimal);
                    }
                } else {
                    // f64 or non-integer, use decimal
                    let decimal = Decimal::from_str_exact(&n.to_string())?;
                    self.tags.push(TypeTag::Decimal);
                    self.decimals.push(decimal);
                }
                self.present_idx += 1;
            }
            serde_json::Value::String(s) => {
                self.presence.set_present(record_idx, true);
                self.tags.push(TypeTag::String);
                self.strings.push(s.clone());
                self.present_idx += 1;
            }
            serde_json::Value::Object(obj) => {
                self.presence.set_present(record_idx, true);
                self.tags.push(TypeTag::Object);
                // Minify JSON
                let minified = serde_json::to_vec(obj)
                    .map_err(|e| JacError::Internal(format!("JSON serialization failed: {}", e)))?;
                self.objects.push(minified);
                self.present_idx += 1;
            }
            serde_json::Value::Array(arr) => {
                self.presence.set_present(record_idx, true);
                self.tags.push(TypeTag::Array);
                // Minify JSON
                let minified = serde_json::to_vec(arr)
                    .map_err(|e| JacError::Internal(format!("JSON serialization failed: {}", e)))?;
                self.arrays.push(minified);
                self.present_idx += 1;
            }
        }

        Ok(())
    }

    /// Finalize column and create field segment
    pub fn finalize(self, opts: &CompressOpts) -> Result<FieldSegment> {
        let present_count = self.presence.count_present();

        // Validate limits
        if present_count > opts.limits.max_records_per_block {
            return Err(JacError::LimitExceeded("Too many present values".to_string()));
        }

        // Build string dictionary if beneficial
        let (string_dict, use_dict) = self.build_string_dictionary(opts)?;

        // Build uncompressed payload in spec order (§4.7)
        let mut payload = Vec::new();

        // 1. Presence bitmap
        payload.extend_from_slice(&self.presence.to_bytes());

        // 2. Type tag stream (3-bit packed)
        let mut tag_packer = TagPacker::new();
        for tag in &self.tags {
            tag_packer.push(*tag as u8);
        }
        payload.extend_from_slice(&tag_packer.finish());

        // 3. String dictionary (if using dictionary encoding)
        if use_dict {
            if let Some(dict) = &string_dict {
                // Write dictionary entries
                for (_, string) in dict.iter().map(|(k, v)| (v, k)).collect::<Vec<_>>() {
                    let string_bytes = string.as_bytes();
                    payload.extend_from_slice(&encode_uleb128(string_bytes.len() as u64));
                    payload.extend_from_slice(string_bytes);
                }
            }
        }

        // 4. Boolean substream (bit-packed)
        if !self.bools.is_empty() {
            let bool_bitmap = PresenceBitmap::from_bools(&self.bools);
            payload.extend_from_slice(&bool_bitmap.to_bytes());
        }

        // 5. Integer substream (varint/delta)
        if !self.ints.is_empty() {
            // Check if delta encoding is beneficial
            let use_delta = self.should_use_delta_encoding(&self.ints);
            if use_delta {
                // Write base value first, then deltas
                if let Some(&base) = self.ints.first() {
                    payload.extend_from_slice(&encode_uleb128(zigzag_encode(base)));
                    for i in 1..self.ints.len() {
                        let delta = self.ints[i] - self.ints[i - 1];
                        payload.extend_from_slice(&encode_uleb128(zigzag_encode(delta)));
                    }
                }
            } else {
                // Regular varint encoding
                for &val in &self.ints {
                    payload.extend_from_slice(&encode_uleb128(zigzag_encode(val)));
                }
            }
        }

        // 6. Decimal substream
        for decimal in &self.decimals {
            payload.extend_from_slice(&decimal.encode()?);
        }

        // 7. String substream (dictionary indices or raw strings)
        if !self.strings.is_empty() {
            if use_dict {
                // Write dictionary indices
                for string in &self.strings {
                    if let Some(&idx) = string_dict.as_ref().unwrap().get(string) {
                        payload.extend_from_slice(&encode_uleb128(idx as u64));
                    } else {
                        return Err(JacError::Internal("String not found in dictionary".to_string()));
                    }
                }
            } else {
                // Write raw strings
                for string in &self.strings {
                    let string_bytes = string.as_bytes();
                    payload.extend_from_slice(&encode_uleb128(string_bytes.len() as u64));
                    payload.extend_from_slice(string_bytes);
                }
            }
        }

        // Add object and array data to string substream (they share it)
        for obj_bytes in &self.objects {
            payload.extend_from_slice(&encode_uleb128(obj_bytes.len() as u64));
            payload.extend_from_slice(obj_bytes);
        }

        for arr_bytes in &self.arrays {
            payload.extend_from_slice(&encode_uleb128(arr_bytes.len() as u64));
            payload.extend_from_slice(arr_bytes);
        }

        // Set encoding flags
        let mut encoding_flags = 0u64;
        if use_dict {
            encoding_flags |= 1 << 0; // ENCODING_FLAG_DICTIONARY
        }
        if !self.ints.is_empty() && self.should_use_delta_encoding(&self.ints) {
            encoding_flags |= 1 << 1; // ENCODING_FLAG_DELTA
        }

        Ok(FieldSegment {
            uncompressed_payload: payload,
            encoding_flags,
            dict_entry_count: string_dict.as_ref().map(|d| d.len()).unwrap_or(0),
            value_count_present: present_count,
        })
    }

    /// Build string dictionary if beneficial
    fn build_string_dictionary(&self, opts: &CompressOpts) -> Result<(Option<HashMap<String, usize>>, bool)> {
        if self.strings.is_empty() {
            return Ok((None, false));
        }

        // Count distinct strings
        let mut string_counts = HashMap::new();
        for string in &self.strings {
            *string_counts.entry(string.clone()).or_insert(0) += 1;
        }

        let distinct_count = string_counts.len();
        let threshold = std::cmp::min(opts.max_dict_entries, std::cmp::max(2, self.strings.len() / 4));

        if distinct_count <= threshold {
            // Build dictionary
            let mut dict = HashMap::new();
            let mut idx = 0;
            for (string, _) in string_counts {
                dict.insert(string, idx);
                idx += 1;
            }
            Ok((Some(dict), true))
        } else {
            Ok((None, false))
        }
    }

    /// Check if delta encoding is beneficial for integers
    fn should_use_delta_encoding(&self, ints: &[i64]) -> bool {
        if ints.len() < 2 {
            return false;
        }

        // Check if sequence is monotonic increasing
        let mut increasing_count = 0;
        for i in 1..ints.len() {
            if ints[i] > ints[i - 1] {
                increasing_count += 1;
            }
        }

        let increasing_ratio = increasing_count as f64 / (ints.len() - 1) as f64;
        increasing_ratio >= 0.95
    }
}

/// Field segment containing encoded data
#[derive(Debug, Clone)]
pub struct FieldSegment {
    /// Uncompressed payload bytes
    pub uncompressed_payload: Vec<u8>,
    /// Encoding flags bitfield
    pub encoding_flags: u64,
    /// Number of dictionary entries
    pub dict_entry_count: usize,
    /// Number of present values
    pub value_count_present: usize,
}

impl FieldSegment {
    /// Compress segment using specified codec and level
    pub fn compress(&self, codec: u8, level: u8) -> Result<Vec<u8>> {
        match codec {
            0 => {
                // No compression
                Ok(self.uncompressed_payload.clone())
            }
            1 => {
                // Zstandard compression
                zstd::encode_all(self.uncompressed_payload.as_slice(), level as i32)
                    .map_err(|e| JacError::DecompressError(format!("Zstd compression failed: {}", e)))
            }
            2 => {
                // Brotli (not implemented in v0.1.0)
                Err(JacError::UnsupportedCompression(2))
            }
            3 => {
                // Deflate (not implemented in v0.1.0)
                Err(JacError::UnsupportedCompression(3))
            }
            _ => Err(JacError::UnsupportedCompression(codec)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_column_builder_basic() {
        let mut builder = ColumnBuilder::new(3);
        let opts = CompressOpts::default();

        // Add some test values
        builder.add_value(0, &json!(42)).unwrap();
        builder.add_value(1, &json!("hello")).unwrap();
        builder.add_value(2, &json!(null)).unwrap();

        let segment = builder.finalize(&opts).unwrap();
        assert_eq!(segment.value_count_present, 3);
        assert!(!segment.uncompressed_payload.is_empty());
    }

    #[test]
    fn test_column_builder_mixed_types() {
        let mut builder = ColumnBuilder::new(5);
        let opts = CompressOpts::default();

        // Add mixed types
        builder.add_value(0, &json!(true)).unwrap();
        builder.add_value(1, &json!(123)).unwrap();
        builder.add_value(2, &json!(45.67)).unwrap();
        builder.add_value(3, &json!("world")).unwrap();
        // Skip index 4 (absent)

        let segment = builder.finalize(&opts).unwrap();
        assert_eq!(segment.value_count_present, 4);
        assert!(!segment.uncompressed_payload.is_empty());
    }

    #[test]
    fn test_column_builder_dictionary_encoding() {
        let mut builder = ColumnBuilder::new(8);
        let opts = CompressOpts::default();

        // Add repeated strings to trigger dictionary encoding
        // With 8 strings and 2 distinct values, threshold = min(4096, 8/8) = 1
        builder.add_value(0, &json!("hello")).unwrap();
        builder.add_value(1, &json!("world")).unwrap();
        builder.add_value(2, &json!("hello")).unwrap();
        builder.add_value(3, &json!("world")).unwrap();
        builder.add_value(4, &json!("hello")).unwrap();
        builder.add_value(5, &json!("world")).unwrap();
        builder.add_value(6, &json!("hello")).unwrap();
        builder.add_value(7, &json!("world")).unwrap();

        let segment = builder.finalize(&opts).unwrap();
        assert_eq!(segment.value_count_present, 8);
        assert!(segment.encoding_flags & 1 != 0); // DICTIONARY flag should be set
        assert_eq!(segment.dict_entry_count, 2); // "hello" and "world"
    }

    #[test]
    fn test_column_builder_delta_encoding() {
        let mut builder = ColumnBuilder::new(5);
        let opts = CompressOpts::default();

        // Add monotonic integers to trigger delta encoding
        builder.add_value(0, &json!(1000)).unwrap();
        builder.add_value(1, &json!(1001)).unwrap();
        builder.add_value(2, &json!(1002)).unwrap();
        builder.add_value(3, &json!(1003)).unwrap();
        builder.add_value(4, &json!(1004)).unwrap();

        let segment = builder.finalize(&opts).unwrap();
        assert_eq!(segment.value_count_present, 5);
        assert!(segment.encoding_flags & 2 != 0); // DELTA flag should be set
    }

    #[test]
    fn test_column_builder_objects_and_arrays() {
        let mut builder = ColumnBuilder::new(2);
        let opts = CompressOpts::default();

        // Add object and array
        builder.add_value(0, &json!({"key": "value"})).unwrap();
        builder.add_value(1, &json!([1, 2, 3])).unwrap();

        let segment = builder.finalize(&opts).unwrap();
        assert_eq!(segment.value_count_present, 2);
        assert!(!segment.uncompressed_payload.is_empty());
    }

    #[test]
    fn test_column_builder_large_integers() {
        let mut builder = ColumnBuilder::new(2);
        let opts = CompressOpts::default();

        // Test large integers that should use decimal encoding
        builder.add_value(0, &json!(i64::MAX)).unwrap();
        builder.add_value(1, &json!(i64::MAX as u64 + 1)).unwrap();

        let segment = builder.finalize(&opts).unwrap();
        assert_eq!(segment.value_count_present, 2);
        assert!(!segment.uncompressed_payload.is_empty());
    }
}
