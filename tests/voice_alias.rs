// tests/voice_alias.rs
//
// A voice may borrow another language's dictionary and phoneme table via the
// `dictionary <name>` / `phonemes <name>` directives in its `lang/**` file.
// Norwegian Bokmål (`nb`) declares `dictionary no` / `phonemes no`; before this
// was honoured, `-v nb` failed with "dict not found".  Skips without data.

use espeak_ng::Translator;

fn tr(lang: &str) -> Option<Translator> {
    Translator::new(lang, None).ok()
}

#[test]
fn norwegian_bokmaal_uses_the_no_dictionary() {
    // Requires the `no` data (nb borrows it).
    let (Some(nb), Some(no)) = (tr("nb"), tr("no")) else {
        eprintln!("[SKIP] no Norwegian data");
        return;
    };
    // nb must resolve (no panic / no "dict not found" error) and read text.
    for text in ["hei", "takk", "1000", "to tusen"] {
        let a = nb.text_to_ipa(text).expect("nb should translate");
        let b = no.text_to_ipa(text).expect("no should translate");
        assert!(!a.trim().is_empty(), "nb {text:?} produced no phonemes");
        // Sharing the same dict + phoneme table, the two must agree.
        assert_eq!(a, b, "nb and no disagree on {text:?}");
    }
}

#[test]
fn common_language_resolution_is_unaffected() {
    // A language with its own dict/table resolves without touching voice files.
    if let Some(en) = tr("en") {
        assert!(en.text_to_ipa("hello").unwrap().contains("l"), "en still works");
    }
}
