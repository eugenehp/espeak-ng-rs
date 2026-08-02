// tests/french_numbers.rs
//
// Language-specific number-form fixes:
//   * French vigesimal 70s/90s (soixante-dix / quatre-vingt-dix).
//   * German/Russian omitting the "one" before hundred/thousand
//     (hundert/tausend, сто/тысяча — not "eins hundert"/"один сто").
// Skips when the relevant voice data is absent.

use espeak_ng::Translator;

fn fr() -> Option<Translator> {
    Translator::new("fr", None).ok()
}

fn tr(lang: &str) -> Option<Translator> {
    Translator::new(lang, None).ok()
}

#[test]
fn ordinal_zero_reads_zeroth() {
    if let Some(t) = tr("en") {
        // "0th" previously dropped the number, leaving just "θ".
        let z = t.text_to_ipa("0th").unwrap();
        assert!(z.contains("zˈiəɹəʊ") && z.ends_with("θ"), "0th should be zeroth: {z}");
        // Regression: other ordinals (incl. scale) still render.
        assert!(t.text_to_ipa("1st").unwrap().contains("fˈɜːst"));
        assert!(t.text_to_ipa("1000000th").unwrap().contains("mˈɪliənθ"));
    }
}

#[test]
fn german_and_russian_omit_one_before_hundred_thousand() {
    if let Some(t) = tr("de") {
        // "hundert"/"tausend", not "eins hundert"/"eins tausend".
        assert!(!t.text_to_ipa("100").unwrap().contains("aɪns"), "de 100: {}", t.text_to_ipa("100").unwrap());
        assert!(!t.text_to_ipa("1000").unwrap().contains("aɪns"), "de 1000: {}", t.text_to_ipa("1000").unwrap());
        // 200 still says "zwei…" (only the "one" is dropped).
        assert!(t.text_to_ipa("200").unwrap().contains("tsv"), "de 200: {}", t.text_to_ipa("200").unwrap());
    }
    if let Some(t) = tr("ru") {
        // 100 = "сто" (starts "st"), not "один сто" (starts "ʌdʲ"/"ɑdʲ").
        let h = t.text_to_ipa("100").unwrap();
        assert!(h.starts_with("st"), "ru 100 should be сто: {h}");
    }
    if let Some(t) = tr("it") {
        // 100 = "cento" (starts "tʃ"), not "uno cento" (starts "ˈuno").
        let h = t.text_to_ipa("100").unwrap();
        assert!(!h.starts_with("ˈuno"), "it 100 should be cento: {h}");
    }
    // Turkish "yüz"/"bin", Hungarian "száz"/"ezer", Slovak "sto".
    if let Some(t) = tr("tr") {
        assert_eq!(t.text_to_ipa("100").unwrap(), "jˈyz", "tr 100 should be yüz");
        assert_eq!(t.text_to_ipa("1000").unwrap(), "bˈin", "tr 1000 should be bin");
    }
    if let Some(t) = tr("hu") {
        assert!(!t.text_to_ipa("100").unwrap().starts_with("ˈɛɟ"), "hu 100 should be száz");
        assert!(!t.text_to_ipa("1000").unwrap().starts_with("ˈɛɟ"), "hu 1000 should be ezer");
    }
    if let Some(t) = tr("sk") {
        assert!(!t.text_to_ipa("100").unwrap().starts_with("j"), "sk 100 should be sto");
    }
    // Croatian/Serbian "sto" (not "jedansto"); Macedonian "сто"/"илјада".
    for lang in ["hr", "sr"] {
        if let Some(t) = tr(lang) {
            assert_eq!(t.text_to_ipa("100").unwrap(), "stˈo", "{lang} 100 should be sto");
        }
    }
    if let Some(t) = tr("mk") {
        assert!(!t.text_to_ipa("100").unwrap().contains("eden"), "mk 100 should be сто");
        assert!(!t.text_to_ipa("1000").unwrap().starts_with("ˈeden"), "mk 1000 should be илјада");
    }
}

#[test]
fn round_thousands_use_thousand_not_hundreds() {
    // A round thousand ≥2000 reads as "N mille"/"N thousand", NOT "twenty
    // hundred"/"vingt cents".  The year form (1100–1900 → "N hundred") stays.
    if let Some(t) = fr() {
        for n in ["2000", "3000", "5000", "9000"] {
            let ipa = t.text_to_ipa(n).unwrap();
            assert!(ipa.contains("mˈil"), "fr {n} should say mille: {ipa}");
            assert!(!ipa.contains("sˈɑ̃"), "fr {n} must not be 'N cents': {ipa}");
        }
        // Year form preserved: 1900 = "dix-neuf cents" (carries the cent sound).
        assert!(t.text_to_ipa("1900").unwrap().contains("sˈɑ̃"), "fr 1900 stays year-form");
    }
    if let Some(t) = tr("en") {
        let ipa = |s: &str| t.text_to_ipa(s).unwrap();
        assert!(ipa("2000").contains("θˈaʊzənd"), "en 2000 → two thousand: {}", ipa("2000"));
        assert!(ipa("5000").contains("θˈaʊzənd"), "en 5000 → five thousand: {}", ipa("5000"));
        assert!(ipa("1000").contains("θˈaʊzənd"), "en 1000 → one thousand: {}", ipa("1000"));
        // Oracle-verified year form kept: 1900 → nineteen hundred.
        assert!(ipa("1900").contains("hˈʌndɹɪd"), "en 1900 → nineteen hundred: {}", ipa("1900"));
        assert!(!ipa("2000").contains("hˈʌndɹɪd"), "en 2000 must not be 'twenty hundred': {}", ipa("2000"));
    }
    if let Some(t) = tr("de") {
        assert!(t.text_to_ipa("2000").unwrap().contains("tˈaʊzənt"), "de 2000 → zweitausend");
    }
}

#[test]
fn french_letter_suffix_ordinals() {
    let Some(t) = fr() else {
        eprintln!("[SKIP] no fr data");
        return;
    };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();
    // A written ordinal (1er, 2e, 21e…) reads as the ordinal *word*, identical
    // to typing that word — the fr dict has no `_No` entries, so this is
    // generated + pronounced by the French rules.
    let cases: &[(&str, &str)] = &[
        ("1er", "premier"),
        ("1re", "première"),
        ("2e", "deuxième"),
        ("2nd", "second"),
        ("3e", "troisième"),
        ("5e", "cinquième"),
        ("9e", "neuvième"),
        ("20e", "vingtième"),
        ("21e", "vingt et unième"),
        ("71e", "soixante et onzième"),
        ("80e", "quatre-vingtième"),
        ("100e", "centième"),
        ("1000e", "millième"),
    ];
    for (digit, word) in cases {
        assert_eq!(ipa(digit), ipa(word), "fr {digit:?} should read as {word:?}");
    }
    // In-sentence: "le 1er janvier" == "le premier janvier".
    assert_eq!(ipa("le 1er janvier"), ipa("le premier janvier"));
    // Not an ordinal: a plain decimal is untouched.
    assert!(ipa("3,5").contains("viʁɡyl"), "fr 3,5 decimal: {}", ipa("3,5"));
}

#[test]
fn italian_irregular_ordinals() {
    let Some(t) = tr("it") else {
        eprintln!("[SKIP] no it data");
        return;
    };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();
    // 1º…10º are irregular (primo…decimo); the dict lacks them, so the regular
    // "-esimo" fallback used to give wrong words ("unesimo" for primo).
    let cases: &[(&str, &str)] = &[
        ("1º", "primo"), ("2º", "secondo"), ("3º", "terzo"), ("4º", "quarto"),
        ("5º", "quinto"), ("6º", "sesto"), ("7º", "settimo"), ("8º", "ottavo"),
        ("9º", "nono"), ("10º", "decimo"),
        ("1ª", "prima"), ("3ª", "terza"), // feminine
    ];
    for (ord, word) in cases {
        assert_eq!(ipa(ord), ipa(word), "it {ord} should read as {word}");
    }
    // 11+ keep the regular -esimo form (undicesimo, ventesimo).
    assert!(ipa("11º").contains("zˈimo"), "it 11º stays -esimo: {}", ipa("11º"));
}

#[test]
fn compositional_teens_compose_from_ten_prefix() {
    // Turkish/Turkmen/Hungarian/Armenian/… have no dedicated teen words; 11–19
    // are "ten"+unit ("on beş", "tizenöt").  These used to produce nothing.
    if let Some(t) = tr("tr") {
        let ipa = |s: &str| t.text_to_ipa(s).unwrap();
        assert!(ipa("15").contains("bˈɛʃ"), "tr 15 should carry beş(5): {}", ipa("15"));
        assert!(ipa("11").contains("bˈiʀ"), "tr 11 should carry bir(1): {}", ipa("11"));
        assert!(!ipa("19").trim().is_empty(), "tr 19 not empty");
        // Teens compose inside larger numbers (years!): 2015 = "iki bin on beş".
        assert!(ipa("2015").contains("bˈɛʃ"), "tr 2015 keeps the teen: {}", ipa("2015"));
    }
    if let Some(t) = tr("hu") {
        // Hungarian uses the "tizen" combining prefix: 11 = "tizenegy".
        assert!(t.text_to_ipa("11").unwrap().contains("izɛn"), "hu 11 → tizen…: {}", t.text_to_ipa("11").unwrap());
    }
    // Regression: languages with dedicated teen words are unchanged.
    if let Some(t) = tr("en") {
        assert!(t.text_to_ipa("15").unwrap().contains("fˈɪftiːn"), "en 15 = fifteen");
    }
    if let Some(t) = tr("de") {
        assert!(!t.text_to_ipa("15").unwrap().trim().is_empty(), "de 15 unchanged");
    }
}

#[test]
fn comma_decimal_reads_fraction_digit_by_digit() {
    // For a comma-decimal language, "3,14" must read as a decimal (separator word
    // + digit-by-digit fraction), not "three [pause] fourteen".  Check the
    // separator word appears and the fraction is not the cardinal "14".
    let cases: &[(&str, &str)] = &[
        ("pl", "pʃɛtsʲˈinɛk"), // przecinek
        ("ro", "vˈirɡulə"),    // virgulă
        ("ca", "kˈoma"),       // coma
        ("cs", "tʃaːrka"),     // čárka
    ];
    for (lang, sep) in cases {
        let Some(t) = tr(lang) else {
            eprintln!("[SKIP] no {lang} data");
            continue;
        };
        let ipa = t.text_to_ipa("3,14").unwrap();
        assert!(ipa.contains(sep), "{lang} 3,14 should say the decimal word {sep:?}: {ipa}");
        // The whole-number reading of "14" must NOT appear as one word — the
        // fraction is read digit by digit, so it differs from a bare "14".
        assert_ne!(ipa, t.text_to_ipa("3 14").unwrap(), "{lang} 3,14 vs 3 14");
    }
}

#[test]
fn space_grouped_thousands_compose() {
    // Continental Europe uses a space as the SI thousands separator: "1 234 567"
    // must read as one number, not three.  English keeps a space as a separator.
    if let Some(t) = fr() {
        let ipa = |s: &str| t.text_to_ipa(s).unwrap();
        assert!(ipa("1 000").contains("mˈil"), "fr 1 000 → mille: {}", ipa("1 000"));
        assert!(ipa("12 000").contains("mˈil"), "fr 12 000 → douze mille: {}", ipa("12 000"));
        // 1 234 567 = un million … mille … (carries "miljɔ̃" and "mil").
        let big = ipa("1 234 567");
        assert!(big.contains("miljɔ̃") && big.contains("mil"), "fr 1 234 567: {big}");
        // A non-3-digit trailing group is NOT merged (12 34 stays two numbers).
        let split = ipa("12 34");
        assert!(split.contains("dˈuz") && split.contains("tʁɑ̃t"), "fr 12 34 must not merge: {split}");
        // Regression: plain grouping-free numbers are unchanged.
        // "1234" = "mille deux cent trente-quatre"; "deux cent" drops the liaison
        // /z/ before consonant-initial "cent" (matches espeak-ng: "døsɑ̃").
        assert!(ipa("1000").contains("mˈil") && ipa("1234").contains("døsɑ̃"), "fr plain: {}", ipa("1234"));
        // A trailing decimal group merges too: "1 234,56" == "1234,56" exactly
        // (final space group is a 3-digit-integer decimal).
        assert_eq!(ipa("1 234,56"), ipa("1234,56"), "fr grouped decimal");
        assert_eq!(ipa("1 234 567,89"), ipa("1234567,89"), "fr grouped decimal + millions");
        // Non-3-digit integer part is not merged ("12 34,5" stays split).
        assert!(ipa("12 34,5").contains("dˈuz") && ipa("12 34,5").contains("tʁɑ̃t"), "fr 12 34,5: {}", ipa("12 34,5"));
    }
    if let Some(t) = tr("fi") {
        assert!(t.text_to_ipa("1 000").unwrap().contains("tˈuhat"), "fi 1 000 → tuhat: {}", t.text_to_ipa("1 000").unwrap());
    }
    if let Some(t) = tr("en") {
        // English is unchanged: "1 000" is "one" + "zero", not "one thousand".
        let en = t.text_to_ipa("1 000").unwrap();
        assert!(en.contains("wˈɒn") && en.contains("zˈiəɹəʊ"), "en 1 000 unchanged: {en}");
    }
}

#[test]
fn french_vigesimal_70s_and_90s() {
    let Some(t) = fr() else {
        eprintln!("[SKIP] no fr data");
        return;
    };
    let ipa = |n: &str| t.text_to_ipa(n).unwrap();

    // 70s carry "soixante" (starts "swas"); 90s carry "quatre-vingt" (starts "katʁ").
    assert!(ipa("70").contains("swas"), "70 should be soixante-dix: {}", ipa("70"));
    assert!(ipa("72").contains("swas"), "72 should keep soixante: {}", ipa("72"));
    assert!(ipa("79").contains("swas"), "79: {}", ipa("79"));
    assert!(ipa("90").contains("katʁ"), "90 should be quatre-vingt-dix: {}", ipa("90"));
    assert!(ipa("92").contains("katʁ"), "92 should keep quatre-vingt: {}", ipa("92"));
    // 99 must NOT be just "neuf" — it carries quatre-vingt.
    assert!(ipa("99").contains("katʁ"), "99 lost quatre-vingt: {}", ipa("99"));

    // Round 70/90 are no longer empty.
    assert!(!ipa("70").trim().is_empty(), "70 empty");
    assert!(!ipa("90").trim().is_empty(), "90 empty");

    // Regression guard: 60 and 80 (which do have simple forms) are unchanged.
    assert!(ipa("60").contains("swas"), "60: {}", ipa("60"));
    assert!(ipa("80").contains("katʁ"), "80: {}", ipa("80"));
    // A full number composes correctly (1999 = mille neuf cent quatre-vingt-dix-neuf).
    assert!(ipa("1999").contains("katʁ"), "1999 tail: {}", ipa("1999"));
}
