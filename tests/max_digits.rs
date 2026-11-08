// C langopts.max_digits (default 14): a run of 5 to 32 digits above the limit, or starting with 0, is spelled in groups set by break_numbers; longer runs are one word. Limits: en/lv/ltg 33, ru/uk/ka 32, ne 22, ko 20. Expected strings come from C espeak-ng master (f0fb11a).

use espeak_ng::Translator;

fn ipa(lang: &str, text: &str) -> Option<String> {
    let t = Translator::new(lang, None).ok()?;
    Some(t.text_to_ipa(text).unwrap())
}

macro_rules! need {
    ($lang:expr, $text:expr) => {
        match ipa($lang, $text) {
            Some(s) => s,
            None => {
                eprintln!("[SKIP] no {} data", $lang);
                return;
            }
        }
    };
}

#[test]
fn german_reads_fifteen_digits_in_groups() {
    assert_eq!(
        need!("de", "123567890123567"),
        "ˈaɪnstsvˈaɪdɾˈaɪ fˈʏnfzˈɛkszˈiːbən ˈaxtnˈɔønnˈʊl ˈaɪnstsvˈaɪdɾˈaɪ fˈʏnfzˈɛkszˈiːbən"
    );
}

#[test]
fn german_reads_twenty_digits_in_groups_from_the_right() {
    assert_eq!(
        need!("de", "12345678901234567890"),
        "ˈaɪnstsvˈaɪ dɾˈaɪfˈiːɾfˈʏnf zˈɛkszˈiːbənˈaxt nˈɔønnˈʊlˈaɪns tsvˈaɪdɾˈaɪfˈiːɾ fˈʏnfzˈɛkszˈiːbən ˈaxtnˈɔønnˈʊl"
    );
}

#[test]
fn a_run_of_more_than_32_digits_is_one_word() {
    let ku = need!("ku", "123568123568123568123568123568123");
    assert!(!ku.contains(' '), "{ku}");
    assert!(ku.starts_with("jˈɛkdˈʊsˈepˈendʒʃˈɛʃhˈɛjʃtjˈɛkdˈʊ"), "{ku}");
}

#[test]
fn kurdish_reads_fifteen_plus_digits_one_by_one() {
    assert_eq!(
        need!("ku", "123567890123567"),
        "jˈɛkdˈʊsˈe pˈendʒʃˈɛʃhˈɛft hˈɛjʃtnˈɛhsˈɪfr jˈɛkdˈʊsˈe pˈendʒʃˈɛʃhˈɛft"
    );
    assert_eq!(
        need!("ku", "12356789012356789012"),
        "jˈɛkdˈʊ sˈepˈendʒʃˈɛʃ hˈɛfthˈɛjʃtnˈɛh sˈɪfrjˈɛkdˈʊ sˈepˈendʒʃˈɛʃ hˈɛfthˈɛjʃtnˈɛh sˈɪfrjˈɛkdˈʊ"
    );
}

#[test]
fn arabic_reads_fifteen_digits_in_groups() {
    assert_eq!(
        need!("ar", "123568123568123"),
        "wˈaːħidʔiθnˈaːnθalˈaːθa xˈamsasˈitːaθamˈaːnijˌa wˈaːħidʔiθnˈaːnθalˈaːθa xˈamsasˈitːaθamˈaːnijˌa wˈaːħidʔiθnˈaːnθalˈaːθa"
    );
}

#[test]
fn fourteen_digits_are_still_a_cardinal() {
    assert!(need!("de", "12345678901234").contains("bɪljˈoːnən"));
}

#[test]
fn english_and_russian_keep_reading_long_numbers_as_cardinals() {
    assert_eq!(
        need!("en", "1235678901235678"),
        "wˈɒn kwɒdɹˈɪliən tˈuːhˈʌndɹɪdən θˈɜːti fˈaɪv tɹˈɪliən sˈɪkshˈʌndɹɪdən sˈɛvənti ˈeɪt bˈɪliən nˈaɪnhˈʌndɹɪdən wˈɒn mˈɪliən tˈuːhˈʌndɹɪdən θˈɜːti fˈaɪv θˈaʊzənd sˈɪkshˈʌndɹɪdən sˈɛvənti ˈeɪt"
    );
    assert!(need!("ru", "1235678901235678").starts_with("ʌdʲˈin kvʌdrʲiɭʲiˈon"));
}

#[test]
fn tamil_groups_digits_in_lakh_units() {
    let out = need!("ta", "0123456789");
    let words: Vec<&str> = out.split_whitespace().collect();
    assert_eq!(words.len(), 4, "{out}");
    assert_eq!(words[0], "sˈuɻijˌʌmonrˌʉiɹˌʌɳɖʉ", "{out}");
    assert_eq!(words[1], "mˈuːnrʉnˌaːnɡʉ", "{out}");
    let long = need!("ta", "12345678901234567");
    assert_eq!(long.split_whitespace().count(), 6, "{long}");
    assert!(long.ends_with("ˈonrʉˌiɹʌɳɖʉ mˈuːnrʉnˌaːnɡʉ ˈaɪndʉˌaːrʉʲˌeːɻʉ"), "{long}");
}

#[test]
fn leading_zero_strings_of_five_digits_or_more_are_grouped() {
    assert_eq!(need!("de", "0123567"), "nˈʊl ˈaɪnstsvˈaɪdɾˈaɪ fˈʏnfzˈɛkszˈiːbən");
    assert_eq!(need!("ku", "0123567"), "sˈɪfr jˈɛkdˈʊsˈe pˈendʒʃˈɛʃhˈɛft");
}
