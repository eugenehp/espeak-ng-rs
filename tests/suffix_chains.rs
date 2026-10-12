// tests/suffix_chains.rs
//
// Standard-suffix endings (`S<n><flags>` in a rules file) where the stem that
// is left over matters: a second ending inside the first (`SUFX_M`), an ending
// that forbids retranslating its stem (`SUFX_Q`), and a suffix that follows
// letters which were spelled out rather than translated.
//
// Expectations are byte-identical to a locally built upstream `espeak-ng` of
// master (`ba90c8e`), taken with `-q --ipa -v <lang>`.

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

/// Dutch `-ige`: `nl_rules` takes the final `e` as one ending (`@) ige (_S1m`)
/// and then finds `ig` inside the stem (`@) ig (_S2dq`).  That inner ending is
/// `SUFX_Q` — "don't retranslate" — so the phonemes its own pass produced for
/// the stem are the ones to keep.  Using the *whole word's* phonemes instead
/// dropped the /ɣ/: `vorige` read `vˈoːrə`.
///
/// The four `na`-words are upstream #2423's own test cases.
#[test]
fn dutch_ige_keeps_the_g() {
    let nl = skip_unless!("nl");
    assert_eq!(nl.text_to_ipa("vorige").unwrap(), "vˈoːrəɣə");
    assert_eq!(nl.text_to_ipa("nodige").unwrap(), "nˈoːdəɣə");
    assert_eq!(nl.text_to_ipa("nadelige").unwrap(), "naːdˈeːləɣə");
    assert_eq!(nl.text_to_ipa("naburige").unwrap(), "naːbˈyrəɣə");
    assert_eq!(nl.text_to_ipa("nalatige").unwrap(), "naːlˈaːtəɣə");
    assert_eq!(nl.text_to_ipa("namalige").unwrap(), "naːmˈaːləɣə");
    // Without the ending the same `g` is word-final and devoices, as before.
    assert_eq!(nl.text_to_ipa("vorig").unwrap(), "vˈoːrəx");
}

/// Dutch spells a long vowel doubled in a closed syllable, so removing a
/// `SUFX_E` suffix has to put the second vowel back — C's `nl` branch of
/// `RemoveEnding`.  `graden` − `en` is the stem `graad`, not `grad`, which is
/// what keeps the vowel long.
#[test]
fn dutch_suffix_restores_the_doubled_vowel() {
    let nl = skip_unless!("nl");
    assert_eq!(nl.text_to_ipa("graden").unwrap(), "ɣrˈaːdən");
    assert_eq!(nl.text_to_ipa("straten").unwrap(), "strˈaːtən");
    assert_eq!(nl.text_to_ipa("maken").unwrap(), "mˈaːkən");
    assert_eq!(nl.text_to_ipa("hoge").unwrap(), "hˈoːɣə");
    // …but only when the stem really ends vowel + consonant: `gronden` has two
    // consonants, so the vowel stays short.
    assert_eq!(nl.text_to_ipa("gronden").unwrap(), "ɣrˈɔndən");
    assert_eq!(nl.text_to_ipa("landen").unwrap(), "lˈɑndən");
}

/// English `'s` after a word that is spelled letter by letter: the letters
/// already spoken stay available as the rule's pre-context, so `s) 's` → `ɪz`
/// wins over the bare `'s` → `z`.
#[test]
fn possessive_after_spelled_letters() {
    let en = skip_unless!("en");
    assert_eq!(en.text_to_ipa("s's").unwrap(), "ˈɛsɪz");
    assert_eq!(en.text_to_ipa("x's").unwrap(), "ˈɛksɪz");
    // Ordinary possessives are unchanged.
    assert_eq!(en.text_to_ipa("dog's").unwrap(), "dˈɒɡz");
    assert_eq!(en.text_to_ipa("boys's").unwrap(), "bˈɔɪzɪz");
    assert_eq!(en.text_to_ipa("cats'").unwrap(), "kˈats");
}

/// Spelling an unpronounceable word is unaffected by carrying that pre-context.
#[test]
fn spelled_abbreviations_are_unchanged() {
    let en = skip_unless!("en");
    assert_eq!(en.text_to_ipa("BBC").unwrap(), "bˌiːbˌiːsˈiː");
    assert_eq!(en.text_to_ipa("USA").unwrap(), "jˌuːˌɛsˈeɪ");
    assert_eq!(en.text_to_ipa("NASA").unwrap(), "nˈasɐ");
}
