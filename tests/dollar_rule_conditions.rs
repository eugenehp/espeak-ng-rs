// tests/dollar_rule_conditions.rs
//
// `$list` and `$p_alt<n>` rule conditions — C's `DollarRule` (dictionary.c).
// Both ask about the word *up to the current match point*: `$list` succeeds if
// that prefix is an entry in the `_list` file (and not a `$only` one), and
// `$p_alt<n>` if the entry carries `$alt<n>`.  The port used to fail both
// conditions outright, which cost Danish the stress of every Latinate word its
// list knows (~300 rules), and Dutch 34 more.
//
// Values from a locally built upstream `espeak-ng` (`ba90c8e`).

use espeak_ng::Translator;

fn tr(lang: &str) -> Option<Translator> {
    let t = Translator::new(lang, Some(std::path::Path::new("espeak-ng-data"))).ok()?;
    t.text_to_ipa("a").ok()?;
    Some(t)
}

macro_rules! skip_unless {
    ($lang:expr) => {
        match tr($lang) {
            Some(t) => t,
            None => {
                eprintln!("[SKIP] no {} data", $lang);
                return;
            }
        }
    };
}

/// Danish stresses the Latinate ending when the stem is a `$alt`-flagged list
/// entry: `fatal` is `fatˈal`, not `fˈatal`.
#[test]
fn danish_p_alt_stress() {
    let da = skip_unless!("da");
    let cases: &[(&str, &str)] = &[
        ("biograf", "bˌʔiʔoɡʁˈ?ɑf"),
        ("biografi", "bˌʔiʔoɡʁ?ɑfˈʔi"),
        ("fatal", "fatˈal"),
        ("monumental", "mˌʔonumˈεntˈal"),
        ("specialist", "spˌʔesjalˈist"),
        ("anale", "anˈalə"),
        ("national", "naʃˈʔonˈal"),
        ("total", "totˈal"),
        ("krystalklar", "kʁʔystˈalklɑ"),
        ("regional", "ʁˌεɡʔiˈʔonˈal"),
        ("central", "sɛntʁˈ?ɑl"),
    ];
    for (word, want) in cases {
        assert_eq!(&da.text_to_ipa(word).unwrap(), want, "da {word}");
    }
}

/// The same condition in Dutch, German and Romanian — these were already right
/// by other rules, so they also guard against the condition now firing where it
/// should not.
#[test]
fn the_other_languages_are_unchanged() {
    if let Some(nl) = tr("nl") {
        assert_eq!(nl.text_to_ipa("model").unwrap(), "moːdˈɛl");
        assert_eq!(nl.text_to_ipa("modellen").unwrap(), "moːdˈɛlən");
        assert_eq!(nl.text_to_ipa("hotel").unwrap(), "hoːtˈɛl");
    }
    if let Some(de) = tr("de") {
        assert_eq!(de.text_to_ipa("Garage").unwrap(), "ɡarˈɑʒə");
        assert_eq!(de.text_to_ipa("Etage").unwrap(), "ˈeːtɑːɡə");
    }
    if let Some(ro) = tr("ro") {
        assert_eq!(ro.text_to_ipa("radio").unwrap(), "rˈadjo");
    }
}

/// A flags-only list entry must reach the rules as `dict_flags0`, which is what
/// `$w_alt<n>` tests: `en_list`'s `banana $alt3` is what selects the `a#` rule.
#[test]
fn english_w_alt_sees_a_flags_only_entry() {
    let en = skip_unless!("en");
    // `ba#n'A:n@` / `ka#T'i:dr@L` upstream — the first vowel is `a#`, not `a`.
    assert_eq!(en.text_to_ipa("banana").unwrap(), "bɐnˈɑːnə");
    assert_eq!(en.text_to_ipa("cathedral").unwrap(), "kɐθˈiːdɹəl");
    // …and a word without the flag keeps its own reading.
    assert_eq!(en.text_to_ipa("banal").unwrap(), "bˈeɪnəl");
    assert_eq!(en.text_to_ipa("canal").unwrap(), "kənˈal");
}
