use jac_format::block::{BlockHeader, FieldDirectoryEntry};
use jac_format::limits::Limits;

fn create_test_field_entry() -> FieldDirectoryEntry {
    FieldDirectoryEntry {
        field_name: "test_field".to_string(),
        compressor: 1,
        compression_level: 15,
        presence_bytes: 125,
        tag_bytes: 47,
        value_count_present: 1000,
        encoding_flags: 0,
        dict_entry_count: 0,
        segment_uncompressed_len: 1000,
        segment_compressed_len: 500,
        segment_offset: 0,
    }
}

fn main() {
    let limits = Limits::default();

    // Create a test header with 1 field (like the test)
    let header = BlockHeader {
        record_count: 1000,
        fields: vec![create_test_field_entry()],
    };

    println!("Creating header with {} records, {} fields", header.record_count, header.fields.len());

    // Encode
    let encoded = header.encode().expect("Failed to encode");
    println!("Encoded {} bytes: {:?}", encoded.len(), encoded);

    // Decode
    match BlockHeader::decode(&encoded, &limits) {
        Ok((decoded, bytes_read)) => {
            println!("Decoded successfully! Read {} bytes", bytes_read);
            println!("Decoded: {} records, {} fields", decoded.record_count, decoded.fields.len());
        }
        Err(e) => {
            println!("Decode error: {:?}", e);
        }
    }
}