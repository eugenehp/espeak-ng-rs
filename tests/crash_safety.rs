// tests/crash_safety.rs
//
// Regression tests locking in memory-safety guarantees for inputs that crash
// (segfault) the upstream C engine.  Rust is memory-safe, so the point is to
// assert these paths return cleanly (Ok or a typed Err) rather than panic.
// Skips when the relevant language data is absent.

use espeak_ng::Translator;

fn works(lang: &str) -> Option<Translator> {
    let t = Translator::new(lang, None).ok()?;
    // Only run if this language's data actually resolves.
    t.text_to_ipa("a").is_ok().then_some(t)
}

#[test]
fn greek_replace_group_chars_do_not_panic() {
    // Upstream espeak-ng #2467: certain Greek letters (ϖ = U+03D6) inside a
    // `.replace` group segfault the C engine.  The Rust port must not panic.
    let Some(t) = works("el") else { eprintln!("[SKIP] no el data"); return };
    for input in ["\u{03D6}", "\u{03D6}\u{03D6}\u{03D6}", "\u{03D6} τεστ", "αϖβ"] {
        // The result content is irrelevant; it must simply not panic.
        let _ = t.text_to_ipa(input);
    }
}

#[test]
fn unknown_and_empty_voice_error_cleanly() {
    // Upstream #2469 segfaults on an empty/unknown phoneme table.  Here it must
    // surface as a typed error, never a panic.
    let Some(_) = works("en") else { eprintln!("[SKIP] no en data"); return };

    let t = Translator::new("definitely-not-a-language", None).expect("builds lazily");
    assert!(t.text_to_ipa("hello").is_err(), "unknown language must error");

    let t = Translator::new("", None).expect("builds lazily");
    assert!(t.text_to_ipa("hello").is_err(), "empty language must error");
}

#[test]
fn adversarial_inputs_do_not_panic() {
    // A grab-bag of odd inputs that have historically tripped tokenizers.
    let Some(t) = works("en") else { return };
    for input in [
        "",
        "   ",
        "\u{0000}\u{FFFD}",
        "((([[[",         // unterminated brackets
        "[[",             // unterminated inline-phoneme open
        "a\u{200B}b",     // zero-width space
        "😀🇬🇧",           // emoji + flag sequence
        "\t\n\r",
        &"a,".repeat(200), // many clause boundaries
    ] {
        let _ = t.text_to_ipa(input);
    }
}
