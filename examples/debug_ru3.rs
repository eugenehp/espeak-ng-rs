use espeak_ng::translate::{Translator, tokenize};
use espeak_ng::dictionary::Dictionary;
use std::path::PathBuf;

fn main() {
    let data_dir = PathBuf::from(espeak_ng::translate::default_data_dir());
    println!("Data dir: {}", data_dir.display());
    
    let dict_path = data_dir.join("ru_dict");
    println!("Dict path: {} exists={}", dict_path.display(), dict_path.exists());
    
    let phontab_path = data_dir.join("phontab");
    println!("Phontab: {} exists={}", phontab_path.display(), phontab_path.exists());
    
    // Try loading dictionary
    let dict_bytes = std::fs::read(&dict_path).expect("read dict");
    println!("Dict size: {} bytes", dict_bytes.len());
    
    match Dictionary::from_bytes("ru", dict_bytes) {
        Ok(d) => println!("Dictionary loaded OK, {} hashtab entries", d.hashtab.len()),
        Err(e) => println!("Dictionary FAILED: {e}"),
    }

    // Try loading phoneme table
    use espeak_ng::phoneme::PhonemeData;
    let mut phdata = PhonemeData::load(&data_dir).expect("phoneme data");
    match phdata.select_table_by_name("ru") {
        Ok(_) => println!("Phoneme table for 'ru' selected OK"),
        Err(e) => println!("Phoneme table FAILED: {e}"),
    }
    
    // Try word_to_phonemes
    use espeak_ng::translate::word_to_phonemes;
    use espeak_ng::dictionary::stress::StressOpts;
    let stress_opts = StressOpts::for_lang("ru");
    let word = "привет";
    let result = word_to_phonemes(word, 
        &Dictionary::from_bytes("ru", std::fs::read(&dict_path).unwrap()).unwrap(),
        &phdata,
        &stress_opts);
    println!("word_to_phonemes({:?}) phonemes: {:?}", word, result.phonemes);
    println!("  dict_flags: {:08x}", result.dict_flags);
    
    // Try IPA conversion
    use espeak_ng::translate::phonemes_to_ipa_lang;
    use espeak_ng::ipa_table::PendingStress;
    let (ipa, _) = phonemes_to_ipa_lang(&result.phonemes, &phdata, PendingStress::None, false, false);
    println!("  IPA: {:?}", ipa);
}
