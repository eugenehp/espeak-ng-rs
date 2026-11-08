// tests/polish_final_l.rs
//
// Polish rules key `ł (_` on letter group A, and C's `SetLetterVowel(tr, 'y')`
// puts `y` in that group.  Without it a word-final `ł` after `y` was dropped.
//
// Values from `espeak-ng` 1.52.0.

use espeak_ng::Translator;

fn tr(lang: &str) -> Option<Translator> {
    let t = Translator::new(lang, Some(std::path::Path::new("espeak-ng-data"))).ok()?;
    t.text_to_ipa("a").ok()?;
    Some(t)
}

#[test]
fn polish_voicing_crosses_number_word_boundaries() {
    let Some(pl) = tr("pl") else {
        eprintln!("[SKIP] no pl data");
        return;
    };
    for (num, ipa) in [
        ("1200", "tˈɨɕɔndʑ dvʲˈɛɕcʲɛ"),
        ("1234", "tˈɨɕɔndʑ dvʲˈɛɕcʲɛ tʃɨdʑˈɛɕcʲitʃtˈɛrɨ"),
        ("1100", "tˈɨɕɔnts stˈɔ"),
    ] {
        assert_eq!(pl.text_to_ipa(num).ok().as_deref(), Some(ipa), "{num}");
    }
}

#[test]
fn polish_final_l_after_y_is_kept() {
    let Some(pl) = tr("pl") else {
        eprintln!("[SKIP] no pl data");
        return;
    };
    for (word, ipa) in [("był", "bˈɨw"), ("szył", "ʃˈɨw"), ("tył", "tˈɨw"), ("dał", "dˈaw")] {
        assert_eq!(pl.text_to_ipa(word).ok().as_deref(), Some(ipa), "{word}");
    }
}
