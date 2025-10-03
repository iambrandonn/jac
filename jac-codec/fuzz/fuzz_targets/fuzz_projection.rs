#![no_main]

use jac_codec::block_decode::{BlockDecoder, DecompressOpts};
use jac_format::Limits;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let opts = DecompressOpts {
        limits: Limits::default(),
        verify_checksums: false,
    };

    if let Ok(decoder) = BlockDecoder::new(data, &opts) {
        // Try to project different field combinations
        let field_names = ["ts", "level", "user", "error", "data", "id", "value"];

        for field_name in &field_names {
            let _ = decoder.project_field(field_name);
        }

        // Try projecting multiple fields
        let _ = decoder.project_fields(&["ts", "level"]);
        let _ = decoder.project_fields(&["user", "error"]);
        let _ = decoder.project_fields(&["ts", "level", "user", "error"]);
    }
});
