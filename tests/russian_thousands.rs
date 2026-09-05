// tests/russian_thousands.rs
//
// Russian scale words decline: the count before "тысяча"/"миллион" selects the
// case (nom-sg / gen-sg / gen-pl) and the trailing "one"/"two" agrees in gender
// (тысяча is feminine, миллион masculine).  The port used to drop the "one" and
// always use the gen-pl form ("тысяч", "два тысяч").  Values confirmed against
// the espeak-ng 1.52.0 C binary (stress/length marks ignored for comparison).

use espeak_ng::Translator;

fn seg(t: &Translator, text: &str) -> String {
    // Strip stress/length; Russian's a/ɑ, e/ɛ, ʲ vowel rendering differs from the
    // C oracle but the segmental words are the same.
    t.text_to_ipa(text)
        .unwrap()
        .chars()
        .filter(|c| !matches!(c, 'ˈ' | 'ˌ' | 'ː'))
        .collect()
}

#[test]
fn thousand_declines_by_count_and_gender() {
    let Ok(t) = Translator::new_default("ru") else { eprintln!("[SKIP] no ru dict"); return };
    // count 1 → feminine "одна" + nom-sg "тысяча".
    assert!(seg(&t, "1000").starts_with("ʌdnɑ") && seg(&t, "1000").ends_with("ʌ"),
            "ru 1000 = 'одна тысяча': {}", seg(&t, "1000"));
    // count 2 → feminine "две" + gen-sg "тысячи".
    assert!(seg(&t, "2000").starts_with("dvʲe") && seg(&t, "2000").ends_with('i'),
            "ru 2000 = 'две тысячи': {}", seg(&t, "2000"));
    // count 5 → gen-pl "тысяч" (no trailing vowel).
    assert!(seg(&t, "5000").ends_with("tʃʲ"), "ru 5000 = 'пять тысяч': {}", seg(&t, "5000"));
    // A teen count is always gen-pl ("одиннадцать тысяч").
    assert!(seg(&t, "11000").ends_with("tʃʲ"), "ru 11000 = '… тысяч': {}", seg(&t, "11000"));
}

#[test]
fn compound_count_keeps_gender() {
    let Ok(t) = Translator::new_default("ru") else { return };
    // "двадцать одна тысяча" — the trailing "one" is feminine even in a compound.
    assert!(seg(&t, "21000").contains("ʌdnɑ"), "ru 21000 has fem 'одна': {}", seg(&t, "21000"));
    // "двадцать две тысячи"
    assert!(seg(&t, "22000").contains("dvʲe"), "ru 22000 has fem 'две': {}", seg(&t, "22000"));
}

#[test]
fn million_is_masculine() {
    let Ok(t) = Translator::new_default("ru") else { return };
    // count 1 → masculine "один" + nom-sg "миллион" (not the gen-pl "миллионов").
    let m = seg(&t, "1000000");
    assert!(m.starts_with("ʌdʲin") && m.ends_with("on"), "ru 1000000 = 'один миллион': {m}");
    // count 2 → masculine "два" + gen-sg "миллиона".
    let m2 = seg(&t, "2000000");
    assert!(m2.starts_with("dvɑ") && m2.ends_with("ona"), "ru 2000000 = 'два миллиона': {m2}");
}

// Czech (M_Variant VAR2) and Polish (VAR3): the scale word declines for a 2–4
// count ("dva tisíce", "dwa tysiące") but not for 5+ ("pět tisíc", "pięć
// tysięcy"); the "one" word is spoken via the dedicated `_1M1` ("tisíc").
#[test]
fn czech_polish_scale_word_declension() {
    if let Ok(t) = Translator::new_default("cs") {
        let s = |x: &str| seg(&t, x);
        assert_eq!(s("2000"), "dva cisitse");    // dva tisíce (2–4 form)
        assert!(s("5000").ends_with("cisits"), "cs 5000 = 'pět tisíc': {}", s("5000"));
        assert_eq!(s("2000000"), "dva milioni"); // dva milióny
        assert!(s("5000000").ends_with('u'), "cs 5000000 = 'pět miliónů': {}", s("5000000"));
    }
    if let Ok(t) = Translator::new_default("pl") {
        let s = |x: &str| seg(&t, x);
        assert_eq!(s("2000"), "dva tɨɕɔntsɛ");    // dwa tysiące (2–4 form)
        assert_eq!(s("2000000"), "dva mʲiljɔnɨ");  // dwa miliony
        // 5+ uses the genitive-plural "tysięcy" (…ɛntsɨ), not "tysiące" (…ɔntsɛ).
        assert!(s("5000").ends_with("ɛntsɨ"), "pl 5000 = 'pięć tysięcy': {}", s("5000"));
    }
}

// Lithuanian (M_Variant VAR4): the scale word is "tūkstantis" for count 1 and
// count ≡ 1 mod 10 (21), "tūkstančiai" for other 2–9 counts, and the genitive
// plural "tūkstančių" for a teen count or count ≡ 0 mod 10 (10, 20, 100).
#[test]
fn lithuanian_scale_word_declension() {
    let Ok(t) = Translator::new_default("lt") else { eprintln!("[SKIP] no lt dict"); return };
    let ends = |x: &str| seg(&t, x).replace('ɪ', "i"); // lt renders unstressed ɪ≈i
    assert!(ends("1000").ends_with("tantʲis"), "lt 1000 = 'tūkstantis': {}", ends("1000"));
    assert!(ends("2000").ends_with("tantɕei"), "lt 2000 = '… tūkstančiai': {}", ends("2000"));
    // teen / ×10 counts → genitive plural "tūkstančių".
    assert!(ends("10000").ends_with("tantɕu"), "lt 10000 = '… tūkstančių': {}", ends("10000"));
    assert!(ends("11000").ends_with("tantɕu"), "lt 11000 = '… tūkstančių': {}", ends("11000"));
    assert!(ends("100000").ends_with("tantɕu"), "lt 100000 = '… tūkstančių': {}", ends("100000"));
    // count ≡ 1 mod 10 (but not a teen) → "tūkstantis".
    assert!(ends("21000").ends_with("tantʲis"), "lt 21000 = '… tūkstantis': {}", ends("21000"));
}

// Slovak (VAR2, like Czech, but "thousand" is feminine only for a bare 2):
// "dve tisíc" (2000) yet "dvadsať dva tisíc" (22000) and "dva milióny".
#[test]
fn slovak_bare_two_is_feminine() {
    let Ok(t) = Translator::new_default("sk") else { eprintln!("[SKIP] no sk dict"); return };
    let s = |x: &str| seg(&t, x);
    assert!(s("2000").starts_with("dve"), "sk 2000 = 'dve tisíc': {}", s("2000"));
    // A compound count keeps the masculine "dva".
    assert!(s("22000").contains("dva") && !s("22000").contains("dve"),
            "sk 22000 = 'dvadsať dva tisíc': {}", s("22000"));
    // Millions take the 2–4 case form "milióny" (…oni), masculine "dva".
    assert!(s("2000000").starts_with("dva") && s("2000000").ends_with("oni"),
            "sk 2000000 = 'dva milióny': {}", s("2000000"));
}

// Croatian (M_Variant VAR5): "tisuću" for count ≡ 1 mod 10 (1, 21), "tisuće" for
// count ≡ 2–4, "tisuća" for 5+/teens; "thousand" is feminine ("dvije") for any
// count.  (bs/sr share VAR5 but the port has the wrong scale word for them.)
#[test]
fn croatian_scale_word_declension() {
    let Ok(t) = Translator::new_default("hr") else { eprintln!("[SKIP] no hr dict"); return };
    let s = |x: &str| seg(&t, x);
    assert!(s("1000").ends_with("tɕʊ"), "hr 1000 = 'tisuću': {}", s("1000"));
    // feminine "dvije" + 2–4 form "tisuće" (…tɕe).
    assert!(s("2000").starts_with("dvije") && s("2000").ends_with("tɕe"),
            "hr 2000 = 'dvije tisuće': {}", s("2000"));
    assert!(s("5000").ends_with("tɕæ"), "hr 5000 = 'pet tisuća': {}", s("5000"));
    // count ≡ 1 mod 10 → the "tisuću" form again.
    assert!(s("21000").ends_with("tɕʊ"), "hr 21000 = '… tisuću': {}", s("21000"));
    // gender carries into a compound ("dvadeset dvije tisuće").
    assert!(s("22000").contains("dvije"), "hr 22000 has fem 'dvije': {}", s("22000"));
    // millions take the masculine "dva" + "milijuna".
    assert!(s("2000000").starts_with("dva") && s("2000000").contains("milij"),
            "hr 2000000 = 'dva milijuna': {}", s("2000000"));
}
