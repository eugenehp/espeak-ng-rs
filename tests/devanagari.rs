// tests/devanagari.rs
//
// Hindi / Marathi / Nepali (Devanagari) reading.  Three things were needed:
//   1. OFFSET_DEVANAGARI (0x900) so the rules find the multi-byte rule chains;
//   2. the Devanagari letter groups — crucially group B (vowel signs + virama)
//      so a consonant followed by a virama/matra drops its inherent vowel;
//   3. the tokenizer keeping the virama (a non-alphabetic combining mark) inside
//      the word, so conjuncts (`स्त`) aren't split into two words;
//   4. word-final schwa deletion (`भारत` "bharatə" → "bharat").
// Previously all Devanagari words produced empty output.  Skips without data.

use espeak_ng::Translator;

#[test]
fn hindi_words_read_with_conjuncts_and_schwa_deletion() {
    let Some(t) = Translator::new("hi", None).ok() else {
        eprintln!("[SKIP] no hi data");
        return;
    };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();
    let cases: &[(&str, &str)] = &[
        ("नमस्ते", "nʌmˈʌsteː"),  // conjunct स्त works
        ("भारत", "bʰˈaːrʌt"),     // word-final schwa deleted (not "…rʌtʌ")
        ("किताब", "kɪtˈaːb"),     // ditto
        ("स्कूल", "skˈuːl"),      // leading conjunct स्क + final schwa deleted
        ("पानी", "pˈaːni"),
    ];
    for (word, want) in cases {
        let got = ipa(word);
        assert_eq!(got, *want, "hi {word:?}");
    }
    // हिंदी → "hindi" (nasalised i may render precomposed or with a combining
    // tilde, so check the stable parts and that no trailing schwa was left).
    let hindi = ipa("हिंदी");
    assert!(hindi.starts_with('h') && hindi.ends_with("di"), "हिंदी → hindi: {hindi}");
    // CVC words: keep the first inherent vowel, delete the word-final one.
    for (word, want) in [("घर", "ɡʰˈʌr"), ("कर", "kˈʌr"), ("मन", "mˈʌn")] {
        assert_eq!(ipa(word), want, "hi {word:?}");
    }
    // A single consonant keeps its inherent vowel (deletion needs another vowel).
    assert!(ipa("क").contains('ə') || ipa("क").contains('ʌ'), "क keeps schwa: {}", ipa("क"));
    // A full sentence reads.
    assert!(!ipa("मेरा नाम राहुल है").trim().is_empty(), "hi sentence");
}

#[test]
fn marathi_and_nepali_use_the_same_devanagari_path() {
    for lang in ["mr", "ne"] {
        let Some(t) = Translator::new(lang, None).ok() else {
            eprintln!("[SKIP] no {lang} data");
            continue;
        };
        let out = t.text_to_ipa("धन्यवाद").unwrap();
        assert!(out.contains("aːd") && !out.trim().is_empty(), "{lang} धन्यवाद: {out}");
    }
}

#[test]
fn other_indic_scripts_read_words() {
    // All the ISCII-layout Indic scripts share the offset + letter-group +
    // combining-mark fix, so a "hello"/"thank you"-type word now reads (a
    // conjunct in each collapses correctly) instead of producing empty output.
    let cases: &[(&str, &str, &str)] = &[
        ("bn", "নমস্কার", "ʃk"),   // Bengali — স্ক conjunct
        ("ta", "வணக்கம்", "kk"),    // Tamil — க்க
        ("te", "నమస్తే", "st"),     // Telugu — స్త
        ("kn", "ನಮಸ್ಕಾರ", "sk"),   // Kannada — ಸ್ಕ
        ("ml", "നമസ്കാരം", "s"),    // Malayalam
        ("gu", "નમસ્તે", "st"),     // Gujarati — સ્ત
        ("or", "ନମସ୍କାର", "sk"),    // Oriya
    ];
    for (lang, word, want) in cases {
        let Some(t) = Translator::new(lang, None).ok() else {
            eprintln!("[SKIP] no {lang} data");
            continue;
        };
        let got = t.text_to_ipa(word).unwrap();
        assert!(!got.trim().is_empty(), "{lang} {word:?} produced no phonemes");
        assert!(got.contains(want), "{lang} {word:?}: want a {want:?} conjunct, got {got:?}");
    }
}
