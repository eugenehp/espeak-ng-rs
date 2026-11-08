// A dictionary entry that only carries a stress flag fixes the stressed syllable,
// so a rule-emitted "stress the preceding vowel" marker must not add a second
// primary stress.  Expected strings are from C espeak-ng 1.52.0.

use espeak_ng::Translator;

#[test]
fn flagged_rule_words_keep_a_single_primary_stress() {
    let Some(t) = Translator::new("it", None).ok() else {
        eprintln!("[SKIP] no it data");
        return;
    };
    let cases = [
        ("invalidano", "invˈalidano"),
        ("oper", "ˈɔper"),
        ("lumache", "lʊmˈake"),
        ("formatosi", "formˈatozɪ"),
        ("kimono", "kimˈɔno"),
        ("corner", "kˈɔrner"),
        ("mitigano", "mˈitiɡano"),
        ("pacifici", "patʃˈifitʃɪ"),
        ("sostenere", "sostenˈɛre"),
    ];
    for (word, want) in cases {
        let got = t.text_to_ipa(word).unwrap_or_default();
        assert_eq!(got, want, "{word}");
    }
}
