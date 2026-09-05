// tests/streaming_synth.rs
//
// Incremental (clause-by-clause) synthesis: `EspeakNg::synth_streaming` and
// `synth_streaming_with_events`.  The contract is that streaming changes *when*
// audio arrives, never *what* it is — so every test here compares the
// concatenated chunks against the buffered `synth()`.

use std::path::Path;

use espeak_ng::{EspeakNg, EventKind};

fn engine() -> EspeakNg {
    EspeakNg::with_data_dir("en", Path::new("espeak-ng-data")).expect("en engine")
}

const MULTI: &str = "One. Two. Three. Four.";

#[test]
fn streamed_audio_matches_buffered_synth() {
    let e = engine();
    let mut streamed = Vec::new();
    let mut chunks = 0;
    let mut finals = 0;
    let rate = e
        .synth_streaming(MULTI, |samples, is_final| {
            chunks += 1;
            if is_final {
                finals += 1;
            }
            streamed.extend_from_slice(samples);
            false
        })
        .expect("stream");

    assert_eq!(rate, e.sample_rate());
    assert_eq!(finals, 1, "exactly one chunk is flagged final");
    assert!(chunks > 1, "a four-clause utterance should arrive in several chunks");

    let (buffered, _) = e.synth(MULTI).expect("synth");
    assert_eq!(streamed, buffered, "streaming must not change the audio");
}

/// A single clause has no internal boundary, so it arrives as one final chunk —
/// and still matches the buffered render.
#[test]
fn single_clause_streams_as_one_chunk() {
    let e = engine();
    let mut streamed = Vec::new();
    let mut chunks = 0;
    e.synth_streaming("hello world", |samples, is_final| {
        chunks += 1;
        assert!(is_final, "a single clause should produce only a final chunk");
        streamed.extend_from_slice(samples);
        false
    })
    .expect("stream");
    assert_eq!(chunks, 1);
    assert_eq!(streamed, e.synth("hello world").expect("synth").0);
}

/// Empty input still calls back exactly once, so a caller can rely on seeing a
/// final chunk for every utterance.
#[test]
fn empty_text_still_signals_completion() {
    let e = engine();
    let mut calls = 0;
    let mut samples_seen = 0;
    e.synth_streaming("", |samples, is_final| {
        calls += 1;
        samples_seen += samples.len();
        assert!(is_final);
        false
    })
    .expect("stream");
    assert_eq!(calls, 1);
    assert_eq!(samples_seen, 0);
}

/// Returning `true` stops synthesis: the remaining clauses are never rendered.
#[test]
fn callback_can_stop_early() {
    let e = engine();
    let mut chunks = 0;
    let mut streamed = 0;
    e.synth_streaming(MULTI, |samples, _is_final| {
        chunks += 1;
        streamed += samples.len();
        true // stop after the first clause
    })
    .expect("stream");

    assert_eq!(chunks, 1, "stopping should end the callbacks");
    let (buffered, _) = e.synth(MULTI).expect("synth");
    assert!(streamed < buffered.len(), "stopping should skip the rest of the utterance");
}

/// The events delivered on the final streaming chunk match `synth_with_events`.
#[test]
fn streaming_events_match_the_buffered_path() {
    let e = engine();
    let mut streamed = Vec::new();
    let mut final_events = Vec::new();
    e.synth_streaming_with_events(MULTI, |samples, is_final, events| {
        streamed.extend_from_slice(samples);
        if is_final {
            final_events = events.to_vec();
        } else {
            assert!(events.is_empty(), "events only arrive on the final chunk");
        }
        false
    })
    .expect("stream");

    let (buffered, _, events) = e.synth_with_events(MULTI).expect("synth_with_events");
    assert_eq!(streamed, buffered);
    assert_eq!(final_events.len(), events.len(), "same number of events");
    for (a, b) in final_events.iter().zip(&events) {
        assert_eq!(a.kind, b.kind);
        assert_eq!(a.text_position, b.text_position);
        assert_eq!(a.audio_position_ms, b.audio_position_ms);
    }
    // One `Sentence` event opens the utterance and one opens each clause after
    // a boundary — the trailing `.` closes a fifth, empty clause.
    let sentences = final_events.iter().filter(|e| e.kind == EventKind::Sentence).count();
    assert!(sentences >= 4, "one Sentence event per clause: {final_events:?}");
}

/// SSML markup splices audio out of order, so it can't be streamed clause by
/// clause — the API falls back to one final chunk rather than producing wrong
/// audio.
#[test]
fn markup_falls_back_to_a_single_chunk() {
    let mut e = engine();
    e.set_markup(true);
    let text = "<speak>One. Two.</speak>";
    let mut chunks = 0;
    let mut streamed = Vec::new();
    e.synth_streaming(text, |samples, is_final| {
        chunks += 1;
        assert!(is_final);
        streamed.extend_from_slice(samples);
        false
    })
    .expect("stream");
    assert_eq!(chunks, 1);
    assert_eq!(streamed, e.synth(text).expect("synth").0);
}

#[cfg(feature = "parallel")]
#[test]
fn parallel_batch_matches_sequential() {
    let e = engine();
    let texts = ["one", "two three", "four five six", "", "seven"];
    let parallel = e.synth_many(&texts);
    assert_eq!(parallel.len(), texts.len(), "results are returned per input");
    for (i, t) in texts.iter().enumerate() {
        let (pcm, rate) = parallel[i].as_ref().expect("synth_many item");
        let (expected, expected_rate) = e.synth(t).expect("sequential synth");
        assert_eq!(*rate, expected_rate);
        assert_eq!(*pcm, expected, "parallel result differs for {t:?}");
    }
}
