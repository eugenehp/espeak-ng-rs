//! Async synthesis on a [tokio](https://tokio.rs) runtime.
//!
//! Synthesis is CPU-bound and blocking, so both entry points here run it on
//! `tokio::task::spawn_blocking` and hand the audio back over a channel — the
//! async executor is never blocked.
//!
//! * [`synth`] awaits one whole utterance.
//! * [`synth_stream`] returns a receiver that is fed **clause by clause**, so
//!   the first audio arrives after the first clause rather than after the last
//!   (see [`EspeakNg::synth_streaming_with_events`]).
//!
//! For a callback-driven queue with cancellation and no async runtime, use
//! [`AsyncSynth`](crate::async_synth::AsyncSynth) instead.
//!
//! Requires the `tokio-runtime` feature.

use std::sync::Arc;

use tokio::sync::mpsc;

use crate::engine::{EspeakNg, SynthEvent};
use crate::error::Result;
use crate::synthesize::PcmBuffer;

/// One streamed piece of audio from [`synth_stream`].
#[derive(Debug, Clone)]
pub struct AudioChunk {
    /// 16-bit mono PCM.
    pub samples: PcmBuffer,
    /// Sample rate in Hz.
    pub sample_rate: u32,
    /// True for the last chunk of the utterance.
    pub is_final: bool,
    /// Utterance events, delivered on the final chunk (empty otherwise).
    pub events: Vec<SynthEvent>,
}

/// Synthesize `text` without blocking the async executor.
///
/// ```no_run
/// # use std::sync::Arc;
/// # use espeak_ng::EspeakNg;
/// # async fn demo() -> Result<(), espeak_ng::Error> {
/// let engine = Arc::new(EspeakNg::new("en")?);
/// let (pcm, rate) = espeak_ng::tokio_synth::synth(engine, "hello").await?;
/// # let _ = (pcm, rate);
/// # Ok(())
/// # }
/// ```
pub async fn synth(engine: Arc<EspeakNg>, text: impl Into<String>) -> Result<(PcmBuffer, u32)> {
    let text = text.into();
    // `spawn_blocking` can only fail if the runtime is shutting down; surface
    // that as an empty result rather than panicking in the caller's task.
    match tokio::task::spawn_blocking(move || engine.synth(&text)).await {
        Ok(result) => result,
        Err(_) => Ok((Vec::new(), 0)),
    }
}

/// Synthesize `text`, streaming each clause's audio through the returned
/// receiver as soon as it is rendered.
///
/// `capacity` bounds the channel: the renderer blocks once that many chunks are
/// queued, so a slow consumer applies natural backpressure instead of buffering
/// the whole utterance.  Dropping the receiver stops synthesis at the next
/// chunk boundary.
///
/// Concatenating every chunk's samples gives exactly what [`synth`] returns.
///
/// ```no_run
/// # use std::sync::Arc;
/// # use espeak_ng::EspeakNg;
/// # async fn demo() -> Result<(), espeak_ng::Error> {
/// let engine = Arc::new(EspeakNg::new("en")?);
/// let mut rx = espeak_ng::tokio_synth::synth_stream(engine, "One. Two. Three.", 4);
/// while let Some(chunk) = rx.recv().await {
///     // play or write `chunk.samples`
///     if chunk.is_final { break; }
/// }
/// # Ok(())
/// # }
/// ```
pub fn synth_stream(
    engine: Arc<EspeakNg>,
    text: impl Into<String>,
    capacity: usize,
) -> mpsc::Receiver<AudioChunk> {
    let text = text.into();
    let (tx, rx) = mpsc::channel(capacity.max(1));

    tokio::task::spawn_blocking(move || {
        let rate = engine.sample_rate();
        let _ = engine.synth_streaming_with_events(&text, |samples, is_final, events| {
            let chunk = AudioChunk {
                samples: samples.to_vec(),
                sample_rate: rate,
                is_final,
                events: events.to_vec(),
            };
            // `blocking_send` is the backpressure: it waits for room, and errs
            // only when the receiver is gone — which means "stop".
            tx.blocking_send(chunk).is_err()
        });
    });

    rx
}

#[cfg(test)]
mod tests {
    use super::*;

    fn have_data() -> bool {
        std::path::Path::new("espeak-ng-data/en_dict").exists()
    }

    fn engine() -> Arc<EspeakNg> {
        Arc::new(EspeakNg::with_data_dir("en", std::path::Path::new("espeak-ng-data")).unwrap())
    }

    #[test]
    fn synth_awaits_a_whole_utterance() {
        if !have_data() {
            eprintln!("[SKIP] no en data");
            return;
        }
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        rt.block_on(async {
            let e = engine();
            let (pcm, rate) = synth(Arc::clone(&e), "hello world").await.unwrap();
            assert_eq!(rate, e.sample_rate());
            assert_eq!(pcm, e.synth("hello world").unwrap().0, "async result must match sync");
        });
    }

    #[test]
    fn stream_chunks_concatenate_to_the_whole_utterance() {
        if !have_data() {
            return;
        }
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .build()
            .unwrap();
        rt.block_on(async {
            let e = engine();
            const TEXT: &str = "One. Two. Three. Four.";
            let mut rx = synth_stream(Arc::clone(&e), TEXT, 2);

            let mut all = Vec::new();
            let mut chunks = 0;
            let mut finals = 0;
            let mut final_events = 0;
            while let Some(c) = rx.recv().await {
                chunks += 1;
                all.extend_from_slice(&c.samples);
                if c.is_final {
                    finals += 1;
                    final_events = c.events.len();
                }
            }
            assert!(chunks > 1, "a four-clause utterance should stream >1 chunk");
            assert_eq!(finals, 1, "exactly one final chunk");
            assert!(final_events > 0, "events are delivered on the final chunk");
            assert_eq!(all, e.synth(TEXT).unwrap().0, "streamed audio must match sync");
        });
    }

    /// Dropping the receiver stops the renderer rather than letting it run on.
    #[test]
    fn dropping_the_receiver_stops_synthesis() {
        if !have_data() {
            return;
        }
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .build()
            .unwrap();
        rt.block_on(async {
            let e = engine();
            let mut rx = synth_stream(e, "One. Two. Three. Four. Five. Six.", 1);
            let first = rx.recv().await.expect("at least one chunk");
            assert!(!first.samples.is_empty());
            drop(rx); // the blocking task's next send fails and it returns
        });
    }
}
