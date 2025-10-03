#![no_main]

use jac_format::varint::{decode_uleb128, decode_zigzag_i64};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = decode_uleb128(data);
    let _ = decode_zigzag_i64(data);
});
