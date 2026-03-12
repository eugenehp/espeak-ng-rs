use espeak_ng::dictionary::{
    Dictionary,
    lookup::hash_word,
    transpose::{transpose_alphabet, TransposeConfig}
};
use std::path::PathBuf;

fn main() {
    let data_dir = PathBuf::from(espeak_ng::translate::default_data_dir());
    let dict_bytes = std::fs::read(data_dir.join("ru_dict")).unwrap();
    let dict = Dictionary::from_bytes("ru", dict_bytes).unwrap();

    // Print what's in the bucket for "да" and "нет"
    for word in &["да", "нет", "привет"] {
        let tr = transpose_alphabet(word, &TransposeConfig::CYRILLIC);
        let h = hash_word(&tr.bytes) as usize;
        println!("\n=== {} → transposed={:02x?} hash={} ===", word, tr.bytes, h);
        
        // Dump the bucket
        let bucket_start = dict.hashtab[h];
        let data = &dict.data;
        let mut pos = bucket_start;
        let mut count = 0;
        loop {
            if pos >= data.len() { println!("  [overrun]"); break; }
            let entry_len = data[pos] as usize;
            if entry_len == 0 { println!("  [bucket end after {} entries]", count); break; }
            
            let wlen_byte = data[pos + 1];
            let compressed = (wlen_byte & 0x40) != 0;
            let no_ph = (wlen_byte & 0x80) != 0;
            let nbytes = (wlen_byte & 0x3f) as usize;
            
            let word_bytes = &data[pos+2..pos+2+nbytes];
            println!("  entry[{}] len={} wlen_byte=0x{:02x} compressed={} no_ph={} word_bytes={:02x?}", 
                count, entry_len, wlen_byte, compressed, no_ph, word_bytes);
            
            pos += entry_len;
            count += 1;
            if count > 10 { println!("  [...]"); break; }
        }
    }
}
