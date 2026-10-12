// tests/pretonic_vowels.rs
//
// C's `nextVowel` condition scans forward to the next vowel in the word and
// stops only at a word boundary.  Languages use it for *pre-tonic* vowel
// quality — Russian `V` is `IF nextVowel(isMaxStress) THEN ChangePhoneme(a)` —
// and a consonant cluster must not hide the stressed vowel.
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

/// The stressed vowel three phonemes away still counts.
#[test]
fn russian_pretonic_vowel_is_not_reduced() {
    let ru = skip_unless!("ru");
    let ipa = |s: &str| ru.text_to_ipa(s).unwrap();
    // `двадцатьпять`: the pre-tonic vowel keeps its `a` quality before the
    // stressed `ˈɑtʲ` three phonemes later (C `dvV_:tsat;p;'At;`).
    assert_eq!(ipa("двадцатьпять"), "dvʌtsatʲpʲˈɑtʲ");
    assert_eq!(ipa("двадцатьодин"), "dvʌtsatʲjˈɵdʲin");
    // Two words: the condition stops at the boundary, so this one *is* reduced.
    assert_eq!(ipa("двадцать пять"), "dvˈɑtsʌtʲ pʲˈɑtʲ");
    assert_eq!(ipa("двадцать"), "dvˈɑtsʌtʲ");
    // The classic reductions are unaffected.
    assert_eq!(ipa("молоко"), "mʌɭʌkˈo");
    assert_eq!(ipa("хорошо"), "xʌrʌʃˈo");
    assert_eq!(ipa("мама"), "mˈɑma");
}

/// English reduces the same way, which is what makes the verb "convert"
/// different from the noun.
#[test]
fn english_unstressed_vowel_reduces_before_the_stress() {
    let en = skip_unless!("en");
    let ipa = |s: &str| en.text_to_ipa(s).unwrap();
    assert_eq!(ipa("to convert"), "tə kənvˈɜːt");
    assert_eq!(ipa("a convert"), "ɐ kˈɒnvɜːt");
    assert_eq!(ipa("photography"), "fətˈɒɡɹəfi");
    assert_eq!(ipa("photograph"), "fˈəʊtəɡɹˌaf");
}

/// A number and its scale word are one word in C (`strcat` into the same
/// phoneme buffer), which is what gives the numerator its pre-tonic vowel.
#[test]
fn russian_fractions_are_one_word() {
    let ru = skip_unless!("ru");
    let ipa = |s: &str| ru.text_to_ipa(s).unwrap();
    assert_eq!(ipa("0,25"), "nˈoɭ ˈi dvˈɑtsatʲpʲˈatʲsˈotøx");
    assert_eq!(ipa("1,2"), "ʌdʲˈin ˈi dvʲˈedʲɪsʲˈɑtøx");
    assert_eq!(ipa("2,5"), "dvˈɑ ˈi pʲˈɑtʲdʲɪsʲˈɑtøx");
    assert_eq!(ipa("2,21"), "dvˈɑ ˈi dvˈɑtsʌtʲʌdnˈɑsˈotʌja");
    assert_eq!(ipa("0,0001"), "nˈoɭ ˈi ʌdnˈɑdʲisʲitʲitˈysʲitʃʲnʌja");
}
