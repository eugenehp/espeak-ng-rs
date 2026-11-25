//! A dictionary from arbitrary bytes must load or fail cleanly, and lookups in
//! whatever loaded must not panic.
#![no_main]

use espeak_ng::dictionary::lookup::{lookup, LookupCtx};
use espeak_ng::dictionary::Dictionary;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(dict) = Dictionary::from_bytes("en", data.to_vec()) {
        let ctx = LookupCtx::default();
        for word in ["a", "hello", "", "\u{ffff}", "zzzzzzzzzzzzzzzz"] {
            let _ = lookup(&dict, word, &ctx);
        }
    }
});
