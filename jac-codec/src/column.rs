//! Column builder for converting records to columnar format

use crate::CompressOpts;
use jac_format::{
    bitpack::{PresenceBitmap, TagPacker},
    varint::{encode_uleb128, zigzag_encode},
    Decimal, JacError, Limits, Result, TypeTag,
};
use serde_json;
use std::cmp::{max, min};
use std::collections::HashMap;

type DictionaryBuild = (Option<(Vec<String>, HashMap<String, usize>)>, bool);

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
    /// Current position in present values
    present_idx: usize,
    /// Security limits snapshot
    limits: Limits,
    /// Maximum dictionary entries permitted for this field
    max_dict_entries: usize,
    /// Canonicalize numbers flag
    canonicalize_numbers: bool,
}

impl ColumnBuilder {
    /// Create new column builder
    pub fn new(record_count: usize, opts: &CompressOpts) -> Self {
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
            present_idx: 0,
            limits: opts.limits.clone(),
            max_dict_entries: opts.max_dict_entries,
            canonicalize_numbers: opts.canonicalize_numbers,
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
                        let decimal = self.build_decimal(&n.to_string())?;
                        self.tags.push(TypeTag::Decimal);
                        self.decimals.push(decimal);
                    }
                } else {
                    // f64 or non-integer, use decimal
                    let decimal = self.build_decimal(&n.to_string())?;
                    self.tags.push(TypeTag::Decimal);
                    self.decimals.push(decimal);
                }
                self.present_idx += 1;
            }
            serde_json::Value::String(s) => {
                self.presence.set_present(record_idx, true);
                self.tags.push(TypeTag::String);
                self.push_string(s)?;
                self.present_idx += 1;
            }
            serde_json::Value::Object(obj) => {
                self.presence.set_present(record_idx, true);
                self.tags.push(TypeTag::Object);
                // Minify JSON
                let minified = serde_json::to_vec(obj)
                    .map_err(|e| JacError::Internal(format!("JSON serialization failed: {}", e)))?;
                self.ensure_string_len(minified.len())?;
                self.objects.push(minified);
                self.present_idx += 1;
            }
            serde_json::Value::Array(arr) => {
                self.presence.set_present(record_idx, true);
                self.tags.push(TypeTag::Array);
                // Minify JSON
                let minified = serde_json::to_vec(arr)
                    .map_err(|e| JacError::Internal(format!("JSON serialization failed: {}", e)))?;
                self.ensure_string_len(minified.len())?;
                self.arrays.push(minified);
                self.present_idx += 1;
            }
        }

        Ok(())
    }

    /// Finalize column and create field segment
    pub fn finalize(self, _opts: &CompressOpts, record_count: usize) -> Result<FieldSegment> {
        let mut trimmed_presence = PresenceBitmap::new(record_count);
        for idx in 0..record_count {
            if self.presence.is_present(idx) {
                trimmed_presence.set_present(idx, true);
            }
        }

        let present_count = trimmed_presence.count_present();

        // Validate limits
        if present_count > self.limits.max_records_per_block {
            return Err(JacError::LimitExceeded(
                "Too many present values".to_string(),
            ));
        }

        // Build string dictionary if beneficial
        let (dictionary_info, use_dict) = self.build_string_dictionary()?;
        let dict_entry_count = dictionary_info
            .as_ref()
            .map(|(entries, _)| entries.len())
            .unwrap_or(0);

        if dict_entry_count > self.limits.max_dict_entries_per_field {
            return Err(JacError::LimitExceeded(
                "Dictionary entry count exceeds limit".to_string(),
            ));
        }

        // Build uncompressed payload in spec order (§4.7)
        let mut payload = Vec::new();

        // 1. Presence bitmap
        let presence_bytes = trimmed_presence.to_bytes();
        if presence_bytes.len() > self.limits.max_presence_bytes {
            return Err(JacError::LimitExceeded(
                "Presence bytes exceed limit".to_string(),
            ));
        }
        payload.extend_from_slice(&presence_bytes);

        // 2. Type tag stream (3-bit packed)
        let mut tag_packer = TagPacker::new();
        for tag in &self.tags {
            tag_packer.push(*tag as u8);
        }
        let tag_bytes = tag_packer.finish();
        if tag_bytes.len() > self.limits.max_tag_bytes {
            return Err(JacError::LimitExceeded(
                "Tag bytes exceed limit".to_string(),
            ));
        }
        payload.extend_from_slice(&tag_bytes);

        // 3. String dictionary (if using dictionary encoding)
        if use_dict {
            if let Some((entries, _)) = &dictionary_info {
                for string in entries {
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
                if let Some((_, dict_map)) = &dictionary_info {
                    for string in &self.strings {
                        if let Some(&idx) = dict_map.get(string) {
                            payload.extend_from_slice(&encode_uleb128(idx as u64));
                        } else {
                            return Err(JacError::Internal(
                                "String not found in dictionary".to_string(),
                            ));
                        }
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
            dict_entry_count,
            value_count_present: present_count,
        })
    }

    /// Build string dictionary if beneficial
    fn build_string_dictionary(&self) -> Result<DictionaryBuild> {
        if self.strings.is_empty() {
            return Ok((None, false));
        }

        // Determine distinct strings in first-occurrence order
        let mut dict_entries = Vec::new();
        let mut dict_map = HashMap::new();
        for string in &self.strings {
            if dict_map.contains_key(string) {
                continue;
            }
            let index = dict_entries.len();
            dict_entries.push(string.clone());
            dict_map.insert(string.clone(), index);
        }

        let distinct_count = dict_entries.len();
        let dict_limit = min(
            self.max_dict_entries,
            self.limits.max_dict_entries_per_field,
        );
        if dict_limit == 0 {
            return Ok((None, false));
        }

        // Per SPEC §4 dictionary encoding uses threshold max(2, total_strings / 4)
        let threshold = min(dict_limit, max(2, self.strings.len() / 4));

        if distinct_count <= threshold {
            Ok((Some((dict_entries, dict_map)), true))
        } else {
            Ok((None, false))
        }
    }

    /// Check if delta encoding is beneficial for integers
    fn should_use_delta_encoding(&self, ints: &[i64]) -> bool {
        if ints.len() < 2 {
            return false;
        }

        for i in 1..ints.len() {
            if ints[i] <= ints[i - 1] {
                return false;
            }
        }

        let mut min_delta = i64::MAX;
        let mut max_delta = i64::MIN;
        for i in 1..ints.len() {
            let delta = ints[i] - ints[i - 1];
            min_delta = min(min_delta, delta);
            max_delta = max(max_delta, delta);
        }

        let first = *ints.first().unwrap() as i128;
        let last = *ints.last().unwrap() as i128;
        let range = last - first;
        if range == 0 {
            return false;
        }

        let delta_span = (max_delta as i128) - (min_delta as i128);
        let delta_ratio = delta_span as f64 / range as f64;
        delta_ratio < 0.5
    }

    fn ensure_string_len(&self, len: usize) -> Result<()> {
        if len > self.limits.max_string_len_per_value {
            Err(JacError::LimitExceeded(
                "String length exceeds limit".to_string(),
            ))
        } else {
            Ok(())
        }
    }

    fn push_string(&mut self, value: &str) -> Result<()> {
        self.ensure_string_len(value.len())?;
        self.strings.push(value.to_string());
        Ok(())
    }

    fn build_decimal(&self, source: &str) -> Result<Decimal> {
        let mut decimal = Decimal::from_str_exact(source)?;
        self.ensure_decimal_limit(&decimal)?;

        if self.canonicalize_numbers {
            self.trim_decimal_trailing_zeros(&mut decimal);
            self.ensure_decimal_limit(&decimal)?;
        }

        Ok(decimal)
    }

    fn ensure_decimal_limit(&self, decimal: &Decimal) -> Result<()> {
        if decimal.digits.len() > self.limits.max_decimal_digits_per_value {
            Err(JacError::LimitExceeded(
                "Decimal digit count exceeds limit".to_string(),
            ))
        } else {
            Ok(())
        }
    }

    fn trim_decimal_trailing_zeros(&self, decimal: &mut Decimal) {
        while decimal.digits.len() > 1
            && decimal.digits.last() == Some(&b'0')
            && decimal.exponent < i32::MAX
        {
            decimal.digits.pop();
            decimal.exponent += 1;
        }
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
                zstd::encode_all(self.uncompressed_payload.as_slice(), level as i32).map_err(|e| {
                    JacError::DecompressError(format!("Zstd compression failed: {}", e))
                })
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
    use serde_json::{json, Number};

    #[test]
    fn test_column_builder_basic() {
        let opts = CompressOpts::default();
        let mut builder = ColumnBuilder::new(3, &opts);

        // Add some test values
        builder.add_value(0, &json!(42)).unwrap();
        builder.add_value(1, &json!("hello")).unwrap();
        builder.add_value(2, &json!(null)).unwrap();

        let segment = builder.finalize(&opts, 3).unwrap();
        assert_eq!(segment.value_count_present, 3);
        assert!(!segment.uncompressed_payload.is_empty());
    }

    #[test]
    fn test_column_builder_mixed_types() {
        let opts = CompressOpts::default();
        let mut builder = ColumnBuilder::new(5, &opts);

        // Add mixed types
        builder.add_value(0, &json!(true)).unwrap();
        builder.add_value(1, &json!(123)).unwrap();
        builder.add_value(2, &json!(45.67)).unwrap();
        builder.add_value(3, &json!("world")).unwrap();
        // Skip index 4 (absent)

        let segment = builder.finalize(&opts, 5).unwrap();
        assert_eq!(segment.value_count_present, 4);
        assert!(!segment.uncompressed_payload.is_empty());
    }

    #[test]
    fn test_column_builder_dictionary_encoding() {
        let opts = CompressOpts::default();
        let mut builder = ColumnBuilder::new(8, &opts);

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

        let segment = builder.finalize(&opts, 8).unwrap();
        assert_eq!(segment.value_count_present, 8);
        assert!(segment.encoding_flags & 1 != 0); // DICTIONARY flag should be set
        assert_eq!(segment.dict_entry_count, 2); // "hello" and "world"
    }

    #[test]
    fn test_column_builder_delta_encoding() {
        let opts = CompressOpts::default();
        let mut builder = ColumnBuilder::new(5, &opts);

        // Add monotonic integers to trigger delta encoding
        builder.add_value(0, &json!(1000)).unwrap();
        builder.add_value(1, &json!(1001)).unwrap();
        builder.add_value(2, &json!(1002)).unwrap();
        builder.add_value(3, &json!(1003)).unwrap();
        builder.add_value(4, &json!(1004)).unwrap();

        let segment = builder.finalize(&opts, 5).unwrap();
        assert_eq!(segment.value_count_present, 5);
        assert!(segment.encoding_flags & 2 != 0); // DELTA flag should be set
    }

    #[test]
    fn test_column_builder_objects_and_arrays() {
        let opts = CompressOpts::default();
        let mut builder = ColumnBuilder::new(2, &opts);

        // Add object and array
        builder.add_value(0, &json!({"key": "value"})).unwrap();
        builder.add_value(1, &json!([1, 2, 3])).unwrap();

        let segment = builder.finalize(&opts, 2).unwrap();
        assert_eq!(segment.value_count_present, 2);
        assert!(!segment.uncompressed_payload.is_empty());
    }

    #[test]
    fn test_column_builder_large_integers() {
        let opts = CompressOpts::default();
        let mut builder = ColumnBuilder::new(2, &opts);

        // Test large integers that should use decimal encoding
        builder.add_value(0, &json!(i64::MAX)).unwrap();
        builder.add_value(1, &json!(i64::MAX as u64 + 1)).unwrap();

        let segment = builder.finalize(&opts, 2).unwrap();
        assert_eq!(segment.value_count_present, 2);
        assert!(!segment.uncompressed_payload.is_empty());
    }

    #[test]
    fn test_column_builder_canonicalizes_decimals_when_enabled() {
        let mut opts = CompressOpts::default();
        opts.canonicalize_numbers = true;
        let mut builder = ColumnBuilder::new(1, &opts);

        let number: Number = "1.2300".parse().unwrap();
        builder
            .add_value(0, &serde_json::Value::Number(number))
            .unwrap();

        let segment = builder.finalize(&opts, 1).unwrap();
        let presence_len = (1 + 7) >> 3;
        let tag_len = ((3 * segment.value_count_present) + 7) >> 3;
        let start = presence_len + tag_len;
        let (decimal, consumed) = Decimal::decode(&segment.uncompressed_payload[start..]).unwrap();
        assert!(consumed > 0);
        assert_eq!(decimal.to_json_string(), "1.23");
    }

    #[test]
    fn test_column_builder_string_length_limit_enforced() {
        let mut opts = CompressOpts::default();
        opts.limits.max_string_len_per_value = 4;
        let mut builder = ColumnBuilder::new(1, &opts);

        let err = builder.add_value(0, &json!("hello"));
        assert!(matches!(err, Err(JacError::LimitExceeded(_))));
    }

    #[test]
    fn test_column_builder_decimal_digit_limit_enforced() {
        let mut opts = CompressOpts::default();
        opts.limits.max_decimal_digits_per_value = 3;
        let mut builder = ColumnBuilder::new(1, &opts);

        let decimal_value: serde_json::Value = serde_json::from_str("1234.5").unwrap();
        let err = builder.add_value(0, &decimal_value);
        assert!(matches!(err, Err(JacError::LimitExceeded(_))));
    }

    #[test]
    fn test_column_builder_dictionary_threshold_falls_back_to_raw() {
        let opts = CompressOpts::default();
        let mut builder = ColumnBuilder::new(9, &opts);

        for idx in 0..9 {
            builder
                .add_value(idx, &json!(format!("value{}", idx)))
                .unwrap();
        }

        let segment = builder.finalize(&opts, 9).unwrap();
        assert_eq!(segment.encoding_flags & 1, 0);
    }

    #[test]
    fn test_column_builder_presence_limit_enforced() {
        let mut opts = CompressOpts::default();
        opts.limits.max_presence_bytes = 0;
        let mut builder = ColumnBuilder::new(1, &opts);
        builder.add_value(0, &json!(true)).unwrap();

        let err = builder.finalize(&opts, 1).unwrap_err();
        assert!(matches!(err, JacError::LimitExceeded(msg) if msg.contains("Presence bytes")));
    }

    #[test]
    fn test_column_builder_tag_limit_enforced() {
        let mut opts = CompressOpts::default();
        opts.limits.max_tag_bytes = 0;
        let mut builder = ColumnBuilder::new(1, &opts);
        builder.add_value(0, &json!(true)).unwrap();

        let err = builder.finalize(&opts, 1).unwrap_err();
        assert!(matches!(err, JacError::LimitExceeded(msg) if msg.contains("Tag bytes")));
    }
}
