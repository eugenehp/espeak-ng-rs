// tests/ssml_integration.rs
//
// SSML markup mode (`-m`) integration tests.
//
// Strategy: the SSML preprocessor transforms markup into the plain text the
// normal pipeline already handles, so we verify **self-consistency** —
// `text_to_ipa_ssml(markup)` must equal `text_to_ipa(plain_equivalent)`.
// This is independent of which espeak-ng data version is installed, so it
// needs no C oracle.  Tests skip automatically when language data is absent.

use espeak_ng::Translator;

fn en() -> Option<Translator> {
    // Uses the default data resolution (ESPEAK_DATA_PATH / ./espeak-ng-data).
    Translator::new("en", None).ok()
}

/// Assert that markup input yields the same IPA as its plain equivalent.
fn assert_markup_eq_plain(t: &Translator, markup: &str, plain: &str) {
    let got = t.text_to_ipa_ssml(markup).expect("ssml ipa");
    let want = t.text_to_ipa(plain).expect("plain ipa");
    assert_eq!(got, want, "\n  markup: {markup:?}\n   plain: {plain:?}");
}

#[test]
fn formatting_tags_are_stripped_content_kept() {
    let Some(t) = en() else { eprintln!("[SKIP] no en data"); return };
    assert_markup_eq_plain(&t, "a <emphasis>b</emphasis> c", "a b c");
    assert_markup_eq_plain(&t, "a <prosody rate=\"50\">b</prosody> c", "a b c");
    assert_markup_eq_plain(&t, "x <foo bar='1'>y</foo> z", "x y z");
    assert_markup_eq_plain(&t, "<speak>hello world</speak>", "hello world");
}

#[test]
fn entities_are_decoded() {
    let Some(t) = en() else { eprintln!("[SKIP] no en data"); return };
    assert_markup_eq_plain(&t, "Tom &amp; Jerry", "Tom & Jerry");
    assert_markup_eq_plain(&t, "x &#65; y", "x A y");
    assert_markup_eq_plain(&t, "x &#x41; y", "x A y");
    assert_markup_eq_plain(&t, "3 &lt; 5", "3 < 5");
}

#[test]
fn say_as_digits_reads_each_digit() {
    let Some(t) = en() else { eprintln!("[SKIP] no en data"); return };
    // `digits` spells each digit: "42" == "4 2" == "four two", not "forty two".
    assert_markup_eq_plain(&t, r#"<say-as interpret-as="digits">42</say-as>"#, "4 2");
    // Distinct from `cardinal`, which reads the whole number.
    let digits = t.text_to_ipa_ssml(r#"<say-as interpret-as="digits">42</say-as>"#).unwrap();
    let cardinal = t.text_to_ipa_ssml(r#"<say-as interpret-as="cardinal">42</say-as>"#).unwrap();
    assert_ne!(digits, cardinal, "digits and cardinal must differ");
}

#[test]
fn say_as_ordinal_reads_ordinals() {
    let Some(t) = en() else { eprintln!("[SKIP] no en data"); return };
    // Ordinal reading: "3" → "3rd" → "third", not "three".
    assert_markup_eq_plain(&t, r#"<say-as interpret-as="ordinal">3</say-as>"#, "3rd");
    assert_markup_eq_plain(&t, r#"<say-as interpret-as="ordinal">21</say-as>"#, "21st");
    assert_markup_eq_plain(&t, r#"<say-as interpret-as="ordinal">12</say-as>"#, "12th");
    // Inline in a clause stays a single clause ("the third item").
    assert_markup_eq_plain(
        &t,
        r#"the <say-as interpret-as="ordinal">3</say-as> item"#,
        "the 3rd item",
    );
    // Distinct from a plain cardinal reading.
    let ord = t.text_to_ipa_ssml(r#"<say-as interpret-as="ordinal">3</say-as>"#).unwrap();
    assert_ne!(ord, t.text_to_ipa("3").unwrap(), "ordinal must differ from cardinal");
}

#[test]
fn voice_name_resolves_in_ssml() {
    let Some(t) = en() else { eprintln!("[SKIP] no en data"); return };
    // A `<voice name="…">` human name resolves like the equivalent language tag.
    let by_name = t.text_to_ipa_ssml(r#"x <voice name="French (France)">bonjour</voice> y"#).unwrap();
    let by_lang = t.text_to_ipa_ssml(r#"x <voice xml:lang="fr">bonjour</voice> y"#).unwrap();
    assert_eq!(by_name, by_lang, "voice name should resolve like the language tag");
    // An unresolvable name falls back to the base voice (no error).
    let fallback = t.text_to_ipa_ssml(r#"<voice name="Nonsense Voice">hello</voice>"#).unwrap();
    assert_eq!(fallback.trim(), t.text_to_ipa("hello").unwrap());
}

#[test]
fn say_as_date_and_time() {
    let Some(t) = en() else { eprintln!("[SKIP] no en data"); return };
    // Date: ISO YYYY-MM-DD → "Month Dayth Year".
    assert_markup_eq_plain(
        &t,
        r#"<say-as interpret-as="date">2024-01-15</say-as>"#,
        "January 15th 2024",
    );
    // Time: HH:MM read as two numbers, with "oh"/"o'clock" refinements.
    assert_markup_eq_plain(&t, r#"<say-as interpret-as="time">14:30</say-as>"#, "14 30");
    assert_markup_eq_plain(&t, r#"<say-as interpret-as="time">9:05</say-as>"#, "9 oh 5");
    assert_markup_eq_plain(&t, r#"<say-as interpret-as="time">14:00</say-as>"#, "14 o'clock");
}

#[test]
fn phoneme_ph_matches_inline_phonemes() {
    let Some(t) = en() else { eprintln!("[SKIP] no en data"); return };
    // `<phoneme ph="X">…</phoneme>` pronounces X (eSpeak mnemonics) and drops
    // the readable content — identical to the always-on `[[X]]` syntax.
    assert_markup_eq_plain(&t, r#"<phoneme ph="h@l'oU">hello</phoneme>"#, "[[h@l'oU]]");
    assert_markup_eq_plain(&t, r#"say <phoneme ph="k">see</phoneme> now"#, "say [[k]] now");
    // Self-closing form.
    assert_markup_eq_plain(&t, r#"x <phoneme ph="t"/> y"#, "x [[t]] y");
    // No `ph`: strip tag, keep content.
    assert_markup_eq_plain(&t, "<phoneme>hello</phoneme>", "hello");
}

#[test]
fn sub_uses_alias() {
    let Some(t) = en() else { eprintln!("[SKIP] no en data"); return };
    assert_markup_eq_plain(
        &t,
        "<sub alias=\"World Health Organization\">WHO</sub>",
        "World Health Organization",
    );
}

#[test]
fn comments_and_pis_discarded() {
    let Some(t) = en() else { eprintln!("[SKIP] no en data"); return };
    assert_markup_eq_plain(&t, "hello<!-- secret -->world", "helloworld");
    assert_markup_eq_plain(&t, "<?xml version='1.0'?>hi there", "hi there");
}

#[test]
fn break_splits_into_two_clauses() {
    let Some(t) = en() else { eprintln!("[SKIP] no en data"); return };
    let ipa = t.text_to_ipa_ssml("one<break time=\"500ms\"/>two").unwrap();
    // Two clause lines, no spurious sentinel character in the output.
    let lines: Vec<&str> = ipa.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(lines.len(), 2, "expected 2 clauses, got {ipa:?}");
    assert!(!ipa.contains('\u{E000}'), "break sentinel leaked into output: {ipa:?}");
}

#[test]
fn voice_switches_language_per_span() {
    let Some(t) = en() else { eprintln!("[SKIP] no en data"); return };
    // Requires French data.
    let fr_ok = Translator::new("fr", None)
        .and_then(|tr| tr.text_to_ipa("bonjour"))
        .is_ok();
    if !fr_ok {
        eprintln!("[SKIP] no fr data");
        return;
    }
    // Confirmed byte-identical to `espeak-ng -q --ipa -m -v en`.
    let ipa = t
        .text_to_ipa_ssml(r#"hello <voice xml:lang="fr">bonjour</voice> bye"#)
        .unwrap();
    assert_eq!(ipa, "həlˈəʊ\nbɔ̃ʒˈuʁ\nbˈaɪ");

    // The middle span must equal a direct French translation.
    let fr = Translator::new("fr", None).unwrap();
    assert_eq!(ipa.lines().nth(1).unwrap(), fr.text_to_ipa("bonjour").unwrap());
}

#[test]
fn plain_text_is_unaffected_by_markup_mode() {
    let Some(t) = en() else { eprintln!("[SKIP] no en data"); return };
    // Text with no markup should be identical with or without -m.
    for s in ["hello world", "the quick brown fox", "42 cats"] {
        assert_eq!(
            t.text_to_ipa_ssml(s).unwrap(),
            t.text_to_ipa(s).unwrap(),
            "markup mode changed plain text {s:?}"
        );
    }
}

#[test]
fn mark_events_are_emitted() {
    use espeak_ng::{EspeakNg, EventKind};
    let Ok(mut e) = EspeakNg::new("en") else { eprintln!("[SKIP] no en data"); return };
    e.set_markup(true);
    let text = r#"hello <mark name="m1"/> world <mark name="done"/>"#;
    let Ok((samples, _rate, events)) = e.synth_with_events(text) else {
        eprintln!("[SKIP] synth failed");
        return;
    };
    assert!(!samples.is_empty());
    let marks: Vec<&str> = events.iter().filter_map(|ev| match &ev.kind {
        EventKind::Mark(n) => Some(n.as_str()),
        _ => None,
    }).collect();
    assert_eq!(marks, vec!["m1", "done"], "mark names/order");
    // A message-terminated event is still present.
    assert!(events.iter().any(|e| matches!(e.kind, EventKind::MsgTerminated)));
    // Events are in non-decreasing audio order and within the clip.
    let end = events.last().unwrap().audio_position_ms;
    assert!(events.iter().all(|e| e.audio_position_ms <= end));
}
