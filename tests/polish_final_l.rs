// Polish rules key "ł (_" on letter group A; C's SetLetterVowel(tr, 'y') puts y in it.
//
// Values from `espeak-ng` 1.52.0 and master (f0fb11a8); they agree except where
// noted.

use espeak_ng::Translator;

fn tr(lang: &str) -> Option<Translator> {
    let t = Translator::new(lang, Some(std::path::Path::new("espeak-ng-data"))).ok()?;
    t.text_to_ipa("a").ok()?;
    Some(t)
}

// Regressive voicing carries across the word boundaries inside a number.
#[test]
fn polish_voicing_crosses_number_word_boundaries() {
    let Some(pl) = tr("pl") else {
        eprintln!("[SKIP] no pl data");
        return;
    };
    // Voiced tysiąc is ndʑ in the 1.52.0 data and ndz in master.
    for (num, ipas) in [
        ("1200", &["tˈɨɕɔndʑ dvʲˈɛɕcʲɛ", "tˈɨɕɔndz dvʲˈɛɕcʲɛ"][..]),
        (
            "1234",
            &[
                "tˈɨɕɔndʑ dvʲˈɛɕcʲɛ tʃɨdʑˈɛɕcʲitʃtˈɛrɨ",
                "tˈɨɕɔndz dvʲˈɛɕcʲɛ tʃɨdʑˈɛɕcʲitʃtˈɛrɨ",
            ][..],
        ),
        ("1100", &["tˈɨɕɔnts stˈɔ"][..]),
    ] {
        let got = pl.text_to_ipa(num).unwrap_or_default();
        assert!(ipas.contains(&got.as_str()), "{num}: {got:?} not in {ipas:?}");
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
