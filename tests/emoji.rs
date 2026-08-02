// tests/emoji.rs
//
// Emoji are read by name via the dictionary's emoji-name entries (which are
// TEXTMODE replacements, e.g. `😀` → text "grinning face", re-translated as
// words).  Previously emoji were dropped entirely.  Skips without en data.

use espeak_ng::Translator;

fn en() -> Option<Translator> {
    Translator::new("en", None).ok()
}

#[test]
fn emoji_read_as_names() {
    let Some(t) = en() else { eprintln!("[SKIP] no en data"); return };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();
    // Each emoji reads the same as spelling out its name.
    let cases: &[(&str, &str)] = &[
        ("😀", "grinning face"),
        ("🎉", "party popper"),
        ("😂", "face with tears of joy"),
        ("💯", "hundred points"),
        ("❤", "red heart"),
    ];
    for (emoji, name) in cases {
        assert_eq!(ipa(emoji), ipa(name), "{emoji} should read as {name:?}");
    }
    // The variation-selector form reads the same as the bare emoji.
    assert_eq!(ipa("❤️"), ipa("❤"), "❤️ (with VS16) == ❤");
    // In a sentence, the emoji is a spoken word between the others.
    assert_eq!(ipa("hello 😀"), ipa("hello grinning face"));
    // A non-emoji symbol is unaffected.
    assert!(!ipa("2×3").trim().is_empty(), "× still works");
}
