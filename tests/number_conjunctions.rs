// tests/number_conjunctions.rs
//
// Compound-number joining words that the port used to omit, each confirmed
// against the espeak-ng 1.52.0 C binary:
//   * Romanian  "și"  between tens and units  (21 → "douăzeci și unu")
//   * Icelandic "og"  between tens and units  (34 → "þrjátíu og fjórir")
//   * Albanian  "e"   between tens→unit and hundreds→remainder
//   * Welsh / Armenian: "100" is "cant" / "harjur", not "un cant" / "mek harjur"

use espeak_ng::Translator;

fn tr(lang: &str) -> Option<Translator> {
    match Translator::new_default(lang) {
        Ok(t) => Some(t),
        Err(_) => { eprintln!("[SKIP] no {lang} dict"); None }
    }
}

#[test]
fn romanian_si_between_tens_and_units() {
    let Some(t) = tr("ro") else { return };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();
    assert!(ipa("21").contains("ʃi"), "ro 21 → 'douăzeci și unu': {}", ipa("21"));
    assert!(ipa("99").contains("ʃi"), "ro 99 has 'și': {}", ipa("99"));
    // No conjunction on a round ten (no units).
    assert!(!ipa("20").contains("ʃi"), "ro 20 has no 'și': {}", ipa("20"));
}

#[test]
fn icelandic_og_between_tens_and_units() {
    let Some(t) = tr("is") else { return };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();
    assert!(ipa("34").contains("ɔɣ"), "is 34 → 'þrjátíu og fjórir': {}", ipa("34"));
    assert!(!ipa("20").contains("ɔɣ"), "is 20 has no 'og': {}", ipa("20"));
}

#[test]
fn albanian_e_both_positions() {
    let Some(t) = tr("sq") else { return };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();
    // "e" between tens and unit.
    assert_eq!(ipa("21"), "ɲˈɪzɛtɛɲˈɪ");
    // "e" between the hundred and the remainder as well.
    assert_eq!(ipa("134"), "ɲˈɪtɕˌindɛ trˈiðjetəɛkˈatər");
}

#[test]
fn afrikaans_is_units_first_germanic() {
    let Some(t) = tr("af") else { return };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();
    // Units-first with "en", like Dutch/German ("vier-en-dertig").
    assert_eq!(ipa("21"), "ˈiənɛntwˈəntəx");   // een-en-twintig
    assert_eq!(ipa("34"), "fˈiːrɛndˈærtəx");    // vier-en-dertig
    // …but Afrikaans keeps "een honderd" / "een duisend" (no omit-one prefix).
    assert_eq!(ipa("100"), "ˈiənhˈɔnərt");
    assert_eq!(ipa("1000"), "ˈiən dˈøysənt");
}

#[test]
fn swahili_na_between_tens_and_units() {
    let Some(t) = tr("sw") else { return };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();
    // "na" joins tens and units ("ishirini na moja").
    assert!(ipa("21").contains("na"), "sw 21 → 'ishirini na moja': {}", ipa("21"));
    assert!(ipa("99").contains("na"), "sw 99 has 'na': {}", ipa("99"));
    assert!(!ipa("20").contains("na"), "sw 20 has no 'na': {}", ipa("20"));
    // Inside a larger number the tens-part still joins with "na", and there is
    // no spurious "na" after the hundred before a compound remainder.
    assert_eq!(ipa("134"), "mˌiamˈoɟa θˌeleθˈininaˈn̩ne");
    // Teens are compositional with "na" too ("kumi na tano" = 15).
    assert_eq!(ipa("15"), "kˈuminatˈano");
}

// The hundreds→remainder conjunction is added only before a *single-word*
// remainder (a unit, a single-word teen, or a round ten), not before a compound
// that already carries its own conjunction.
#[test]
fn conditional_hundreds_conjunction() {
    if let Some(t) = tr("af") {
        let ipa = |s: &str| t.text_to_ipa(s).unwrap();
        assert_eq!(ipa("101"), "ˈiənhˈɔnərtɛn ˈiən");        // honderd EN een
        assert_eq!(ipa("115"), "ˈiənhˈɔnərtɛn fˈɛɪftin");    // honderd EN vyftien
        assert_eq!(ipa("120"), "ˈiənhˈɔnərtɛn twˈəntəx");    // honderd EN twintig
        assert_eq!(ipa("134"), "ˈiənhˈɔnərt fˈiːrɛndˈærtəx"); // no extra "en"
    }
    if let Some(t) = tr("sw") {
        let ipa = |s: &str| t.text_to_ipa(s).unwrap();
        assert_eq!(ipa("101"), "mˌiamˈoɟana mˈoɟa");        // mia moja NA moja
        assert_eq!(ipa("120"), "mˌiamˈoɟanˌa iʃirˈini");    // mia moja NA ishirini
        // 115 (teen) and 134 (compound) already carry "na" internally → no extra.
        assert_eq!(ipa("115"), "mˌiamˈoɟa kˈuminatˈano");
        assert_eq!(ipa("134"), "mˌiamˈoɟa θˌeleθˈininaˈn̩ne");
    }
}

#[test]
fn amharic_omits_one_before_hundred() {
    let Some(t) = tr("am") else { return };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();
    assert_eq!(ipa("100"), "mˈəto");           // "መቶ", not "አንድ መቶ"
    assert!(ipa("200").contains("mˈəto"), "am 200 keeps its multiplier: {}", ipa("200"));
    // …but "1000" keeps its "አንድ" (and).
    assert!(ipa("1000").starts_with("ʔˈand"), "am 1000 keeps 'and': {}", ipa("1000"));
}

#[test]
fn turkic_omit_one_before_hundred() {
    // Kazakh "100" is "жүз" (not "бір жүз"), but "1000" keeps "бір мың".
    if let Some(t) = tr("kk") {
        let ipa = |s: &str| t.text_to_ipa(s).unwrap();
        assert_eq!(ipa("100"), "ʒˈuz");
        assert!(ipa("1000").starts_with("bˈɪr"), "kk 1000 keeps 'bir': {}", ipa("1000"));
        assert!(ipa("200").contains("ʒˈuz"), "kk 200 keeps its multiplier: {}", ipa("200"));
    }
    // Azerbaijani drops "bir" before both "yüz" and "min".
    if let Some(t) = tr("az") {
        let ipa = |s: &str| t.text_to_ipa(s).unwrap();
        assert_eq!(ipa("100"), "jˈyz");
        assert_eq!(ipa("1000"), "mˈin");
        assert!(ipa("2000").starts_with("icˈi"), "az 2000 keeps 'iki': {}", ipa("2000"));
    }
}

#[test]
fn welsh_and_armenian_omit_the_one_before_hundred() {
    if let Some(t) = tr("cy") {
        let ipa = |s: &str| t.text_to_ipa(s).unwrap();
        assert_eq!(ipa("100"), "kˈant", "cy 100 = 'cant', not 'un cant'");
        // 200 keeps its multiplier ("dau gant").
        assert!(ipa("200").starts_with("dˈaɨ"), "cy 200 keeps 'dau': {}", ipa("200"));
    }
    if let Some(t) = tr("hy") {
        let ipa = |s: &str| t.text_to_ipa(s).unwrap();
        // "harjur", not "mek harjur" — no leading "one".
        assert!(!ipa("100").contains("mˈek"), "hy 100 drops 'mek': {}", ipa("100"));
        assert!(ipa("100").contains("aɹjˈuɹ"), "hy 100 = 'harjur': {}", ipa("100"));
        // 200 keeps its multiplier ("yerku harjur").
        assert!(ipa("200").starts_with("jeɹkˈu"), "hy 200 keeps 'yerku': {}", ipa("200"));
    }
}
