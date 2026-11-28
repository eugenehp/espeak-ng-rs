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
fn german_and_russian_omit_one_before_hundred_thousand() {
    if let Some(t) = tr("de") {
        // "hundert"/"tausend", not "eins hundert"/"eins tausend".
        assert!(
            !t.text_to_ipa("100").unwrap().contains("aɪns"),
            "de 100: {}",
            t.text_to_ipa("100").unwrap()
        );
        assert!(
            !t.text_to_ipa("1000").unwrap().contains("aɪns"),
            "de 1000: {}",
            t.text_to_ipa("1000").unwrap()
        );
        // 200 still says "zwei…" (only the "one" is dropped).
        assert!(
            t.text_to_ipa("200").unwrap().contains("tsv"),
            "de 200: {}",
            t.text_to_ipa("200").unwrap()
        );
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
        assert_eq!(
            t.text_to_ipa("100").unwrap(),
            "jˈyz",
            "tr 100 should be yüz"
        );
        assert_eq!(
            t.text_to_ipa("1000").unwrap(),
            "bˈin",
            "tr 1000 should be bin"
        );
    }
    if let Some(t) = tr("hu") {
        assert!(
            !t.text_to_ipa("100").unwrap().starts_with("ˈɛɟ"),
            "hu 100 should be száz"
        );
        assert!(
            !t.text_to_ipa("1000").unwrap().starts_with("ˈɛɟ"),
            "hu 1000 should be ezer"
        );
    }
    if let Some(t) = tr("sk") {
        assert!(
            !t.text_to_ipa("100").unwrap().starts_with("j"),
            "sk 100 should be sto"
        );
    }
    // Croatian/Serbian "sto" (not "jedansto"); Macedonian "сто"/"илјада".
    // Serbian's `o` is the open `ɔ`, Croatian's the close `o` — as upstream has
    // it, so the two are checked separately.
    for (lang, want) in [("hr", "stˈo"), ("sr", "stˈɔ")] {
        if let Some(t) = tr(lang) {
            assert_eq!(
                t.text_to_ipa("100").unwrap(),
                want,
                "{lang} 100 should be sto"
            );
        }
    }
    if let Some(t) = tr("mk") {
        assert!(
            !t.text_to_ipa("100").unwrap().contains("eden"),
            "mk 100 should be сто"
        );
        assert!(
            !t.text_to_ipa("1000").unwrap().starts_with("ˈeden"),
            "mk 1000 should be илјада"
        );
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
        assert!(
            t.text_to_ipa("1900").unwrap().contains("sˈɑ̃"),
            "fr 1900 stays year-form"
        );
    }
    if let Some(t) = tr("en") {
        let ipa = |s: &str| t.text_to_ipa(s).unwrap();
        assert!(
            ipa("2000").contains("θˈaʊzənd"),
            "en 2000 → two thousand: {}",
            ipa("2000")
        );
        assert!(
            ipa("5000").contains("θˈaʊzənd"),
            "en 5000 → five thousand: {}",
            ipa("5000")
        );
        assert!(
            ipa("1000").contains("θˈaʊzənd"),
            "en 1000 → one thousand: {}",
            ipa("1000")
        );
        // Oracle-verified year form kept: 1900 → nineteen hundred.
        assert!(
            ipa("1900").contains("hˈʌndɹɪd"),
            "en 1900 → nineteen hundred: {}",
            ipa("1900")
        );
        assert!(
            !ipa("2000").contains("hˈʌndɹɪd"),
            "en 2000 must not be 'twenty hundred': {}",
            ipa("2000")
        );
    }
    if let Some(t) = tr("de") {
        assert!(
            t.text_to_ipa("2000").unwrap().contains("tˈaʊzənt"),
            "de 2000 → zweitausend"
        );
    }
}

#[test]
fn compositional_teens_compose_from_ten_prefix() {
    // Turkish/Turkmen/Hungarian/Armenian/… have no dedicated teen words; 11–19
    // are "ten"+unit ("on beş", "tizenöt").  These used to produce nothing.
    if let Some(t) = tr("tr") {
        let ipa = |s: &str| t.text_to_ipa(s).unwrap();
        assert!(
            ipa("15").contains("bˈɛʃ"),
            "tr 15 should carry beş(5): {}",
            ipa("15")
        );
        assert!(
            ipa("11").contains("bir"),
            "tr 11 should carry bir(1): {}",
            ipa("11")
        );
        assert!(!ipa("19").trim().is_empty(), "tr 19 not empty");
        // Teens compose inside larger numbers (years!): 2015 = "iki bin on beş".
        assert!(
            ipa("2015").contains("bˈɛʃ"),
            "tr 2015 keeps the teen: {}",
            ipa("2015")
        );
    }
    if let Some(t) = tr("hu") {
        // Hungarian uses the "tizen" combining prefix: 11 = "tizenegy".
        assert!(
            t.text_to_ipa("11").unwrap().contains("izɛn"),
            "hu 11 → tizen…: {}",
            t.text_to_ipa("11").unwrap()
        );
    }
    // Regression: languages with dedicated teen words are unchanged.
    if let Some(t) = tr("en") {
        assert!(
            t.text_to_ipa("15").unwrap().contains("fˈɪftiːn"),
            "en 15 = fifteen"
        );
    }
    if let Some(t) = tr("de") {
        assert!(
            !t.text_to_ipa("15").unwrap().trim().is_empty(),
            "de 15 unchanged"
        );
    }
}

#[test]
fn comma_decimal_reads_fraction_digit_by_digit() {
    // For a comma-decimal language, "3,14" must read as a decimal (separator word
    // + digit-by-digit fraction), not "three [pause] fourteen".  Check the
    // separator word appears and the fraction is not the cardinal "14".
    let cases: &[(&str, &str)] = &[
        ("pl", "pʃɛtɕˈinɛk"), // przecinek
        ("ro", "vˈirɡulə"),   // virgulă
        // coma — secondary stress inside the number group (`NUM_SINGLE_STRESS`),
        // which is what upstream says: `tɾˈɛs kˌomə kətˌorzə`.
        ("ca", "kˌomə"),
        ("cs", "tʃaːrka"), // čárka
    ];
    for (lang, sep) in cases {
        let Some(t) = tr(lang) else {
            eprintln!("[SKIP] no {lang} data");
            continue;
        };
        let ipa = t.text_to_ipa("3,14").unwrap();
        assert!(
            ipa.contains(sep),
            "{lang} 3,14 should say the decimal word {sep:?}: {ipa}"
        );
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
        assert!(
            ipa("1 000").contains("mˈil"),
            "fr 1 000 → mille: {}",
            ipa("1 000")
        );
        assert!(
            ipa("12 000").contains("mˈil"),
            "fr 12 000 → douze mille: {}",
            ipa("12 000")
        );
        // 1 234 567 = un million … mille … (carries "miljɔ̃" and "mil").
        let big = ipa("1 234 567");
        assert!(
            big.contains("miljˈɔ̃") && big.contains("mˈil"),
            "fr 1 234 567: {big}"
        );
        // A non-3-digit trailing group is NOT merged (12 34 stays two numbers).
        let split = ipa("12 34");
        assert!(
            split.contains("dˈuz") && split.contains("tʁɑ̃t"),
            "fr 12 34 must not merge: {split}"
        );
        // Regression: plain grouping-free numbers are unchanged.
        // "1234" = "mille deux cent trente-quatre"; "deux cent" drops the liaison
        // /z/ before consonant-initial "cent" (matches espeak-ng: "døsɑ̃").
        assert!(
            ipa("1000").contains("mˈil") && ipa("1234").contains("døsɑ̃"),
            "fr plain: {}",
            ipa("1234")
        );
        // A trailing decimal group merges too: "1 234,56" == "1234,56" exactly
        // (final space group is a 3-digit-integer decimal).
        assert_eq!(ipa("1 234,56"), ipa("1234,56"), "fr grouped decimal");
        assert_eq!(
            ipa("1 234 567,89"),
            ipa("1234567,89"),
            "fr grouped decimal + millions"
        );
        // Non-3-digit integer part is not merged ("12 34,5" stays split).
        assert!(
            ipa("12 34,5").contains("dˈuz") && ipa("12 34,5").contains("tʁɑ̃t"),
            "fr 12 34,5: {}",
            ipa("12 34,5")
        );
    }
    if let Some(t) = tr("fi") {
        assert!(
            t.text_to_ipa("1 000").unwrap().contains("tˈuhat"),
            "fi 1 000 → tuhat: {}",
            t.text_to_ipa("1 000").unwrap()
        );
    }
    if let Some(t) = tr("en") {
        // English is unchanged: "1 000" is "one" + "zero", not "one thousand".
        let en = t.text_to_ipa("1 000").unwrap();
        assert!(
            en.contains("wˈɒn") && en.contains("zˈiəɹəʊ"),
            "en 1 000 unchanged: {en}"
        );
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
    assert!(
        ipa("70").contains("swas"),
        "70 should be soixante-dix: {}",
        ipa("70")
    );
    assert!(
        ipa("72").contains("swas"),
        "72 should keep soixante: {}",
        ipa("72")
    );
    assert!(ipa("79").contains("swas"), "79: {}", ipa("79"));
    assert!(
        ipa("90").contains("katʁ"),
        "90 should be quatre-vingt-dix: {}",
        ipa("90")
    );
    assert!(
        ipa("92").contains("katʁ"),
        "92 should keep quatre-vingt: {}",
        ipa("92")
    );
    // 99 must NOT be just "neuf" — it carries quatre-vingt.
    assert!(
        ipa("99").contains("katʁ"),
        "99 lost quatre-vingt: {}",
        ipa("99")
    );

    // Round 70/90 are no longer empty.
    assert!(!ipa("70").trim().is_empty(), "70 empty");
    assert!(!ipa("90").trim().is_empty(), "90 empty");

    // Regression guard: 60 and 80 (which do have simple forms) are unchanged.
    assert!(ipa("60").contains("swas"), "60: {}", ipa("60"));
    assert!(ipa("80").contains("katʁ"), "80: {}", ipa("80"));
    // A full number composes correctly (1999 = mille neuf cent quatre-vingt-dix-neuf).
    assert!(ipa("1999").contains("katʁ"), "1999 tail: {}", ipa("1999"));
}

fn check(lang: &str, cases: &[(&str, &str)]) {
    let Some(t) = tr(lang) else {
        eprintln!("[SKIP] no {lang} data");
        return;
    };
    for &(text, want) in cases {
        assert_eq!(t.text_to_ipa(text).unwrap(), want, "{lang} {text:?}");
    }
}

// Each case is C espeak-ng's own IPA.
/// "0th" is "zeroth"; scale ordinals keep their suffix.
#[test]
fn english_zero_and_scale_ordinals() {
    check(
        "en",
        &[
            ("0th", "zˈiəɹəʊθ"),
            ("1st", "fˈɜːst"),
            ("1000000th", "wˈɒn mˈɪliən θ"),
        ],
    );
}

/// French has no `_#` suffixes, so only the dictionary's `(1 er)` entries
/// make an ordinal; "2e" is the number and the letter, as in C.
#[test]
fn french_suffix_ordinals() {
    check(
        "fr",
        &[
            ("1er", "pʁəmjˈe"),
            ("1re", "ˈœ\u{303} ʁə-"),
            ("2e", "dˈø ˈə"),
            ("2nd", "dˈøz ˌɛndˈe"),
            ("3e", "tʁwˈa ˈə"),
            ("21e", "vɛ\u{303}teˈœ\u{303} ˈə"),
            ("100e", "sˈɑ\u{303} ˈə"),
            ("le 1er janvier", "lə- pʁəmjˈe ʒɑ\u{303}vjˈe"),
            ("3,5", "tʁwa viʁɡyl sˈɛ\u{303}k"),
        ],
    );
}

/// Italian ordinals: `_4ox` for a number on its own, `_ord` after the rest.
#[test]
fn italian_ordinals() {
    check(
        "it",
        &[
            ("1º", "prˈimo"),
            ("2º", "sekˈondo"),
            ("3º", "tˈɛrtso"),
            ("4º", "kwˈaːrto"),
            ("5º", "kwˈinto"),
            ("6º", "sˈɛsto"),
            ("7º", "sˈɛtːimo"),
            ("8º", "otːˈavo"),
            ("9º", "nˈono"),
            ("10º", "dˈɛtʃimo"),
            ("1ª", "prˈima"),
            ("3ª", "tˈɛrtsa"),
            ("11º", "ʊnditʃˈɛzimo"),
            ("21º", "vˈentʊnˈɛzimo"),
        ],
    );
}
