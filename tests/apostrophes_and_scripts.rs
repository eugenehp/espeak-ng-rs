// tests/apostrophes_and_scripts.rs
//
// Two input-handling rules taken from C's `TranslateClause`:
//
//   * the typographic apostrophes (U+2019, U+00B4, U+2032, U+0092) are mapped to
//     a plain `'` before the text is split into words, and an apostrophe belongs
//     to the word only *between* two letters — so `don’t` is one word and
//     `hello'` is just "hello".  A clause-initial apostrophe loses its pause
//     (C's `words[0].pre_pause = 0`), while a real bracket keeps it.
//   * Serbo-Croatian is digraphic: bs/hr/sr share one dictionary and declare
//     Cyrillic as their own alphabet, so Cyrillic text must not be spelled out
//     or routed to Russian.
//
// Values from a locally built upstream `espeak-ng` (`ba90c8e`).

use espeak_ng::Translator;

fn tr(lang: &str) -> Option<Translator> {
    let t = Translator::new(lang, Some(std::path::Path::new("espeak-ng-data"))).ok()?;
    t.text_to_ipa("a").ok()?;
    Some(t)
}

#[test]
fn typographic_apostrophes_read_as_apostrophes() {
    let en = match tr("en") {
        Some(t) => t,
        None => {
            eprintln!("[SKIP] no en data");
            return;
        }
    };
    let plain = en.text_to_ipa("don't").unwrap();
    for smart in ["don’t", "don´t", "don′t"] {
        assert_eq!(en.text_to_ipa(smart).unwrap(), plain, "{smart}");
    }
    // …and it is no longer read as a word of its own ("don" + "T").
    assert_eq!(plain, "dˈəʊnt");
    assert_eq!(en.text_to_ipa("O’Brien").unwrap(), "əʊbɹˈaɪən");
}

/// An apostrophe that is not between two letters is not part of the word.
#[test]
fn a_trailing_apostrophe_is_not_part_of_the_word() {
    let en = match tr("en") {
        Some(t) => t,
        None => return,
    };
    let hello = en.text_to_ipa("hello").unwrap();
    assert_eq!(en.text_to_ipa("hello'").unwrap(), hello);
    assert_eq!(en.text_to_ipa("hello’").unwrap(), hello);
    // The possessive of a plural reads as the plural.
    assert_eq!(en.text_to_ipa("dogs'").unwrap(), en.text_to_ipa("dogs").unwrap());
    assert_eq!(en.text_to_ipa("James'").unwrap(), en.text_to_ipa("James").unwrap());
}

/// A clause-initial apostrophe has no pause; a bracket does.
#[test]
fn a_clause_initial_apostrophe_has_no_pause() {
    let en = match tr("en") {
        Some(t) => t,
        None => return,
    };
    let hello = en.text_to_ipa("hello").unwrap();
    assert_eq!(en.text_to_ipa("'hello").unwrap(), hello);
    assert_eq!(en.text_to_ipa("'hello'").unwrap(), hello);
    // Mid-clause it pauses, like any quote.  (The pause itself is not rendered
    // in IPA, so this compares the two spellings rather than the pause.)
    assert_eq!(en.text_to_ipa("say 'hello").unwrap(), en.text_to_ipa("say ‘hello").unwrap());
    assert_eq!(en.text_to_ipa("(hello)").unwrap(), en.text_to_ipa("‘hello").unwrap());
}

/// bs/hr/sr all read the Cyrillic pangram as Serbo-Croatian.
/// A `$textmode` replacement is re-translated without that normalisation, as in
/// C: `it_emoji`'s `€ simbolo dell’euro` keeps the curly apostrophe and so reads
/// as two words.
#[test]
fn a_textmode_replacement_keeps_its_own_apostrophe() {
    let Some(it) = tr("it") else {
        eprintln!("[SKIP] no it data");
        return;
    };
    assert_eq!(
        it.text_to_ipa("€").unwrap(),
        "s\u{2c8}imbolo d\u{2c8}\u{25b}l \u{2c8}\u{25b}\u{28a}ro"
    );
}

/// A language whose own `.replace` table transliterates a second script reads
/// it natively: C applies that table per character *before* deciding anything
/// about scripts, so Macedonian and Uzbek read their Latin forms, Uzbek and
/// Lingua Franca Nova their Cyrillic ones.  The table is keyed on lower case,
/// so a capitalised word transliterates too.
#[test]
fn a_transliterating_language_reads_its_second_script() {
    if let Some(mk) = tr("mk") {
        assert_eq!(mk.text_to_ipa("abc").unwrap(), mk.text_to_ipa("абц").unwrap());
        assert_eq!(mk.text_to_ipa("Dzidarski").unwrap(), mk.text_to_ipa("Ѕидарски").unwrap());
        // …and the Cyrillic it is written in is unaffected.
        assert_eq!(mk.text_to_ipa("Скопје").unwrap(), "skˈopje");
    }
    if let Some(lfn) = tr("lfn") {
        assert_eq!(lfn.text_to_ipa("Тота").unwrap(), lfn.text_to_ipa("tota").unwrap());
    }
    if let Some(uz) = tr("uz") {
        assert_eq!(uz.text_to_ipa("Ўзбекча").unwrap(), uz.text_to_ipa("ózbekça").unwrap());
        assert_eq!(uz.text_to_ipa("oʻzbekcha").unwrap(), "ozb\u{2c8}ektS\u{e6}");
    }
    // A word in a script the language does *not* transliterate still goes
    // elsewhere — English spells out Cyrillic, as C does.
    if let Some(en) = tr("en") {
        assert!(en.text_to_ipa("привет").unwrap().split_whitespace().count() >= 6);
    }
}

#[test]
fn serbo_croatian_reads_both_scripts() {
    let cyrillic = "Љубазни фењерџија";
    let latin = "ljubazni fenjerdžija";
    let mut seen = 0;
    for lang in ["sr", "bs", "hr"] {
        let Some(t) = tr(lang) else { continue };
        seen += 1;
        // Each voice reads the two scripts identically — that is the property
        // that broke, since bs/hr used to spell the Cyrillic out letter by
        // letter and switch to Russian partway.  (The three voices differ in
        // how their own phoneme tables render the result, so only `sr` is
        // pinned to an exact string.)
        assert_eq!(t.text_to_ipa(latin).unwrap(), t.text_to_ipa(cyrillic).unwrap(), "{lang}");
        assert!(
            t.text_to_ipa(cyrillic).unwrap().starts_with('\u{28e}'),
            "{lang} reads it as Serbo-Croatian: {:?}",
            t.text_to_ipa(cyrillic).unwrap()
        );
        if lang == "sr" {
            assert_eq!(
                t.text_to_ipa(cyrillic).unwrap(),
                "\u{28e}\u{2c8}ub\u{250}zn\u{26a} f\u{2c8}\u{25b}\u{272}erd\u{292}\u{2cc}\u{26a}j\u{250}"
            );
        }
    }
    if seen == 0 {
        eprintln!("[SKIP] no hbs data");
    }
}
