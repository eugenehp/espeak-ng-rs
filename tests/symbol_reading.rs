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
    let Some(t) = en() else { eprintln!("[SKIP] no en data"); return };
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
    let Some(t) = en() else { eprintln!("[SKIP] no en data"); return };
    // Previously these Unicode symbols were dropped entirely.
    let times = t.text_to_ipa("2×3").unwrap();
    assert!(times.contains("tˈaɪmz"), "× → times: {times}");
    let div = t.text_to_ipa("10÷2").unwrap();
    assert!(div.contains("dɪvˈaɪdɪd") && div.contains("baɪ"), "÷ → divided by: {div}");
    let deg = t.text_to_ipa("50°").unwrap();
    assert!(deg.contains("dɪɡɹˈiːz"), "° → degrees: {deg}");
    // U+2212 MINUS SIGN → "minus" (a negative amount reads correctly).
    let minus = t.text_to_ipa("−5").unwrap();
    assert!(minus.contains("mˈaɪnəs"), "− → minus: {minus}");
    let sect = t.text_to_ipa("§5").unwrap();
    assert!(sect.contains("sˈɛkʃən"), "§ → section: {sect}");
    // Math relation/operator symbols (previously dropped).
    assert!(t.text_to_ipa("∞").unwrap().contains("ɪnfˈɪnɪti"), "∞ → infinity");
    assert!(t.text_to_ipa("√4").unwrap().contains("skwˈeə ɹˈuːt"), "√ → square root");
    assert!(t.text_to_ipa("3≠4").unwrap().contains("nˌɒt ˈiːkwəl"), "≠ → not equal to");
    assert!(t.text_to_ipa("x≤5").unwrap().contains("lˈɛs ðɐn ɔː ˈiːkwəl"), "≤ → less than or equal");
    // Temperature symbols (multi-word readings).
    let cel = t.text_to_ipa("20℃").unwrap();
    assert!(cel.contains("dɪɡɹˈiːz") && cel.contains("sˈɛlsɪəs"), "℃ → degrees celsius: {cel}");
    let fah = t.text_to_ipa("72℉").unwrap();
    assert!(fah.contains("dɪɡɹˈiːz") && fah.contains("fˈaɹənhˌaɪt"), "℉ → degrees fahrenheit: {fah}");
    // The two-character forms °C / °F read the same as the single-char ℃ / ℉
    // (the trailing letter is the scale, not a stray "see"/"eff").
    assert_eq!(t.text_to_ipa("20°C").unwrap(), t.text_to_ipa("20℃").unwrap(), "°C == ℃");
    assert_eq!(t.text_to_ipa("72°F").unwrap(), t.text_to_ipa("72℉").unwrap(), "°F == ℉");
    // A bare degree (an angle) stays just "degrees".
    let ang = t.text_to_ipa("90°").unwrap();
    assert!(ang.contains("dɪɡɹˈiːz") && !ang.contains("sˈɛlsɪəs"), "90° stays degrees: {ang}");
    // Trademark / copyright signs (previously dropped).
    assert_eq!(t.text_to_ipa("™").unwrap(), t.text_to_ipa("trademark").unwrap(), "™ → trademark");
    assert_eq!(t.text_to_ipa("©").unwrap(), t.text_to_ipa("copyright").unwrap(), "© → copyright");
    assert!(t.text_to_ipa("Windows®").unwrap().contains("ɹˈɛdʒɪstəd"), "® → registered");
    // Per-mille / per-ten-thousand (previously dropped).
    assert_eq!(t.text_to_ipa("5‰").unwrap(), t.text_to_ipa("5 per thousand").unwrap(), "‰");
    assert_eq!(t.text_to_ipa("50‱").unwrap(), t.text_to_ipa("50 per ten thousand").unwrap(), "‱");
}

#[test]
fn arithmetic_and_degree_symbols_read_in_other_languages() {
    // Previously `+ − °` fell to the dictionary character-name spell-out for
    // non-English voices, which produced garbage letters (German `+` →
    // "ˈɪːɜˌaʊɐ", `°` → "r.ˈʊʑç").  Now they read the proper word.
    let cases: &[(&str, &str, &str)] = &[
        ("de", "5+3", "plˈʊs"),   // plus
        ("de", "5−2", "mˈiːnʊs"), // minus
        ("de", "20°", "ɡrˈɑːt"),  // grad
        ("fr", "5+3", "ply"),     // plus
        ("fr", "20°", "dəɡʁ"),    // degrés
        ("es", "5+3", "mˈas"),    // más
        ("es", "20°", "ɡɾˈaðos"), // grados
        ("it", "20°", "ɡəɾˈadi"), // gradi
        ("nl", "20°", "ɣrˈɑdən"), // graden
        ("de", "5×3", "mˈɑːl"),   // mal (times)
        ("fr", "5×3", "fwˈa"),    // fois
        ("es", "5×3", "poɾ"),     // por
        ("de", "6÷2", "ɡətˈaɪlt"), // geteilt durch
        ("fr", "6÷2", "divisˈe"),  // divisé par
        ("ru", "50%", "prʌtsˈɛnt"), // процент (was garbage "f")
        ("bg", "50%", "protsʲˈent"),
    ];
    for (lang, text, want) in cases {
        let Some(t) = Translator::new(lang, None).ok() else {
            eprintln!("[SKIP] no {lang} data");
            continue;
        };
        let ipa = t.text_to_ipa(text).unwrap();
        assert!(ipa.contains(want), "{lang} {text:?} → want {want:?}, got {ipa:?}");
    }
}

#[test]
fn non_english_currency_symbols_are_spoken() {
    // The euro (and $/£) used to be dropped for non-English voices (`de 5€` →
    // "fünf").  Now the symbol reads as the currency word after the amount,
    // identical to writing "<amount> <word>".
    let cases: &[(&str, &str, &str)] = &[
        ("de", "5€", "5 euro"),
        ("de", "€5", "5 euro"),
        ("fr", "5€", "5 euros"),
        ("es", "5€", "5 euros"),
        ("it", "5€", "5 euro"),
        ("pt", "5€", "5 euros"),
        ("nl", "5€", "5 euro"),
        ("de", "5$", "5 dollar"),
        ("fr", "5£", "5 livres"),
    ];
    for (lang, sym, expanded) in cases {
        let Some(t) = Translator::new(lang, None).ok() else {
            eprintln!("[SKIP] no {lang} data");
            continue;
        };
        let got = t.text_to_ipa(sym).unwrap();
        // The symbol added a spoken word (it is no longer dropped)…
        assert_ne!(got, t.text_to_ipa("5").unwrap(), "{lang} {sym}: symbol dropped");
        // …and reads the same as the written-out amount.
        assert_eq!(got, t.text_to_ipa(expanded).unwrap(), "{lang} {sym} ≠ {expanded:?}");
    }

    // Amount of exactly one apocopates the "one" before the currency noun:
    // "ein Euro" (not "eins"), "un euro" (not "uno").
    let one: &[(&str, &str, &str)] = &[
        ("de", "1€", "ein euro"),
        ("de", "1$", "ein dollar"),
        ("es", "1€", "un euro"),
        ("it", "1€", "un euro"),
    ];
    for (lang, sym, expanded) in one {
        if let Some(t) = Translator::new(lang, None).ok() {
            assert_eq!(
                t.text_to_ipa(sym).unwrap(),
                t.text_to_ipa(expanded).unwrap(),
                "{lang} {sym} should read as {expanded:?}"
            );
        }
    }
}

#[test]
fn stylized_letters_normalize_to_ascii() {
    let Some(t) = en() else { eprintln!("[SKIP] no en data"); return };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();
    let hello = ipa("Hello");
    // Every mathematical / circled style of "Hello" reads the same as plain ASCII.
    for styled in ["𝐇𝐞𝐥𝐥𝐨", "𝐻𝑒𝑙𝑙𝑜", "𝓗𝓮𝓵𝓵𝓸", "𝕳𝖊𝖑𝖑𝖔", "𝙷𝚎𝚕𝚕𝚘", "Ⓗⓔⓛⓛⓞ"] {
        assert_eq!(ipa(styled), hello, "{styled} should read as Hello");
    }
    // Stylized digits normalise too.
    assert_eq!(ipa("𝟏𝟐𝟑"), ipa("123"), "bold digits → 123");
}

#[test]
fn no_abbreviation_reads_number() {
    let Some(t) = en() else { eprintln!("[SKIP] no en data"); return };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();
    // "No." before a number reads "number" (a port convenience; the `№` sign
    // instead reads "numero", matching espeak-ng — the two are not the same).
    assert_eq!(ipa("No. 5"), ipa("number 5"), "No. 5 → number 5");
    assert_eq!(ipa("issue No. 42"), ipa("issue number 42"));
    // The word "no" is left alone when no number follows.
    assert!(ipa("No way").contains("nˈəʊ"), "No way stays 'no'");
    assert!(ipa("No. I refuse").contains("nˈəʊ"), "No. I refuse stays 'no'");
}

#[test]
fn dimension_x_reads_by() {
    let Some(t) = en() else { eprintln!("[SKIP] no en data"); return };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();
    // A lowercase x between two numbers is a dimension: "8x10" → "eight by ten".
    for (dim, spelled) in [("8x10", "8 by 10"), ("2x4", "2 by 4"), ("5 x 7", "5 by 7")] {
        assert_eq!(ipa(dim), ipa(spelled), "{dim} → {spelled}");
    }
    // With no number after the x (algebra, multipliers) it stays the letter.
    assert!(ipa("2x=4").contains("ˈɛks"), "2x= stays letter x: {}", ipa("2x=4"));
    assert!(ipa("3x faster").contains("ˈɛks"), "3x faster stays letter x");
    // A `0` side means it's not a dimension (hex literal `0x1F`).
    assert!(ipa("0x1F").contains("ˈɛks"), "0x1F stays letter x (hex): {}", ipa("0x1F"));
}

#[test]
fn common_abbreviations_expand() {
    let Some(t) = en() else { eprintln!("[SKIP] no en data"); return };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();
    // Abbreviations that otherwise read as unpronounceable clusters expand to the
    // full word (case-insensitive, period-tolerant).
    let cases: &[(&str, &str)] = &[
        ("vs", "versus"),
        ("govt", "government"),
        ("Blvd", "boulevard"),
        ("Sgt", "sergeant"),
        ("Rd", "road"),
        ("Prof", "professor"),
        ("approx", "approximately"),
    ];
    for (abbr, word) in cases {
        assert_eq!(ipa(abbr), ipa(word), "{abbr} should expand to {word}");
    }
    // Ordinals ending in "rd" are NOT the abbreviation (3rd = "third", not "road").
    assert!(ipa("3rd").contains("θˈɜːd"), "3rd stays third: {}", ipa("3rd"));
    // A real word matching nothing is untouched.
    assert!(ipa("road").contains("ɹˈəʊd"), "road unchanged");
}

#[test]
fn superscript_powers_read_squared_cubed() {
    let Some(t) = en() else { eprintln!("[SKIP] no en data"); return };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();
    // Superscript ² / ³ are powers, not the digit "two"/"three".
    assert_eq!(ipa("5²"), ipa("5 squared"), "5² → five squared");
    assert_eq!(ipa("x³"), ipa("x cubed"), "x³ → x cubed");
    assert_eq!(ipa("E=mc²"), ipa("E = mc squared"), "E=mc² → …mc squared");
    // Subscripts stay digits (chemical formulae): H₂O → "H two O".
    assert!(ipa("H₂O").contains("tˈuː"), "H₂O keeps the digit: {}", ipa("H₂O"));
    assert!(!ipa("H₂O").contains("skwˈeəd"), "H₂O is not 'squared'");
}

#[test]
fn unit_abbreviations_are_spelled_out() {
    let Some(t) = en() else { eprintln!("[SKIP] no en data"); return };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();
    // A unit after a number reads as the full word (British spelling), pluralised
    // unless the amount is one.  The abbreviation may abut the number or not.
    let cases: &[(&str, &str)] = &[
        ("5km", "5 kilometres"),
        ("1km", "1 kilometre"),   // singular
        ("10 kg", "10 kilograms"),
        ("500ml", "500 millilitres"),
        ("20MB", "20 megabytes"),
        ("100Hz", "100 hertz"),   // invariant plural
        ("2.5km", "2.5 kilometres"),
    ];
    for (abbr, spelled) in cases {
        assert_eq!(ipa(abbr), ipa(spelled), "{abbr} should read as {spelled}");
    }
    // A non-unit word after a number, or a unit with no number, is untouched.
    assert!(ipa("5 cats").contains("kˈats"), "5 cats unchanged");
    assert!(ipa("km").contains("kˈəm"), "bare km unchanged: {}", ipa("km"));
}

#[test]
fn units_in_other_languages() {
    // The metric units read in the language's own words (de Kilometer, fr
    // kilomètres, es kilómetros, it chilometri).
    let cases: &[(&str, &str, &str)] = &[
        ("de", "5km", "5 kilometer"),
        ("de", "10kg", "10 kilogramm"),
        ("fr", "5km", "5 kilomètres"),
        ("fr", "1km", "1 kilomètre"),
        ("es", "5km", "5 kilómetros"),
        ("it", "5km", "5 chilometri"),
    ];
    for (lang, abbr, spelled) in cases {
        let Some(t) = Translator::new(lang, None).ok() else {
            eprintln!("[SKIP] no {lang} data");
            continue;
        };
        assert_eq!(t.text_to_ipa(abbr).unwrap(), t.text_to_ipa(spelled).unwrap(), "{lang} {abbr} → {spelled}");
    }
}

#[test]
fn decades_read_as_plurals() {
    let Some(t) = en() else { eprintln!("[SKIP] no en data"); return };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();
    // "1980s" used to read "…eighty ess"; now it's a spoken decade.
    let cases: &[(&str, &str)] = &[
        ("1980s", "nineteen eighties"),
        ("1990s", "nineteen nineties"),
        ("1900s", "nineteen hundreds"),
        ("1850s", "eighteen fifties"),
        ("80s", "eighties"),
        ("90s", "nineties"),
        ("20s", "twenties"),
        // The possessive-apostrophe spelling is also a decade.
        ("1990's", "nineteen nineties"),
        ("80's", "eighties"),
    ];
    for (digit, word) in cases {
        assert_eq!(ipa(digit), ipa(word), "{digit} should read as {word}");
    }
    // Not a decade: a non-round number keeps its normal reading + trailing 's'.
    assert_eq!(ipa("1985"), ipa("1985"), "sanity");
    assert!(ipa("5s").contains("ˈɛs"), "5s stays 'five ess': {}", ipa("5s"));
    // 2000s are ambiguous and deliberately left alone.
    assert!(ipa("2000s").contains("ˈɛs"), "2000s left alone: {}", ipa("2000s"));
}

#[test]
fn ordinary_punctuation_is_silent() {
    let Some(t) = en() else { eprintln!("[SKIP] no en data"); return };
    // None of these add a spoken word between "x" and "y".
    for input in ["x ! y", "x ( y", "x ) y", "x < y", "x > y", "x ? y", "x ; y"] {
        let ipa = t.text_to_ipa(input).unwrap().replace('\n', " ");
        let words: Vec<&str> = ipa.split_whitespace().collect();
        assert_eq!(words.len(), 2, "punctuation {input:?} was spoken: {ipa:?}");
    }
}

#[test]
fn punct_announces_punctuation_by_name() {
    let Some(mut t) = en() else { eprintln!("[SKIP] no en data"); return };
    // Baseline: ordinary punctuation is silent (two spoken words).
    let base = t.text_to_ipa("a, b.").unwrap().replace('\n', " ");
    assert_eq!(base.split_whitespace().count(), 2, "baseline {base:?}");

    // `--punct` (all): comma and period are announced by name.
    t.options.punct = Some(Vec::new());
    assert_eq!(t.text_to_ipa("a, b.").unwrap(), "ˈeɪ kˈɒmə bˈiː dˈɒt");

    // `--punct=","` (only comma): the period stays silent.
    t.options.punct = Some(vec![',']);
    let only_comma = t.text_to_ipa("a, b.").unwrap();
    assert!(only_comma.contains("kˈɒmə"), "comma should be announced: {only_comma:?}");
    assert!(!only_comma.contains("dˈɒt"), "period should stay silent: {only_comma:?}");
}

#[test]
fn punct_reaches_the_code_stream() {
    // The `-x` / synthesis path (`translate_to_codes`) must also emit punct names.
    let Some(mut t) = en() else { return };
    let count = |t: &Translator, s: &str| {
        t.translate_to_codes(s).unwrap().iter().filter(|c| !c.is_boundary).count()
    };
    let plain = count(&t, "a, b.");
    t.options.punct = Some(Vec::new());
    let punct = count(&t, "a, b.");
    assert!(punct > plain, "--punct added no codes to -x path: {punct} vs {plain}");
}

#[test]
fn spoken_symbols_reach_the_code_stream() {
    let Some(t) = en() else { eprintln!("[SKIP] no en data"); return };
    // Regression: `translate_to_codes` (the `-x` / synthesis path) dropped
    // spoken symbols even though `--ipa` (`text_to_ipa`) rendered them — the
    // emission match had no `Token::Punctuation` arm.  `&` → "and" must now emit.
    let n = |s: &str| {
        t.translate_to_codes(s)
            .expect("codes")
            .iter()
            .filter(|c| !c.is_boundary)
            .count()
    };
    assert!(n("a & b") > n("a b"), "spoken '&' added no codes");
    assert!(n("5 % x") > n("5 x"), "spoken '%' added no codes");
    // Non-spoken punctuation stays silent (no extra codes vs plain text).
    assert_eq!(n("a (b) c"), n("a b c"), "parentheses should be silent");
}

#[test]
fn ampersand_reads_and_not_ampersand() {
    let Some(t) = en() else { eprintln!("[SKIP] no en data"); return };
    // Confirmed identical to the oracle.
    assert_eq!(t.text_to_ipa("Tom & Jerry").unwrap(), "tˈɒm and dʒˈɛɹi");
}

#[test]
fn percent_after_number() {
    let Some(t) = en() else { eprintln!("[SKIP] no en data"); return };
    let ipa = t.text_to_ipa("50 % off").unwrap();
    assert!(ipa.contains("pəsˈɛnt"), "expected 'percent' in {ipa:?}");
}

#[test]
fn numero_sign_reads_number_word() {
    // № is read as each language's own "numero" word, but only where espeak-ng
    // defines one.  In these languages the sign is spoken, so "№5" differs from
    // a bare "5".  (Values follow the C oracle.)
    for lang in ["en", "de", "it", "pt", "nl", "ru", "bg", "cs"] {
        let Some(t) = Translator::new(lang, None).ok() else {
            eprintln!("[SKIP] no {lang} data");
            continue;
        };
        let n = t.text_to_ipa("№5").unwrap();
        let bare = t.text_to_ipa("5").unwrap();
        assert!(!n.is_empty() && n != bare, "{lang} №5 dropped the numero sign: {n:?} (bare {bare:?})");
    }
    // English reads "numero" (matching espeak-ng), not "number".
    if let Some(t) = en() {
        assert!(t.text_to_ipa("№5").unwrap().contains("njˈuːmə"), "en №5 = numero five");
    }
    // Other languages have no numero word in espeak-ng, so the sign is dropped
    // (the bare number is spoken).
    for lang in ["fr", "es", "uk", "pl"] {
        let Some(t) = Translator::new(lang, None).ok() else {
            eprintln!("[SKIP] no {lang} data");
            continue;
        };
        assert_eq!(
            t.text_to_ipa("№5").unwrap(),
            t.text_to_ipa("5").unwrap(),
            "{lang} №5 should drop the sign"
        );
    }
}

#[test]
fn multiply_and_divide_read_across_languages() {
    // × and ÷ were dropped (silent) outside en/de/fr/es/it/nl; each language
    // now reads its own operator word, so "2×3" / "10÷2" differ from the bare
    // digit sequence (the operator is spoken, not dropped).
    for lang in ["pt", "pl", "cs", "ro", "sv", "da", "nb", "tr", "ru", "uk", "bg"] {
        let Some(t) = Translator::new(lang, None).ok() else {
            eprintln!("[SKIP] no {lang} data");
            continue;
        };
        let times = t.text_to_ipa("2×3").unwrap();
        let divide = t.text_to_ipa("10÷2").unwrap();
        let two_three = format!("{} {}", t.text_to_ipa("2").unwrap(), t.text_to_ipa("3").unwrap());
        assert_ne!(times.replace(' ', ""), two_three.replace(' ', ""),
            "{lang} 2×3 dropped the ×: {times:?}");
        assert!(divide.split_whitespace().count() >= 3,
            "{lang} 10÷2 dropped the ÷: {divide:?}");
    }
}

#[test]
fn plus_minus_sign_reads_across_languages() {
    // ± was dropped outside en/de/fr/nl; the languages below now read it.
    for lang in ["es", "it", "pt", "pl", "cs", "ro", "sv", "da", "nb", "tr", "ru", "uk", "bg"] {
        let Some(t) = Translator::new(lang, None).ok() else {
            eprintln!("[SKIP] no {lang} data");
            continue;
        };
        let pm = t.text_to_ipa("±5").unwrap();
        let five = t.text_to_ipa("5").unwrap();
        assert_ne!(pm.replace(' ', ""), five.replace(' ', ""),
            "{lang} ±5 dropped the sign: {pm:?}");
    }
}

#[test]
fn hebrew_and_ukrainian_operators() {
    // Hebrew: + − × ÷ ° were dropped; now read (via niqqud-pointed words that
    // espeak-Hebrew can pronounce).  Ukrainian: ° was dropped.
    if let Some(he) = Translator::new("he", None).ok() {
        let two_three = format!("{} {}", he.text_to_ipa("2").unwrap(), he.text_to_ipa("3").unwrap());
        for expr in ["2+3", "2×3", "10÷2", "20°"] {
            let got = he.text_to_ipa(expr).unwrap();
            assert_ne!(got.replace(' ', ""), two_three.replace(' ', ""),
                "he {expr} dropped the operator: {got:?}");
            assert!(!got.trim().is_empty());
        }
        assert!(he.text_to_ipa("-5").unwrap().contains("mˈinus"), "he -5: {}", he.text_to_ipa("-5").unwrap());
        assert!(he.text_to_ipa("20°").unwrap().contains("maʔˈalot"), "he 20°: {}", he.text_to_ipa("20°").unwrap());
    }
    if let Some(uk) = Translator::new("uk", None).ok() {
        let deg = uk.text_to_ipa("20°").unwrap();
        assert_ne!(deg.replace(' ', ""), uk.text_to_ipa("20").unwrap().replace(' ', ""),
            "uk 20° dropped the degree sign: {deg:?}");
    }
}

#[test]
fn degree_sign_reads_across_languages() {
    // ° was dropped in sv/da/nb/ru/bg; now read as "degrees" (a fixed plural,
    // correct for the common temperature case).  "20°" must differ from "20".
    for lang in ["sv", "da", "nb", "ru", "bg"] {
        let Some(t) = Translator::new(lang, None).ok() else {
            eprintln!("[SKIP] no {lang} data");
            continue;
        };
        let deg = t.text_to_ipa("20°").unwrap();
        let bare = t.text_to_ipa("20").unwrap();
        assert_ne!(deg.replace(' ', ""), bare.replace(' ', ""),
            "{lang} 20° dropped the degree sign: {deg:?}");
        // A negative temperature composes the minus + degree reading.
        assert!(t.text_to_ipa("-40°").unwrap().split_whitespace().count() >= 3,
            "{lang} -40° should read minus … degrees: {}", t.text_to_ipa("-40°").unwrap());
    }
}
