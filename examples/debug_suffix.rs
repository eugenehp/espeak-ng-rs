//! Debug a suffix rule chain: what `*_rules` returns for a word and its stems.
//!
//! `cargo run --example debug_suffix -- nl vorige vorig` prints the `end_type`
//! (suffix length in its low 6 bits, `SUFX_*` flags above), where the suffix
//! starts, and the stem/suffix phoneme split for each word.
use std::path::Path;
use espeak_ng::dictionary::file::Dictionary;
use espeak_ng::dictionary::lookup::{lookup, LookupCtx};
use espeak_ng::dictionary::rules::translate_rules;
use espeak_ng::phoneme::PhonemeData;

fn main() {
    let mut args = std::env::args().skip(1);
    let lang = args.next().unwrap_or_else(|| "nl".into());
    let data = Path::new("espeak-ng-data");
    let dict = Dictionary::load(&lang, data).unwrap();
    let mut phdata = PhonemeData::load(data).unwrap();
    phdata.select_table_by_name(&lang).ok();
    let mnem = |codes: &[u8]| -> String {
        codes.iter().take_while(|&&c| c != 0)
            .map(|&c| phdata.get(c).map(|p| p.mnemonic_str().to_string())
                 .unwrap_or_else(|| format!("<{c}>")))
            .collect::<Vec<_>>().join(" ")
    };
    let letter_bits = [0u8; 256];
    for word in args {
        let mut buf = Vec::with_capacity(word.len() + 3);
        buf.push(b' ');
        buf.extend_from_slice(word.as_bytes());
        buf.push(b' ');
        buf.push(0);
        let (mut vc, mut sc) = (0i32, 0i32);
        let r = translate_rules(&dict, &buf, 1, 0, 0, &letter_bits, 0, &mut vc, &mut sc);
        println!(
            "{word:>10}: end_type=0x{:05x} (len={}) suffix_start={}\n{:>12}stem=[{}] suffix=[{}]",
            r.end_type,
            r.end_type & 0x3f,
            r.suffix_start,
            "",
            mnem(&r.phonemes),
            mnem(&r.end_phonemes),
        );
        let ctx = LookupCtx { lookup_symbol: true, ..Default::default() };
        if let Some(l) = lookup(&dict, &word, &ctx) {
            println!("{:>12}list entry: found={} [{}]", "", l.flags1.found(), mnem(&l.phonemes));
        }
    }
}
