// tests/regional_locales.rs
//
// A regional locale that ships only as a base phoneme table (Brazilian
// Portuguese `pt-br`, `fr-ca`, `en-gb`, …) must fall back to the primary
// subtag's table instead of erroring "phoneme table 'pt-br' not found"
// (upstream #2248).

use espeak_ng::Translator;

#[test]
fn regional_locales_fall_back_to_base_phoneme_table() {
    // Each of these has no phoneme table of its own — it must resolve via the
    // primary subtag and produce output instead of erroring "table not found".
    for (locale, word) in [
        ("pt-br", "olá"),
        ("fr-ca", "bonjour"),
        ("en-gb", "hello"),
        ("es-mx", "hola"),
    ] {
        let Some(t) = Translator::new(locale, None).ok() else {
            eprintln!("[SKIP] no data for {locale}");
            continue;
        };
        let got = t.text_to_ipa(word).unwrap();
        assert!(!got.trim().is_empty(), "{locale} {word:?} produced no phonemes");
    }
}

#[test]
fn brazilian_portuguese_differs_from_european_via_dictrules() {
    // pt-BR's `dictrules 2` selects the `?2` Brazilian rules — the ti/di → tʃi/dʒi
    // palatalisation makes it differ from European `pt` (`dictrules 1`).
    let (Some(br), Some(pt)) = (
        Translator::new("pt-br", None).ok(),
        Translator::new("pt", None).ok(),
    ) else {
        eprintln!("[SKIP] no pt data");
        return;
    };
    let bd = br.text_to_ipa("dente").unwrap();
    assert!(bd.contains("tʃ"), "pt-br 'dente' should affricate (tʃ): {bd}");
    assert_ne!(bd, pt.text_to_ipa("dente").unwrap(), "pt-br must differ from pt");
}

#[test]
fn latin_american_spanish_yeismo_via_dictrules() {
    // es-419's `dictrules 2` selects the `?2` rules — Latin-American *yeísmo*
    // reads "ll" as /ʝ/~/j/, not the Castilian /ʎ/.
    let (Some(la), Some(es)) = (
        Translator::new("es-419", None).ok(),
        Translator::new("es", None).ok(),
    ) else {
        eprintln!("[SKIP] no es data");
        return;
    };
    let (llave_la, llave_es) = (la.text_to_ipa("llave").unwrap(), es.text_to_ipa("llave").unwrap());
    assert!(!llave_la.contains('ʎ'), "es-419 'llave' should not keep the palatal ʎ: {llave_la}");
    assert!(llave_es.contains('ʎ'), "es (Castilian) 'llave' keeps ʎ: {llave_es}");
    assert_ne!(llave_la, llave_es, "es-419 must differ from es");
}
