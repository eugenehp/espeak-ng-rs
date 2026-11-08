// Expected strings are espeak-ng 1.52.0 output.

use espeak_ng::Translator;

const GROUP: &str = "nˈaɪnti nˈaɪn";
const HUNDRED: &str = "nˈaɪnhˈʌndɹɪdən";
const ONE_TO_NINE: &str = "wˈɒntˈuːθɹˈiːfˈɔːfˈaɪvsˈɪkssˈɛvənˈeɪtnˈaɪn";

fn english() -> Option<Translator> {
    let t = Translator::new("en", None).ok();
    if t.is_none() {
        eprintln!("[SKIP] no en data");
    }
    t
}

fn nines(scale_words: &[&str]) -> String {
    let mut parts = vec![format!("{GROUP}")];
    for word in scale_words {
        parts.push(format!("{word} {HUNDRED} {GROUP}"));
    }
    parts.join(" ")
}

#[test]
fn nineteen_digits_read_as_a_number() {
    let Some(t) = english() else { return };
    let expected = nines(&[
        "kwɪntˈɪliən",
        "kwɒdɹˈɪliən",
        "tɹˈɪliən",
        "bˈɪliən",
        "mˈɪliən",
        "θˈaʊzənd",
    ])
    .replacen(GROUP, "nˈaɪn", 1);
    assert_eq!(t.text_to_ipa("9999999999999999999").unwrap(), expected);
}

#[test]
fn twenty_digits_read_as_a_number() {
    let Some(t) = english() else { return };
    let expected = nines(&[
        "kwɪntˈɪliən",
        "kwɒdɹˈɪliən",
        "tɹˈɪliən",
        "bˈɪliən",
        "mˈɪliən",
        "θˈaʊzənd",
    ]);
    assert_eq!(t.text_to_ipa("99999999999999999999").unwrap(), expected);
}

#[test]
fn twenty_one_digits_keep_group_values() {
    let Some(t) = english() else { return };
    let expected = "wˈɒnhˈʌndɹɪdən twˈɛnti θɹˈiː kwɪntˈɪliən fˈɔːhˈʌndɹɪdən fˈɪfti sˈɪks kwɒdɹˈɪliən sˈɛvənhˈʌndɹɪdən ˈeɪti nˈaɪn tɹˈɪliən twˈɛlv bˈɪliən θɹˈiːhˈʌndɹɪdən fˈɔːti fˈaɪv mˈɪliən sˈɪkshˈʌndɹɪdən sˈɛvənti ˈeɪt θˈaʊzənd nˈaɪnhˈʌndɹɪdən wˈɒn";
    assert_eq!(t.text_to_ipa("123456789012345678901").unwrap(), expected);
}

#[test]
fn scale_words_extend_past_the_u64_range() {
    let Some(t) = english() else { return };
    assert_eq!(
        t.text_to_ipa("1000000000000000000000000").unwrap(),
        "wˈɒn sɛptˈɪliən"
    );
    assert_eq!(
        t.text_to_ipa("1000000000000000000000000000").unwrap(),
        "wˈɒn ɒktˈɪliən"
    );
    assert_eq!(
        t.text_to_ipa("1000000000000000000000000000000").unwrap(),
        "wˈɒn nɒnˈɪliən"
    );
}

#[test]
fn more_than_thirty_two_digits_read_digit_by_digit() {
    let Some(t) = english() else { return };
    let got = t.text_to_ipa("123456789012345678901234567890123").unwrap();
    let unit = format!("{ONE_TO_NINE}zˈiəɹəʊ");
    let expected = format!("{unit}{unit}{unit}wˈɒntˈuːθɹˈiː");
    assert_eq!(got, expected);
}

#[test]
fn hundred_digits_keep_stress_marks_up_to_the_c_word_limit() {
    let Some(t) = english() else { return };
    let input = "3074185296".repeat(10);
    let ten = "θɹˈiːzˈiəɹəʊsˈɛvənfˈɔːwˈɒnˈeɪtfˈaɪvtˈuːnˈaɪnsˈɪks";
    let expected = format!("{}θɹˈiːz", ten.repeat(4));
    assert_eq!(t.text_to_ipa(&input).unwrap().trim_end(), expected);
}

// Output of espeak-ng at commit f0fb11a8: `-q --ipa -v en` on the test input.
const C_MASTER_100K: &str = include_str!("fixtures/en_digits_100k_c_master.ipa");

#[test]
fn hundred_thousand_digits_match_c_master_without_hanging() {
    let Some(t) = english() else { return };
    let input: String = (0..100_000).map(|i| char::from(b'1' + (i % 9) as u8)).collect();
    let got = t.text_to_ipa(&input).unwrap();
    assert_eq!(got.trim_end(), C_MASTER_100K.trim_end());
}
