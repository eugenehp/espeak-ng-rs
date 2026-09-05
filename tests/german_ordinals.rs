// tests/german_ordinals.rs
//
// German forms a compound ordinal by suffixing the whole cardinal
// ("einundzwanzig" + "ste" = "einundzwanzigste"), not by tens-ordinal +
// units-ordinal.  Compound ordinals used to read the wrong two-word form
// ("zwanzig erste" for 21.); they now read as one word, consistent with the
// round-ten ordinals (20. → "zwanzigste").

use espeak_ng::Translator;

#[test]
fn compound_ordinals_are_one_word() {
    let Some(t) = Translator::new("de", None).ok() else {
        eprintln!("[SKIP] no de data");
        return;
    };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();

    // A compound ordinal must not split into "zwanzig erste" (two words with the
    // units read as a separate ordinal).  The old bug produced "erste"/"zweite"
    // etc. as a trailing word — guard against that stress pattern.
    for (input, tail_of_20s) in [("21.", "tsvˈantsɪçtə"), ("22.", "tsvˈantsɪçtə")] {
        let got = ipa(input);
        assert!(got.ends_with(tail_of_20s), "{input} should end like '…zwanzigste': {got}");
        assert!(!got.contains("ˈeːɾstə"), "{input} must not read a separate 'erste': {got}");
    }
    // 21. reads as one compound cardinal + suffix (matches '20.' ending).
    assert_eq!(ipa("21."), format!("ˌaɪn ʊnt{}", ipa("20.")));
    // Round and small ordinals are unchanged.
    assert_eq!(ipa("1."), "ˈeːɾstə");
    assert_eq!(ipa("3."), "dɾˈɪtə");
    assert_eq!(ipa("20."), "tsvˈantsɪçtə");
    assert_eq!(ipa("100."), "hˈʊndɜtstə");
    // A German date reads naturally.
    assert!(ipa("der 21. Mai").contains("ʊnttsvˈantsɪçtə"), "{}", ipa("der 21. Mai"));
}

#[test]
fn dutch_compound_ordinals_are_one_word() {
    // Dutch forms compound ordinals the same way ("eenentwintigste"), not
    // "twintig eerste".
    let Some(t) = Translator::new("nl", None).ok() else {
        eprintln!("[SKIP] no nl data");
        return;
    };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();
    for input in ["21e", "22e", "33e"] {
        assert!(!ipa(input).contains("ˈɪːrstə"), "{input} must not read a separate 'eerste': {}", ipa(input));
    }
    // 21e = "een en" + the '20e' ending (one compound word).  The compound
    // demotes the second element's stress to secondary, as upstream does
    // (`ˈeːnɛntʋˌɪntəxstə`), so compare the segments rather than the marks.
    let strip = |s: String| s.chars().filter(|c| !matches!(c, 'ˈ' | 'ˌ')).collect::<String>();
    assert_eq!(strip(ipa("21e")), format!("eːnɛn{}", strip(ipa("20e"))));
    assert!(ipa("21e").contains("tʋˌɪntəx"), "the compound's tail is secondary: {}", ipa("21e"));
    // Round/small unchanged.
    assert_eq!(ipa("1e"), "ˈɪːrstə");
    assert_eq!(ipa("100e"), "hˈɔndɛrtstə");
}

#[test]
fn german_one_apocopates_in_compounds() {
    // "einundzwanzig", not "einsundzwanzig": the unit 1 before "und" is "ein"
    // (no final "s"), while standalone 1 and final-position 1 keep "eins".
    let Some(t) = Translator::new("de", None).ok() else {
        eprintln!("[SKIP] no de data");
        return;
    };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();

    assert_eq!(ipa("1"), "ˈaɪns", "standalone 1 = eins");
    // 21..91 use "ein" (aɪn), never "eins" (aɪns) before "und".
    for n in ["21", "31", "41", "91"] {
        let got = ipa(n);
        assert!(got.contains("aɪn ʊnt"), "{n} should be 'ein und…': {got}");
        assert!(!got.contains("aɪns"), "{n} must not use 'eins': {got}");
    }
    // …but 101 / 1001 keep "eins" (they end in it).
    assert!(ipa("101").ends_with("ˈaɪns"), "101 = hunderteins: {}", ipa("101"));
    assert!(ipa("1001").ends_with("ˈaɪns"), "1001 = tausendeins: {}", ipa("1001"));
}

#[test]
fn english_compound_ordinals_unaffected() {
    // The change is gated to German (compound_cardinal_suffix); English keeps
    // the "twenty-first" tens + units-ordinal form.
    let Some(t) = Translator::new("en", None).ok() else {
        eprintln!("[SKIP] no en data");
        return;
    };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();
    assert_eq!(ipa("21st"), "twˈɛnti fˈɜːst");
    assert_eq!(ipa("33rd"), "θˈɜːti θˈɜːd");
    assert_eq!(ipa("101st"), "wˈɒnhˈʌndɹɪd fˈɜːst");
}
