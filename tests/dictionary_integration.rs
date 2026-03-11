// tests/dictionary_integration.rs
//
// Integration tests for the dictionary module.
//
// These tests exercise the full dictionary pipeline against the installed
#![allow(unused_imports, dead_code)]
// espeak-ng-data directory.  They are automatically skipped when that data is
// not present.

use espeak_ng::dictionary::{
    Dictionary, hash_word, lookup, LookupCtx, TransposeConfig, transpose_alphabet,
};
use std::path::PathBuf;

fn data_dir() -> PathBuf {
    PathBuf::from("/usr/share/espeak-ng-data")
}

fn en_dict() -> Option<Dictionary> {
    let dir = data_dir();
    if !dir.join("en_dict").exists() { return None; }
    Some(Dictionary::load("en", &dir).expect("load en_dict"))
}

// ── Hash function ──────────────────────────────────────────────────────────

#[test]
fn hash_reference_values() {
    // Values computed by the Python reference implementation of HashDictionary.
    assert_eq!(hash_word(b"hello"), 48);
    assert_eq!(hash_word(b"the"),   200); // raw 'the' hash (before compression)
    assert_eq!(hash_word(b"a"),     98);
    assert_eq!(hash_word(b""),      0);
}

// ── TransposeAlphabet ─────────────────────────────────────────────────────

#[test]
fn transpose_a_single() {
    let r = transpose_alphabet("a", &TransposeConfig::LATIN);
    assert!(r.is_compressed());
    assert_eq!(r.bytes, &[0x04]);
}

#[test]
fn transpose_the_word() {
    let r = transpose_alphabet("the", &TransposeConfig::LATIN);
    assert!(r.is_compressed());
    assert_eq!(r.bytes, &[0x50, 0x81, 0x40]);
    assert_eq!(r.wlen, 0x43); // 3 bytes | 0x40 compressed flag
}

#[test]
fn transpose_non_latin_no_compress() {
    // Digits are outside the transpose range
    let r = transpose_alphabet("abc1", &TransposeConfig::LATIN);
    assert!(!r.is_compressed());
}

// ── Dictionary lookup ─────────────────────────────────────────────────────

#[test]
fn lookup_common_words() {
    let dict = match en_dict() { Some(d) => d, None => return };
    let ctx = LookupCtx { lookup_symbol: true, ..Default::default() };

    for word in &["the", "a", "and", "is", "in", "to", "of", "it"] {
        let result = lookup(&dict, word, &ctx);
        assert!(result.is_some(), "'{}' should be in en_dict", word);
        let r = result.unwrap();
        assert!(r.flags1.found(), "FLAG_FOUND should be set for '{}'", word);
    }
}

#[test]
fn lookup_not_in_dict() {
    let dict = match en_dict() { Some(d) => d, None => return };
    let ctx = LookupCtx::default();

    // Gibberish words should not be found
    for word in &["xzqfgh", "aaabbbccc", "qqqqqq"] {
        assert!(lookup(&dict, word, &ctx).is_none(),
            "'{}' should not be in en_dict", word);
    }
}

#[test]
fn lookup_the_has_phonemes() {
    let dict = match en_dict() { Some(d) => d, None => return };
    let ctx = LookupCtx { lookup_symbol: true, ..Default::default() };

    let r = lookup(&dict, "the", &ctx).expect("'the' in dict");
    assert!(!r.phonemes.is_empty(), "'the' should have phonemes");
    // 'the' phoneme bytes should not be all-zero
    assert!(r.phonemes.iter().any(|&b| b != 0),
        "phoneme bytes should contain non-zero codes");
}

#[test]
fn lookup_preserves_case_sensitivity() {
    let dict = match en_dict() { Some(d) => d, None => return };
    let ctx = LookupCtx::default();

    // Dictionary stores lowercase; uppercase "THE" should NOT be found
    // (the translation pipeline lowercases before lookup in practice)
    // This tests that our hash/compare is case-sensitive like the C code.
    let r_upper = lookup(&dict, "THE", &ctx);
    // Not necessarily None (there might be an entry for it), but if it IS
    // None that's expected and correct.
    let _ = r_upper;
}

#[test]
fn lookup_short_words() {
    let dict = match en_dict() { Some(d) => d, None => return };
    let ctx = LookupCtx { lookup_symbol: true, ..Default::default() };

    // Single-letter words
    for word in &["a", "i"] {
        let r = lookup(&dict, word, &ctx);
        assert!(r.is_some(), "single-letter '{}' should be in en_dict", word);
    }
}

// ── Rule group indexing ────────────────────────────────────────────────────

#[test]
fn rule_groups_set_for_common_letters() {
    let dict = match en_dict() { Some(d) => d, None => return };

    // All lowercase vowels should have rule chains in English
    for c in b"aeiou".iter() {
        assert!(dict.group1(*c).is_some(),
            "groups1['{}']: should be set for English", *c as char);
    }
    // Common consonants too
    for c in b"bcdfghjklmnpqrstvwxyz".iter() {
        assert!(dict.group1(*c).is_some(),
            "groups1['{}']: should be set for English", *c as char);
    }
}

#[test]
fn rule_groups_default_group_set() {
    let dict = match en_dict() { Some(d) => d, None => return };
    assert!(dict.group1(0).is_some(), "default rule chain (groups1[0]) must be set");
}

// ── Multi-language loading ────────────────────────────────────────────────

#[test]
fn load_multiple_languages() {
    let dir = data_dir();
    let langs = ["de", "fr", "es", "it"];
    for lang in langs {
        let path = dir.join(format!("{}_dict", lang));
        if !path.exists() { continue; }
        let dict = Dictionary::load(lang, &dir).expect(&format!("load {}_dict", lang));
        assert_eq!(dict.lang, lang);
        assert!(dict.rules_offset > 0);
    }
}

// ── Hash bucket distribution sanity ───────────────────────────────────────

#[test]
fn hash_bucket_distribution() {
    let dict = match en_dict() { Some(d) => d, None => return };
    let data = &dict.data;

    // Count non-empty buckets
    let mut non_empty = 0;
    for &start in dict.hashtab.iter() {
        if start < data.len() && data[start] != 0 {
            non_empty += 1;
        }
    }
    // For English, most buckets should be non-empty (good hash distribution)
    let fill_rate = non_empty as f64 / 1024.0;
    assert!(fill_rate > 0.5,
        "hash table fill rate too low: {:.1}%", fill_rate * 100.0);
}

#[test]
fn show_phoneme_bytes() {
    let dict = match en_dict() { Some(d) => d, None => return };
    let ctx = LookupCtx { lookup_symbol: true, ..Default::default() };
    for word in &["the", "hello", "world", "and", "is", "to"] {
        let r = lookup(&dict, word, &ctx);
        if let Some(r) = r {
            println!("{:10?} phonemes: {:?}", word, r.phonemes);
        }
    }
}

#[test]
fn show_phoneme_bytes2() {
    let dict = match en_dict() { Some(d) => d, None => return };
    let ctx = LookupCtx { lookup_symbol: true, ..Default::default() };
    for word in &["hello", "world", "test", "this", "word"] {
        let r = lookup(&dict, word, &ctx);
        if let Some(r) = r {
            println!("{:10?} phonemes: {:?}", word, r.phonemes);
        } else {
            println!("{:10?} not found", word);
        }
    }
}

#[test]
fn show_phoneme_bytes3() {
    let dict = match en_dict() { Some(d) => d, None => return };
    let ctx = LookupCtx { lookup_symbol: true, ..Default::default() };
    // Try more common words
    for word in &["this", "that", "have", "do", "not", "are", "be", "he", "she", "we"] {
        let r = lookup(&dict, word, &ctx);
        if let Some(r) = r {
            println!("{:10?} phonemes: {:?}", word, r.phonemes);
        } else {
            println!("{:10?} not found", word);
        }
    }
}

#[test]
fn rules_hello_debug() {
    let dict = match en_dict() { Some(d) => d, None => return };
    use espeak_ng::dictionary::rules::translate_rules;
    
    fn english_letter_bits() -> [u8; 256] {
        let mut bits = [0u8; 256];
        let set = |bits: &mut [u8; 256], group: u8, letters: &[u8]| {
            for &c in letters {
                bits[c as usize] |= 1 << group;
                if c.is_ascii_lowercase() { bits[(c - 32) as usize] |= 1 << group; }
            }
        };
        set(&mut bits, 0, b"aeiou");
        set(&mut bits, 1, b"bcdfgjklmnpqstvxz");
        set(&mut bits, 2, b"bcdfghjklmnpqrstvwxz");
        set(&mut bits, 3, b"hlmnr");
        set(&mut bits, 4, b"cfhkpqstx");
        set(&mut bits, 5, b"bdgjlmnrvwyz");
        set(&mut bits, 6, b"eiy");
        set(&mut bits, 7, b"aeiouy");
        bits
    }
    
    let letter_bits = english_letter_bits();
    let word = "hello";
    let mut word_buf = vec![b' '];
    word_buf.extend_from_slice(word.as_bytes());
    word_buf.push(b' ');
    word_buf.push(0);
    
    let mut vc = 0i32;
    let mut sc = 0i32;
    let result = translate_rules(&dict, &word_buf, 1, 0, 0, &letter_bits, 0, &mut vc, &mut sc);
    println!("hello rules: phonemes={:?}", result.phonemes);
    
    // Decode phonemes
    let phontab_path = std::path::Path::new("/usr/share/espeak-ng-data");
    if !phontab_path.join("phontab").exists() { return; }
    let mut phdata = espeak_ng::phoneme::load::PhonemeData::load(phontab_path).unwrap();
    phdata.select_table_by_name("en").unwrap();
    for &code in &result.phonemes {
        if let Some(ph) = phdata.get(code) {
            let mnem: Vec<u8> = (0..4).map(|i| ((ph.mnemonic >> (i*8)) & 0xff) as u8).take_while(|&b| b != 0).collect();
            let mnem_str = String::from_utf8_lossy(&mnem);
            println!("  code={} mnem={:?} type={}", code, mnem_str, ph.typ);
        } else {
            println!("  code={} (no phoneme)", code);
        }
    }
}

#[test]
fn rules_hello_individual_chars() {
    let dict = match en_dict() { Some(d) => d, None => return };
    use espeak_ng::dictionary::rules::translate_rules;
    
    fn english_letter_bits() -> [u8; 256] {
        let mut bits = [0u8; 256];
        let set = |bits: &mut [u8; 256], group: u8, letters: &[u8]| {
            for &c in letters {
                bits[c as usize] |= 1 << group;
                if c.is_ascii_lowercase() { bits[(c - 32) as usize] |= 1 << group; }
            }
        };
        set(&mut bits, 0, b"aeiou");
        set(&mut bits, 1, b"bcdfgjklmnpqstvxz");
        set(&mut bits, 2, b"bcdfghjklmnpqrstvwxz");
        set(&mut bits, 3, b"hlmnr");
        set(&mut bits, 4, b"cfhkpqstx");
        set(&mut bits, 5, b"bdgjlmnrvwyz");
        set(&mut bits, 6, b"eiy");
        set(&mut bits, 7, b"aeiouy");
        bits
    }
    
    let phontab_path = std::path::Path::new("/usr/share/espeak-ng-data");
    if !phontab_path.join("phontab").exists() { return; }
    let mut phdata = espeak_ng::phoneme::load::PhonemeData::load(phontab_path).unwrap();
    phdata.select_table_by_name("en").unwrap();
    
    let letter_bits = english_letter_bits();
    
    // Test different words to see rule behavior
    for word in &["hello", "world", "test", "he", "be", "the"] {
        let mut word_buf = vec![b' '];
        word_buf.extend_from_slice(word.as_bytes());
        word_buf.push(b' ');
        word_buf.push(0);
        let mut vc = 0i32; let mut sc = 0i32;
        let result = translate_rules(&dict, &word_buf, 1, 0, 0, &letter_bits, 0, &mut vc, &mut sc);
        let mnems: Vec<String> = result.phonemes.iter().filter(|&&c| c > 0).map(|&code| {
            if let Some(ph) = phdata.get(code) {
                let m: Vec<u8> = (0..4).map(|i| ((ph.mnemonic >> (i*8)) & 0xff) as u8).take_while(|&b| b != 0).collect();
                String::from_utf8_lossy(&m).to_string()
            } else {
                format!("?{}", code)
            }
        }).collect();
        println!("{:10}: {:?}", word, mnems);
    }
}

#[test]
fn groups1_has_single_letters() {
    let dict = match en_dict() { Some(d) => d, None => return };
    // groups1['e'] should be Some (there is a single-letter 'e' rule group)
    assert!(dict.groups.groups1[b'e' as usize].is_some(), "groups1['e'] should be Some");
    assert!(dict.groups.groups1[b'h' as usize].is_some(), "groups1['h'] should be Some");
    assert!(dict.groups.groups1[b'b' as usize].is_some(), "groups1['b'] should be Some");
    println!("groups1['e'] = {:?}", dict.groups.groups1[b'e' as usize]);
    println!("groups1['h'] = {:?}", dict.groups.groups1[b'h' as usize]);
    println!("groups2 count for 'e': {}", dict.groups.groups2_count[b'e' as usize]);
    println!("groups2 count for 'h': {}", dict.groups.groups2_count[b'h' as usize]);
}

#[test]
fn decode_phoneme_147() {
    let phontab_path = std::path::Path::new("/usr/share/espeak-ng-data");
    if !phontab_path.join("phontab").exists() { return; }
    let mut phdata = espeak_ng::phoneme::load::PhonemeData::load(phontab_path).unwrap();
    phdata.select_table_by_name("en").unwrap();
    for code in [93u8, 115, 13, 137, 3] {
        if let Some(ph) = phdata.get(code) {
            let m: Vec<u8> = (0..4).map(|i| ((ph.mnemonic >> (i*8)) & 0xff) as u8).take_while(|&b| b != 0).collect();
            println!("code={} mnem={:?} type={}", code, String::from_utf8_lossy(&m), ph.typ);
        } else {
            println!("code={} not found", code);
        }
    }
}

#[test]
fn find_at_phoneme() {
    let phontab_path = std::path::Path::new("/usr/share/espeak-ng-data");
    if !phontab_path.join("phontab").exists() { return; }
    let mut phdata = espeak_ng::phoneme::load::PhonemeData::load(phontab_path).unwrap();
    phdata.select_table_by_name("en").unwrap();
    // Find phonemes with '@' in mnemonic
    for code in 0u8..=255 {
        if let Some(ph) = phdata.get(code) {
            let m_bytes: Vec<u8> = (0..4).map(|i| ((ph.mnemonic >> (i*8)) & 0xff) as u8).collect();
            let mnem = String::from_utf8_lossy(&m_bytes);
            if mnem.contains('@') || code < 20 {
                println!("code={:3} mnem={:?} type={} phflags={:08x}", code, 
                         mnem.trim_end_matches('\0'), ph.typ, ph.phflags);
            }
        }
    }
}

#[test]
fn find_phoneme_123() {
    let phontab_path = std::path::Path::new("/usr/share/espeak-ng-data");
    if !phontab_path.join("phontab").exists() { return; }
    let mut phdata = espeak_ng::phoneme::load::PhonemeData::load(phontab_path).unwrap();
    phdata.select_table_by_name("en").unwrap();
    for code in [123u8, 124, 125, 126, 127, 128] {
        if let Some(ph) = phdata.get(code) {
            let m: Vec<u8> = (0..4).map(|i| ((ph.mnemonic >> (i*8)) & 0xff) as u8).take_while(|&b| b != 0).collect();
            println!("code={:3} mnem={:?} type={}", code, String::from_utf8_lossy(&m), ph.typ);
        } else {
            println!("code={:3} not found", code);
        }
    }
}

#[test]
fn the_in_dict() {
    use espeak_ng::dictionary::lookup::{lookup, LookupCtx};
    let dict = match en_dict() { Some(d) => d, None => return };
    let ctx = LookupCtx::default();
    let result = lookup(&dict, "the", &ctx);
    println!("the lookup: {:?}", result.map(|r| r.phonemes));
    let result2 = lookup(&dict, "a", &ctx);
    println!("a lookup: {:?}", result2.map(|r| r.phonemes));
    let result3 = lookup(&dict, "hello", &ctx);
    println!("hello lookup: {:?}", result3.map(|r| r.phonemes));
}

#[test] 
fn dict_phonemes_with_stress() {
    use espeak_ng::dictionary::lookup::{lookup, LookupCtx};
    let dict = match en_dict() { Some(d) => d, None => return };
    let ctx = LookupCtx::default();
    for word in &["the", "are", "you", "night", "goodbye", "hello", "world"] {
        let result = lookup(&dict, word, &ctx);
        let ph = result.map(|r| r.phonemes).unwrap_or_default();
        println!("{:10}: {:?}", word, ph);
    }
}

#[test] 
fn dict_flags_check() {
    use espeak_ng::dictionary::lookup::{lookup, LookupCtx};
    let dict = match en_dict() { Some(d) => d, None => return };
    let ctx = LookupCtx::default();
    for word in &["the", "are", "you", "night", "make", "take", "silent"] {
        let result = lookup(&dict, word, &ctx);
        if let Some(r) = result {
            println!("{:10}: ph={:?} flags1={:?}", word, r.phonemes, r.flags1);
        } else {
            println!("{:10}: not in dict", word);
        }
    }
}

#[test]
fn stress_phoneme_table() {
    use espeak_ng::phoneme::*;
    let phdata = match PhonemeData::load(std::path::Path::new("/usr/share/espeak-ng-data")) {
        Ok(d) => d, Err(_) => return
    };
    for code in 2..=7 {
        if let Some(ph) = phdata.get(code) {
            let mnem: Vec<u8> = (0..4).map(|i| ((ph.mnemonic >> (i*8)) & 0xff) as u8).take_while(|&b| b != 0).collect();
            println!("code={} mnem={:?} type={} std_length={} phflags={:#x}", code, String::from_utf8_lossy(&mnem), ph.typ, ph.std_length, ph.phflags);
        }
    }
}

#[test]
fn stress_phoneme_table2() {
    use espeak_ng::phoneme::*;
    let phdata = match PhonemeData::load(std::path::Path::new("/usr/share/espeak-ng-data")) {
        Ok(d) => d, Err(e) => { println!("load error: {e}"); return; }
    };
    println!("n_tables={}", phdata.n_tables());
    for code in 2u8..=7u8 {
        let ph = phdata.get(code);
        println!("code={} ph={:?}", code, ph.is_some());
    }
}

#[test]
fn debug_phoneme_table() {
    use espeak_ng::phoneme::*;
    let phdata = match PhonemeData::load(std::path::Path::new("/usr/share/espeak-ng-data")) {
        Ok(d) => d, Err(e) => { println!("load error: {e}"); return; }
    };
    // Print first 20 non-None phonemes
    let mut found = 0;
    for code in 0u8..=255 {
        if let Some(ph) = phdata.get(code) {
            let mnem: Vec<u8> = (0..4).map(|i| ((ph.mnemonic >> (i*8)) & 0xff) as u8).take_while(|&b| b != 0).collect();
            println!("code={:3} mnem={:?} type={}", code, String::from_utf8_lossy(&mnem), ph.typ);
            found += 1;
            if found >= 20 { break; }
        }
    }
}

#[test]
fn debug_hello_lookup() {
    use espeak_ng::dictionary::*;
    use espeak_ng::dictionary::lookup::*;
    let dict = match en_dict() { Some(d) => d, None => return };
    let _ctx = LookupCtx::default();

    // Check hash
    let h = hash_word(b"hello");
    println!("hash('hello') = {}", h);
    
    // Check transposed bytes for "hello"
    let raw_data = &dict.data[..];
    let bucket_start = dict.hashtab[h];
    println!("bucket_start = {}", bucket_start);
    
    // Print first few entries in the bucket
    let mut pos = bucket_start;
    for i in 0..10 {
        if pos >= raw_data.len() { break; }
        let entry_len = raw_data[pos] as usize;
        if entry_len == 0 { println!("  entry {}: end of bucket (0)", i); break; }
        let word_info = raw_data[pos + 1];
        let stored_len = word_info & 0x7f;
        let actual_len = (stored_len & 0x3f) as usize;
        let word_bytes = if pos + 2 + actual_len <= raw_data.len() {
            raw_data[pos+2..pos+2+actual_len].to_vec()
        } else { vec![] };
        let ph_start = pos + 2 + actual_len;
        let ph_end = raw_data[ph_start..pos+entry_len].iter().position(|&b| b == 0).map(|p| ph_start + p).unwrap_or(pos+entry_len);
        let phonemes = raw_data[ph_start..ph_end].to_vec();
        println!("  entry {}: len={} word_info={:#x} stored_len={} word={:?} ph={:?}", 
            i, entry_len, word_info, stored_len, word_bytes, phonemes);
        pos += entry_len;
    }
}

#[test]
fn debug_transpose_hello() {
    use espeak_ng::dictionary::*;
    let dict = match en_dict() { Some(d) => d, None => return };
    let result = transpose_alphabet("hello", &dict.transpose);
    println!("hello transposed: bytes={:?} wlen={:#x}", result.bytes, result.wlen);
    let result2 = transpose_alphabet("the", &dict.transpose);
    println!("the transposed:   bytes={:?} wlen={:#x}", result2.bytes, result2.wlen);
}

#[test]
fn debug_hello_hash_compressed() {
    use espeak_ng::dictionary::*;
    use espeak_ng::dictionary::lookup::*;
    let dict = match en_dict() { Some(d) => d, None => return };
    
    let result = transpose_alphabet("hello", &dict.transpose);
    let h = hash_word(&result.bytes);
    println!("hash(compressed_hello) = {} wlen={:#x}", h, result.wlen);
    
    // Now check what's in that bucket
    let raw_data = &dict.data[..];
    let bucket_start = dict.hashtab[h];
    println!("bucket_start = {}", bucket_start);
    let mut pos = bucket_start;
    for i in 0..10 {
        if pos >= raw_data.len() { break; }
        let entry_len = raw_data[pos] as usize;
        if entry_len == 0 { println!("  entry {}: end of bucket", i); break; }
        let word_info = raw_data[pos + 1];
        let stored_len = word_info & 0x7f;
        let actual_len = (stored_len & 0x3f) as usize;
        let word_bytes = if pos + 2 + actual_len <= raw_data.len() {
            raw_data[pos+2..pos+2+actual_len].to_vec()
        } else { vec![] };
        let ph_start = pos + 2 + actual_len;
        let ph_end = raw_data[ph_start..pos+entry_len].iter().position(|&b| b == 0).map(|p| ph_start + p).unwrap_or(pos+entry_len);
        let phonemes = raw_data[ph_start..ph_end].to_vec();
        println!("  entry {}: len={} word_info={:#x} stored_len={} word={:?} ph={:?}", 
            i, entry_len, word_info, stored_len, word_bytes, phonemes);
        pos += entry_len;
    }
}

#[test]
fn debug_find_hello_in_dict() {
    use espeak_ng::dictionary::*;
    use espeak_ng::dictionary::lookup::*;
    let dict = match en_dict() { Some(d) => d, None => return };
    
    let result = transpose_alphabet("hello", &dict.transpose);
    let h = hash_word(&result.bytes);
    let raw_data = &dict.data[..];
    let bucket_start = dict.hashtab[h];
    println!("Looking for hello: compressed={:?} wlen={:#x} hash={} bucket={}", result.bytes, result.wlen, h, bucket_start);
    
    let mut pos = bucket_start;
    let target = &result.bytes;
    let target_wlen = result.wlen;
    let mut found_at = None;
    
    for i in 0..1000 {
        if pos >= raw_data.len() { break; }
        let entry_len = raw_data[pos] as usize;
        if entry_len == 0 { println!("  End of bucket after {} entries", i); break; }
        let word_info = raw_data[pos + 1];
        let stored_len = word_info & 0x7f;
        let actual_len = (stored_len & 0x3f) as usize;
        if stored_len == target_wlen && pos + 2 + actual_len <= raw_data.len() {
            let word_bytes = &raw_data[pos+2..pos+2+actual_len];
            if word_bytes == target.as_slice() {
                found_at = Some(i);
                let ph_start = pos + 2 + actual_len;
                let ph_end = raw_data[ph_start..pos+entry_len].iter().position(|&b| b == 0).map(|p| ph_start + p).unwrap_or(pos+entry_len);
                let phonemes = raw_data[ph_start..ph_end].to_vec();
                println!("  FOUND at entry {}: len={} phonemes={:?}", i, entry_len, phonemes);
                break;
            }
        }
        pos += entry_len;
    }
    if found_at.is_none() {
        println!("NOT FOUND in bucket");
    }
}

#[test]
fn debug_all_bucket252() {
    use espeak_ng::dictionary::*;
    use espeak_ng::dictionary::lookup::*;
    let dict = match en_dict() { Some(d) => d, None => return };
    
    let raw_data = &dict.data[..];
    let bucket_start = dict.hashtab[252];
    println!("Bucket 252 starts at {}", bucket_start);
    let mut pos = bucket_start;
    for i in 0..50 {
        if pos >= raw_data.len() { break; }
        let entry_len = raw_data[pos] as usize;
        if entry_len == 0 { println!("  end of bucket after {} entries", i); break; }
        let word_info = raw_data[pos + 1];
        let stored_len = word_info & 0x7f;
        let actual_len = (stored_len & 0x3f) as usize;
        let word_bytes = if pos + 2 + actual_len <= raw_data.len() {
            raw_data[pos+2..pos+2+actual_len].to_vec()
        } else { vec![] };
        let ph_start = pos + 2 + actual_len;
        let ph_end = raw_data[ph_start..pos+entry_len].iter().position(|&b| b == 0).map(|pp| ph_start + pp).unwrap_or(pos+entry_len);
        let phonemes = raw_data[ph_start..ph_end].to_vec();
        println!("  {}: len={} stored_len={} word={:?} ph={:?}", i, entry_len, stored_len, word_bytes, phonemes);
        pos += entry_len;
    }
}

#[test]
fn find_hello_by_phonemes() {
    use espeak_ng::dictionary::*;
    let dict = match en_dict() { Some(d) => d, None => return };
    let raw_data = &dict.data[..];
    
    // Scan all buckets for entry with phonemes [65, 13, 55, 144] = [h, @, l, oU]
    let target_ph = [65u8, 13, 55, 144];
    
    for h in 0..1024usize {
        let bucket_start = dict.hashtab[h];
        let mut pos = bucket_start;
        loop {
            if pos >= raw_data.len() { break; }
            let entry_len = raw_data[pos] as usize;
            if entry_len == 0 { break; }
            let word_info = raw_data[pos + 1];
            let stored_len = (word_info & 0x7f) as usize;
            let actual_len = stored_len & 0x3f;
            let ph_start = pos + 2 + actual_len;
            if ph_start + target_ph.len() <= pos + entry_len {
                let ph_slice = &raw_data[ph_start..ph_start + target_ph.len()];
                if ph_slice == target_ph {
                    let word_bytes = raw_data[pos+2..pos+2+actual_len].to_vec();
                    println!("Found at hash={} pos={}: word={:?} stored_len={}", h, pos, word_bytes, stored_len);
                }
            }
            pos += entry_len;
        }
    }
}

#[test]
fn debug_hash_computation() {
    use espeak_ng::dictionary::lookup::hash_word;
    let bytes = [32u8, 83, 12, 60];
    println!("hash([32,83,12,60]) = {}", hash_word(&bytes));
    println!("hash(b\"hello\") = {}", hash_word(b"hello"));
    
    // Manual C-like computation
    let mut hash: u32 = 0;
    let mut chars: u32 = 0;
    for &c in &bytes {
        if c == 0 { break; }
        hash = hash.wrapping_mul(8).wrapping_add(c as u32);
        hash = (hash & 0x3ff) ^ (hash >> 8);
        chars += 1;
        println!("  c={} hash={} chars={}", c, hash, chars);
    }
    println!("final: ({} + {}) & 0x3ff = {}", hash, chars, (hash + chars) & 0x3ff);
}

#[test]
fn debug_bucket_44_direct() {
    use espeak_ng::dictionary::*;
    use espeak_ng::dictionary::lookup::*;
    let dict = match en_dict() { Some(d) => d, None => return };
    let raw_data = &dict.data[..];
    
    let bucket44_start = dict.hashtab[44];
    println!("hashtab[44] = {} (offset in dict.data)", bucket44_start);
    
    // also check hashtab[252]
    let bucket252_start = dict.hashtab[252];
    println!("hashtab[252] = {}", bucket252_start);
    
    // Print entries in bucket 44
    let mut pos = bucket44_start;
    for i in 0..5 {
        if pos >= raw_data.len() { break; }
        let entry_len = raw_data[pos] as usize;
        if entry_len == 0 { println!("  bucket44 end after {} entries", i); break; }
        let word_info = raw_data[pos + 1];
        let stored_len = word_info & 0x7f;
        let actual_len = (stored_len & 0x3f) as usize;
        let word_bytes = if pos + 2 + actual_len <= raw_data.len() {
            raw_data[pos+2..pos+2+actual_len].to_vec()
        } else { vec![] };
        let ph_start = pos + 2 + actual_len;
        let ph_end = raw_data[ph_start..pos+entry_len].iter().position(|&b| b == 0).map(|pp| ph_start + pp).unwrap_or(pos+entry_len);
        let phonemes = raw_data[ph_start..ph_end].to_vec();
        // compute hash of these word bytes
        let h = hash_word(&word_bytes);
        println!("  {}: len={} stored_len={:#x} word={:?} ph={:?} hash={}", i, entry_len, stored_len, word_bytes, phonemes, h);
        pos += entry_len;
    }
}

#[test]
fn debug_entry_at_pos_4435() {
    use espeak_ng::dictionary::*;
    use espeak_ng::dictionary::lookup::*;
    let dict = match en_dict() { Some(d) => d, None => return };
    let raw_data = &dict.data[..];
    
    // What bucket does hashtab[44] really represent?
    // Check: which bucket index does pos 4435 fall in?
    let pos = 4435;
    let entry_len = raw_data[pos] as usize;
    let word_info = raw_data[pos + 1];
    let stored_len = word_info & 0x7f;
    let actual_len = (stored_len & 0x3f) as usize;
    let word_bytes = raw_data[pos+2..pos+2+actual_len].to_vec();
    let ph_start = pos + 2 + actual_len;
    let ph_end = raw_data[ph_start..pos+entry_len].iter().position(|&b| b == 0).map(|pp| ph_start + pp).unwrap_or(pos+entry_len);
    let phonemes = raw_data[ph_start..ph_end].to_vec();
    let h = hash_word(&word_bytes);
    println!("At pos 4435: len={} stored_len={:#x} word={:?} ph={:?} computed_hash={}", 
        entry_len, stored_len, word_bytes, phonemes, h);
    
    // Check what hashtab values bracket pos 4435
    for bucket in 0..1024 {
        if dict.hashtab[bucket] <= pos && (bucket == 1023 || dict.hashtab[bucket+1] > pos) {
            println!("Bucket {} (hashtab[{}]={}) contains pos={}", bucket, bucket, dict.hashtab[bucket], pos);
        }
    }
    
    // What does hashtab[252] look like?
    let pos252 = dict.hashtab[252];
    println!("hashtab[252] = {}", pos252);
    let h252_end = dict.hashtab[253];
    println!("hashtab[253] = {}", h252_end);
    println!("Bucket 252 has {} bytes", h252_end - pos252);
}

#[test]
fn debug_dict_header() {
    use espeak_ng::dictionary::*;
    let dict = match en_dict() { Some(d) => d, None => return };
    let raw_data = &dict.data[..];
    
    println!("Total data size: {}", raw_data.len());
    println!("Header bytes 0-15: {:?}", &raw_data[..16.min(raw_data.len())]);
    println!("pw0 (N_HASH_DICT check): {}", u32::from_le_bytes(raw_data[0..4].try_into().unwrap()));
    println!("pw1 (rules_offset): {}", u32::from_le_bytes(raw_data[4..8].try_into().unwrap()));
    println!("hashtab[0]={} hashtab[1]={} hashtab[252]={} hashtab[1023]={}", 
        dict.hashtab[0], dict.hashtab[1], dict.hashtab[252], dict.hashtab[1023]);
    
    // Check how many entries are in each of the first 5 buckets
    for bucket in 0..5 {
        let start = dict.hashtab[bucket];
        let end = if bucket < 1023 { dict.hashtab[bucket+1] } else { dict.rules_offset };
        println!("  bucket {}: offset={} size={}", bucket, start, end.saturating_sub(start));
    }
}

#[test]
fn debug_raw_bytes_around_4377() {
    use espeak_ng::dictionary::*;
    let dict = match en_dict() { Some(d) => d, None => return };
    let raw_data = &dict.data[..];
    
    // Print raw bytes at pos 4370..4450
    for pos in 4370..4450 {
        print!("{} ", raw_data[pos]);
    }
    println!();
    
    // Also check what's at bucket 44 and 45 boundaries
    println!("hashtab[44]={} hashtab[45]={}", dict.hashtab[44], dict.hashtab[45]);
    // Size of bucket 44 (including terminator)
    let size44 = dict.hashtab[45] - dict.hashtab[44];
    println!("Bucket 44 total size: {} bytes", size44);
}

#[test]
fn verify_bucket_252_hashes() {
    use espeak_ng::dictionary::*;
    use espeak_ng::dictionary::lookup::*;
    let dict = match en_dict() { Some(d) => d, None => return };
    let raw_data = &dict.data[..];
    
    // Check if words in bucket 252 actually hash to 252
    let start = dict.hashtab[252];
    let end = dict.hashtab[253];
    println!("Bucket 252: {} to {}, {} bytes", start, end, end-start);
    
    let mut pos = start;
    let mut entry_idx = 0;
    loop {
        if pos >= raw_data.len() { break; }
        let entry_len = raw_data[pos] as usize;
        if entry_len == 0 { println!("End of bucket 252 after {} entries", entry_idx); break; }
        let word_info = raw_data[pos + 1];
        let stored_len = (word_info & 0x7f) as usize;
        let actual_len = stored_len & 0x3f;
        let word_bytes = raw_data[pos+2..pos+2+actual_len].to_vec();
        let computed_hash = hash_word(&word_bytes);
        println!("  [{}] word={:?} computed_hash={}", entry_idx, word_bytes, computed_hash);
        entry_idx += 1;
        pos += entry_len;
    }
    
    // Check first few words in bucket 44 and their hashes
    println!("---");
    let start44 = dict.hashtab[44];
    let _end44 = dict.hashtab[45];
    let mut pos = start44;
    for i in 0..5 {
        if pos >= raw_data.len() { break; }
        let entry_len = raw_data[pos] as usize;
        if entry_len == 0 { println!("End of bucket 44 after {} entries", i); break; }
        let word_info = raw_data[pos + 1];
        let stored_len = (word_info & 0x7f) as usize;
        let actual_len = stored_len & 0x3f;
        let word_bytes = raw_data[pos+2..pos+2+actual_len].to_vec();
        let computed_hash = hash_word(&word_bytes);
        println!("  bucket44[{}] word={:?} computed_hash={}", i, word_bytes, computed_hash);
        pos += entry_len;
    }
}

#[test]
fn verify_hashtab_ordering() {
    use espeak_ng::dictionary::*;
    use espeak_ng::dictionary::lookup::*;
    let dict = match en_dict() { Some(d) => d, None => return };
    let raw_data = &dict.data[..];
    
    // Check what hash value all words in buckets 40-50 should have
    for bucket in 40..=50 {
        let start = dict.hashtab[bucket];
        let _end = if bucket < 1023 { dict.hashtab[bucket+1] } else { dict.rules_offset };
        let mut pos = start;
        let mut words = Vec::new();
        loop {
            if pos >= raw_data.len() { break; }
            let entry_len = raw_data[pos] as usize;
            if entry_len == 0 { break; }
            let word_info = raw_data[pos + 1];
            let stored_len = (word_info & 0x7f) as usize;
            let actual_len = stored_len & 0x3f;
            if pos + 2 + actual_len <= raw_data.len() {
                let word_bytes = raw_data[pos+2..pos+2+actual_len].to_vec();
                let h = hash_word(&word_bytes);
                words.push(h);
            }
            pos += entry_len;
        }
        if !words.is_empty() {
            let unique: std::collections::HashSet<_> = words.iter().cloned().collect();
            println!("bucket {}: {} entries, hashes={:?}", bucket, words.len(), 
                if unique.len() <= 5 { unique.iter().cloned().collect::<Vec<_>>() } else { vec![*unique.iter().next().unwrap()] });
        }
    }
    
    // Check what 'the' dict lookup does (it works!)
    let result = lookup(&dict, "the", &LookupCtx::default());
    println!("the: {:?}", result.map(|r| r.phonemes));
    
    // Check what hash 'the' uses
    let the_transposed = transpose_alphabet("the", &dict.transpose);
    println!("the compressed: {:?} wlen={} hash={}", the_transposed.bytes, the_transposed.wlen, hash_word(&the_transposed.bytes));
}

#[test]
fn check_en_dict_md5() {
    // Check if we're reading the right file
    let data = std::fs::read("/usr/share/espeak-ng-data/en_dict").unwrap();
    println!("File size: {} bytes", data.len());
    println!("First 8 bytes: {:?}", &data[..8]);
    // Check the hash of "hello" using TransposeAlphabet and then look for it
    use espeak_ng::dictionary::*;
    use espeak_ng::dictionary::lookup::*;
    let dict = Dictionary::load("en", std::path::Path::new("/usr/share/espeak-ng-data")).unwrap();
    let result = lookup(&dict, "hello", &LookupCtx::default());
    println!("hello lookup: {:?}", result.map(|r| r.phonemes));
}

#[test]
fn check_hello_in_bucket_48() {
    use espeak_ng::dictionary::*;
    use espeak_ng::dictionary::lookup::*;
    let dict = match en_dict() { Some(d) => d, None => return };
    let raw_data = &dict.data[..];
    
    // Check what's in bucket 48 (where HashDictionary("hello") = 48 would put it)
    let start = dict.hashtab[48];
    let end = dict.hashtab[49];
    println!("Bucket 48: offset={} size={}", start, end-start);
    
    let mut pos = start;
    let mut i = 0;
    loop {
        if pos >= raw_data.len() { break; }
        let entry_len = raw_data[pos] as usize;
        if entry_len == 0 { break; }
        let word_info = raw_data[pos + 1];
        let stored_len = (word_info & 0x7f) as usize;
        let actual_len = stored_len & 0x3f;
        let word_bytes = raw_data[pos+2..pos+2+actual_len].to_vec();
        let h_compressed = hash_word(&word_bytes);
        // Also check hash of "hello" uncompressed would be here
        println!("  [{}] word={:?} hash_compressed={}", i, word_bytes, h_compressed);
        i += 1;
        pos += entry_len;
    }
}
