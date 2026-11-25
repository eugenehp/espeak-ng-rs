// tests/open_issue_regressions.rs
//
// One case per fixed GitHub issue; the expected strings are C espeak-ng's, run
// on the bundled data.

use std::io::Write;
use std::process::{Command, Stdio};

use espeak_ng::dictionary::file::Dictionary;
use espeak_ng::Translator;

fn check(lang: &str, cases: &[(&str, &str)]) {
    let Ok(t) = Translator::new(lang, None) else {
        eprintln!("[SKIP] no {lang} data");
        return;
    };
    for (text, want) in cases {
        assert_eq!(&t.text_to_ipa(text).unwrap(), want, "{lang} {text:?}");
    }
}

#[test]
fn issue_7_english_initial_x() {
    check("en", &[("xa", "zˈɑː"), ("xoab", "zˈəʊab"), ("xaiph", "zˈeɪf")]);
}

#[test]
fn issue_8_kurdish_and_before_hundreds() {
    check("ku", &[("1999", "hɛzˈar unˈɛhsˈɛdu nˈodunˈɛh")]);
}

#[test]
fn issue_9_arabic_syllabic_ain() {
    check("ar", &[("7", "sˈabʕʕˌa"), ("307", "θalˈaːθumˌiʔawa sˈabʕʕˌa")]);
}

#[test]
fn issue_10_single_stress_in_compound_numerals() {
    check("tr", &[("21", "jirmˌibˈir")]);
    check("es", &[("1234", "mˈil ðosθjˈentos tɾˌeɪntaikwˈatɾo")]);
    check("de", &[("21", "ˈaɪn ʊnttsvˈantsɪç")]);
}

#[test]
fn issue_11_switched_words_keep_their_voicing() {
    check("de", &[("job", "(en)dʒˈɒb(de)")]);
    check("nl", &[("website", "(en)wˈɛbsaɪt(nl)")]);
}

#[test]
fn issues_12_13_14_german_and_dutch_rules() {
    check("de", &[("florian", "flˈoːriːˌɑːn"), ("monat", "mˈoːnɑːt"), ("sagt", "zˈɑːkt")]);
    check("nl", &[("nasaal", "naːzˈaːl"), ("hebt", "hˈɛpt"), ("schrompel", "sxrˈɔmpəl")]);
}

#[test]
fn issue_15_italian_single_primary() {
    check("it", &[("invalidano", "invˈalidano"), ("oper", "ˈɔper"), ("lumache", "lʊmˈake")]);
}

#[test]
fn issue_17_whole_word_flags_place_prefix_stress() {
    check("en", &[("output", "ˈaʊtpʊt"), ("hyperspace", "hˈaɪpəspˌeɪs"), ("torment", "tˈɔːmɛnt")]);
}

#[test]
fn issue_18_french_acronyms() {
    check("fr", &[("ac", "ˌasˈe"), ("USA", "ˌyˌɛsˈa"), ("cia", "sˌejˈa")]);
}

#[test]
fn issues_20_21_russian_and_polish() {
    check("ru", &[("трёхсотой", "trʲʌxsˈotʌj")]);
    check("pl", &[("był", "bˈɨw"), ("szył", "ʃˈɨw")]);
}

#[test]
fn issues_30_37_38_long_numbers() {
    check("en", &[("0800", "zˈiəɹəʊˈeɪtzˈiəɹəʊzˈiəɹəʊ")]);
    check(
        "de",
        &[(
            "12345678901234567890",
            "ˈaɪnstsvˈaɪ dɾˈaɪfˈiːɾfˈʏnf zˈɛkszˈiːbənˈaxt nˈɔønnˈʊlˈaɪns tsvˈaɪdɾˈaɪfˈiːɾ fˈʏnfzˈɛkszˈiːbən ˈaxtnˈɔønnˈʊl",
        )],
    );
    let Ok(t) = Translator::new("en", None) else {
        return;
    };
    let ipa = t.text_to_ipa("99999999999999999999").unwrap();
    assert!(ipa.starts_with("nˈaɪnti nˈaɪn kwɪntˈɪliən"), "{ipa}");
    // Every digit of a huge number is spoken and stressed.
    let ipa = t.text_to_ipa(&"7".repeat(100_000)).unwrap();
    assert_eq!(ipa.matches("sˈɛvən").count(), 100_000);
}

#[test]
fn issue_44_french_scale_words_keep_their_stress() {
    check("fr", &[("30001", "tʁɑ̃t mˈil ˈœ̃")]);
}

#[test]
fn issues_31_32_long_input_is_linear() {
    let Ok(t) = Translator::new("en", None) else {
        return;
    };
    // Quadratic time made these run for minutes.
    let start = std::time::Instant::now();
    t.text_to_ipa(&"a".repeat(300_000)).unwrap();
    t.text_to_ipa(&".".repeat(300_000)).unwrap();
    assert!(start.elapsed().as_secs() < 60, "{:?}", start.elapsed());
}

#[test]
fn issue_28_truncated_rules_are_an_error() {
    // Header, 1024 empty hash buckets, then a lone RULE_GROUP_START.
    let mut data = 1024u32.to_le_bytes().to_vec();
    let rules = 8 + 1024u32;
    data.extend(rules.to_le_bytes());
    data.extend(std::iter::repeat(0u8).take(1024));
    data.push(6);
    assert!(Dictionary::from_bytes("en", data).is_err());
}

#[test]
fn issue_33_invalid_utf8_is_replaced() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_espeak_cli"))
        .args(["-q", "--ipa", "-v", "en", "--stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(b"hello \xff\xfe world \xe2\x82").unwrap();
    drop(stdin);
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    let ipa = String::from_utf8_lossy(&out.stdout);
    assert!(ipa.contains("həlˈəʊ") && ipa.contains("wˈɜːld"), "{ipa}");
}

#[test]
fn issue_9_arabic_thousands() {
    check(
        "ar",
        &[
            ("2026", "ʔalfˈaːn wa sˈitːawˌaʕiʃrˈuːn"),
            ("3074", "θalˈaːθatˌu ʔaːlˈaːf wa ʔˈarbaʕʕˌawasˌabʕʕˈuːn"),
        ],
    );
}

#[test]
fn issue_19_symbols_and_accented_letters() {
    check("fr", &[("²", "pyisˈɑ̃s dˈø"), ("½", "œ̃dəmˈi"), ("ç", "sˌesedˈij")]);
    check("es", &[("²", "dˈos"), ("ç", "θˌeθeðˈiʝa")]);
}

#[test]
fn issue_26_suffix_added_after_stress() {
    check("es", &[("kg", "kˌiloɣɾˈamo"), ("aprox", "ˌapɾoksimˈaðamˈente")]);
}

#[test]
fn issue_16_foreign_letters_name_their_alphabet() {
    check("en", &[("د", "ˈaɹəbɪk(ar)dˈaːl(en)"), ("ר", "hˈiːbɹuːrˈeʃ"), ("ڈ", "ˈaɹəbɪkdˈe")]);
    check("de", &[("د", "arˈɑbɪʃ(ar)dˈaːl(de)")]);
}

#[test]
fn issues_22_23_47_portuguese() {
    check(
        "pt",
        &[
            ("estar", "ɨʃtˈɐɹ"),
            ("fomos", "fˈumʊʃ"),
            ("ter", "tˈɨɹ"),
            ("os amigos", "uz ɐmˈiɡʊʃ"),
            ("cento e um", "sˈeɪŋtw i ũŋ"),
            ("trump", "(en)tɹˈʌmp(pt-pt)"),
        ],
    );
}

#[test]
fn issue_27_clause_breaks_before_quotes() {
    check("en", &[("He said: \"hello\"", "hiː sˈɛd\nhəlˈəʊ")]);
    check("ku", &[("Wî ji min re got: \"were\".", "wi ʒɪ mˈɪn rɛ ɡot\nwɛɾˈɛ")]);
}

#[test]
fn unclosed_ssml_tag_at_the_end_does_not_panic() {
    let Ok(t) = Translator::new("en", None) else {
        return;
    };
    for text in ["<", "hello <", "<speak>hi<", "<speak", "</"] {
        t.text_to_ipa_ssml(text).unwrap();
    }
}

/// A dictionary whose only entry is for "a", with the given raw entry bytes.
fn dictionary_with_entry(entry: &[u8]) -> Option<Dictionary> {
    // The same entry in every bucket, so the lookup is sure to reach it.
    let mut body = Vec::new();
    for _ in 0..1024 {
        body.push(entry.len() as u8 + 1);
        body.extend_from_slice(entry);
        body.push(0);
    }
    let mut data = 1024u32.to_le_bytes().to_vec();
    data.extend((8 + body.len() as u32).to_le_bytes());
    data.extend(body);
    data.push(7); // RULE_GROUP_END
    Dictionary::from_bytes("en", data).ok()
}

#[test]
fn issue_29_malformed_dictionary_flags_do_not_panic() {
    let ctx = espeak_ng::dictionary::lookup::LookupCtx::default();
    // `wlen` 0x41: one byte, compressed ("a" is code 1).  Flag bytes 64, 170
    // and 255 shift past the 32-bit flag words.
    for flag in [64u8, 131, 163, 164, 170, 255] {
        for word in [[0xc1u8, 1], [0xc1, b'a'], [0x81, b'a']] {
            let entry = [word[0], word[1], flag];
            if let Some(dict) = dictionary_with_entry(&entry) {
                let _ = espeak_ng::dictionary::lookup::lookup(&dict, "a", &ctx);
            }
        }
    }
}

#[test]
fn issue_29_phoneme_table_including_itself_does_not_recurse() {
    let Ok(mut phdata) = espeak_ng::phoneme::load::PhonemeData::load(std::path::Path::new(
        "espeak-ng-data",
    )) else {
        return;
    };
    let n = phdata.tables.len() as u8;
    // Itself, a later table, and one that doesn't exist.
    for includes in [1, 2, n, 255] {
        phdata.tables[0].includes = includes;
        let _ = phdata.select_table(0);
        let _ = phdata.select_table_by_name("en");
    }
}

#[test]
fn klatt_variants_use_their_own_glottal_source() {
    let dir = std::env::temp_dir();
    let wav = |voice: &str| {
        let path = dir.join(format!("espeak_rs_{voice}.wav"));
        let status = Command::new(env!("CARGO_BIN_EXE_espeak_cli"))
            .args(["-v", &format!("en+{voice}"), "-w"])
            .arg(&path)
            .arg("hello")
            .status()
            .unwrap();
        assert!(status.success());
        let bytes = std::fs::read(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        bytes
    };
    let voices = ["klatt", "klatt2", "klatt3", "klatt4", "klatt5", "klatt6", "m1"];
    let audio: Vec<_> = voices.iter().map(|v| wav(v)).collect();
    for (i, a) in audio.iter().enumerate() {
        for (j, b) in audio.iter().enumerate().skip(i + 1) {
            assert!(a != b, "{} and {} sound the same", voices[i], voices[j]);
        }
    }
}

#[test]
fn a_loud_echo_is_capped() {
    // `robosoft` asks for an echo of 1000; uncapped, it grows until it clips.
    let path = std::env::temp_dir().join("espeak_rs_robosoft.wav");
    let status = Command::new(env!("CARGO_BIN_EXE_espeak_cli"))
        .args(["-v", "en+robosoft", "-w"])
        .arg(&path)
        .arg("hello world")
        .status()
        .unwrap();
    assert!(status.success());
    let bytes = std::fs::read(&path).unwrap();
    let _ = std::fs::remove_file(&path);
    let samples = bytes[44..].chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]]));
    let clipped = samples.clone().filter(|s| s.unsigned_abs() >= 32767).count();
    assert!(clipped * 100 < samples.count(), "{clipped} samples clip");
}

#[test]
fn numbers_agree_with_their_scale_words() {
    // Values from C espeak-ng f0fb11a8.
    let cases = [
        ("hi", "100000", "'e:k l'a:k#"),
        ("ro", "2026", "d'ou@ m'iI d'ou,@zetSI^_SiS'ase"),
        ("el", "3074", "tr'is Cil^'aDes ,ebdom,indat'eser,a"),
        ("uk", "1000", "od'en t'esjatSa"),
        ("es", "1000000000", "'uno m'il miJ^'ones"),
    ];
    for (lang, text, expected) in cases {
        let out = Command::new(env!("CARGO_BIN_EXE_espeak_cli"))
            .args(["-q", "-x", "-v", lang, text])
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), expected, "{lang} {text}");
    }
}
