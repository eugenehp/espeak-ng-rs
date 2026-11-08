// tests/max_digits.rs
//
// C `langopts.max_digits`: a digit string longer than the language's limit
// (14 unless the language sets another) is read digit by digit, in groups of
// three digits run together.  The limit is 33 for en, 32 for ru/uk/ka, 22 for
// ne and 20 for ko.  Expected strings come from C espeak-ng master (f0fb11a).

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
fn leading_zero_strings_of_five_digits_or_more_are_grouped() {
    assert_eq!(need!("de", "0123567"), "nˈʊl ˈaɪnstsvˈaɪdɾˈaɪ fˈʏnfzˈɛkszˈiːbən");
    assert_eq!(need!("ku", "0123567"), "sˈɪfr jˈɛkdˈʊsˈe pˈendʒʃˈɛʃhˈɛft");
}
