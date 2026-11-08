// C NUM_AND_HUNDRED (ku, ar): hundreds in a group after a higher group take the "_0and" prefix, fused into one word. Expected strings come from C espeak-ng master (f0fb11a).

use espeak_ng::Translator;

fn ku(text: &str) -> Option<String> {
    let t = Translator::new("ku", None).ok()?;
    Some(t.text_to_ipa(text).unwrap())
}

#[test]
fn hundreds_after_thousands_take_the_linking_u() {
    let Some(_) = ku("1") else {
        eprintln!("[SKIP] no ku data");
        return;
    };
    assert_eq!(ku("12345").unwrap(), "danzdˈɛ hɛzˈar usˈesˈɛdu tʃˈɪlupˈendʒ");
    assert_eq!(ku("1999").unwrap(), "hɛzˈar unˈɛhsˈɛdu nˈodunˈɛh");
    assert_eq!(ku("30741").unwrap(), "sˈi hɛzˈar uhˈɛftsˈɛdu tʃˈɪlujˈɛk");
}

#[test]
fn numbers_without_a_preceding_group_get_no_extra_u() {
    let Some(_) = ku("1") else {
        eprintln!("[SKIP] no ku data");
        return;
    };
    assert_eq!(ku("101").unwrap(), "sˈɛdu jˈɛk");
    assert_eq!(ku("1000").unwrap(), "hɛzˈar");
    assert_eq!(ku("300").unwrap(), "sˈesˈɛd");
    assert_eq!(ku("100000").unwrap(), "sˈɛd hɛzˈar");
    assert_eq!(ku("1100000").unwrap(), "mɪljˈon usˈɛd hɛzˈar");
}

#[test]
fn arabic_hundreds_after_a_higher_group_take_the_prefix() {
    let Ok(t) = Translator::new("ar", None) else {
        eprintln!("[SKIP] no ar data");
        return;
    };
    assert_eq!(t.text_to_ipa("1100").unwrap(), "ʔˈalf wamˈiʔa");
    assert_eq!(t.text_to_ipa("100100").unwrap(), "mˈiʔa ʔˈalf wamˈiʔa");
}

#[test]
fn units_group_after_thousands_keeps_its_conjunction() {
    let Some(_) = ku("1") else {
        eprintln!("[SKIP] no ku data");
        return;
    };
    let out = ku("2024").unwrap();
    assert!(out.starts_with("dˈʊ hɛzˈar u b"), "{out}");
}
