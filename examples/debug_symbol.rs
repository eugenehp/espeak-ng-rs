//! Debug the dictionary lookup behind a symbol's spoken name.
//!
//! `cargo run --example debug_symbol -- de ∑ €` prints what the language's
//! dictionary holds for each bare character key: the entry's flags, whether it
//! is a `$textmode` replacement, and the bytes it yields.
use std::path::Path;
use espeak_ng::dictionary::file::Dictionary;
use espeak_ng::dictionary::lookup::{lookup, LookupCtx};

fn main() {
    let mut args = std::env::args().skip(1);
    let lang = args.next().unwrap_or_else(|| "de".into());
    let chars: Vec<String> = args.collect();
    let data = Path::new("espeak-ng-data");
    let dict = Dictionary::load(&lang, data).unwrap();
    for s in &chars {
        let ctx = LookupCtx { lookup_symbol: true, ..Default::default() };
        match lookup(&dict, s, &ctx) {
            Some(r) => println!(
                "{lang} {s:?}: flags=0x{:08x} found={} textmode={} phonemes={:?} ({})",
                r.flags1.0,
                r.flags1.found(),
                r.flags1.textmode(),
                String::from_utf8_lossy(&r.phonemes),
                r.phonemes.len(),
            ),
            None => println!("{lang} {s:?}: no entry"),
        }
    }
}
