// tests/async_cancel.rs
//
// `AsyncSynth::cancel` semantics — the port's analogue of upstream #2288, which
// fixes `espeak_ng_Cancel` so nothing already buffered is still delivered after
// the call returns.  There is no audio device here, so the contract is about the
// callback: once `cancel()` returns, no further chunk for a cancelled utterance
// may arrive, and `synchronize()` must not block.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use espeak_ng::async_synth::AsyncSynth;

fn have_data() -> bool {
    std::path::Path::new("espeak-ng-data/en_dict").exists()
}

#[test]
fn cancel_stops_delivery_and_drains_the_queue() {
    if !have_data() {
        eprintln!("[SKIP] no en data");
        return;
    }
    let after_cancel = Arc::new(AtomicUsize::new(0));
    let cancelled = Arc::new(AtomicBool::new(false));
    let chunks = Arc::new(AtomicUsize::new(0));
    let (a, c, n) = (Arc::clone(&after_cancel), Arc::clone(&cancelled), Arc::clone(&chunks));

    let synth = AsyncSynth::new("en", None, move |_chunk| {
        n.fetch_add(1, Ordering::SeqCst);
        if c.load(Ordering::SeqCst) {
            a.fetch_add(1, Ordering::SeqCst);
        }
        false
    })
    .expect("engine");

    for _ in 0..20 {
        synth.speak("the quick brown fox jumps over the lazy dog");
    }
    // Let the worker get going, then cancel.
    while chunks.load(Ordering::SeqCst) == 0 {
        std::hint::spin_loop();
    }
    // Mark *after* `cancel()` returns: the contract is about what happens from
    // then on, and a chunk already inside the callback when cancel is called is
    // not a leak.
    synth.cancel();
    cancelled.store(true, Ordering::SeqCst);

    let start = Instant::now();
    synth.synchronize();
    assert!(
        start.elapsed() < Duration::from_secs(5),
        "synchronize() should return promptly after cancel"
    );
    assert!(!synth.is_speaking(), "the queue must be empty after cancel");

    // Nothing may be delivered once `cancel()` has returned.
    let leaked = after_cancel.load(Ordering::SeqCst);
    assert_eq!(leaked, 0, "{leaked} chunks were delivered after cancel returned");
    // …and the other nineteen utterances never ran.
    assert!(
        chunks.load(Ordering::SeqCst) < 20,
        "the queued utterances should have been dropped"
    );
}

/// Cancelling an idle worker is a no-op, and the next utterance still speaks.
#[test]
fn cancel_when_idle_does_not_break_the_next_utterance() {
    if !have_data() {
        return;
    }
    let finals = Arc::new(AtomicUsize::new(0));
    let f = Arc::clone(&finals);
    let synth = AsyncSynth::new("en", None, move |chunk| {
        if chunk.is_final {
            f.fetch_add(1, Ordering::SeqCst);
        }
        false
    })
    .expect("engine");

    synth.cancel();
    synth.speak("hello");
    synth.synchronize();
    assert_eq!(finals.load(Ordering::SeqCst), 1, "the utterance after cancel should speak");
}
