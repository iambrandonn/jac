//! Security limits and configuration

/// Security limits to prevent decompression bombs
#[derive(Debug, Clone)]
pub struct Limits {
    /// Maximum records per block (default: 100,000, hard: 1,000,000)
    pub max_records_per_block: usize,
    /// Maximum fields per block (default: 4,096, hard: 65,535)
    pub max_fields_per_block: usize,
    /// Maximum uncompressed segment length (hard: 64 MiB)
    pub max_segment_uncompressed_len: usize,
    /// Maximum uncompressed block total (hard: 256 MiB)
    pub max_block_uncompressed_total: usize,
    /// Maximum dictionary entries per field (default: 4,096, hard: 65,535)
    pub max_dict_entries_per_field: usize,
    /// Maximum string length per value (hard: 16 MiB)
    pub max_string_len_per_value: usize,
    /// Maximum decimal digits per value (hard: 65,536)
    pub max_decimal_digits_per_value: usize,
    /// Maximum presence bytes per field (hard: 32 MiB)
    pub max_presence_bytes: usize,
    /// Maximum tag bytes per field (hard: 32 MiB)
    pub max_tag_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_records_per_block: 100_000,
            max_fields_per_block: 4_096,
            max_segment_uncompressed_len: 64 * 1024 * 1024,
            max_block_uncompressed_total: 256 * 1024 * 1024,
            max_dict_entries_per_field: 4_096,
            max_string_len_per_value: 16 * 1024 * 1024,
            max_decimal_digits_per_value: 65_536,
            max_presence_bytes: 32 * 1024 * 1024,
            max_tag_bytes: 32 * 1024 * 1024,
        }
    }
}

