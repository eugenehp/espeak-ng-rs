// tests/dict_symbol_names.rs
//
// A symbol character is read by the name its *own language's dictionary* gives
// it — the `*_emoji`/`*_list` symbol block that 86 of the shipped languages
// carry — instead of the small hardcoded per-language table that used to be
// consulted first (and that covered 31 characters in one language each).
//
// Every expectation here is byte-identical to a locally built upstream
// `espeak-ng` of master (`ba90c8e`), taken with `-q --ipa -v <lang>`.
//
// Skips (rather than fails) when the language's data isn't present.

use espeak_ng::Translator;

fn tr(lang: &str) -> Option<Translator> {
    Translator::new(lang, None).ok()
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

/// The reading comes from the dictionary, so it is the language's own and
/// covers every symbol the data names — not just the hardcoded handful.
#[test]
fn symbols_read_with_their_dictionary_name() {
    let en = skip_unless!("en");
    // `en_emoji` says "sum sign", not the hardcoded table's "sum".
    assert_eq!(en.text_to_ipa("∑").unwrap(), "sˈʌm sˈaɪn");
    assert_eq!(en.text_to_ipa("→").unwrap(), "ɹˈaɪt ˈaɹəʊ");
    // Currencies and marks that had no table entry at all and were dropped.
    assert_eq!(en.text_to_ipa("£").unwrap(), "pˈaʊnd");
    assert_eq!(en.text_to_ipa("¥").unwrap(), "jˈɛn");
    assert_eq!(en.text_to_ipa("¶").unwrap(), "pˈaɹəɡɹˌaf");
}

#[test]
fn every_language_gets_its_own_symbol_names() {
    // U+2212 MINUS SIGN used to be silent outside the ~20 languages in the
    // table; `hu_emoji` names it (and upstream #2481 changed that very entry
    // from "mínuszjel" to "mínusz", which was unreachable before).
    let hu = skip_unless!("hu");
    assert_eq!(hu.text_to_ipa("−").unwrap(), "mˈiːnus");
    let de = skip_unless!("de");
    assert_eq!(de.text_to_ipa("∑").unwrap(), "zˈʊməntsˌaɪçən");
}

/// German names every symbol with a capitalised noun, which the replacement
/// text used to be translated as-is — the rules matched nothing, so a one-word
/// reading fell through to being read as raw phoneme *codes*.
#[test]
fn capitalised_replacement_text_is_translated_as_a_word() {
    let de = skip_unless!("de");
    assert_eq!(de.text_to_ipa("😀").unwrap(), "ɡɾˈɪnzəndəs ɡəzˈɪçt");
    assert_eq!(de.text_to_ipa("👍").unwrap(), "dˈaʊmən hˈoːx");
    assert_eq!(de.text_to_ipa("❤️").unwrap(), "rˈoːtəs hˈɛɾts");
}

/// C classifies some codepoints as clause punctuation (`clause_type_from_codepoint`).
/// They break the clause and are never spoken, even where the dictionary names
/// them — which it does for all of these.
#[test]
fn clause_punctuation_is_not_read_by_name() {
    let hi = skip_unless!("hi");
    // Devanagari danda: `hi_emoji` calls it "पूर्ण विराम", and C speaks none of
    // it — the danda ends the sentence instead (one clause per line).
    assert_eq!(hi.text_to_ipa("क । ख").unwrap(), "kˈə\nkʰˈə");
    let en = skip_unless!("en");
    // These live in the emoji blocks but are a question/exclamation mark to C,
    // so they break the clause rather than being read.
    for s in ["❓", "❗", "‼"] {
        let got = en.text_to_ipa(&format!("a {s} b")).unwrap();
        assert_eq!(got, "ˈeɪ\nbˈiː", "{s} should break the clause, not speak: {got}");
    }
    // `⁉` carries *both* properties, which matches none of C's cases, so it is
    // read by name.
    assert_eq!(en.text_to_ipa("⁉").unwrap(), "ɛksklɐmˈeɪʃən kwˈɛstʃən mˈɑːk");
}

/// An operator the tokenizer synthesises from a leading `-` keeps the word for
/// the operation; the character written literally reads by its dictionary name,
/// which in several languages is the name of the *sign* ("знак мінуса").
#[test]
fn an_operator_is_not_read_as_the_sign_name() {
    let uk = skip_unless!("uk");
    assert_eq!(uk.text_to_ipa("−").unwrap(), "znˈak miːnˈusa");
    let minus_five = uk.text_to_ipa("-5").unwrap();
    assert!(minus_five.starts_with("mˈiːnus"), "uk -5: {minus_five}");
}

/// C applies the clause-end stress attributes (`$u+`, `$strend`, `$strend2`,
/// `$unstressend`) to *every* word of a symbol's reading, but only when the
/// symbol itself ends the clause.  Translating each word in isolation left the
/// function words reduced: `÷` read "divided baɪ".
#[test]
fn clause_end_stress_applies_to_the_whole_reading() {
    let en = skip_unless!("en");
    // "by" is `baI $u+` — full stress at a clause end, reduced in running text.
    assert_eq!(en.text_to_ipa("÷").unwrap(), "dɪvˈaɪdɪd bˈaɪ");
    assert_eq!(en.text_to_ipa("a ÷").unwrap(), "ɐ dɪvˈaɪdɪd bˈaɪ");
    assert_eq!(en.text_to_ipa("a ÷ b").unwrap(), "ɐ dɪvˈaɪdɪd baɪ bˈiː");
    // "up" is `,Vp $strend2`; a following word, even an unstressed one, keeps
    // it reduced.
    assert_eq!(en.text_to_ipa("👍").unwrap(), "θˈʌmz ˈʌp");
    assert_eq!(en.text_to_ipa("👍 the").unwrap(), "θˈʌmz ˌʌp ðə");
    // Spanish and Italian promote their articles/prepositions the same way.
    if let Some(es) = tr("es") {
        assert_eq!(es.text_to_ipa("™").unwrap(), "sˈimbolo ðˈe mˈaɾka kˌomeɾθjˈal");
    }
    if let Some(it) = tr("it") {
        assert_eq!(it.text_to_ipa("😀").unwrap(), "fatʃːˈina kˈon ˈun ɡrˈan sorɾˈizo");
    }
    // `$unstressend` goes the other way: Hungarian reduces those words.
    if let Some(hu) = tr("hu") {
        assert_eq!(hu.text_to_ipa("→").unwrap(), "jˌobː ˌoldɑlrɑ mˈutɑtoː ɲˈiːl");
    }
}

/// A symbol that is read by name occupies a position in the clause, so the word
/// before it is no longer clause-final (`a ÷` keeps "a" reduced).  Punctuation
/// that is *not* read still does not.
#[test]
fn a_spoken_symbol_takes_a_clause_position() {
    let en = skip_unless!("en");
    assert!(en.text_to_ipa("a ÷").unwrap().starts_with('ɐ'), "{:?}", en.text_to_ipa("a ÷"));
    // A comma ends the clause, so "a" takes its clause-final form.
    assert!(en.text_to_ipa("a ,").unwrap().starts_with("ˈeɪ"), "{:?}", en.text_to_ipa("a ,"));
}

/// Symbol entries can sit behind a dictionary condition — Portuguese names `§`
/// once per variety — so the lookup has to carry the language's conditions.
#[test]
fn conditional_symbol_entries_are_found() {
    let pt = skip_unless!("pt");
    assert_eq!(pt.text_to_ipa("§").unwrap(), "pˌɐɾˈaɡɾɐfʊ");
    if let Some(br) = tr("pt-br") {
        assert_eq!(br.text_to_ipa("§").unwrap(), "pˌaɾˈaɡɾɐfʊ");
    }
}

/// A character the language has no name for is named in the default voice
/// instead of being dropped — C's `LookupCharName` fallback.
#[test]
fn unnamed_symbols_fall_back_to_the_default_voice() {
    let sv = skip_unless!("sv");
    assert_eq!(sv.text_to_ipa("#").unwrap(), "(en)hˈaʃ(sv)");
    // …while a symbol the language *does* name keeps its own reading.
    assert_eq!(sv.text_to_ipa("%").unwrap(), "prɔsˈɛnt");
    if let Some(ja) = tr("ja") {
        assert_eq!(ja.text_to_ipa("%").unwrap(), "(en)pəsˈɛnt(ja)");
    }
    // The switched reading is the target's *character name* (`_@` → "at sign"),
    // which is what `LookupCharName` reads there — not the short reading of the
    // character itself ("at").
    if let Some(he) = tr("he") {
        assert_eq!(he.text_to_ipa("@").unwrap(), "(en)ˈatsaɪn(he)");
        assert_eq!(he.text_to_ipa("&").unwrap(), "(en)ˈampəsˌand(he)");
    }
}
