// tests/word_phonemes.rs
//
// `Translator::text_to_ipa_words` — each source word with the phonemes it was
// rendered as (the Rust form of upstream PR #2533).
//
// The property that makes the API trustworthy is that it is a *view* of the
// ordinary rendering, not a second implementation: joining the per-word
// phonemes must reproduce `text_to_ipa` exactly, for every language and for
// text the tokenizer rewrites (numbers, abbreviations, switches).

use espeak_ng::Translator;

fn tr(lang: &str) -> Option<Translator> {
    Translator::new(lang, Some(std::path::Path::new("espeak-ng-data"))).ok()
}

/// Joining the words reproduces the flat rendering, and every span that is
/// known points at the word it belongs to.
#[test]
fn words_reproduce_the_flat_rendering() {
    let cases: &[(&str, &str)] = &[
        ("en-us", "Version 2.5 uses .NET."),
        ("en-us", "The quick brown fox jumps over the lazy dog."),
        ("en-us", "I don't know; it's 42% of $1,234.56!"),
        ("en-us", "well-known co-operate"),
        ("en-us", "Mr. Smith went to Washington D.C. on Jan. 3rd"),
        ("en-us", "a ( b ) c"),
        ("en", "[[h@l'oU]] world"),
        ("en", "~ ~ ~ ~"),
        ("en", "његов hello"),   // a word this language's rules can't read
        ("de", "3,14 und 1.000 Euro"),
        ("fr", "Les droits de l'homme, 1 234 567."),
        ("ru", "Все люди рождаются свободными 0,25"),
        ("es", "Todos los seres humanos nacen libres e iguales."),
        ("pl", "Wszyscy ludzie rodzą się wolni i równi"),
        ("af", "die hotel is nommer een"), // phonSWITCH to en
        ("cmn", "你好世界"),
        ("hi", "सभी मनुष्यों को गौरव"),
        ("ar", "يولد جميع الناس أحرارا"),
        ("tr", "Bütün insanlar hür, haysiyet ve haklar"),
        ("he", "כל בני אדם נולדו בני חורין"),
        ("vi", "Tất cả mọi người sinh ra đều được tự do"),
    ];
    let mut checked = 0;
    for (lang, text) in cases {
        let Some(t) = tr(lang) else {
            eprintln!("[SKIP] no {lang} data");
            continue;
        };
        checked += 1;
        let flat: Vec<String> = t
            .text_to_ipa(text)
            .unwrap()
            .split_whitespace()
            .map(str::to_string)
            .collect();
        let words = t.text_to_ipa_words(text).unwrap();
        let joined: Vec<String> = words
            .iter()
            .flat_map(|w| w.phonemes.split_whitespace().map(str::to_string))
            .collect();
        assert_eq!(joined, flat, "{lang}: {text}");
        for w in &words {
            assert!(!w.phonemes.is_empty(), "{lang}: empty pair for {:?}", w.word);
            assert!(w.offset + w.length <= text.len(), "{lang}: span past the end");
            if w.length > 0 {
                assert_eq!(
                    text[w.offset..w.offset + w.length].to_lowercase(),
                    w.word.to_lowercase(),
                    "{lang}: span does not point at the word"
                );
            }
        }
    }
    assert!(checked > 0, "no language data at all");
}

/// The cases upstream's PR calls out: a number expands to several spoken words
/// under one source token, and the spans stay exact.
#[test]
fn one_token_can_map_to_several_spoken_words() {
    let Some(t) = tr("en-us") else {
        eprintln!("[SKIP] no en data");
        return;
    };
    let text = "Version 2.5 uses .NET.";
    let words = t.text_to_ipa_words(text).unwrap();
    let pairs: Vec<(&str, &str)> =
        words.iter().map(|w| (w.word.as_str(), w.phonemes.as_str())).collect();
    assert_eq!(
        pairs,
        vec![
            ("Version", "vˈɜːʒən"),
            ("2.5", "tˈuː pɔɪnt fˈaɪv"),
            ("uses", "jˈuːsᵻz"),
            // The port tokenizes `.NET` as the symbol and the word, so it
            // reports two pairs where upstream's example shows one.
            (".", "dˈɑːt"),
            ("NET", "ˌɛnˌiːtˈiː"),
        ]
    );
    assert_eq!(words[1].offset, 8);
    assert_eq!(words[1].length, 3);
}

/// Text with nothing to say yields no pairs (and no panic).
#[test]
fn nothing_to_say_is_no_pairs() {
    let Some(t) = tr("en") else { return };
    for text in ["", "   ", "...", "\u{1}10S"] {
        assert!(t.text_to_ipa_words(text).unwrap().is_empty(), "{text:?}");
    }
}

/// The same thing through the engine (`espeak_TextToWordPhonemePairs`'s place).
#[test]
fn the_engine_exposes_it_too() {
    use espeak_ng::EspeakNg;
    let Ok(e) = EspeakNg::builder()
        .voice("en-us")
        .data_dir(std::path::Path::new("espeak-ng-data"))
        .build()
    else {
        eprintln!("[SKIP] no en data");
        return;
    };
    let words = e.text_to_phoneme_words("hello world").unwrap();
    assert_eq!(words.len(), 2);
    assert_eq!(words[0].word, "hello");
    assert_eq!(words[1].word, "world");
    assert_eq!(
        format!("{} {}", words[0].phonemes, words[1].phonemes),
        e.text_to_phonemes("hello world").unwrap()
    );
}
