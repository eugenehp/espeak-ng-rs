// tests/korean.rs
//
// Korean (ko) Hangul reading.  Precomposed Hangul syllables (U+AC00–U+D7A3) are
// decomposed into conjoining jamo — leading consonant (U+1100+L), vowel
// (U+1161+V), optional trailing consonant (U+11A7+T) — the form ko_rules
// matches, with `letter_bits_offset = 0x1100` so groups3 finds the jamo groups.
// The leading ㅇ (silent vowel-syllable placeholder) is omitted.  Previously all
// Hangul produced empty output.  Skips without data.

use espeak_ng::Translator;

#[test]
fn hangul_syllables_are_read() {
    let Some(t) = Translator::new("ko", None).ok() else {
        eprintln!("[SKIP] no ko data");
        return;
    };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();

    // Consonant-initial syllables read directly.
    assert_eq!(ipa("한"), "hˈɐn"); // han
    assert_eq!(ipa("사랑"), "sɐɾˈɐŋ"); // sarang (love) — 2LLH puts the stress on the heavy 2nd syllable

    // Vowel-initial syllables: the silent leading ㅇ must NOT voice — 안 is "an",
    // not "iɯŋan".
    assert_eq!(ipa("안"), "ˈɐn");
    for (word, want) in [("아", "ˈɐ"), ("이", "ˈi"), ("오", "ˈo")] {
        assert_eq!(ipa(word), want, "ko {word}");
    }

    // Common phrases produce sensible, non-empty output.
    for word in ["안녕", "안녕하세요", "감사합니다", "한국", "김치"] {
        let got = ipa(word);
        assert!(!got.trim().is_empty(), "ko {word:?} produced no phonemes");
        // No stray "ng"-cluster from a mishandled leading ㅇ.
        assert!(!got.starts_with('ŋ'), "ko {word:?} should not start with ŋ: {got}");
    }
    // "감사합니다" (thank you) begins with a voiced word-initial ㄱ.
    assert!(ipa("감사합니다").starts_with('ɡ'), "감… should start voiced: {}", ipa("감사합니다"));
}
