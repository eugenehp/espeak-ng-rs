/// Debug: show raw phoneme codes for Russian words + IPA rendering details.
use espeak_ng::phoneme::PhonemeData;
use espeak_ng::translate::default_data_dir;
use std::path::PathBuf;

fn main() {
    let data_dir = PathBuf::from(default_data_dir());
    let mut phdata = PhonemeData::load(&data_dir).unwrap();
    phdata.select_table_by_name("ru").unwrap();

    println!("Russian phoneme table selected");
    println!();

    // Check specific phoneme codes we see from Russian words
    let codes_of_interest = [
        (35u8, "а (a vowel)"),
        (73,   "д (d consonant)"),
        (66,   "м (m consonant)"),
        (126,  "? (mама stressed a?)"),
        (48,   "п (p consonant)"),
        (34,   "р (r consonant)"),
        (37,   "и (i vowel)"),
        (85,   "? (stress?)"),
        (36,   "е (e vowel)"),
        (47,   "т (t consonant)"),
        (50,   "н (n consonant)"),
    ];

    for (code, desc) in &codes_of_interest {
        if let Some(ph) = phdata.get(*code) {
            let mnem_bytes = ph.mnemonic.to_le_bytes();
            let mnem_str: String = mnem_bytes.iter()
                .take_while(|&&b| b != 0)
                .map(|&b| b as char)
                .collect();
            let ipa_from_bytecode = phdata.phoneme_ipa_string(ph.program);
            println!("code={code:3} {desc}");
            println!("  type={} mnemonic={mnem_str:?} program=0x{:04x}",
                ph.typ, ph.program);
            println!("  ipa_from_bytecode={ipa_from_bytecode:?}");
            println!();
        } else {
            println!("code={code:3} {desc} -> NOT FOUND in active table");
        }
    }
}
