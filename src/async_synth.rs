//! Asynchronous, **streaming** synthesis runtime.
//!
//! Port of eSpeak NG's `fifo.c` / `event.c` / `espeak_command.c`: a background
//! worker processes an ordered, **cancellable command queue** (speak /
//! set-parameter / set-voice) and delivers each utterance's audio to a callback
//! **in chunks** — mirroring `espeak_SetSynthCallback` (`wav`, `numsamples`,
//! return non-zero to stop).  The caller is never blocked during synthesis and
//! can stop from the callback or via [`cancel`](AsyncSynth::cancel) (#172).
//!
//! Cancellation is checked **between chunks**, so it takes effect part-way
//! through a long utterance's delivery (the underlying render is still whole-
//! utterance internally — true incremental *rendering* is future work).

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};

use crate::engine::{EspeakNg, Parameter, SynthEvent};
use crate::error::Result;

/// Number of samples delivered per callback chunk.
const CHUNK_SAMPLES: usize = 4096;

/// A queued command processed in order by the worker (mirrors the
/// `espeak_command.c` command set).
enum Command {
    Speak(String),
    SetParameter(Parameter, i32),
    SetVoice(String),
}

/// One streamed audio chunk handed to the [`AsyncSynth`] callback.  Return
/// `true` from the callback to stop synthesis (drops the rest of this utterance
/// and the queue), like eSpeak's non-zero callback return.
pub struct AsyncChunk<'a> {
    /// The text this audio belongs to.
    pub text: &'a str,
    /// 16-bit mono PCM samples for this chunk.
    pub samples: &'a [i16],
    /// Sample rate in Hz (22 050).
    pub sample_rate: u32,
    /// True for the last chunk of the utterance.
    pub is_final: bool,
    /// Utterance events, delivered on the final chunk (empty otherwise).
    pub events: &'a [SynthEvent],
}

#[derive(Default)]
struct State {
    queue: VecDeque<Command>,
    busy: bool,
    cancel: bool,
    stop: bool,
}

struct Shared {
    state: Mutex<State>,
    work_available: Condvar,
    became_idle: Condvar,
}

/// A background streaming-synthesis worker with a cancellable command queue.
/// Dropping it stops the worker and joins its thread.
pub struct AsyncSynth {
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
}

impl AsyncSynth {
    /// Start a worker for `lang`, streaming each utterance's audio to `callback`
    /// (invoked on the worker thread; return `true` to stop).
    pub fn new<F>(lang: &str, data_dir: Option<PathBuf>, callback: F) -> Result<Self>
    where
        F: FnMut(AsyncChunk) -> bool + Send + 'static,
    {
        let engine = match data_dir {
            Some(dir) => EspeakNg::with_data_dir(lang, &dir)?,
            None => EspeakNg::new(lang)?,
        };
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            work_available: Condvar::new(),
            became_idle: Condvar::new(),
        });
        let worker_shared = Arc::clone(&shared);
        let mut callback = callback;
        let worker = thread::spawn(move || worker_loop(engine, worker_shared, &mut callback));
        Ok(AsyncSynth { shared, worker: Some(worker) })
    }

    fn enqueue(&self, cmd: Command) {
        self.shared.state.lock().unwrap().queue.push_back(cmd);
        self.shared.work_available.notify_one();
    }

    /// Enqueue text for synthesis; returns immediately.
    pub fn speak(&self, text: impl Into<String>) {
        self.enqueue(Command::Speak(text.into()));
    }

    /// Enqueue a parameter change, applied to all *subsequent* queued utterances.
    pub fn set_parameter(&self, param: Parameter, value: i32) {
        self.enqueue(Command::SetParameter(param, value));
    }

    /// Enqueue a voice change, applied to all *subsequent* queued utterances.
    pub fn set_voice(&self, name: impl Into<String>) {
        self.enqueue(Command::SetVoice(name.into()));
    }

    /// Clear the queue and stop the in-flight utterance's delivery (`espeak_Cancel`).
    pub fn cancel(&self) {
        let mut st = self.shared.state.lock().unwrap();
        st.queue.clear();
        st.cancel = true;
        drop(st);
        self.shared.became_idle.notify_all();
    }

    /// Block until the queue drains and the worker is idle (`espeak_Synchronize`).
    pub fn synchronize(&self) {
        let mut st = self.shared.state.lock().unwrap();
        while !st.queue.is_empty() || st.busy {
            st = self.shared.became_idle.wait(st).unwrap();
        }
    }

    /// True if the worker is busy or has queued work.
    pub fn is_speaking(&self) -> bool {
        let st = self.shared.state.lock().unwrap();
        !st.queue.is_empty() || st.busy
    }
}

impl Drop for AsyncSynth {
    fn drop(&mut self) {
        {
            let mut st = self.shared.state.lock().unwrap();
            st.stop = true;
            st.queue.clear();
        }
        self.shared.work_available.notify_all();
        if let Some(h) = self.worker.take() {
            let _ = h.join();
        }
    }
}

fn worker_loop(
    mut engine: EspeakNg,
    shared: Arc<Shared>,
    callback: &mut dyn FnMut(AsyncChunk) -> bool,
) {
    loop {
        let cmd = {
            let mut st = shared.state.lock().unwrap();
            while st.queue.is_empty() && !st.stop {
                shared.became_idle.notify_all();
                st = shared.work_available.wait(st).unwrap();
            }
            if st.stop {
                return;
            }
            st.cancel = false;
            st.busy = true;
            st.queue.pop_front().unwrap()
        };

        match cmd {
            Command::SetParameter(p, v) => engine.set_parameter(p, v),
            Command::SetVoice(name) => engine.set_voice(&name),
            Command::Speak(text) => {
                if let Ok((samples, sample_rate, events)) = engine.synth_with_events(&text) {
                    stream_utterance(&shared, callback, &text, &samples, sample_rate, &events);
                }
            }
        }

        let mut st = shared.state.lock().unwrap();
        st.busy = false;
        if st.queue.is_empty() {
            shared.became_idle.notify_all();
        }
    }
}

/// Deliver one utterance's PCM in [`CHUNK_SAMPLES`]-sized chunks, honouring
/// cancellation (checked between chunks) and callback-driven stop.
fn stream_utterance(
    shared: &Shared,
    callback: &mut dyn FnMut(AsyncChunk) -> bool,
    text: &str,
    samples: &[i16],
    sample_rate: u32,
    events: &[SynthEvent],
) {
    let n = samples.len();
    let mut off = 0;
    // An empty utterance still signals completion with one final empty chunk.
    if n == 0 {
        callback(AsyncChunk { text, samples: &[], sample_rate, is_final: true, events });
        return;
    }
    while off < n {
        if shared.state.lock().unwrap().cancel {
            break;
        }
        let end = (off + CHUNK_SAMPLES).min(n);
        let is_final = end == n;
        let stop = callback(AsyncChunk {
            text,
            samples: &samples[off..end],
            sample_rate,
            is_final,
            events: if is_final { events } else { &[] },
        });
        off = end;
        if stop {
            // Callback asked to stop: drop the rest of the queue too.
            shared.state.lock().unwrap().queue.clear();
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn have_data() -> bool {
        std::path::Path::new("espeak-ng-data/en_dict").exists()
    }

    #[test]
    fn streams_utterances_in_chunks() {
        if !have_data() {
            eprintln!("[SKIP] no en data");
            return;
        }
        let chunks = Arc::new(AtomicUsize::new(0));
        let finals = Arc::new(AtomicUsize::new(0));
        let total = Arc::new(AtomicUsize::new(0));
        let (c, f, t) = (Arc::clone(&chunks), Arc::clone(&finals), Arc::clone(&total));
        let synth = AsyncSynth::new("en", None, move |chunk| {
            c.fetch_add(1, Ordering::SeqCst);
            t.fetch_add(chunk.samples.len(), Ordering::SeqCst);
            if chunk.is_final {
                f.fetch_add(1, Ordering::SeqCst);
            }
            false
        })
        .unwrap();

        synth.speak("hello world how are you today");
        synth.speak("goodbye");
        synth.synchronize();

        // Two utterances → two "final" chunks; the long one spans several chunks.
        assert_eq!(finals.load(Ordering::SeqCst), 2, "expected 2 final chunks");
        assert!(chunks.load(Ordering::SeqCst) > 2, "long utterance should stream >1 chunk");
        // Concatenated chunks equal a direct synth of both.
        let direct: usize = {
            let e = EspeakNg::new("en").unwrap();
            e.synth("hello world how are you today").unwrap().0.len()
                + e.synth("goodbye").unwrap().0.len()
        };
        assert_eq!(total.load(Ordering::SeqCst), direct, "streamed samples must match direct synth");
    }

    #[test]
    fn callback_stop_drops_remaining() {
        if !have_data() {
            return;
        }
        let count = Arc::new(AtomicUsize::new(0));
        let c = Arc::clone(&count);
        let synth = AsyncSynth::new("en", None, move |_chunk| {
            c.fetch_add(1, Ordering::SeqCst);
            true // stop after the very first chunk
        })
        .unwrap();
        for _ in 0..10 {
            synth.speak("the quick brown fox");
        }
        synth.synchronize();
        // Returning `true` on the first chunk drops the rest of the queue.
        assert!(count.load(Ordering::SeqCst) < 10, "callback stop did not drop the queue");
    }

    #[test]
    fn set_parameter_applies_to_later_utterances() {
        if !have_data() {
            return;
        }
        // Per-utterance sample totals, accumulated across chunks.
        let per_utt = Arc::new(Mutex::new(Vec::<usize>::new()));
        let running = Arc::new(AtomicUsize::new(0));
        let (p, r) = (Arc::clone(&per_utt), Arc::clone(&running));
        let synth = AsyncSynth::new("en", None, move |chunk| {
            r.fetch_add(chunk.samples.len(), Ordering::SeqCst);
            if chunk.is_final {
                p.lock().unwrap().push(r.swap(0, Ordering::SeqCst));
            }
            false
        })
        .unwrap();

        synth.speak("hello world"); // default rate
        synth.set_parameter(Parameter::Rate, 90); // slow, applies to the next
        synth.speak("hello world"); // slow rate → longer
        synth.synchronize();

        let utts = per_utt.lock().unwrap();
        assert_eq!(utts.len(), 2, "expected two utterances: {utts:?}");
        assert!(utts[1] > utts[0], "queued SetParameter(slow) did not lengthen later utterance: {utts:?}");
    }
}
