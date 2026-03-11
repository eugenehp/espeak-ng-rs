//! Debug phoneme data for key codes
use std::path::Path;
use espeak_ng::phoneme::PhonemeData;
use espeak_ng::translate::Translator;
use espeak_ng::synthesize::{bytecode, phondata::SpectSeq};

fn main() {
    let data_path = Path::new("/usr/share/espeak-ng-data");
    let phondata_bytes = std::fs::read(data_path.join("phondata")).unwrap();
    let phonindex_bytes = std::fs::read(data_path.join("phonindex")).unwrap();
    let mut phdata = PhonemeData::load(data_path).unwrap();
    phdata.select_table_by_name("en").unwrap();
    let translator = Translator::new_default("en").unwrap();
    
    for word in &["the", "one", "hello"] {
        let codes = translator.translate_to_codes(word).unwrap();
        println!("{}: {:?}", word, codes.iter().map(|c| c.code).collect::<Vec<_>>());
        for code in &codes {
            if code.is_boundary || code.code < 10 { continue; }
            if let Some(ph) = phdata.get(code.code) {
                let mnem = ph.mnemonic_str();
                let extract = if ph.program > 0 {
                    let e = bytecode::scan_phoneme(ph.program, &phonindex_bytes);
                    format!("fmt={:?} wav={:?}", e.fmt_addr, e.wav_addr)
                } else { "no program".into() };
                println!("  code={} mnem={:?} typ={} program={} | {}", 
                    code.code, mnem, ph.start_type, ph.program, extract);
            }
        }
    }
}
