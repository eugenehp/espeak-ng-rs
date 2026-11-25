//! The phoneme-table parser on arbitrary bytes.
#![no_main]

use espeak_ng::phoneme::load::parse_phontab;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = parse_phontab(data);
});
