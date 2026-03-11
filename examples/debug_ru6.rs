use espeak_ng::dictionary::{Dictionary, lookup::{lookup, LookupCtx, hash_word}, transpose::{transpose_alphabet, TransposeConfig}};
use std::path::PathBuf;

fn main() {
    let data_dir = PathBuf::from(espeak_ng::translate::default_data_dir());
    let dict_bytes = std::fs::read(data_dir.join("ru_dict")).unwrap();
    let dict = Dictionary::from_bytes("ru", dict_bytes).unwrap();
    let ctx = LookupCtx::default();

    for word in &["да", "нет", "привет"] {
        let word_utf8 = word.as_bytes();
        let tr = transpose_alphabet(word, &TransposeConfig::CYRILLIC);
        
        // Compute hash exactly as lookup_dict2 does:
        let ix = tr.bytes.len();
        let mut hash_buf = tr.bytes.clone();
        if ix < word_utf8.len() {
            hash_buf.extend_from_slice(&word_utf8[ix..]);
        }
        let h = hash_word(&hash_buf);
        
        println!("word={:?}", word);
        println!("  word_utf8={:02x?}", word_utf8);
        println!("  transposed={:02x?} compressed={}", tr.bytes, tr.is_compressed());
        println!("  hash_buf={:02x?} -> hash={}", hash_buf, h);
        
        // Also check what's in that bucket
        let bucket_start = dict.hashtab[h];
        let data = &dict.data;
        let mut pos = bucket_start;
        let mut found_match = false;
        loop {
            if pos >= data.len() { break; }
            let entry_len = data[pos] as usize;
            if entry_len == 0 { break; }
            let word_info = data[pos + 1];
            let stored_len = (word_info & 0x7f) as usize; // byte count
            let nbytes_actual = stored_len & 0x3f;
            if nbytes_actual == ix && pos + 2 + nbytes_actual <= data.len() {
                let entry_word = &data[pos+2..pos+2+nbytes_actual];
                if entry_word == tr.bytes.as_slice() {
                    println!("  ** MATCH at pos={} winfo=0x{:02x}", pos, word_info);
                    found_match = true;
                }
            }
            pos += entry_len;
        }
        if !found_match { println!("  ** no match in bucket {}", h); }
        
        let result = lookup(&dict, word, &ctx);
        println!("  lookup result: {:?}", result);
        println!();
    }
}
