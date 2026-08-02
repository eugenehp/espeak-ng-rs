// tests/embedded_commands.rs
//
// Inline embedded commands (`\x01[±]<value><letter>`, C `CTRL_EMBEDDED`).  They
// used to garble the spoken text (`\x01+10P hello` → "ten pee hello"); now they
// are parsed and stripped, and the pitch/speed/volume levels are applied.

use espeak_ng::{translate::parse_embedded_commands, EspeakNg, Translator};

#[test]
fn commands_are_parsed_and_stripped() {
    let (stripped, cmds) = parse_embedded_commands("\u{01}+10P go \u{01}-5S \u{01}50A");
    assert_eq!(stripped, " go  ");
    assert_eq!(cmds.len(), 3);
    assert_eq!((cmds[0].letter, cmds[0].value, cmds[0].relative), ('P', 10, 1));
    assert_eq!((cmds[1].letter, cmds[1].value, cmds[1].relative), ('S', 5, -1));
    assert_eq!((cmds[2].letter, cmds[2].value, cmds[2].relative), ('A', 50, 0));
    // A stray \x01 at end, and an unknown command letter, are consumed/dropped.
    assert_eq!(parse_embedded_commands("hi\u{01}").0, "hi");
    assert_eq!(parse_embedded_commands("a\u{01}9zb").0, "ab"); // \x01 9 z (invalid) dropped
    assert!(parse_embedded_commands("plain text").1.is_empty());
}

#[test]
fn commands_no_longer_garble_the_text() {
    let Some(t) = Translator::new("en", None).ok() else { eprintln!("[SKIP]"); return };
    // The command must be consumed, not read as "ten pee".
    let ipa = t.text_to_ipa("\u{01}10P hello").unwrap();
    assert_eq!(ipa, t.text_to_ipa("hello").unwrap(), "command leaked into phonemes: {ipa}");
    assert!(!ipa.contains("tˈɛn"), "'10' was spoken: {ipa}");
}

#[test]
fn speed_command_changes_duration() {
    let Ok(e) = EspeakNg::new("en") else { eprintln!("[SKIP]"); return };
    let rate = e.sample_rate().max(1) as f32;
    let dur = |text: &str| e.synth(text).unwrap().0.len() as f32 / rate;
    let plain = dur("hello world how are you");
    let slow = dur("\u{01}90S hello world how are you");   // 90 wpm
    let fast = dur("\u{01}400S hello world how are you");  // 400 wpm
    assert!(slow > plain * 1.1, "\\x01 90S should slow down: {slow} vs {plain}");
    assert!(fast < plain * 0.9, "\\x01 400S should speed up: {fast} vs {plain}");
}
