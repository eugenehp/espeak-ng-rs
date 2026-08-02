// tests/urdu_sindhi.rs
//
// Urdu (ur) and Sindhi (sd) reading.  Both use the Arabic script (U+0600 block)
// and ship substantial rule sets, but — unlike Arabic/Farsi — were missing the
// `letter_bits_offset = 0x600` that groups3 needs, so every word produced empty
// output.  Adding the offset restores them.

use espeak_ng::Translator;

fn reads(lang: &str, cases: &[(&str, &str)]) {
    let Some(t) = Translator::new(lang, None).ok() else {
        eprintln!("[SKIP] no {lang} data");
        return;
    };
    for (word, frag) in cases {
        let got = t.text_to_ipa(word).unwrap();
        assert!(!got.trim().is_empty(), "{lang} {word:?} produced no phonemes");
        assert!(
            got.contains(frag),
            "{lang} {word:?}: expected to contain {frag:?}, got {got:?}",
        );
    }
}

#[test]
fn urdu_words_read() {
    // اردو → "urdu", کتاب → "kitab", پاکستان → "pakistan".
    reads("ur", &[("اردو", "rd"), ("کتاب", "kɪtˈaːb"), ("پاکستان", "stˈaːn")]);
}

#[test]
fn sindhi_words_read() {
    // سنڌي → "sindhi", ڪتاب → "kitab".
    reads("sd", &[("سنڌي", "sˈɪnd"), ("ڪتاب", "kɪtˈaːb")]);
}
