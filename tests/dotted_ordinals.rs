// tests/dotted_ordinals.rs
//
// Ordinals and the dot after a number, as C reads them.  Only `NUM_ORDINAL_DOT`
// languages read a dot as an ordinal mark, and only before a lower-case word
// (`CheckDotOrdinal`); a dot before `º`/`ª` is a dot, not part of the ordinal,
// in Portuguese as in C (`1.º` is "um ordinal masculino").

use espeak_ng::Translator;

fn check(lang: &str, cases: &[(&str, &str)]) {
    let Some(t) = Translator::new(lang, None).ok() else {
        eprintln!("[SKIP] no {lang} data");
        return;
    };
    for &(text, want) in cases {
        assert_eq!(t.text_to_ipa(text).unwrap(), want, "{lang} {text:?}");
    }
}

#[test]
fn spanish_ordinals_and_dots() {
    check(
        "es",
        &[
            ("3º", "tɛɾθˈɛɾo"),
            ("2ª", "seɣˈunda"),
            ("10º", "ðˈɛθimo"),
            ("3.", "tɾˈes"),
            ("3.5", "tɾˈes punto θˈinko"),
        ],
    );
}

#[test]
fn portuguese_and_italian_ordinals() {
    check(
        "pt",
        &[
            ("1º", "pɹˌimˈeɪɾʊ"),
            ("1.º", "ˈum ˌɔɾədinˈalmaskulˈi\u{303}nʊ"),
        ],
    );
    check("it", &[("2º", "sekˈondo")]);
}

#[test]
fn german_dot_ordinal_needs_a_following_word() {
    check("de", &[("3.", "dɾˈaɪ"), ("am 3. mai", "am dɾˈɪtə mˈaɪ")]);
}
