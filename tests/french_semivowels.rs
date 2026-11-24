//! French [ɥ] (GAPS §4 #20, upstream PR #2280) as an opt-in IPA refinement:
//! the default output is unchanged (still byte-identical to the C library),
//! and `french_semivowels` writes the glide where French has it.

use espeak_ng::{french_semivowels, text_to_ipa};

fn refined(word: &str) -> String {
    french_semivowels(&text_to_ipa("fr", word).unwrap())
}

#[test]
fn default_output_is_unchanged() {
    assert_eq!(text_to_ipa("fr", "nuit").unwrap(), "nyˈi");
    assert_eq!(text_to_ipa("fr", "huit").unwrap(), "yˈit");
}

#[test]
fn glide_where_french_has_it() {
    for (word, want) in [
        ("nuit", "nˈɥi"),
        ("huit", "ˈɥit"),
        ("lui", "lˈɥi"),
        ("pluie", "plˈɥi"),
        ("bruit", "bʁˈɥi"),
        ("fruit", "fʁˈɥi"),
        ("puis", "pˈɥi"),
        ("huile", "ˈɥil"),
        ("suisse", "sˈɥis"),
        ("nuage", "nˈɥaʒ"),
        ("juin", "ʒˈɥɛ̃"),
        ("tuer", "tˈɥe"),
        ("duel", "dˈɥɛl"),
        ("lueur", "lˈɥœʁ"),
        ("nuance", "nˈɥɑ̃s"),
        ("conduire", "kɔ̃dˈɥiʁ"),
        ("linguiste", "lɛ̃ɡˈɥist"),
        ("aiguille", "ɛɡˈɥij"),
    ] {
        assert_eq!(refined(word), want, "{word}");
    }
}

#[test]
fn vowel_kept_after_a_cluster_and_across_words() {
    assert_eq!(refined("cruel"), "kʁyˈɛl");
    assert_eq!(refined("gluant"), "ɡlyˈɑ̃");
    assert_eq!(refined("tu as"), "ty ˈas");
    assert_eq!(refined("une rue"), text_to_ipa("fr", "une rue").unwrap());
}
