fn main() {
    use std::path::Path;
    use espeak_ng::phoneme::PhonemeData;
    use espeak_ng::translate::Translator;
    
    let data_path = Path::new("/usr/share/espeak-ng-data");
    let mut phdata = PhonemeData::load(data_path).expect("load");
    phdata.select_table_by_name("en").expect("select en");
    
    let translator = Translator::new_default("en").expect("translator");
    
    let words = ["hello", "world", "one", "the", "a", "test", "three"];
    for word in &words {
        print!("{word}: ");
        if let Ok(codes) = translator.translate_to_codes(word) {
            for code in &codes {
                if code.is_boundary { print!("[||] "); continue; }
                let c = code.code;
                if c <= 15 { print!("[ctrl:{}] ", c); continue; }
                let ph_opt = phdata.get(c);
                if let Some(ph) = ph_opt {
                    let mnem = {
                        let m = ph.mnemonic;
                        let bytes: [u8; 4] = m.to_le_bytes();
                        String::from_utf8_lossy(&bytes[..bytes.iter().position(|&b| b == 0).unwrap_or(4)]).to_string()
                    };
                    print!("{}(code={} typ={} std_len={} len_mod={}) ", mnem, c, ph.typ, ph.std_length, ph.length_mod);
                } else {
                    print!("?(code={}) ", c);
                }
            }
        }
        println!();
    }
}
