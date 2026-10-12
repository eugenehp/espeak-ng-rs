// tests/geminates_and_alt_vowels.rs
//
// Two phoneme-list passes the port was missing, both visible in Italian:
//
//   * `MakePhonemeList` lengthens a fricative, nasal or liquid marked with the
//     lengthener (`:`) by *doubling* it, and drops the lengthener.  Italian
//     writes its geminates that way (`mm` compiles to `m:`), so `somma` reads
//     `sˈomma`, not `sˈomːa`.  Every other type keeps the `:`.
//
//   * `ApplySpecialAttribute2` — for a language with `LOPT_ALT & 2` (it, pt, sl)
//     and a word flagged `$alt`/`$alt2`, the vowel after the primary stress
//     changes quality: `$alt2` opens it (`O`→`o`), `$alt` closes it.
//
// Expectations are byte-identical to a locally built upstream `espeak-ng`
// (`ba90c8e`).

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

#[test]
fn italian_geminates_are_doubled() {
    let it = skip_unless!("it");
    assert_eq!(it.text_to_ipa("mamma").unwrap(), "mˈamma");
    assert_eq!(it.text_to_ipa("bello").unwrap(), "bˈɛllo"); // liquid
    assert_eq!(it.text_to_ipa("rosso").unwrap(), "rˈosso"); // fricative
    // A phoneme that is *itself* long keeps its own mnemonic.
    assert_eq!(it.text_to_ipa("faccina").unwrap(), "fatʃːˈina");
}

#[test]
fn a_lengthened_stop_keeps_its_lengthener() {
    // Only fricatives, voiced fricatives, nasals and liquids double; Finnish
    // `kukka` keeps `kː`, as in C.
    let fi = skip_unless!("fi");
    assert_eq!(fi.text_to_ipa("kukka").unwrap(), "kˈukːa");
}

#[test]
fn alt2_opens_the_stressed_vowel() {
    let it = skip_unless!("it");
    // `somma`'s stem `somm` is a flags-only `$alt2` entry, so the `O` the rules
    // chose opens to `o`.  (The flags of a flags-only stem entry were being
    // dropped, which is why this read `sˈɔmma`.)
    assert_eq!(it.text_to_ipa("somma").unwrap(), "sˈomma");
    if let Some(pt) = tr("pt") {
        assert_eq!(pt.text_to_ipa("catorze").unwrap(), "kˌɐtˈoɾzɨ");
    }
}
