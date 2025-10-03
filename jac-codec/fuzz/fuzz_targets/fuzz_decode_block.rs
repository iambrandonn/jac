#![no_main]

use jac_codec::block_decode::DecompressOpts;
use jac_codec::BlockDecoder;
use jac_format::Limits;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let opts = DecompressOpts {
        limits: Limits::default(),
        verify_checksums: false,
    };

    let _ = BlockDecoder::new(data, &opts);
});
