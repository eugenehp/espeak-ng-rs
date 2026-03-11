//! Print actual frame fheight values for "ah" phoneme  
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
    
    for word in &["ah", "hello"] {
        let codes = translator.translate_to_codes(word).unwrap();
        println!("{}: codes={:?}", word, codes.iter().map(|c| (c.code, c.is_boundary)).collect::<Vec<_>>());
        
        for code in &codes {
            if code.is_boundary || code.code == 0 { continue; }
            if let Some(ph) = phdata.get(code.code) {
                println!("  code={} program={}", code.code, ph.program);
                if ph.program == 0 { continue; }
                
                let extract = bytecode::scan_phoneme(ph.program, &phonindex_bytes);
                println!("    fmt_addr={:?}", extract.fmt_addr);
                
                if let Some(addr) = extract.fmt_addr {
                    if let Some(seq) = SpectSeq::parse(&phondata_bytes, addr as usize) {
                        const STEP_MS: f64 = 64.0 * 1000.0 / 22050.0;
                        println!("    frames={} klatt={}", seq.frames.len(), seq.is_klatt);
                        for (i, f) in seq.frames.iter().enumerate() {
                            println!("    frame[{}]: len_steps={} (~{:.1}ms) rms={} fheight={:?} ffreq={:?}",
                                i, f.length, f.length as f64 * STEP_MS,
                                f.rms, &f.fheight[..6], &f.ffreq[..6]);
                        }
                    }
                }
            }
        }
    }
}
