// tests/camelcase.rs
//
// Mixed-case / CamelCase word splitting (upstream issue #228).  espeak's
// TranslateClause inserts a word break at a lowercase→uppercase boundary
// (`c = ' '`), so `CamelCase` reads as "Camel Case" and `iPhone` as "i Phone".
// An all-uppercase run has no lower→upper edge, so acronyms (`HTML`) and the
// upper-prefix case (`JABberwocky`, the specific shape #228 is about) stay
// whole.  Irish (`ga`) is the sole language whose `UpperCaseInWord` suppresses
// the split (lenition prefixes legitimately mix case).

use espeak_ng::Translator;

fn en() -> Option<Translator> {
    Translator::new("en", None).ok()
}

#[test]
fn lowercase_to_uppercase_splits_into_words() {
    let Some(t) = en() else {
        eprintln!("[SKIP] no en data");
        return;
    };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();
    // Each split word gets its own stress + a space, identical to writing the
    // words separately.
    assert_eq!(ipa("CamelCase"), "kˈaməl kˈeɪs");
    assert_eq!(ipa("iPhone"), "ˈaɪ fˈəʊn");
    assert_eq!(ipa("fooBar"), "fˈuː bˈɑː");
    // The split boundary is byte-identical to a literal space.
    assert_eq!(ipa("fooBar"), ipa("foo bar"));
    assert_eq!(ipa("McDonald"), ipa("Mc Donald"));
}

#[test]
fn uppercase_runs_and_plain_words_stay_whole() {
    let Some(t) = en() else {
        eprintln!("[SKIP] no en data");
        return;
    };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();
    // A pure acronym (ends in uppercase, no trailing lowercase run) and a plain
    // word have no split point → one word (no interior space).  `IDs` also stays
    // whole: the lowercase `s` has no further letter after it (needs next2 alpha).
    for w in ["hello", "HTML", "USA", "IDs"] {
        let got = ipa(w);
        assert!(!got.contains(' '), "{w:?} should stay one word, got {got:?}");
    }
    // An apostrophe isn't a lowercase letter, so it never triggers a split.
    assert!(!ipa("I'm").contains(' '), "I'm stays one word: {}", ipa("I'm"));
    // A capital at a genuine word start (after a space) is not a split point.
    assert_eq!(ipa("Hello World"), "həlˈəʊ wˈɜːld");
}

#[test]
fn uppercase_to_lowercase_run_splits_at_last_capital() {
    let Some(t) = en() else {
        eprintln!("[SKIP] no en data");
        return;
    };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();
    // espeak splits at the last uppercase of a run when it is followed by a
    // lowercase letter + another letter: an acronym glued to a capitalised word
    // separates ("HTMLParser" → "HTML Parser", "JABberwocky" → "JA Bberwocky").
    assert_eq!(ipa("HTMLParser"), ipa("HTML Parser"));
    assert_eq!(ipa("IThink"), ipa("I Think"));
    for w in ["HTMLParser", "JABberwocky", "IThink", "XMLHttp"] {
        assert!(ipa(w).contains(' '), "{w:?} should split, got {:?}", ipa(w));
    }
}

#[test]
fn dutch_keeps_initial_ij_digraph_whole() {
    // The upper→lower split would break "IJssel" into "I Jssel", but Dutch's
    // "IJ" is a digraph — espeak exempts a word-initial IJ for `nl` only.
    let ipa_en = en().map(|t| t.text_to_ipa("IJssel").unwrap());
    if let Some(en) = &ipa_en {
        assert!(en.contains(' '), "en should split IJssel: {en:?}");
    }
    if let Some(t) = Translator::new("nl", None).ok() {
        let nl = t.text_to_ipa("IJssel").unwrap();
        assert!(!nl.contains(' '), "nl keeps IJssel whole: {nl:?}");
    } else {
        eprintln!("[SKIP] no nl data");
    }
}

#[test]
fn irish_keeps_lenition_prefixes_whole() {
    // `ga`'s UpperCaseInWord suppresses the split; a lenition-style prefix such
    // as "nGael" must stay a single word.
    let Some(t) = Translator::new("ga", None).ok() else {
        eprintln!("[SKIP] no ga data");
        return;
    };
    let got = t.text_to_ipa("nGael").unwrap();
    assert!(!got.trim().is_empty(), "ga nGael produced nothing");
    assert!(!got.contains(' '), "ga nGael should stay one word, got {got:?}");
}
