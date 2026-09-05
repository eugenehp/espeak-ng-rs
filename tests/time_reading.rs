// tests/time_reading.rs
//
// "H:MM" (and "H:MM:SS") is a time: the colon is a silent time separator, not a
// clause boundary.  The port used to break the clause at every ':' — "10:30"
// came out as two clauses ("tˈɛn⏎θˈɜːti").  Now the colon is silent, the hour
// drops a leading zero, and connector-word languages (de "uhr", fr "heures",
// it "e") insert it between hour and minute for a valid hour (0–23).  Values
// were confirmed against the espeak-ng 1.52.0 C binary.

use espeak_ng::Translator;

fn tr(lang: &str) -> Option<Translator> {
    match Translator::new_default(lang) {
        Ok(t) => Some(t),
        Err(_) => { eprintln!("[SKIP] no {lang} dict"); None }
    }
}

#[test]
fn english_times_read_as_hour_minute_no_break() {
    let Some(t) = tr("en") else { return };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();
    assert_eq!(ipa("10:30"), "tˈɛn θˈɜːti");          // ten thirty, one clause
    assert!(!ipa("10:30").contains('\n'), "10:30 must not break");
    assert_eq!(ipa("10:05"), "tˈɛn zˈiəɹəʊ fˈaɪv");   // minute keeps its leading zero
    assert_eq!(ipa("02:30"), "tˈuː θˈɜːti");          // hour drops its leading zero
    assert_eq!(ipa("00:30"), "zˈiəɹəʊ θˈɜːti");       // hour "00" → "zero"
    assert_eq!(ipa("10:30:45"), "tˈɛn θˈɜːti fˈɔːti fˈaɪv"); // H:MM:SS
    assert_eq!(ipa("100:30"), "wˈɒnhˈʌndɹɪd θˈɜːti"); // any hour, colon still silent
}

#[test]
fn german_time_inserts_uhr_for_valid_hours_only() {
    let Some(t) = tr("de") else { return };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();
    // "uhr" connector present for a valid hour (0–23) …
    assert!(ipa("10:30").contains("uːɾ"), "de 10:30 → 'uhr': {}", ipa("10:30"));
    assert!(ipa("23:30").contains("uːɾ"), "de 23:30 → 'uhr': {}", ipa("23:30"));
    assert!(!ipa("10:30").contains('\n'), "de 10:30 must not break");
    // … absent for an out-of-range hour.
    assert!(!ipa("24:30").contains("uːɾ"), "de 24:30 → no 'uhr': {}", ipa("24:30"));
    assert!(!ipa("100:30").contains("uːɾ"), "de 100:30 → no 'uhr': {}", ipa("100:30"));
}

#[test]
fn french_and_italian_connectors() {
    if let Some(t) = tr("fr") {
        // dix heures trente — the /z/ liaison of "dix" surfaces before "heures".
        let ipa = t.text_to_ipa("10:30").unwrap();
        assert!(ipa.contains("œʁ") && !ipa.contains('\n'), "fr 10:30: {ipa}");
        assert!(ipa.starts_with("dˈiz"), "fr 10:30 liaison: {ipa}");
    }
    if let Some(t) = tr("it") {
        let ipa = t.text_to_ipa("10:30").unwrap();
        assert!(ipa.contains(" ˈeː ") || ipa.contains(" e "), "it 10:30 connector 'e': {ipa}");
        assert!(!ipa.contains('\n'), "it 10:30 must not break: {ipa}");
    }
}

#[test]
fn non_times_read_the_colon_not_a_break() {
    let Some(t) = tr("en") else { return };
    // A one- or three-digit field after ':' is not a time (not an hour:minute
    // pair).  In English the ':' is read as "colon" (a ratio like "3:2" →
    // "three colon two"), not a clause break.
    for s in ["10:3", "10:300", "3:2"] {
        let ipa = t.text_to_ipa(s).unwrap();
        assert!(!ipa.contains('\n'), "{s} must not break: {ipa:?}");
        assert!(ipa.contains("kˈəʊlən"), "{s} reads ':' as 'colon': {ipa:?}");
    }
}

#[test]
fn non_time_colon_other_languages_drop_it() {
    // German/French read the non-time colon as nothing (not "Doppelpunkt"), and
    // never break the clause — "3:2" → "drei zwei" / "trois deux".
    for lang in ["de", "fr"] {
        let Some(t) = tr(lang) else { continue };
        let ipa = t.text_to_ipa("3:2").unwrap();
        assert!(!ipa.contains('\n'), "{lang} 3:2 must not break: {ipa:?}");
        // no "colon" word for these languages
        assert!(!ipa.contains("kˈəʊlən"), "{lang} 3:2 drops the colon: {ipa:?}");
    }
}
