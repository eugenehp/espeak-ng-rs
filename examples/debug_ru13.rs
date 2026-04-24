/// Debug phoneme codes for remaining incorrect Russian words.
use espeak_ng::dictionary::Dictionary;
use espeak_ng::phoneme::PhonemeData;
use espeak_ng::dictionary::stress::StressOpts;
use espeak_ng::translate::{word_to_phonemes, default_data_dir, LangOptions};
use std::path::PathBuf;

fn main() {
    let data_dir = PathBuf::from(default_data_dir());
    let dict = Dictionary::load("ru", &data_dir).unwrap();
    let mut phdata = PhonemeData::load(&data_dir).unwrap();
    phdata.select_table_by_name("ru").unwrap();
    let stress = StressOpts::for_lang("ru");
    let lang_opts = LangOptions::for_lang("ru");

    let words = ["да", "мама", "а", "Россия", "мыло"];
    for word in &words {
        let lower = word.to_lowercase();
        let result = word_to_phonemes(&lower, &dict, &phdata, &stress, &lang_opts);
        let c_ipa = std::process::Command::new("espeak-ng")
            .args(["-v", "ru", "-q", "--ipa", word])
            .output().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default();
        let c_x = std::process::Command::new("espeak-ng")
            .args(["-v", "ru", "-q", "-X", word])
            .output().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default();
        
        print!("{word}: codes=");
        for &code in &result.phonemes {
            if code == 0 { break; }
            if let Some(ph) = phdata.get(code) {
                let m = ph.mnemonic.to_le_bytes();
                let ms: String = m.iter().take_while(|&&b| b != 0).map(|&b| b as char).collect();
                print!("[{code}:{ms}] ");
            } else {
                print!("[{code}:?] ");
            }
        }
        println!();
        println!("  Rust IPA: {}", espeak_ng::text_to_ipa("ru", word).unwrap_or_default());
        println!("  C    IPA: {c_ipa}");
        println!("  C    -X : {c_x}");
        println!();
    }
}
