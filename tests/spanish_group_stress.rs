// tests/spanish_group_stress.rs
//
// The "Spanish group" (es/an/ia/ca/pap) shares a stress rule but not its flags.
// Catalan adds `S_NO_AUTO_2 | S_FIRST_PRIMARY` and Papiamento uses
// `S_FINAL_VOWEL_UNSTRESSED` where Spanish uses `S_FINAL_SPANISH`; the port had
// a leftover arm that overwrote all five with Spanish's own flags, so every
// Catalan word of three syllables or more picked up an automatic secondary
// stress — which also blocked the vowel reduction that depends on it
// (`entenem` was `ˌentənˈɛm`, not `əntənˈɛm`).
//
// Values from a locally built upstream `espeak-ng` (`ba90c8e`).

use espeak_ng::Translator;

fn tr(lang: &str) -> Option<Translator> {
    let t = Translator::new(lang, Some(std::path::Path::new("espeak-ng-data"))).ok()?;
    t.text_to_ipa("a").ok()?;
    Some(t)
}

/// Catalan: no automatic secondary stress, so the pre-stress vowels reduce.
#[test]
fn catalan_has_no_automatic_secondary_stress() {
    let Some(ca) = tr("ca") else {
        eprintln!("[SKIP] no ca data");
        return;
    };
    let cases: &[(&str, &str)] = &[
        ("entenem", "əntənˈɛm"),
        ("Barcelona", "bərsəlˈonə"),
        ("televisió", "teleβiziˈo"),
        ("animal", "ənimˈal"),
        ("universitat", "uniβərsitˈat"),
        ("casa", "kˈazə"),
        ("paraula", "pəɾˈawlə"),
        ("telefon", "telefˈon"),
    ];
    for (word, want) in cases {
        assert_eq!(&ca.text_to_ipa(word).unwrap(), want, "ca {word}");
        assert!(
            !ca.text_to_ipa(word).unwrap().contains('ˌ'),
            "ca {word} should carry no secondary stress"
        );
    }
    // A decimal separator inside a number group keeps *its* secondary stress,
    // which comes from `NUM_SINGLE_STRESS`, not from the automatic fill.
    assert_eq!(ca.text_to_ipa("3,14").unwrap(), "tɾˈɛs kˌomə kətˌorzə");
}

/// Spanish, Aragonese and Interlingua *do* use it — the fix must not take
/// theirs away.
#[test]
fn spanish_keeps_its_secondary_stress() {
    if let Some(es) = tr("es") {
        assert_eq!(es.text_to_ipa("animal").unwrap(), "ˌanimˈal");
        assert_eq!(es.text_to_ipa("universidad").unwrap(), "ˌuniβˌeɾsiðˈad");
    }
    for lang in ["an", "ia"] {
        if let Some(t) = tr(lang) {
            assert_eq!(t.text_to_ipa("animal").unwrap(), "ˌanimˈal", "{lang}");
        }
    }
}

/// Papiamento has `S_NO_AUTO_2` too, and its own final-vowel rule.
#[test]
fn papiamento_has_its_own_flags() {
    let Some(pap) = tr("pap") else {
        eprintln!("[SKIP] no pap data");
        return;
    };
    assert_eq!(pap.text_to_ipa("animal").unwrap(), "animˈal");
    assert_eq!(pap.text_to_ipa("kasa").unwrap(), "kˈasa");
}
