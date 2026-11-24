//! French spellings the port read differently from C espeak-ng 1.52 (every
//! expectation below is C's own output, `espeak-ng -v fr -q --ipa`):
//!
//! - letters with no rule of their own (`à`, `î`) are read as their
//!   unaccented base and the word translated again — C's `remove_accent`
//!   branch of `TranslateRules` — so "là" is not "l" and "boîte" not "bɔt";
//! - `y` is a vowel in French (`SetLetterVowel(tr, 'y')`), so an s between it
//!   and a vowel is voiced ("analyse") and "moyen" ends in [ɛ̃].

use espeak_ng::text_to_ipa;

fn check(pairs: &[(&str, &str)]) {
    for (word, c) in pairs {
        assert_eq!(text_to_ipa("fr", word).unwrap(), *c, "{word}");
    }
}

#[test]
fn letters_without_rules_fall_back_to_their_base() {
    check(&[
        ("là", "lˈa"),
        ("voilà", "vwalˈa"),
        ("déjà", "deʒˈa"),
        ("au-delà", "odəlˈa"),
        ("île", "ˈil"),
        ("dîner", "dinˈe"),
        ("gîte", "ʒˈit"),
        ("aîné", "ɛnˈe"),
        ("boîte", "bwˈat"),
        ("plaît", "plˈɛ"),
        ("maître", "mˈɛtʁ"),
        ("connaître", "kɔnˈɛtʁ"),
        ("chaîne", "ʃˈɛn"),
        ("entraîner", "ɑ̃tʁɛnˈe"),
    ]);
}

#[test]
fn a_lone_accented_letter_is_still_read_as_itself() {
    check(&[("à", "ˈa")]);
}

#[test]
fn y_is_a_vowel() {
    check(&[
        ("analyse", "analˈiz"),
        ("physique", "fizˈik"),
        ("moyen", "mwajˈɛ̃"),
        ("crayon", "kʁɛjˈɔ̃"),
        ("pays", "pɛˈi"),
        ("lycée", "lisˈe"),
    ]);
}

#[test]
fn letters_with_rules_are_unchanged() {
    check(&[("pâte", "pˈaːt"), ("hôtel", "otˈɛl"), ("maïs", "maˈis"), ("Noël", "nɔˈɛl"), ("été", "etˈe")]);
}

/// Multi-word dictionary entries, C's hyphen flags, and the stress that moves
/// with them — all C's own output with the bundled data (`espeak-ng --path=.`).
#[test]
fn multi_word_entries_and_hyphens() {
    check(&[
        ("parce que", "paʁskˈə"),
        ("parce qu'il", "paʁskˈil"),
        ("parce quelque", "pˈaʁs kˈɛlkə"),
        ("je sais parce que", "ʒə- sˈɛ paʁskˈə"),
        ("celle-là", "sɛl lˈa"),
        ("celle-ci", "sɛl sˈi"),
        ("ceux-là", "sø lˈa"),
        ("qu'est-ce que", "kɛskˈə"),
        ("est-ce que", "ɛs kˈə"),
        ("est-ce", "ˈɛs"),
        ("dis-moi", "dˈimwˈa"),
    ]);
}

#[test]
fn a_vowel_turned_glide_loses_its_stress() {
    check(&[("demi-heure", "dəmjˈœʁ"), ("semi-auto", "səmjotˈo"), ("joues", "ʒw")]);
}

#[test]
fn text_entries_and_switched_words() {
    check(&[
        ("km", "kilomˈɛtʁ"),
        ("5 km", "sˈɛ̃k kilomˈɛtʁ"),
        ("week-end", "(en)wˈiːkˈɛnd(fr)"),
        ("le week-end", "lə- (en)wˈiːkˈɛnd(fr)"),
        ("c'est le football", "sɛ lə- (en)fˈʊtbɔːl(fr)"),
    ]);
}
