// tests/word_events.rs
//
// Word/sentence boundary events (`EspeakNg::synth_with_events`): each spoken
// word gets a `Word` event carrying its start time in the audio and a
// best-effort source `text_position`, bracketed by `Sentence` and
// `End`/`MsgTerminated`.  Skips when the local espeak-ng data is absent.

use std::path::Path;

use espeak_ng::{EspeakNg, EventKind};

fn engine(voice: &str) -> Option<EspeakNg> {
    let dir = Path::new("espeak-ng-data");
    if !dir.join("phontab").exists() {
        eprintln!("[SKIP] no local espeak-ng-data");
        return None;
    }
    EspeakNg::builder().voice(voice).data_dir(dir).build().ok()
}

#[test]
fn two_words_get_ordered_word_events() {
    let Some(e) = engine("en") else { return };
    let (pcm, rate, events) = e.synth_with_events("hello world").expect("synth");
    assert_eq!(rate, 22050);

    let words: Vec<_> = events
        .iter()
        .filter(|ev| matches!(ev.kind, EventKind::Word(_)))
        .collect();
    assert_eq!(words.len(), 2, "expected one Word event per spoken word: {events:?}");

    // Word indices are 0,1 and source positions land on "hello"@0, "world"@6.
    assert_eq!(words[0].kind, EventKind::Word(0));
    assert_eq!(words[1].kind, EventKind::Word(1));
    assert_eq!(words[0].text_position, 0);
    assert_eq!(words[1].text_position, 6);

    // Audio times: word 0 at/near the start, word 1 strictly later but before
    // the audio ends.
    assert!(words[0].audio_position_ms <= words[1].audio_position_ms);
    assert!(words[1].audio_position_ms > 0, "second word must start after the first");
    let total_ms = pcm.len() as u32 * 1000 / rate;
    assert!(words[1].audio_position_ms < total_ms, "word start must be within the audio");

    // Bracketed by Sentence … MsgTerminated.
    assert_eq!(events.first().map(|e| &e.kind), Some(&EventKind::Sentence));
    assert!(events.iter().any(|e| e.kind == EventKind::MsgTerminated));
    assert!(events.iter().any(|e| e.kind == EventKind::End));
}

#[test]
fn single_word_gets_one_word_event() {
    let Some(e) = engine("en") else { return };
    let (_pcm, _rate, events) = e.synth_with_events("hello").expect("synth");
    let words = events.iter().filter(|ev| matches!(ev.kind, EventKind::Word(_))).count();
    assert_eq!(words, 1, "single word → one Word event: {events:?}");
    assert_eq!(
        events.iter().find(|e| matches!(e.kind, EventKind::Word(_))).unwrap().text_position,
        0
    );
}

#[test]
fn multi_clause_emits_multiple_sentence_events() {
    let Some(e) = engine("en") else { return };
    let (_pcm, _rate, events) = e.synth_with_events("Hello there. How are you?").expect("synth");

    let sentences: Vec<_> = events.iter().filter(|ev| ev.kind == EventKind::Sentence).collect();
    assert!(sentences.len() >= 2, "two clauses → ≥2 Sentence events: {events:?}");
    assert_eq!(sentences[0].audio_position_ms, 0, "first sentence at the start");
    assert!(sentences[1].audio_position_ms > 0, "second sentence starts later");

    // The full stream (minus terminal markers) stays in audio order, with each
    // Sentence marker landing at or before its clause's first Word.
    let times: Vec<u32> = events
        .iter()
        .filter(|e| !matches!(e.kind, EventKind::End | EventKind::MsgTerminated))
        .map(|e| e.audio_position_ms)
        .collect();
    assert!(times.windows(2).all(|w| w[0] <= w[1]), "events out of audio order: {times:?}");
}

#[test]
fn word_audio_positions_are_monotonic() {
    let Some(e) = engine("en") else { return };
    let (_pcm, _rate, events) = e.synth_with_events("one two three four").expect("synth");
    let times: Vec<u32> = events
        .iter()
        .filter(|ev| matches!(ev.kind, EventKind::Word(_)))
        .map(|ev| ev.audio_position_ms)
        .collect();
    assert_eq!(times.len(), 4, "four words expected: {events:?}");
    assert!(times.windows(2).all(|w| w[0] <= w[1]), "word times must not go backwards: {times:?}");
}
