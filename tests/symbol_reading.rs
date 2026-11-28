// tests/symbol_reading.rs
//
// Reading technical symbols by name (# $ % & * + / = @ ~).
//
// eSpeak NG speaks these symbols as words by default (without `--punct`),
// while ordinary punctuation (. , ; : ! ? ' " ( ) …) is silent.  The exact
// IPA values below were confirmed byte-identical to the C oracle
// (`espeak-ng -q --ipa -v en "x <sym> y"`), except `@` which inherits a
// pre-existing final-/t/ rendering quirk in the word "at".

use espeak_ng::Translator;

fn en() -> Option<Translator> {
    Translator::new("en", None).ok()
}

#[test]
fn technical_symbols_are_spoken_by_name() {
    let Some(t) = en() else {
        eprintln!("[SKIP] no en data");
        return;
    };
    let cases = [
        ("x # y", "ˈɛks hˈaʃ wˈaɪ"),
        ("x $ y", "ˈɛks dˈɒlə wˈaɪ"),
        ("x % y", "ˈɛks pəsˈɛnt wˈaɪ"),
        ("x & y", "ˈɛks and wˈaɪ"),
        ("x * y", "ˈɛks ˈastəɹˌɪsk wˈaɪ"),
        ("x + y", "ˈɛks plˈʌs wˈaɪ"),
        ("x / y", "ˈɛks slˈaʃ wˈaɪ"),
        ("x = y", "ˈɛks ˈiːkwəlz wˈaɪ"),
        ("x ~ y", "ˈɛks tˈɪldə wˈaɪ"),
    ];
    for (input, want) in cases {
        assert_eq!(t.text_to_ipa(input).unwrap(), want, "input {input:?}");
    }
}

#[test]
fn math_and_degree_symbols_are_spoken() {
    let Some(t) = en() else {
        eprintln!("[SKIP] no en data");
        return;
    };
    // Previously these Unicode symbols were dropped entirely.
    let times = t.text_to_ipa("2×3").unwrap();
    assert!(times.contains("tˈaɪmz"), "× → times: {times}");
    let div = t.text_to_ipa("10÷2").unwrap();
    assert!(
        div.contains("dɪvˈaɪdɪd") && div.contains("baɪ"),
        "÷ → divided by: {div}"
    );
    let deg = t.text_to_ipa("50°").unwrap();
    assert!(deg.contains("dɪɡɹˈiːz"), "° → degrees: {deg}");
    // U+2212 MINUS SIGN → "minus" (a negative amount reads correctly).
    let minus = t.text_to_ipa("−5").unwrap();
    assert!(minus.contains("mˈaɪnəs"), "− → minus: {minus}");
    let sect = t.text_to_ipa("§5").unwrap();
    assert!(sect.contains("sˈɛkʃən"), "§ → section: {sect}");
    // Math relation/operator symbols (previously dropped).
    assert!(
        t.text_to_ipa("∞").unwrap().contains("ɪnfˈɪnɪti"),
        "∞ → infinity"
    );
    assert!(
        t.text_to_ipa("√4").unwrap().contains("skwˈeə ɹˈuːt"),
        "√ → square root"
    );
    assert!(
        t.text_to_ipa("3≠4").unwrap().contains("nˌɒt ˈiːkwəl"),
        "≠ → not equal to"
    );
    // "or" links into the following vowel with the English linking r
    // (`IfNextVowelAppend(r-)` in the phoneme's own program), as upstream does.
    assert!(
        t.text_to_ipa("x≤5").unwrap().contains("ɔːɹ ˈiːkwəl"),
        "≤ → less than or equal"
    );
    // The single-character temperature signs have no English reading in
    // the data, so upstream says nothing for them.
    assert_eq!(t.text_to_ipa("20℃").unwrap(), "twˈɛnti");
    assert_eq!(t.text_to_ipa("72℉").unwrap(), "sˈɛvənti tˈuː");
    // A bare degree (an angle) stays just "degrees".
    let ang = t.text_to_ipa("90°").unwrap();
    assert!(
        ang.contains("dɪɡɹˈiːz") && !ang.contains("sˈɛlsɪəs"),
        "90° stays degrees: {ang}"
    );
    // Trademark / copyright signs (previously dropped).
    // "trade mark", which is what `en_emoji` names U+2122 and what upstream
    // speaks; it was "trademark" while the reading came from a hardcoded table.
    assert_eq!(
        t.text_to_ipa("™").unwrap(),
        t.text_to_ipa("trade mark").unwrap(),
        "™ → trade mark"
    );
    assert_eq!(
        t.text_to_ipa("©").unwrap(),
        t.text_to_ipa("copyright").unwrap(),
        "© → copyright"
    );
    assert!(
        t.text_to_ipa("Windows®").unwrap().contains("ɹˈɛdʒɪstəd"),
        "® → registered"
    );
    // Per-mille / per-ten-thousand: no reading in the data, silent upstream.
    assert_eq!(t.text_to_ipa("5‰").unwrap(), "fˈaɪv");
    assert_eq!(t.text_to_ipa("50‱").unwrap(), "fˈɪfti");
}

#[test]
fn arithmetic_and_degree_symbols_read_in_other_languages() {
    // Byte-identical to upstream (a C build of the same data) for each case;
    // where C's data has no reading the symbol is silent.
    let cases: &[(&str, &str, &str)] = &[
        ("de", "5+3", "fˈʏnf plˈʊs dɾˈaɪ"),
        ("de", "5−2", "fˈʏnf mˈiːnʊs tsvˈaɪ"),
        ("de", "20°", "tsvˈantsɪç ɡɾˈɑːt"),
        ("fr", "5+3", "sˈɛ̃k plˈys tʁwˈa"),
        ("fr", "20°", "vˈɛ̃ dəɡʁˈe"),
        ("es", "5+3", "θˈinko mˈas tɾˈes"),
        ("es", "20°", "bˈeɪnte ɣɾˈaðos"),
        ("it", "20°", "vˈentɪ ɡrˈaːdɪ"),
        ("nl", "20°", "tʋˈɪntəx ɣrˈaːdən"),
        ("de", "5×3", "fˈʏnf mˈɑːl dɾˈaɪ"),
        ("fr", "5×3", "sˈɛ̃k myltipliˈepˈaʁ tʁwˈa"),
        ("es", "5×3", "θˈinko tɾˈes"),
        ("de", "6÷2", "zˈɛks ɡətˈaɪlt dʊɐç tsvˈaɪ"),
        ("fr", "6÷2", "sˈi divizˈepˈaʁ dˈø"),
        ("ru", "50%", "pʲʌdʲdʲɪsʲˈjat prʌtsˈɛnt"),
        ("bg", "50%", "pˌeddesˈet protsˈent"),
    ];
    for (lang, text, want) in cases {
        let Some(t) = Translator::new(lang, None).ok() else {
            eprintln!("[SKIP] no {lang} data");
            continue;
        };
        assert_eq!(t.text_to_ipa(text).unwrap(), *want, "{lang} {text:?}");
    }
}

#[test]
fn currency_symbols_are_read_in_place() {
    // C reads the symbol's dictionary name where it stands.
    let cases: &[(&str, &str, &str)] = &[
        ("en", "$4.50", "dˈɒlə fˈɔː pɔɪnt fˈaɪv zˈiəɹəʊ"),
        ("en", "$5 million", "dˈɒlə fˈaɪv mˈɪliən"),
        ("en", "5$", "fˈaɪv dˈɒlə"),
        ("de", "5$", "fˈʏnf dɔlˈɑː"),
        ("de", "1€", "ˈaɪns ˈɔøroː"),
        ("es", "5€", "θˈinko ˈeʊɾo"),
        ("fr", "5£", "sˈɛ̃k lˈivʁ"),
        ("it", "$5", "dˈɔllarɪ tʃˈinkwe"),
        ("pt", "5€", "sˈiŋku ˈeʊɾʊ"),
    ];
    for (lang, text, want) in cases {
        let Some(t) = Translator::new(lang, None).ok() else {
            eprintln!("[SKIP] no {lang} data");
            continue;
        };
        assert_eq!(&t.text_to_ipa(text).unwrap(), want, "{lang} {text}");
    }
}

#[test]
fn ordinary_punctuation_is_silent() {
    let Some(t) = en() else {
        eprintln!("[SKIP] no en data");
        return;
    };
    // None of these add a spoken word between "x" and "y".
    for input in [
        "x ! y", "x ( y", "x ) y", "x < y", "x > y", "x ? y", "x ; y",
    ] {
        let ipa = t.text_to_ipa(input).unwrap().replace('\n', " ");
        let words: Vec<&str> = ipa.split_whitespace().collect();
        assert_eq!(words.len(), 2, "punctuation {input:?} was spoken: {ipa:?}");
    }
}

#[test]
fn punct_announces_punctuation_by_name() {
    let Some(mut t) = en() else {
        eprintln!("[SKIP] no en data");
        return;
    };
    // Baseline: ordinary punctuation is silent (two spoken words).
    let base = t.text_to_ipa("a, b.").unwrap().replace('\n', " ");
    assert_eq!(base.split_whitespace().count(), 2, "baseline {base:?}");

    // `--punct` (all): comma and period are announced by name.
    t.options.punct = Some(Vec::new());
    assert_eq!(t.text_to_ipa("a, b.").unwrap(), "ˈeɪ kˈɒmə bˈiː dˈɒt");

    // `--punct=","` (only comma): the period stays silent.
    t.options.punct = Some(vec![',']);
    let only_comma = t.text_to_ipa("a, b.").unwrap();
    assert!(
        only_comma.contains("kˈɒmə"),
        "comma should be announced: {only_comma:?}"
    );
    assert!(
        !only_comma.contains("dˈɒt"),
        "period should stay silent: {only_comma:?}"
    );
}

#[test]
fn punct_reaches_the_code_stream() {
    // The `-x` / synthesis path (`translate_to_codes`) must also emit punct names.
    let Some(mut t) = en() else { return };
    let count = |t: &Translator, s: &str| {
        t.translate_to_codes(s)
            .unwrap()
            .iter()
            .filter(|c| !c.is_boundary && c.marker.is_none())
            .count()
    };
    let plain = count(&t, "a, b.");
    t.options.punct = Some(Vec::new());
    let punct = count(&t, "a, b.");
    assert!(
        punct > plain,
        "--punct added no codes to -x path: {punct} vs {plain}"
    );
}

#[test]
fn spoken_symbols_reach_the_code_stream() {
    let Some(t) = en() else {
        eprintln!("[SKIP] no en data");
        return;
    };
    // Regression: `translate_to_codes` (the `-x` / synthesis path) dropped
    // spoken symbols even though `--ipa` (`text_to_ipa`) rendered them — the
    // emission match had no `Token::Punctuation` arm.  `&` → "and" must now emit.
    // Pauses don't count as speech: a bracket is silent but still pauses
    // (`LOPT_BRACKET_PAUSE`), so `a (b) c` gains two pauses per bracket.
    let n = |s: &str| {
        t.translate_to_codes(s)
            .expect("codes")
            .iter()
            .filter(|c| {
                !c.is_boundary && c.marker.is_none() && c.code != espeak_ng::phoneme::PHON_PAUSE
            })
            .count()
    };
    assert!(n("a & b") > n("a b"), "spoken '&' added no codes");
    assert!(n("5 % x") > n("5 x"), "spoken '%' added no codes");
    // Non-spoken punctuation stays silent (no extra spoken codes vs plain text).
    assert_eq!(n("a (b) c"), n("a b c"), "parentheses should be silent");
    // …and the pause is there.
    let pauses = |s: &str| {
        t.translate_to_codes(s)
            .expect("codes")
            .iter()
            .filter(|c| c.code == espeak_ng::phoneme::PHON_PAUSE)
            .count()
    };
    assert_eq!(
        pauses("a (b) c"),
        pauses("a b c") + 4,
        "brackets should pause"
    );
}

#[test]
fn ampersand_reads_and_not_ampersand() {
    let Some(t) = en() else {
        eprintln!("[SKIP] no en data");
        return;
    };
    // Confirmed identical to the oracle.
    assert_eq!(t.text_to_ipa("Tom & Jerry").unwrap(), "tˈɒm and dʒˈɛɹi");
}

#[test]
fn percent_after_number() {
    let Some(t) = en() else {
        eprintln!("[SKIP] no en data");
        return;
    };
    let ipa = t.text_to_ipa("50 % off").unwrap();
    assert!(ipa.contains("pəsˈɛnt"), "expected 'percent' in {ipa:?}");
}

#[test]
fn numero_sign_reads_number_word() {
    // Byte-identical to upstream (a C build of the same data) for each case;
    // where C's data has no reading the symbol is silent.
    let cases: &[(&str, &str, &str)] = &[
        ("en", "№5", "njˈuːməɹˌəʊ fˈaɪv"),
        ("de", "№5", "nˈuːmeːrˌoː fˈʏnf"),
        ("it", "№5", "nˈuːmero tʃˈinkwe"),
        ("pt", "№5", "nˈũmeɾʊ sˈiŋku"),
        ("nl", "№5", "nˈɵmər vˈɛɪf"),
        ("ru", "№5", "nˈomʲir pʲˈɑtʲ"),
        ("bg", "№5", "nˈomer pˈet"),
        ("cs", "№5", "tʃˈiːslo pjˈet"),
        ("fr", "№5", "sˈɛ̃k"),
        ("es", "№5", "θˈinko"),
        ("uk", "№5", "pjˈat"),
        ("pl", "№5", "pʲˈɛɲtɕ"),
    ];
    for (lang, text, want) in cases {
        let Some(t) = Translator::new(lang, None).ok() else {
            eprintln!("[SKIP] no {lang} data");
            continue;
        };
        assert_eq!(t.text_to_ipa(text).unwrap(), *want, "{lang} {text:?}");
    }
}

#[test]
fn multiply_and_divide_read_across_languages() {
    // Byte-identical to upstream (a C build of the same data) for each case;
    // where C's data has no reading the symbol is silent.
    let cases: &[(&str, &str, &str)] = &[
        ("pt", "2×3", "dˈoɪʒ vˈezɨʃ tɹˈeʃ"),
        ("pt", "10÷2", "dˈɛʒ dˌividˈidʊpˈor dˈoɪʃ"),
        ("pl", "2×3", "dvˈa rˈazɨ tʃˈɨ"),
        ("pl", "10÷2", "dʑˈɛɕɛɲdʑ dʑɛlˈɛɲʲɛ dvˈa"),
        ("cs", "2×3", "dvˈa krˈaːt tr̝̊ˈi"),
        ("cs", "10÷2", "dˈesed ɟˈeleno dvˈa"),
        ("ro", "2×3", "dˈoɪ trˈeɪ"),
        ("ro", "10÷2", "zˈetʃe dˈoɪ"),
        ("sv", "2×3", "tvˈoː ɡˈɔŋər trˈeː"),
        ("sv", "10÷2", "tˈiːuː dˈeːlat meːd tvˈoː"),
        ("da", "2×3", "tˈo ɡˈaŋe tʁˈʔe"),
        ("da", "10÷2", "tˈʔi dˌiʋidˈeʁʌð mˌɛð tˈo"),
        ("nb", "2×3", "tˈuː trˈeː"),
        ("nb", "10÷2", "tˈiː tˈuː"),
        ("tr", "2×3", "icˈi ˈytʃ"),
        ("tr", "10÷2", "ˈon icˈi"),
        ("ru", "2×3", "dvˈɑ trʲˈi"),
        ("ru", "10÷2", "dʲˈesʲɪtʲ dvˈɑ"),
        ("uk", "2×3", "dβˈa trˈɪ"),
        ("uk", "10÷2", "dˈɛsjat dβˈa"),
        ("bg", "2×3", "dvˈe trˈi"),
        ("bg", "10÷2", "dˈeset dvˈe"),
    ];
    for (lang, text, want) in cases {
        let Some(t) = Translator::new(lang, None).ok() else {
            eprintln!("[SKIP] no {lang} data");
            continue;
        };
        assert_eq!(t.text_to_ipa(text).unwrap(), *want, "{lang} {text:?}");
    }
}

#[test]
fn plus_minus_sign_reads_across_languages() {
    // ± was dropped outside en/de/fr/nl; the languages below now read it.
    for lang in [
        "es", "it", "pt", "pl", "cs", "ro", "sv", "da", "nb", "tr", "ru", "uk", "bg",
    ] {
        let Some(t) = Translator::new(lang, None).ok() else {
            eprintln!("[SKIP] no {lang} data");
            continue;
        };
        let pm = t.text_to_ipa("±5").unwrap();
        let five = t.text_to_ipa("5").unwrap();
        assert_ne!(
            pm.replace(' ', ""),
            five.replace(' ', ""),
            "{lang} ±5 dropped the sign: {pm:?}"
        );
    }
}

#[test]
fn hebrew_and_ukrainian_operators() {
    // Byte-identical to upstream (a C build of the same data) for each case;
    // where C's data has no reading the symbol is silent.
    let cases: &[(&str, &str, &str)] = &[
        ("he", "2+3", "ʃtˈajim (en)plˈʌs(he) ʃalˈoʃ"),
        ("he", "2×3", "ʃtˈajim ʃalˈoʃ"),
        ("he", "10÷2", "ˈeser ʃtˈajim"),
        ("he", "20°", "esrˈim"),
        ("he", "-5", "χamˈeʃ"),
        ("he", "−", "sˈimnə mˈinvs"),
        ("uk", "20°", "dvˈatsjat"),
    ];
    for (lang, text, want) in cases {
        let Some(t) = Translator::new(lang, None).ok() else {
            eprintln!("[SKIP] no {lang} data");
            continue;
        };
        assert_eq!(t.text_to_ipa(text).unwrap(), *want, "{lang} {text:?}");
    }
}

#[test]
fn degree_sign_reads_across_languages() {
    // Byte-identical to upstream (a C build of the same data) for each case;
    // where C's data has no reading the symbol is silent.
    let cases: &[(&str, &str, &str)] = &[
        ("sv", "20°", "ɕˈʉːɡɔ ɡrˈadər"),
        ("sv", "-40°", "mˈiːnɵs fˈœʈɪ ɡrˈadər"),
        ("da", "20°", "tˈyːʔu"),
        ("da", "-40°", "fˈʔœʌ"),
        ("nb", "20°", "ʂˈʉːa"),
        ("nb", "-40°", "mˈiːnʉːs fˈøːrtiː"),
        ("ru", "20°", "dvˈɑtsʌtʲ"),
        ("ru", "-40°", "sˈorʌk"),
        ("bg", "20°", "dvˈadesˌet"),
        ("bg", "-40°", "tʃˌetirˈidesˌet"),
    ];
    for (lang, text, want) in cases {
        let Some(t) = Translator::new(lang, None).ok() else {
            eprintln!("[SKIP] no {lang} data");
            continue;
        };
        assert_eq!(t.text_to_ipa(text).unwrap(), *want, "{lang} {text:?}");
    }
}

/// Each case is C espeak-ng's own IPA for the text.
fn oracle_check(lang: &str, cases: &[(&str, &str)]) {
    let Some(t) = Translator::new(lang, None).ok() else {
        eprintln!("[SKIP] no {lang} data");
        return;
    };
    for &(text, want) in cases {
        assert_eq!(t.text_to_ipa(text).unwrap(), want, "{lang} {text:?}");
    }
}

/// Numbers with letters, units and abbreviations, read as C reads them: the
/// letters by the rules (`D_) s (` after a digit is `z`), no expansion.
#[test]
fn numbers_with_letters_as_c() {
    oracle_check(
        "en",
        &[
            ("1980s", "nˈaɪntiːnhˈʌndɹɪdən ˈeɪti z"),
            ("1990s", "nˈaɪntiːnhˈʌndɹɪdən nˈaɪnti z"),
            ("80s", "ˈeɪti z"),
            ("90s", "nˈaɪnti z"),
            ("1985", "nˈaɪntiːnhˈʌndɹɪdən ˈeɪti fˈaɪv"),
            ("5s", "fˈaɪv z"),
            ("2000s", "tˈuː θˈaʊzənd z"),
            ("vs", "vˌiːˈɛs"),
            ("govt", "ɡˈɒvt"),
            ("Blvd", "bˈʊləvˌɑːd"),
            ("Sgt", "ˌɛsdʒˌiːtˈiː"),
            ("Rd", "ˌɑːdˈiː"),
            ("Prof", "pɹˈɒf"),
            ("approx", "ɐpɹˈɒks"),
            ("3rd", "θˈɜːd"),
            ("8x10", "ˈeɪt ˈɛks tˈɛn"),
            ("2x4", "tˈuː ˈɛks fˈɔː"),
            ("5 x 7", "fˈaɪv ˈɛks sˈɛvən"),
            ("2x=4", "tˈuː ˈɛks ˈiːkwəlz fˈɔː"),
            ("3x faster", "θɹˈiː ˈɛks fˈastə"),
            ("0x1F", "zˈiəɹəʊ ˈɛks wˈɒn ˈɛf"),
            ("No. 5", "nˈəʊ\nfˈaɪv"),
            ("issue No. 42", "ˈɪʃuː nˈəʊ\nfˈɔːti tˈuː"),
            ("No way", "nˈəʊ wˈeɪ"),
            ("No. I refuse", "nˈəʊ\naɪ ɹɪfjˈuːz"),
            ("5²", "fˈaɪv tˈuː"),
            ("x³", "ˈɛks θɹˈiː"),
            ("E=mc²", "ˈiː ˈiːkwəlz ˌɛmsˈiː tˈuː"),
            ("H₂O", "ˈeɪtʃ tˈuː ˈəʊ"),
            ("5km", "fˈaɪv kˌeɪˈɛm"),
            ("1km", "wˈɒn kˌeɪˈɛm"),
            ("10 kg", "tˈɛn kˌeɪdʒˈiː"),
            ("500ml", "fˈaɪvhˈʌndɹɪd ˌɛmˈɛl"),
            ("20MB", "twˈɛnti ˌɛmbˈiː"),
            ("100Hz", "wˈɒnhˈʌndɹɪd ˌeɪtʃzˈɛd"),
            ("2.5km", "tˈuː pɔɪnt fˈaɪv kˌeɪˈɛm"),
            ("5 cats", "fˈaɪv kˈats"),
            ("km", "kˌeɪˈɛm"),
        ],
    );
    oracle_check(
        "de",
        &[("5km", "fˈʏnf kˌɑːˈɛm"), ("10kg", "tsˈeːn kˌɑːɡˈeː")],
    );
    oracle_check(
        "fr",
        &[
            ("5km", "sˈɛ\u{303}k kilomˈɛtʁ"),
            ("1km", "ˈœ\u{303} kilomˈɛtʁ"),
        ],
    );
    oracle_check("es", &[("5km", "θˈinko kˌaˈɛme")]);
    oracle_check("it", &[("5km", "tʃˈinkwe kˌapːaˈɛmme")]);
}

/// Stylized letters and digits, read as C reads them (by their codes).
#[test]
fn stylized_letters_as_c() {
    oracle_check(
        "en",
        &[
            ("𝐇𝐞𝐥𝐥𝐨", "həlˈəʊ"),
            ("𝓗𝓮𝓵𝓵𝓸", "həlˈəʊ"),
            ("Ⓗⓔⓛⓛⓞ", "lˌɛtətˌuːfˌɔːdˈiːsˌɛvənlˌɛtətˌuːfˈɔːdˌiːfˌɔːlˌɛtətˈuːfˌɔːdˌiːbˈiːlˌɛtətˌuːfˌɔːdˈiːbˌiːlˌɛtətˌuːfˈɔːdˌiːˈiː"),
            ("𝟏𝟐𝟑", "wˈɒnhˈʌndɹɪdən twˈɛnti θɹˈiː"),
        ],
    );
}
