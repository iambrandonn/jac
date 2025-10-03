#![no_main]

use jac_format::bitpack::{pack_presence_bitmap, unpack_presence_bitmap, pack_type_tags, unpack_type_tags};
use jac_format::TypeTag;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if data.is_empty() {
        return;
    }

    // Test presence bitmap packing/unpacking
    let record_count = (data.len() % 1000) + 1; // 1-1000 records
    let mut presence = Vec::new();

    for i in 0..record_count {
        presence.push((data[i % data.len()] % 2) == 1);
    }

    let packed = pack_presence_bitmap(&presence);
    let unpacked = unpack_presence_bitmap(&packed, record_count);

    // Verify round-trip
    if let Ok(unpacked) = unpacked {
        assert_eq!(presence, unpacked);
    }

    // Test type tag packing/unpacking
    let present_count = presence.iter().filter(|&&p| p).count();
    if present_count > 0 {
        let mut type_tags = Vec::new();
        for i in 0..present_count {
            let tag_value = (data[i % data.len()] % 7) + 1; // 1-7 (avoid reserved 0)
            type_tags.push(TypeTag::from_u8(tag_value).unwrap_or(TypeTag::Null));
        }

        let packed_tags = pack_type_tags(&type_tags);
        let unpacked_tags = unpack_type_tags(&packed_tags, present_count);

        // Verify round-trip
        if let Ok(unpacked_tags) = unpacked_tags {
            assert_eq!(type_tags, unpacked_tags);
        }
    }
});
