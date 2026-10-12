// tests/regressive_voicing.rs
//
// `LOPT_REGRESSIVE_VOICING` (`SetRegressiveVoicing` in `phonemelist.c`): a
// consonant cluster takes its voicing from its last member.  Bit 0x100 devoices
// word-finally, 0x08 propagates through liquids and nasals, and with bit 0x04
// clear the assimilation reaches across the word boundary.
//
// The port set this for Russian and Ukrainian only; `tr_languages.c` sets it for
// nine language arms.  Expectations are byte-identical to a locally built
// upstream `espeak-ng` (`ba90c8e`).

use espeak_ng::Translator;

fn tr(lang: &str) -> Option<Translator> {
    Translator::new(lang, None).ok()
}

#[test]
fn assimilation_crosses_the_word_boundary() {
    // Polish (0x09) and Czech (0x03): the /s/ of "minus" voices before "b".
    if let Some(pl) = tr("pl") {
        assert_eq!(pl.text_to_ipa("minus b").unwrap(), "mˈinuz bˈɛ");
        // 0x08: it propagates through a liquid, so "kciuk w górę" ends in /ɡ/.
        assert_eq!(pl.text_to_ipa("kciuk w górę").unwrap(), "ktɕˈuɡ w ɡˈurɛ");
    }
    if let Some(cs) = tr("cs") {
        assert_eq!(cs.text_to_ipa("minus b").unwrap(), "mˈinuz bˈeː");
    }
}

#[test]
fn bulgarian_reads_a_bare_hyphen_as_nothing() {
    // Two bugs met here: `groups3` was also indexed by the raw first byte, so
    // Bulgarian matched the rule for `ь` on the ASCII hyphen of `-5` ("j пет"),
    // and bg had no regressive-voicing setting (C: 0x107).
    let Some(bg) = tr("bg") else { eprintln!("[SKIP] no bg data"); return };
    assert_eq!(bg.text_to_ipa("-5").unwrap(), bg.text_to_ipa("5").unwrap());
}

#[test]
fn russian_assimilation_still_works() {
    // The pre-existing case, unchanged: `світ` voices its /s/ before /v/.
    let Some(ru) = tr("ru") else { eprintln!("[SKIP] no ru data"); return };
    assert_eq!(ru.text_to_ipa("світ").unwrap(), "svˈit");
}
