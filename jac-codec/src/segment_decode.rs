//! Field segment decoder

use std::convert::TryFrom;

use bitvec::prelude::*;
use jac_format::{
    bitpack::{PresenceBitmap, TagUnpacker},
    constants::{ENCODING_FLAG_DELTA, ENCODING_FLAG_DICTIONARY},
    decimal::Decimal,
    varint::{decode_uleb128, zigzag_decode},
    FieldDirectoryEntry, JacError, Limits, Result, TypeTag,
};
use serde_json::{self, Value};

/// Field segment decoder capable of projecting values from a single field
pub struct FieldSegmentDecoder {
    record_count: usize,
    values: Vec<Option<Value>>,
}

impl FieldSegmentDecoder {
    /// Create a new segment decoder from the compressed payload
    pub fn new(
        compressed: &[u8],
        dir_entry: &FieldDirectoryEntry,
        record_count: usize,
        limits: &Limits,
    ) -> Result<Self> {
        if dir_entry.segment_uncompressed_len > limits.max_segment_uncompressed_len {
            return Err(JacError::LimitExceeded(format!(
                "Segment uncompressed length {} exceeds limit {}",
                dir_entry.segment_uncompressed_len, limits.max_segment_uncompressed_len
            )));
        }

        // Decompress payload according to compressor
        let mut decompressed = Vec::with_capacity(dir_entry.segment_uncompressed_len);
        match dir_entry.compressor {
            0 => {
                if compressed.len() != dir_entry.segment_compressed_len {
                    return Err(JacError::CorruptBlock);
                }
                decompressed.extend_from_slice(compressed);
            }
            1 => {
                decompressed = zstd::decode_all(compressed).map_err(|e| {
                    JacError::DecompressError(format!("Zstd decompression failed: {}", e))
                })?;
            }
            other => return Err(JacError::UnsupportedCompression(other)),
        }

        if decompressed.len() != dir_entry.segment_uncompressed_len {
            return Err(JacError::CorruptBlock);
        }

        if dir_entry.presence_bytes > limits.max_presence_bytes {
            return Err(JacError::LimitExceeded(format!(
                "Presence bytes {} exceeds limit {}",
                dir_entry.presence_bytes, limits.max_presence_bytes
            )));
        }
        if dir_entry.tag_bytes > limits.max_tag_bytes {
            return Err(JacError::LimitExceeded(format!(
                "Tag bytes {} exceeds limit {}",
                dir_entry.tag_bytes, limits.max_tag_bytes
            )));
        }

        let expected_presence_bytes = (record_count + 7) >> 3;
        if dir_entry.presence_bytes != expected_presence_bytes {
            return Err(JacError::CorruptBlock);
        }

        let expected_tag_bytes = ((3 * dir_entry.value_count_present) + 7) >> 3;
        if dir_entry.tag_bytes != expected_tag_bytes {
            return Err(JacError::CorruptBlock);
        }

        let mut cursor = 0;

        let presence_end = cursor + dir_entry.presence_bytes;
        if presence_end > decompressed.len() {
            return Err(JacError::UnexpectedEof);
        }
        let presence =
            PresenceBitmap::from_bytes(&decompressed[cursor..presence_end], record_count);
        cursor = presence_end;

        let present_count = presence.count_present();
        if present_count != dir_entry.value_count_present {
            return Err(JacError::CorruptBlock);
        }

        let tag_end = cursor + dir_entry.tag_bytes;
        if tag_end > decompressed.len() {
            return Err(JacError::UnexpectedEof);
        }
        let mut tag_unpacker = TagUnpacker::new(&decompressed[cursor..tag_end], present_count);
        cursor = tag_end;

        let mut tags = Vec::with_capacity(present_count);
        for raw in &mut tag_unpacker {
            let tag = TypeTag::from_u8(raw)?;
            tags.push(tag);
        }

        if tags.len() != present_count {
            return Err(JacError::CorruptBlock);
        }

        // Dictionary entries (if any)
        let has_dictionary = dir_entry.encoding_flags & ENCODING_FLAG_DICTIONARY != 0;
        let mut dictionary = Vec::new();
        if has_dictionary {
            if dir_entry.dict_entry_count == 0 {
                return Err(JacError::CorruptBlock);
            }

            for _ in 0..dir_entry.dict_entry_count {
                let (len_raw, len_bytes) = decode_uleb128(&decompressed[cursor..])?;
                cursor += len_bytes;
                let string_len = usize::try_from(len_raw).map_err(|_| JacError::CorruptBlock)?;
                if string_len > limits.max_string_len_per_value {
                    return Err(JacError::LimitExceeded(format!(
                        "Dictionary string length {} exceeds limit {}",
                        string_len, limits.max_string_len_per_value
                    )));
                }

                let end = cursor + string_len;
                if end > decompressed.len() {
                    return Err(JacError::UnexpectedEof);
                }

                let entry = std::str::from_utf8(&decompressed[cursor..end])
                    .map_err(|_| JacError::CorruptBlock)?
                    .to_string();
                dictionary.push(entry);
                cursor = end;
            }

            if dictionary.len() != dir_entry.dict_entry_count {
                return Err(JacError::CorruptBlock);
            }
        } else if dir_entry.dict_entry_count != 0 {
            return Err(JacError::CorruptBlock);
        }

        // Count tags for substream sizing
        let bool_count = tags
            .iter()
            .filter(|tag| matches!(tag, TypeTag::Bool))
            .count();
        let int_count = tags
            .iter()
            .filter(|tag| matches!(tag, TypeTag::Int))
            .count();
        let decimal_count = tags
            .iter()
            .filter(|tag| matches!(tag, TypeTag::Decimal))
            .count();
        let string_count = tags
            .iter()
            .filter(|tag| matches!(tag, TypeTag::String))
            .count();
        let object_count = tags
            .iter()
            .filter(|tag| matches!(tag, TypeTag::Object))
            .count();
        let array_count = tags
            .iter()
            .filter(|tag| matches!(tag, TypeTag::Array))
            .count();

        // Boolean substream
        let mut bool_values = Vec::with_capacity(bool_count);
        if bool_count > 0 {
            let bool_bytes = (bool_count + 7) >> 3;
            let end = cursor + bool_bytes;
            if end > decompressed.len() {
                return Err(JacError::UnexpectedEof);
            }
            let bits = BitVec::<u8, Lsb0>::from_slice(&decompressed[cursor..end]);
            bool_values.extend(bits.iter().take(bool_count).map(|bit| *bit));
            cursor = end;
        }

        // Integer substream
        let mut int_values = Vec::with_capacity(int_count);
        if int_count > 0 {
            if dir_entry.encoding_flags & ENCODING_FLAG_DELTA != 0 {
                let (base_raw, base_bytes) = decode_uleb128(&decompressed[cursor..])?;
                cursor += base_bytes;
                let mut current = zigzag_decode(base_raw);
                int_values.push(current);
                for _ in 1..int_count {
                    let (delta_raw, delta_bytes) = decode_uleb128(&decompressed[cursor..])?;
                    cursor += delta_bytes;
                    let delta = zigzag_decode(delta_raw);
                    current = current
                        .checked_add(delta)
                        .ok_or_else(|| JacError::CorruptBlock)?;
                    int_values.push(current);
                }
            } else {
                for _ in 0..int_count {
                    let (value_raw, value_bytes) = decode_uleb128(&decompressed[cursor..])?;
                    cursor += value_bytes;
                    let value = zigzag_decode(value_raw);
                    int_values.push(value);
                }
            }
        }

        // Decimal substream
        let mut decimal_values = Vec::with_capacity(decimal_count);
        for _ in 0..decimal_count {
            let (decimal, consumed) = Decimal::decode(&decompressed[cursor..])?;
            if decimal.digits.len() > limits.max_decimal_digits_per_value {
                return Err(JacError::LimitExceeded(format!(
                    "Decimal digit length {} exceeds limit {}",
                    decimal.digits.len(),
                    limits.max_decimal_digits_per_value
                )));
            }
            cursor += consumed;
            decimal_values.push(decimal);
        }

        // String substream (shared for strings, objects, arrays)
        let mut string_values = Vec::with_capacity(string_count);
        if has_dictionary {
            for _ in 0..string_count {
                let (index_raw, index_bytes) = decode_uleb128(&decompressed[cursor..])?;
                cursor += index_bytes;
                let index = usize::try_from(index_raw).map_err(|_| JacError::CorruptBlock)?;
                let value = dictionary
                    .get(index)
                    .ok_or(JacError::DictionaryError)?
                    .clone();
                string_values.push(value);
            }
        } else {
            for _ in 0..string_count {
                let (len_raw, len_bytes) = decode_uleb128(&decompressed[cursor..])?;
                cursor += len_bytes;
                let string_len = usize::try_from(len_raw).map_err(|_| JacError::CorruptBlock)?;
                if string_len > limits.max_string_len_per_value {
                    return Err(JacError::LimitExceeded(format!(
                        "String length {} exceeds limit {}",
                        string_len, limits.max_string_len_per_value
                    )));
                }
                let end = cursor + string_len;
                if end > decompressed.len() {
                    return Err(JacError::UnexpectedEof);
                }
                let value = std::str::from_utf8(&decompressed[cursor..end])
                    .map_err(|_| JacError::CorruptBlock)?
                    .to_string();
                cursor = end;
                string_values.push(value);
            }
        }

        let mut object_values = Vec::with_capacity(object_count);
        for _ in 0..object_count {
            let (len_raw, len_bytes) = decode_uleb128(&decompressed[cursor..])?;
            cursor += len_bytes;
            let json_len = usize::try_from(len_raw).map_err(|_| JacError::CorruptBlock)?;
            if json_len > limits.max_string_len_per_value {
                return Err(JacError::LimitExceeded(format!(
                    "Object length {} exceeds limit {}",
                    json_len, limits.max_string_len_per_value
                )));
            }
            let end = cursor + json_len;
            if end > decompressed.len() {
                return Err(JacError::UnexpectedEof);
            }
            let value = serde_json::from_slice::<Value>(&decompressed[cursor..end])
                .map_err(|_| JacError::CorruptBlock)?;
            if !value.is_object() {
                return Err(JacError::CorruptBlock);
            }
            cursor = end;
            object_values.push(value);
        }

        let mut array_values = Vec::with_capacity(array_count);
        for _ in 0..array_count {
            let (len_raw, len_bytes) = decode_uleb128(&decompressed[cursor..])?;
            cursor += len_bytes;
            let json_len = usize::try_from(len_raw).map_err(|_| JacError::CorruptBlock)?;
            if json_len > limits.max_string_len_per_value {
                return Err(JacError::LimitExceeded(format!(
                    "Array length {} exceeds limit {}",
                    json_len, limits.max_string_len_per_value
                )));
            }
            let end = cursor + json_len;
            if end > decompressed.len() {
                return Err(JacError::UnexpectedEof);
            }
            let value = serde_json::from_slice::<Value>(&decompressed[cursor..end])
                .map_err(|_| JacError::CorruptBlock)?;
            if !value.is_array() {
                return Err(JacError::CorruptBlock);
            }
            cursor = end;
            array_values.push(value);
        }

        if cursor != decompressed.len() {
            return Err(JacError::CorruptBlock);
        }

        // Reconstruct values per record
        let mut values = vec![None; record_count];
        let mut present_idx = 0;
        let mut bool_idx = 0;
        let mut int_idx = 0;
        let mut decimal_idx = 0;
        let mut string_idx = 0;
        let mut object_idx = 0;
        let mut array_idx = 0;

        for (record_idx, slot) in values.iter_mut().enumerate() {
            if !presence.is_present(record_idx) {
                continue;
            }

            let tag = tags.get(present_idx).ok_or(JacError::CorruptBlock)?;
            let value = match tag {
                TypeTag::Null => Value::Null,
                TypeTag::Bool => {
                    let val = bool_values
                        .get(bool_idx)
                        .copied()
                        .ok_or(JacError::CorruptBlock)?;
                    bool_idx += 1;
                    Value::Bool(val)
                }
                TypeTag::Int => {
                    let val = int_values
                        .get(int_idx)
                        .copied()
                        .ok_or(JacError::CorruptBlock)?;
                    int_idx += 1;
                    Value::Number(val.into())
                }
                TypeTag::Decimal => {
                    let decimal = decimal_values
                        .get(decimal_idx)
                        .ok_or(JacError::CorruptBlock)?;
                    decimal_idx += 1;
                    let number_value: Value = serde_json::from_str(&decimal.to_json_string())
                        .map_err(|_| JacError::CorruptBlock)?;
                    let number = number_value
                        .as_number()
                        .cloned()
                        .ok_or(JacError::CorruptBlock)?;
                    Value::Number(number)
                }
                TypeTag::String => {
                    let string = string_values
                        .get(string_idx)
                        .ok_or(JacError::CorruptBlock)?
                        .clone();
                    string_idx += 1;
                    Value::String(string)
                }
                TypeTag::Object => {
                    let obj = object_values
                        .get(object_idx)
                        .ok_or(JacError::CorruptBlock)?
                        .clone();
                    object_idx += 1;
                    obj
                }
                TypeTag::Array => {
                    let arr = array_values
                        .get(array_idx)
                        .ok_or(JacError::CorruptBlock)?
                        .clone();
                    array_idx += 1;
                    arr
                }
            };

            *slot = Some(value);
            present_idx += 1;
        }

        Ok(Self {
            record_count,
            values,
        })
    }

    /// Retrieve the decoded value for a specific record index
    pub fn get_value(&self, record_idx: usize) -> Result<Option<Value>> {
        if record_idx >= self.record_count {
            return Err(JacError::Internal("Record index out of bounds".to_string()));
        }
        Ok(self.values[record_idx].clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        block_builder::{BlockBuilder, BlockData},
        Codec, CompressOpts, TryAddRecordOutcome,
    };
    use serde_json::{json, Map};

    fn map_from_json(value: serde_json::Value) -> Map<String, Value> {
        value.as_object().unwrap().clone()
    }

    fn build_block(
        records: &[Map<String, Value>],
        configure: impl FnOnce(&mut CompressOpts),
    ) -> (BlockData, Limits) {
        let mut opts = CompressOpts::default();
        opts.block_target_records = records.len().max(1);
        configure(&mut opts);
        let limits = opts.limits.clone();

        let mut builder = BlockBuilder::new(opts.clone());
        for record in records {
            match builder
                .try_add_record(record.clone())
                .expect("try add record")
            {
                TryAddRecordOutcome::Added => {}
                TryAddRecordOutcome::BlockFull { .. } => {
                    panic!("unexpected block flush in segment decode test helper")
                }
            }
        }

        (builder.finalize().unwrap(), limits)
    }

    fn field_decoder(
        block: &BlockData,
        limits: &Limits,
        field_name: &str,
    ) -> (FieldSegmentDecoder, FieldDirectoryEntry) {
        let record_count = block.header.record_count;
        let (field_index, entry) = block
            .header
            .fields
            .iter()
            .enumerate()
            .find(|(_, entry)| entry.field_name == field_name)
            .map(|(idx, entry)| (idx, entry.clone()))
            .expect("field not found");

        let decoder =
            FieldSegmentDecoder::new(&block.segments[field_index], &entry, record_count, limits)
                .unwrap();

        (decoder, entry)
    }

    #[test]
    fn test_segment_decoder_roundtrip() {
        let records = vec![
            map_from_json(json!({"mixed": true})),
            map_from_json(json!({"mixed": 123})),
            map_from_json(json!({})),
            map_from_json(json!({"mixed": {"nested": "value"}})),
        ];

        let (block, limits) = build_block(&records, |_| {});
        let (decoder, _) = field_decoder(&block, &limits, "mixed");

        assert_eq!(decoder.get_value(0).unwrap(), Some(json!(true)));
        assert_eq!(decoder.get_value(1).unwrap(), Some(json!(123)));
        assert_eq!(decoder.get_value(2).unwrap(), None);
        assert_eq!(
            decoder.get_value(3).unwrap(),
            Some(json!({"nested": "value"}))
        );
    }

    #[test]
    fn test_segment_decoder_dictionary_strings() {
        let records = vec![
            map_from_json(json!({"name": "alice"})),
            map_from_json(json!({"name": "alice"})),
            map_from_json(json!({"name": "bob"})),
            map_from_json(json!({"name": "bob"})),
        ];

        let (block, limits) = build_block(&records, |_| {});
        let (decoder, entry) = field_decoder(&block, &limits, "name");
        assert!(entry.encoding_flags & ENCODING_FLAG_DICTIONARY != 0);
        assert_eq!(decoder.get_value(0).unwrap(), Some(json!("alice")));
        assert_eq!(decoder.get_value(2).unwrap(), Some(json!("bob")));
    }

    #[test]
    fn test_segment_decoder_delta_integers() {
        let records = vec![
            map_from_json(json!({"seq": 1000})),
            map_from_json(json!({"seq": 1001})),
            map_from_json(json!({"seq": 1002})),
            map_from_json(json!({"seq": 1003})),
        ];

        let (block, limits) = build_block(&records, |_| {});
        let (decoder, entry) = field_decoder(&block, &limits, "seq");
        assert!(entry.encoding_flags & ENCODING_FLAG_DELTA != 0);
        assert_eq!(decoder.get_value(3).unwrap(), Some(json!(1003)));
    }

    #[test]
    fn test_segment_decoder_decompress_error() {
        let records = vec![map_from_json(json!({"value": 1}))];
        let (block, limits) = build_block(&records, |_| {});
        let (decoder, entry) = field_decoder(&block, &limits, "value");
        assert_eq!(decoder.get_value(0).unwrap(), Some(json!(1)));

        let mut corrupt_entry = entry.clone();
        let mut corrupt_bytes = block.segments[0].clone();
        if corrupt_bytes.len() > 1 {
            corrupt_bytes.truncate(corrupt_bytes.len() - 1);
        } else {
            corrupt_bytes.clear();
        }
        corrupt_entry.segment_compressed_len = corrupt_bytes.len();

        assert!(matches!(
            FieldSegmentDecoder::new(&corrupt_bytes, &corrupt_entry, 1, &limits),
            Err(JacError::DecompressError(_))
        ));
    }

    #[test]
    fn test_segment_decoder_dictionary_index_error() {
        let records = vec![
            map_from_json(json!({"name": "alice"})),
            map_from_json(json!({"name": "alice"})),
        ];

        let (block, limits) = build_block(&records, |opts| {
            opts.default_codec = Codec::None;
        });
        let (decoder, entry) = field_decoder(&block, &limits, "name");
        assert!(decoder.get_value(0).is_ok());

        let mut corrupt_bytes = block.segments[0].clone();
        let mut offset = entry.presence_bytes + entry.tag_bytes;
        for _ in 0..entry.dict_entry_count {
            let (len, len_bytes) = decode_uleb128(&corrupt_bytes[offset..]).unwrap();
            offset += len_bytes + len as usize;
        }

        // Set dictionary index to an out-of-range value (10)
        corrupt_bytes[offset] = 10;

        assert!(matches!(
            FieldSegmentDecoder::new(&corrupt_bytes, &entry, block.header.record_count, &limits),
            Err(JacError::DictionaryError)
        ));
    }

    #[test]
    fn test_segment_decoder_reserved_tag() {
        let entry = FieldDirectoryEntry {
            field_name: "reserved".to_string(),
            compressor: 0,
            compression_level: 0,
            presence_bytes: 1,
            tag_bytes: 1,
            value_count_present: 1,
            encoding_flags: 0,
            dict_entry_count: 0,
            segment_uncompressed_len: 2,
            segment_compressed_len: 2,
            segment_offset: 0,
        };

        let compressed = vec![0x01, 0x07];
        let limits = Limits::default();

        match FieldSegmentDecoder::new(&compressed, &entry, 1, &limits) {
            Err(JacError::UnsupportedFeature(_)) => {}
            Err(err) => panic!("unexpected error: {:?}", err),
            Ok(_) => panic!("expected error for reserved tag"),
        }
    }

    #[test]
    fn test_segment_decoder_value_count_mismatch() {
        let entry = FieldDirectoryEntry {
            field_name: "mismatch".to_string(),
            compressor: 0,
            compression_level: 0,
            presence_bytes: 1,
            tag_bytes: 1,
            value_count_present: 1,
            encoding_flags: 0,
            dict_entry_count: 0,
            segment_uncompressed_len: 2,
            segment_compressed_len: 2,
            segment_offset: 0,
        };

        let compressed = vec![0x00, 0x00];
        let limits = Limits::default();

        match FieldSegmentDecoder::new(&compressed, &entry, 1, &limits) {
            Err(JacError::CorruptBlock) => {}
            Err(err) => panic!("unexpected error: {:?}", err),
            Ok(_) => panic!("expected corrupt block error"),
        }
    }
}
