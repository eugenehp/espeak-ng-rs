use espeak_ng::dictionary::{Dictionary, lookup::{lookup, LookupCtx, hash_word}, transpose::{transpose_alphabet, TransposeConfig}};
use std::path::PathBuf;

fn main() {
    let data_dir = PathBuf::from(espeak_ng::translate::default_data_dir());
    let dict_bytes = std::fs::read(data_dir.join("ru_dict")).unwrap();
    let dict = Dictionary::from_bytes("ru", dict_bytes).unwrap();
    let ctx = LookupCtx::default();

    for word in &["привет", "да", "нет", "я", "я".repeat(1).as_str()] {
        let tr = transpose_alphabet(word, &TransposeConfig::CYRILLIC);
        let h = hash_word(&tr.bytes);
        println!("word={:?}  transposed={:02x?}  compressed={}  hash={}", 
            word, tr.bytes, tr.is_compressed(), h);
        let result = lookup(&dict, word, &ctx);
        println!("  lookup = {:?}", result);
    }
}
