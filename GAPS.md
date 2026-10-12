# Gaps vs. upstream eSpeak NG

Tracking document comparing this pure-Rust port against upstream
[`espeak-ng/espeak-ng`](https://github.com/espeak-ng/espeak-ng)
(`src/libespeak-ng/`, 35 `.c` modules).

**Legend:** ❌ not started · 🟡 partial · ✅ done (listed for context)
**Oracle:** a locally built upstream `espeak-ng` is the reference for every probe in §3c (`git clone --depth 1 … && cmake -B build -DCMAKE_BUILD_TYPE=Release -DUSE_MBROLA=OFF -DUSE_LIBSONIC=OFF -DUSE_LIBPCAUDIO=OFF && cmake --build build`; run it as `ESPEAK_DATA_PATH=<build> <build>/src/espeak-ng -q -x -v <lang>`). The repo's own harness takes the same binary through `ESPEAK_NG_BIN` (`cargo test --test oracle_comparison`).
**Snapshot:** 2026-07-31, upstream `84a37de` (emoji reading, #2463) · **current re-check:** 2026-10-11, upstream `ba90c8e`.

**Upstream re-check (4, 2026-10-11):** master HEAD is now `ba90c8e` (2026-09-22) — **67 commits ahead of the `84a37de` snapshot**, 25 of them since re-check 3's `f135499`, and still no 1.53.0 release cut. Only **six** of the 67 touch `src/libespeak-ng/`; the rest are the Android app (12 commits), `dictsource` (4 PRs), docs and CI. **Nothing engine-side is outstanding from the newest commits:** #2470 (libsonic event rescale) landed here in the 2026-09-03 pass (`src/engine.rs:777-795`) and #2478 (`common.h` `extern "C"`) is C-only; upstream has been Android/CI-only since 2026-09-09. **The four merged data PRs are already bundled** — they merged 2026-08-18/19, before the 2026-09-03 data refresh — and this re-check verified that by probe, not by date: all 20 of upstream's new `test_phon ku` expectations match exactly (`mafan` → `maf'an`, `naxwaze` → `n'axwaz,E#`, `radibe` → `R'adIb,E#`, `tu` → `t'U`), the six *changed* `fa` readings match (`العهد` → `'ol?ahd`, `اتوکش` → `ot'ukeS`, `آنتروپیک` → `AntR'opik`), the new `hu` rules fire (`látászavara` → `l'a:ta:SzAvAR2A`) and `nl` `na`+CVC+`ige` takes `na:`. **Status corrections:** §3 **#38, #40 and #41 are no longer ❌** — all three landed in the 2026-09-03 passes and those row notes (including the `bytecode.rs:498-516` line numbers) were stale. What the re-check *did* find is four behavioural divergences, recorded as **§3c**. Upstream's own new tests were used as the oracle: `tests/language-pronunciation.test` (+40 `ku` lines and one `ar` case) and `tests/crash.test` (+15 vectors — all 14 new vector/voice pairs give sane output here, e.g. `-v de "ईघ"` → `(hi)'i:g#(de)`).

The port covers the **text → phonemes/IPA** path and a **working harmonic
synthesizer** that reads real `phondata`. The gaps are concentrated in
**prosody, alternate synthesizers, SSML, the compilers, and the runtime/audio
layer**, plus a long tail of per-language number/rule details.

---

## Verification tooling

- **Oracle sweep** (`scripts/oracle_sweep.py`) — runs upstream's own
  `tests/language-pronunciation.test` corpus plus a symbol/emoji/suffix set through both
  engines and reports how many cases are byte-identical, grouping the rest by the kind of
  difference. `record` twice, `compare` with `--baseline` to see what a change moved.
- **C oracle** (`c-oracle` / `bundled-espeak`) — bit-exact reference for `--ipa`/`-x` text output. Every ✅/🟡 text feature above was checked against it.
- **`wav-analysis` feature** (`src/analysis/mod.rs`, dev-only, no extra deps) — WAV I/O + duration, RMS, energy-envelope and YIN pitch-contour extraction, plus `compare()` for reference-vs-candidate similarity. Enables **quantitative** verification of the PCM-domain gaps (intonation, durations, sound icons) where bit-exact PCM won't match. Run `cargo run --example wav_compare --features wav-analysis -- ref.wav cand.wav`; unit + real-synth tests.
- **Spectral analysis** (`src/analysis/mod.rs` + `src/bin/spectral.rs`, `wav-analysis`, **pure-Rust radix-2 FFT, no deps**) — per-frame Hann-windowed magnitude spectrum with `spectral_centroid` (brightness), `spectral_flatness` (tonal↔noise), `spectral_rolloff`, `band_energy`, and `spectral_similarity()` (log-spectrum correlation). `compare()` now also reports `spectral_corr` + centroids. A **deterministic** timbre metric (no ASR noise) for diagnosing consonant spectral character. Run `cargo run --features wav-analysis --bin spectral -- <a.wav> [b.wav]`. Finding: for "so", Rust's /s/ region is **brighter** than the oracle (centroid 3145 vs 2254 Hz, more 1–3 kHz energy; overall spectral-shape corr 0.93) — the voiceless-fricative WAV spectral tilt is the next lever.
- **ASR intelligibility harness** (`scripts/asr_validate.py`) — synthesizes text with the Rust engine (and the C oracle), transcribes each clip with OpenAI Whisper, and reports **word accuracy (1 − WER)**. Catches whether the audio is *recognisable*, which acoustic metrics can't. Run `python3 scripts/asr_validate.py --rust ./target/release/espeak-ng-rs --oracle <bin> --data <dir> --model base`.

## 1. Whole subsystems not ported

| # | Upstream module | Size | Status | Impact / notes |
|---|---|---|---|---|
| 1 | `intonation.c` | 36 KB | ✅ | **`CalcPitches` ported (2026-09-03/04, `src/synthesize/intonation.rs` + `envelopes.rs`).** The 20 pitch envelopes (128 bytes each) were extracted programmatically from upstream's `intonation.c`; `TONE_HEAD_TABLE`/`TONE_NUCLEUS_TABLE`, the 68-byte `TUNE` records parsed from the `intonations` data file, `SetPitch`/`CountPitchVowels`/`CountIncrements`/`SetPitchGradient`/`SetHeadIntonation`/`CalcPitchSegment` and the tune- and table-driven `CalcPitches` paths are all ported, along with `PUNCT_TO_TONE` clause-type selection and Mandarin tone sandhi (`CalcPitches_Tone`). Wired into the render loop: one `Syllable` per vowel carrying its stress, then per-syllable pitch from the tune; consonants carry the neighbouring syllable's pitch as C does. `PITCH_ADJUST_TAB` + `base_pitch_hz` fixed a systematic ~35 Hz offset (the port had been treating the voice's `pitch2` as base F0 rather than `(pitch1 - 9)`). **Oracle-verified:** mean F0 on "hello world" 108.6 Hz vs the C binary's 108.4. 6 unit tests. **Remaining:** none known; residual F0 differences on longer text are within the duration model's error. |
| 2 | `klatt.c` | 33 KB | ✅ | **Fully ported (2026-09-04, `src/synthesize/klatt.rs`).** `synthesize_frames_klatt` is now a direct port of upstream's `parwave()` + `Wavegen_Klatt()` + `SetSynth_Klatt()` + `frame_init()` + `pitch_synch_par_reset()`, replacing the earlier placeholder (a bare cascade driven by a phase counter). Ported in full: the **cascade** (nasal antiresonator `Rnz` via `setzeroabc` with C's `a == 0` guard, nasal pole `Rnpc`, then R8c…R1c) **and the parallel branch** (R1p/Rnpp excited by voicing, R2p…R6p by frication plus the first difference of voicing, summed with alternating signs, plus the bypass path); both **glottal source models** (`impulsive_source`'s doublet through `RGL`, and `natural_source`'s quadratic pulse with the 224-entry `B0` shape table) at 4× oversampling through `RLP`; spectral tilt (`TLTdb` → `decay`/`onemd`), open-phase breathiness (`Aturb`), glottal skew, the 88-entry `DBtoLIN` table, F0 `flutter`, and the fade-in/out ramps. `espeak_rand` is reproduced exactly, upstream's `- min` bias included, so noise can be compared sample-for-sample. **Oracle-verified:** a sustained vowel through `-v en+klatt` matches the C binary at spectral-envelope **correlation 0.997, RMS difference 2.4 dB**. 10 unit tests. **Routing corrected (2026-09-04):** `flush_run` used to send any run whose frames carried `FRFLAG_KLATT` through Klatt — but that flag only marks a frame as carrying the extra parallel-resonator parameters, and *every* shipped `en` frame has it, so the standard voices were being rendered by the wrong engine. C routes to Klatt only when the voice sets `klattv[0]`; the port now matches, worth **4.19 → 2.79 dB** mean spectral-envelope RMS difference over an eight-sentence sweep (§36). **Deliberate divergences** (documented on `synthesize_frames_klatt`): a frame without `FRFLAG_KLATT` falls back to the `KlattInit` defaults instead of C's silence (upstream zeroes every `klattp`, so `amp_voice = DBtoLIN(-7) = 0`), and the run is peak-normalized rather than scaled by `wdata->amplitude` because the caller runs its own AGC. |
| 3 | `ssml.c` (+ SSML parts of `readclause.c`) | 30 KB | 🟡 | **Markup mode (`-m`) implemented** in `src/translate/ssml.rs`: entity decoding (`&amp; &lt; &#NN; &#xHH;`), `<say-as characters>`, `<sub alias>`, `<break>`/`<p>`/`<s>` clause boundaries, comment/PI discard, and strip-but-keep-content for `<emphasis>`/`<prosody>`/`<voice>`/unknown tags. Wired to `Translator::text_to_ipa_ssml`, `TextToPhonemesOptions::markup`, CLI `-m`/`--markup`. 10 unit + 6 integration tests, verified vs C oracle. **`[[…]]` inline phonemes implemented** (`parse_inline_phonemes` in `src/translate/mod.rs`, always-on; greedy longest-mnemonic match; 6 tests, clean cases byte-identical to oracle). **`-x`/synthesis parity (2026-08-01):** `translate_to_codes` built the inline-phoneme entry but its code-emission `match` dropped it (bare `_ => {}`), so `-x "[[k]]"` was empty and `[[…]]` synthesised silently; now emitted like a word — `-x "[[h@l'oU]]"` → `h@l'oU` (exact round-trip), and audio of `[[…]]` renders. **`<voice xml:lang>` language switching implemented** (segment-based `process_markup` → per-language delegation; byte-identical to oracle for `hello <voice xml:lang="fr">bonjour</voice> bye`). **`<voice name>` resolution (2026-08-01):** the `<voice>` selector may be a human name (`name="French (France)"`) or code; `resolve_voice_lang` matches it via `find_voice` and falls back to the base voice if unmatched — previously an unresolvable selector *errored* ("dict not found"); now graceful. **`<phoneme ph="">` implemented** (`src/translate/ssml.rs`): the `ph` attribute (eSpeak mnemonics) is bridged to the always-on `[[…]]` inline-phoneme span and the element content is discarded — end-to-end byte-identical to `[[ph]]` via the IPA path (2 unit + 1 integration test). **`<say-as interpret-as="digits">` implemented (2026-08-01):** reads each digit ("42"→"four two", "007"→"zero zero seven") by routing to the spell mechanism; `cardinal`/`number` still read the whole number. 2 tests. **`<say-as interpret-as="ordinal">` implemented (2026-08-01):** via a new **per-span `Segment.interpret` (`ssml::SayAs`) attribute** threaded from the SSML walker through `text_to_ipa_with_options` — the walker flushes+tags the ordinal span, and the translate layer (which knows the language) appends the English ordinal suffix (`english_ordinal_suffix`: 1st/2nd/3rd/11th/21st) so the number path reads it as an ordinal ("3"→"third", "21"→"twenty first", "100"→"one hundredth"); stays in-clause. Non-English passes through (no regression). 2 tests. **This `interpret` hook is reusable for `date`/`time`.** **`telephone`/`tel` also implemented (2026-08-01):** routes to the spell path (digits individual, `+`→"plus", `-` silent) — `<say-as tel>+1-800-555</say-as>` → "plus one eight zero zero five five five". **`date`/`time` implemented (2026-08-01):** via the same `interpret` hook — `date` renders ISO `YYYY-MM-DD` → "Month Dayth Year" ("2024-01-15" → "January fifteenth …"); `time` renders `HH:MM` → hour+minute with "oh"/"o'clock" ("14:30"→"fourteen thirty", "9:05"→"nine oh five", "14:00"→"fourteen o'clock"). English best-effort (non-English passes through). The full say-as family (characters/glyphs/digits/telephone/ordinal/date/time/cardinal) is now covered. **Audio-path SSML fixed (2026-08-01):** the engine `synth` markup path only handled `<audio>` sound icons — **all other SSML tags were spoken literally** (e.g. `-m -w` of `<emphasis>hi</emphasis>` said "emphasis hi emphasis"). Now each text run goes through `crate::translate::ssml_to_speech_text` — strip + entity decode + `<say-as>` spelling **and** the language-aware interpret modes (ordinal/date/time) — so the audio path is at **parity with `--ipa`** for SSML (verified byte-identical: `-m` audio of `<say-as ordinal>3</say-as>` == plain "3rd", `<say-as date>2024-01-15</say-as>` == "January 15th 2024"). **`-x -m` fixed too (2026-08-01):** the `-x` mnemonic path fed *raw* text to `translate_to_codes`, so `-m` tags/entities were spoken literally ("emphasis hello slash emphasis"); now it runs `ssml_to_speech_text` first — `-x -m <emphasis>hello</emphasis>` == `-x hello`. **All three output surfaces (`--ipa`, `-x`, audio) now process SSML consistently** (voice-language *switching* still only on `--ipa`). **Document-level `<prosody rate/pitch/volume>` (2026-08-01):** when a single `<prosody>` wraps the whole utterance, its multipliers (`50%`/keywords/`+20%`) scale the global rate/pitch/volume for audio (`ssml::document_prosody` + CLI `apply_prosody`); verified rate 50%→longer / 200%→shorter WAV (== `-s 88`/`-s 350`); scoped/partial prosody is not applied (no regression). **Remaining:** *scoped* prosody (per-span acoustic params in the engine), `<phoneme alphabet="ipa">`, in-bracket stress-digit notation, cosmetic empty clause lines around standalone `<voice>`. |
| 4 | `synth_mbrola.c`, `mbrowrap.c`, `compilembrola.c` | ~36 KB | ✅ (bar the external binary) | **Ported (2026-09-04, `src/mbrola.rs`).** `MbrolaTable::compile` ports `compilembrola.c` (the `phsource/mbrola/<db>` text format: `<control> <phoneme> <next|NULL|VWL> <percent> <name1> [<name2>]`, `volume`, `//` comments, C's 39-byte line truncation); `parse`/`to_bytes` handle the binary `<db>_phtrans` (4-byte volume + 24-byte records + zero terminator); `mbr_name` ports `GetMbrName` with **all seven `control` bits** (skip-next, match-previous, word-initial, no-cross-word, name-prefix, stressed-only, word-final) and the `NULL`/`VWL`/`:`/`_` context sentinels; `write_pitch` ports `WritePitch` (envelope extremes → up to five `(position%, Hz)` targets, `env100 = 80`, the split-vowel ±`percent` cases) on top of the ported `SetPitch2`; `translate` ports `MbrolaTranslate`. **Oracle-checked on data:** all **49 shipped `_phtrans` tables round-trip byte-for-byte**, and the compiler reproduces the shipped `en1_phtrans` rows exactly. **Wired end to end:** `Synthesizer::mbrola_phonemes` builds the phoneme list (mnemonic, type, stress, word stress, duration, and the syllable's `env`/`pitch1`/`pitch2` from the ported intonation), `voices::load_mbrola_voice` reads `voices/mb/mb-<db>` (its `mbrola <db> <phtrans>` directive and the best-priority `language` line — `mb-en1` → `en`, `mb-de4` → `de`), `--pho` writes the script and `--compile-mbrola=<src>` compiles a table. A `-v mb-*` voice hands the `.pho` to the external `mbrola` binary and, because neither that binary nor the diphone databases are shipped, falls back to the built-in synthesizer on the voice's own language with a clear warning. 20 tests. **Only remaining gap: the diphone audio itself**, which needs the separately-licensed `mbrola` binary + databases — the C oracle can't produce `.pho` here either (`-v mb-en1 --pho` → "voice does not exist"), so the `.pho` text is verified structurally rather than byte-for-byte. |
| 5 | `compiledict.c` | 42 KB | 🟡 | **List-section compiler implemented (2026-08-01, `src/dictionary/compile.rs`):** `compile_dict(entries, transpose)` packs word → pronunciation exception entries into the binary `_dict` format (magic + rules_offset header, 1024 hash buckets with the exact entry encoding — transpose-compressed word, null-term phonemes, flag bytes). **Round-trip verified** against the reader (compile → `from_bytes` → `lookup` returns the same phonemes/flags). **Rules-section compiler started (2026-08-01):** `compile_rules(groups)` + `compile_dict_with_rules` emit the binary rule-group format (`RULE_GROUP_START` + name + `[<match letters> RULE_PHONEMES <phonemes> 0]…` + `RULE_GROUP_END`) — the inverse of `build_groups`/`match_rule`. **Round-trip verified:** compile a group → the reader registers `groups1['a']` at the right offset with correct rule bytes. 3 tests. **List DSL parser done (2026-08-01):** `parse_list_dsl(source, phdata)` reads `<lang>_list` source text (`word<ws>phonemes[<ws>flags]` per line, `//` comments) into `ListEntry`s, converting phoneme **mnemonics→codes** via the active phoneme table. **Verified end-to-end against real en data** (source → entries → `compile_dict` → `from_bytes` → `lookup` returns the codes) — the first compiler slice validated with *shipped* data, not just synthetic. **Rules DSL parser done (2026-08-01):** `parse_rules_dsl(source, phdata)` reads `<lang>_rules` source — `.group <name>` directives + `match<ws>phonemes` rules (mnemonics→codes, group letter stripped from the match) → `RuleGroup`s; verified with real en data (parse → `compile_rules` → reader registers the groups). So both source DSLs (`_list` + `_rules`) now parse to compiler input. **Top-level `compile_dictionary` driver done (2026-08-01):** reads `<dir>/<lang>_list`+`_rules` dictsource files → DSL parsers → `_dict` binary (the `compiledict.c` entry point) — the dict compiler is now **end-to-end**: dictsource → compile → read → lookup, and a compiled **no-context rule actually fires** through the real translate engine (`word_to_phonemes` emits the rule's phoneme). 7 tests. **Remaining:** rule **context/conditions/stress** (`(`/`)` pre/post-context, `RULE_CONDITION`, `$` flags); the real espeak-ng `dictsource/` files (not in this checkout — drop them in and `compile_dictionary` yields real `_dict`s); >255-byte entries. |
| 6 | `compiledata.c` | 72 KB | 🟡 | **phontab compiler implemented (2026-08-01, `phoneme/table.rs` `compile_phontab`):** builds the binary phoneme-table file (`[n_tables]` header + per-table 4-byte header + 32-byte name + 16-byte entries) — the inverse of `parse_phontab`. **Round-trip verified** (compile → parse → identical names/includes/mnemonics/codes + full 16-byte entries). **Phoneme-program bytecode compiler started (2026-08-01, `synthesize/bytecode.rs` `compile_program`/`ProgInstn`):** encodes a phoneme program to `phonindex` bytecode (`i_FMT`/`i_WAV` with the exact address+param bit-packing, `RETURN`, raw words) — the inverse of `InterpretPhoneme`/`scan_phoneme`. **Round-trip verified:** compile `[i_FMT(addr,param), RETURN]` → `scan_phoneme` reads back `fmt_addr`+`fmt_param` exactly (1 test). **Phondata SPECT_SEQ frame writer done (2026-08-01):** `SpectFrame::to_bytes_t2` + `compile_spect_seq(frames)` emit the `phondata` frame-sequence format (`length_total`/`n_frames`/`sqflags` header + 44-byte `frame_t2` entries) — inverse of `SpectSeq::parse`; **round-trip verified** (compile → parse → byte-identical frames). So the phoneme-data path now round-trips end-to-end: program bytecode (`compile_program`) → FMT frames (`compile_spect_seq`). Both frame types now round-trip: non-Klatt `frame_t2` (44 B) **and** Klatt `frame_t` (64 B). **WAV-sample writer done (2026-08-01):** `sample::compile_wav_sample` emits the consonant-WAV `phondata` format (header + 16-bit LE data); round-trip verified (structural + `parse_wav_sample` decodes the right count). So the acoustic-data compiler now covers **both** phoneme forms — FMT frame sequences and WAV samples. **Phoneme-source DSL parser done (2026-08-01, `phoneme/source.rs` `parse_phoneme_dsl`):** parses `phonemetable`/`phoneme … endphoneme` source (mnemonic + code, manner/place/voicing via the `PhonemeFeature`/`apply_feature` machinery, `pause`/`stress`/`vowel`/`liquid`, `length`/`lengthmod`) into `PhonemeTabList`s — the table-level front-end, symmetric with the dict rules DSL. **Round-trip verified** (source → `compile_phontab` → `parse_phontab`, now `pub`). Program-body lines (`FMT`/`WAV`/`IF`) pass through. **Remaining:** more program opcodes (conditions/jumps/`CALLPH`/IPA-name); a source parser for the program *body* (FMT/WAV → `phonindex`/`phondata`). |
| 7 | `fifo.c`, `event.c`, `espeak_command.c` | ~31 KB | 🟡 | **Async runtime implemented (2026-08-01, `src/async_synth.rs`):** `AsyncSynth` runs a background worker owning its own `EspeakNg`, with a **cancellable request queue** (`Mutex<VecDeque>` + `Condvar`) and a per-utterance callback (PCM + events). `speak` enqueues and returns immediately; `cancel` clears the queue and stops in-flight delivery (`espeak_Cancel`, #172); `synchronize` blocks until drained; `is_speaking`; `Drop` stops+joins. **Streaming + commands (2026-08-01):** audio is delivered to the callback **in `CHUNK_SAMPLES` chunks** (`AsyncChunk` with `is_final`/events; the callback returns `true` to stop, mirroring `espeak_SetSynthCallback`), and the queue is now a **command list** — `set_parameter`/`set_voice` apply to subsequent utterances (verified: a queued slow-rate command lengthens the next utterance). Cancellation is checked **between chunks**. 3 tests. **Word/sentence events implemented (2026-08-01):** `synth_with_events` emits `Sentence` (one per clause) + one `EventKind::Word(i)` per spoken word (start time in audio + best-effort source `text_position`) + `End`/`MsgTerminated`, via `Synthesizer::synthesize_codes_with_marks` recording the sample offset at each `WordBoundary`/`ClauseBoundary`. 4 integration tests. **Incremental rendering done (2026-09-04):** `Synthesizer::render` now takes an optional sink and flushes each **clause** as it finishes, exposed as `EspeakNg::synth_streaming` / `synth_streaming_with_events`; `AsyncSynth` uses the latter, so a queued utterance starts producing audio after its first clause rather than its last. The rendering *order* is unchanged, so concatenated chunks equal `synth()` byte-for-byte (asserted in `tests/streaming_synth.rs`); sound icons, SSML and the tempo/pitch post-pass fall back to a single final chunk, and a voice with `echo` is never split (the echo rings across boundaries). **Optional runtimes:** `parallel` (rayon) adds `EspeakNg::synth_many` for batch rendering; `tokio-runtime` adds `espeak_ng::tokio_synth` (`synth` awaits an utterance, `synth_stream` returns a bounded `mpsc::Receiver` fed clause by clause, with backpressure and drop-to-cancel), both off the executor via `spawn_blocking`. 13 tests. **Remaining:** exact word `text_position` through number/abbrev expansion (needs `sourceix` threading). |
| 8 | libsonic (`sonic`) | — | 🟡 | **Pitch-preserving tempo scaling implemented (2026-08-01, `synthesize::tempo::change_tempo`):** pitch-synchronous overlap-add (AMDF period detection + period merge/repeat with crossfade) time-scales PCM without shifting pitch — the `sonic` core. Verified on synthetic sines (2×→~0.5× length, pitch held; 0.5×→~2× length). **Pitch-shift added (2026-08-01, `change_pitch`):** duration-preserving pitch scaling (stretch + resample); 200 Hz ×1.5 → ~300 Hz same length, verified. **Engine hook added (2026-08-01):** `EspeakNg::with_post_tempo`/`with_post_pitch` post-process the final PCM (opt-in; `1.0` = byte-identical default). The base-F0 pitch / frame-duration rate remain the primary controls (better for normal voice); the sonic hook is for effect voices / independent post-scaling. |
| 9 | `soundicon.c` | 5 KB | ✅ (+track #2465) | **Implemented** (`src/soundicon/mod.rs`): both trigger paths. **SSML `<audio src="…">fallback</audio>`** (via `EspeakNg::set_markup(true)` / CLI `-m`) plays the referenced WAV, speaking the fallback text if it can't be loaded; **punctuation icons** register a char→WAV mapping (`EspeakNg::add_soundicon(ch, path)`, mirroring the `soundicon <char> <file>` voice directive). The engine splits text into speakable runs + icon events, synthesises runs normally and splices the icon PCM with a 10 ms lead-in (C `DoPause(10)`), amplitude-matched to C's `EMBED_I`/`PlayWave` (amp 21). The WAV loader is **more robust than C** (which requires exact mono/16-bit/rate and punts to a commented-out `sox` call): a dependency-free decoder handles PCM 8/16/24/32-bit + IEEE float, any channel count (down-mixed), and any rate (linearly resampled). Fast path unchanged when no icons/markup. 9 unit + 4 integration tests. **Note:** no soundicon *data* ships with espeak-ng — it is entirely opt-in (apps supply their own clips or use `<audio>`). **Upstream #2465 (open):** 3 ASan-confirmed OOB accesses loading `<audio src>` — `soundicon_tab` has no upper bound (`N_SOUNDICON_TAB` = 80). The port's loader must bound the icon table and reject malformed WAV without panicking; add a defensive test (fuzz an oversized `<audio>` set + a truncated/garbage WAV). |
| 10 | Audio backends (`pcaudiolib` / PortAudio / ALSA / Pulse) | — | ❌ | WAV via `-w` / `--stdout` only (documented). No device playback. |

## 2. Partially ported (gaps within a ported module)

| # | Upstream module | Size | Status | Gap |
|---|---|---|---|---|
| 11 | `voices.c` | 41 KB | 🟡 | `VoiceSpec` exists. **Voice-variant *name parsing* (2026-08-01):** `split_voice_variant` (`translate/mod.rs`) splits `base+variant` (`en+f3` → `en`,`f3`); `Translator::new` and `VoiceSpec::by_name`/`builder().language` resolve the **base** for dict/phontab (previously `-v en+f3` failed with "dict not found") and store the variant in `VoiceSpec::variant`. **Variant-file parsing (2026-08-01):** `voices::VariantParams` + `load_variant(data_dir, name)` parse `voices/!v/<name>` (pitch/formant/flutter/voicing/consonants/roughness/echo/stressAmp/stressAdd/breath/breathw + name/gender/age); the CLI warns on a missing `+variant` and continues on the base voice. `--voices` listing also landed (`src/voices.rs`, §16). **Variant PITCH applied to synthesis (2026-08-01):** `make_voice_params` scales `pitch_hz` by the variant's `pitch` base relative to eSpeak's default (82) — `variant_scaled_pitch` in `engine.rs`, variant-only path (default unchanged). Directionally verified end-to-end: `en+f3` mean F0 > `en` by >1.2× (`wav-analysis` YIN, `synthesis_analysis.rs`). Not byte-exact vs `voices.c` pitch math (oracle-gated). **Variant FORMANT scaling applied — freq + height + width (2026-08-01):** `make_voice_params` copies the variant's full `formant <i> <freq%> <height%> <width%>` lines into `VoiceParams.formant_{freq,height,width}_pct[7]` (`variant_formant_pct`), and the harmonic path pre-scales each frame's `ffreq`/`fheight`/`fwidth`/`fright` (`scale_frame_formants`) — the `voice.{freq,height,width}[i]` mapping. Variant-only path (all-100 default is byte-identical). Verified via Goertzel spectral probes (raising formants 60 % moves energy up >2×; doubling F2 height raises its band >1.5×). **Property-based voice matching implemented (2026-08-01):** `voices::find_voice` + `VoiceQuery` score every voice by language (exact/locale-prefix/same-family × **per-language** priority — `en` is prio 2 in en-GB but 3 in en-US, so "en" → en-gb), name (human-readable or a code), and gender; `EspeakNg::set_voice_by_query` is the `SetVoiceByProperties` equivalent and resolves voice *names* ("Afrikaans" → af) that plain `set_voice` can't. Needed a new `VoiceEntry.languages: Vec<(code, priority)>`. 2 tests. **CLI `-v` name matching wired (2026-08-01):** `-v "French (France)"` / `-v "Afrikaans"` now resolve via `find_voice` (falling back only when the value isn't already a language tag, then through `resolve_dict_stem` so a locale primary like `fr-fr` maps to the `fr` phoneme table). **Variant ECHO applied (2026-08-01):** `apply_echo` post-pass (`echo=(delayed*amp)>>8`, feedback + i16-saturating, faithful to `wavegen.c`), wired from the variant's `echo <delay_ms> <amp>`; default no-op (byte-identical). **Variant FLUTTER applied (2026-08-01):** `flutter_factor` per-phoneme F0 wobble (directional `Flutter()`), from the variant's `flutter <n>`; `flutter<=0` default is byte-identical. **Variant stressAmp applied (2026-08-01):** `stress_amp_ratio` rescales `amp_factor` by `stressAmp[level]/STRESS_AMPS_EN[level]` from the variant; all-zero default (unset) is byte-identical. **Voice `dictionary`/`phonemes` directives honoured (2026-08-01):** `VoiceEntry.{dictionary,phonemes}` parsed; `resolve_dict_stem` falls back to the voice's `dictionary <name>` and `resolve_phoneme_table` to its `phonemes <name>` when the code-named files are absent — Norwegian Bokmål `nb` (→ `no`) and Western Armenian `hyw` (→ `hy`) previously failed with "dict not found", now resolve. Fallback-only (common langs untouched). **Remaining:** variant *acoustic* application — pitch + full formant line + echo + flutter + stressAmp all done; **breath, roughness** remain (genuinely oracle-gated: aspiration-noise mixing / jitter). (Upstream #2472 adds variant "ali".) |
| 12 | `numbers.c` | 53 KB | 🟡 | Grouped scale composition works, but not every language's number/ordinal format (see §4 PRs). **Comma-grouped thousands + European decimal swap added (2026-08-01):** `NumberGrammar.group_separator`/`decimal_separator` — en uses `,`-group/`.`-decimal, while de/es/fr/nl/mt + Nordic use `.`-group/`,`-decimal (fixes `de "3,14"`→"drei komma…" and `de "1.000"`→"eintausend"). Per-language, default en-style off. **Space (SI/ISO) thousands grouping added (2026-08-01):** `NumberGrammar.space_group` (auto-enabled for the comma-decimal languages) merges `1 234 567` → one number via `apply_space_grouping` (1–3 digit lead + `⎵<3 digits>` groups; anything not exactly 3 digits ends the run) — `fr "1 234 567"`→"un million deux cent trente-quatre mille…", `fi "1 000"`→"tuhat"; English keeps a space as a separator. **Grouped decimals added (2026-08-01):** the final space group may be a 3-digit-integer decimal, so `fr "1 234,56"` merges to == `fr "1234,56"` (and `1 234 567,89` == `1234567,89`). **French letter-suffix ordinals done (2026-08-01):** `1er`/`2e`/`21e`… generate the ordinal word (premier/deuxième/vingt et unième) for 1..=1000 since the fr dict lacks `_No`. Remaining: full per-language ordinal words for other langs. **Round-thousands year-form over-application fixed (2026-08-01):** 2000+ no longer read as "N hundred" (was `2000`→"vingt-cent"); now "deux mille"/"two thousand"/"zweitausend", year form kept for 1100–1900. **Portuguese cardinals generated (2026-08-01):** the pt dict lacks number keys (`1`/`5`/`1000`→empty, `2000`→"dois"), so pt cardinals 0–999,999,999 are now spelled as text (European Portuguese, with the "e"/cem-cento/milhão rules) and pronounced via the rules. |
| 13 | `tr_languages.c` | 75 KB | 🟡 | `lang/` files are read, but hardcoded per-language special cases and replacement tables are only partially reflected. **Upstream #2476 (open):** a missing `break` in the ~1000-line `SelectTranslator()` `switch` silently gives one language a neighbour's `langopts` (this reached a green PR undetected) — the port's language-selection should be structured (table/`match`) so C-style fallthrough can't happen; worth a test asserting each language's opts are independent. **Done (2026-08-01):** `lang_number_grammars_are_independent` asserts en/es/fr/de/nl each get their own `NumberGrammar` (no neighbour inheritance) and unknown langs fall back to `default` — the port already uses a non-fallthrough `match`, so this locks in correct arm assignment. |
| 14 | `readclause.c` | 31 KB | 🟡 | Clause splitting works; **`[[…]]` phoneme input and technical-symbol name reading (`# $ % & * + / = @ ~`) now implemented** (`is_spoken_symbol`/`lookup_symbol_name`, 5 tests, 9/10 symbols byte-identical to oracle). **`-x`/synthesis parity (2026-08-01):** `translate_to_codes` had no `Token::Punctuation` arm, so spoken symbols rendered in `--ipa` but were **dropped in `-x`** and silent in audio; added to the word-emission arm (non-spoken punctuation has empty phonemes → no-op) — `-x "a & b"` → `'eI and b'i:`, matching `--ipa`. **Capital-letter announcement (`-k 2` → `_cap` "capital", byte-exact) implemented** (`cap_ipa` in `src/translate/mod.rs`, 3 tests). **`--punct`/`--punct=CHARS` implemented (2026-08-01):** announces punctuation by name — `LangOptions.punct` (`None`/all/subset) + `punct_covers`; a covered `Token::ClauseBoundary`/`Token::Punctuation` becomes a `Word` entry via `lookup_symbol_name` (`,`→"kˈɒmə", `.`→"dˈɒt"), gated so the default path is byte-identical. Works in `--ipa` and `-x` (with END_WORD bracketing for spacing since punctuation absorbs its whitespace). 3 tests. **Reaches audio too (2026-08-01):** `--punct`, `-g` (word gap) and `-k` (capitals) now thread through the engine `synth` path — `make_translator` applies `word_gap`/`capitals`/`punct`; new `EspeakNg::set_punctuation_list` + `Parameter::{Punctuation,Capitals}` handling; the `-k 2` capital announcement was ported into `translate_to_codes` (was `text_to_ipa`-only). Verified: `--punct`/`-g`/`-k` each lengthen the WAV (`runtime_options_audio.rs`). **`-k` completed (2026-10-11, §3d):** `-k 1`'s capital sound now belongs to the word, as in C (`_#X1h@l'oU`, no gap), and `-k <n>` with `n >= 3` raises the pitch for a capitalised word by `n` Hz (3 = C's default 20) and restores it afterwards on a short-pause carrier — all 20 `-k`/text combinations probed are byte-identical in `-x` and `--ipa`. Embedded commands (`\001…`) are implemented as `Token::Embedded` → `CodeMarker::Embedded` (rate/pitch/amplitude from that point on). |
| 15 | `phonemelist.c` | 19 KB | 🟡 | **`phonSWITCH` mid-clause language switching implemented** (`TranslateEntryKind::LangSwitch` in `src/translate/mod.rs`): `_^_LL` dict entries re-translate the word with the target language's translator and mark it `(target)…(orig)`; bare `phonSWITCH` → default voice `en`. 4 tests, byte-identical to oracle (af→en/de/fr). Also fixed promotion corrupting switch entries. **Remaining:** script-detection switches (non-Latin input), `<voice xml:lang>` wiring (now feasible atop this), `-x` mnemonic path still uses the old `"SW"`-mnemonic check. **Upstream (track):** merged #2456 clears `phoneme_tab` on a table switch so absent codes resolve to NULL (follow-on to #2459/§19); **#2475 (merged 2026-08-02, `503708d3`)** reverts #2128 and fixes a NULL `phoneme_tab` **crash in four shipped voices** underneath it — it also adds `phonemelist.c` "keep the word start when dropping an untranslatable phoneme", which is **not ported** (§3 #42); #2492 (merged 2026-08-03) fixes the same class at `i_APPEND_IFNEXTVOWEL` (§3 #41); open #2469 is a related segfault — `espeak -v alex` → "Unknown phoneme table: ''" → SIGSEGV. **⇒ the port already resolves a missing/empty phoneme table to a clean `Error::InvalidData` (never a panic)** — `find_table`/`select_table_by_name` return errors (test `load.rs:669`), and a `LangSwitch` entry always carries the `PHON_SWITCH` byte so the re-translation path can't index-panic; unknown switch targets go through `if let Ok(Translator::new)`. Verified 2026-08-01; a regression test would lock it in. Open #2479: the automatic script-based switch target is hardcoded to `en` (→ en-gb) upstream and isn't configurable — make it configurable when script-detection switching lands. |
| 16 | CLI (`espeak-ng.c`) | — | 🟡 | `-f` (file), `--stdin`, `-m` (SSML), **`-k` (capitals, byte-exact)**, **`-g` (word gap → `_` pause per word in `-x`, byte-exact)**, **`--voices[=<lang>]` (voice listing)** now wired; **`-z` (no final sentence pause)** suppresses the ~0.30 s end-of-utterance trailing silence (§35) via `VoiceParams.no_final_pause` + `EspeakNg::set_no_final_pause`, verified by WAV sample count (`runtime_options_audio.rs`); **`-b <n>` input encoding (1=UTF-8, 2=Latin-1, 4=UCS-2)** now decodes `-f`/`--stdin` bytes via the `encoding` module (`gather_text` → `decode_to_string`) — a Latin-1 file that fails as UTF-8 reads correctly with `-b 2` (`input_encoding.rs`); `-l` accepted for arg-parity. **`--voices` (2026-08-01):** new `src/voices.rs` walks `<data>/lang/**`, parses `name`/`language <code> [pty]`/`gender`/`age`, and lists all 145 voices sorted by code (filterable by language prefix). Listing-only (identification lines), not the full acoustic voice spec. 2 unit + 2 CLI tests. **`--compile[=<lang>]` implemented (2026-08-01):** compiles `<data>/<lang>_list`+`_rules` → binary `<lang>_dict` via `compile_dictionary` (the `espeak-ng.c` `--compile`/`--compile-debug` path); verified end-to-end (`--compile=en` then `-x hello` → `h@l'oU` from the compiled dict), missing-source fails cleanly. **`--compile-phonemes[=<src>]` implemented (2026-08-01):** parses a phoneme-table source (`parse_phoneme_dsl`) → binary `phontab` (`compile_phontab`); verified end-to-end (source → phontab → `parse_phontab` round-trips), missing-source fails cleanly. Table only (phonindex/phondata not regenerated). **Option sweep (2026-10-11, §3d):** `--tie[=<c>]`, `--sep[=<c>]` (fixed: optional argument, per-phoneme splitting, stress grouping, and upstream's one-character/sticky-tie-flag interaction), `--ipa=1|2|3`, `--split[=<minutes>]`, `--ssml-break=<percentage>`, `--load` (via `voices::load_voice_file` + `EspeakNg::set_voice_pitch`), `-d <device>` (consumed, unused) and a help text that lists every implemented option. **Remaining:** audio playback; `--compile-intonations` (no-op — it compiles `phsource/intonation`, which ships with upstream's repo, not with installed data); `--load` beyond `language`/`dictionary`/`pitch` (the rest of a voice file's directives are still looked up per-language in the data dir). |

## 3. Recent upstream commits not reflected here

| # | Ref | Description | Notes |
|---|---|---|---|
| 17 ✅ | #2463 (`84a37de`, 2026-07-25) **+ 2026-07-22 cluster** (#2449, #2451, #2452, #2454, #2455, #2457) | Emoji reading instead of spelling out — flag emoji, ZWJ sequences, skin-tone modifiers (base name + modifier), CLDR 48.2 data for 28+ langs, `$textmode` replacement segmentation | **Core done 2026-08-01** — emoji tokenised as sequences (base + VS16 + skin-tone + ZWJ + flag pairs) and read via the dict's TEXTMODE emoji-name entries (`word_to_phonemes` now re-translates TEXTMODE replacement text). `😀`→"grinning face", `❤️`→"red heart". Data-driven for the shipped langs (en verified); ja follow-up bug #2464 (silent descriptions) **fixed 2026-08-01** — the cause was missing katakana support, now folded to hiragana (see §5 #2464 + progress log). |
| 18 ✅ | #2431 | Russian feminine forms for decimal fractions | **Done 2026-10-11 (§3e).** The feminine numerator and `_0Z<n>s` suffix selection were already there; what was missing is that C `strcat`s the scale word onto the numerator *inside one word*, so `ru "0,25"` now reads `nˈoɭ ˈi dvˈɑtsatʲpʲˈatʲsˈotøx` — byte-identical to C, as are 0,1 / 0,2 / 0,11 / 1,2 / 2,5 / 21,2 / 11,2 / 0,021 / 0,5 / 3,7 / 0,0001. |
| 19 | #2459 (`121d8dd`) | NULL-deref guard after mid-clause phoneme-table switch | Ties to missing `phonSWITCH` (§15). |
| — | #2456 (2026-07-22) | Clear `phoneme_tab` on table switch so absent codes resolve to NULL | Follow-on to #2459; ties to the open crash cluster #2475/#2469 (§15). |
| — | Android / build / libsonic build commits | N/A to a pure-Rust port | — |
| 38 ✅ | #2511 (`f135499`, 2026-09-01) | **Arabic voice reads Han ideographs with the Mandarin translator** instead of spelling "Chinese letter". Adds `langopts.alt_alphabet` + `alt_alphabet_lang`, the `AL_IDEOGRAPHS` alphabet flag, `ShouldSplitIdeographs()` (per-ideograph word splitting for a *base* language that nominates an ideographic alternate), and `tone_ph_data` on `PHONEME_LIST` + `TonePhoneme()` / `InterpretPhoneme2WithData()` so a tone phoneme resolved under one table survives the switch back | ❌ **Probe 2026-09-03:** `-v ar "إذا 如果您因为翻译 النص"` → `ʔˈiðaː ɣ ʔˈannˈas̪s̪` — the whole Han run collapses to a stray `ɣ`. **Two gaps:** (a) no `alt_alphabet` concept (a language cannot nominate a fallback translator for a script range); (b) **no tone-phoneme support at all** — `tone_ph` appears nowhere in `src/`, which also blocks zh/yue/vi tone rendering generally (cf. §1.1 intonation). **Re-check 4 (2026-10-11): landed.** `alt_alphabet` / `alt_alphabet_lang` exist (`src/translate/mod.rs:5029-5056`) and the switch fires: `-v ar "إذا 如果您因为翻译 النص"` → `?'iDa: (cmn)z.'u35_| k'uo214_| n'i35n j'i55n w'ei51 f'a55n j'i51_|(ar) ?ann'as[s[`. Residual, now tracked as **§3c #55**: word grouping and stress inside the run. |
| 39 | `1f1e3fa3` (2026-09-01) | `TonePhoneme()` accessor replacing five `tone_ph_data`/`phoneme_tab` ternaries; alphabets-table realignment | Refactor, no behaviour change. Adopt the same shape if/when tone phonemes land (#38). |
| 40 🟡 | #2495 (`56f2e9c7`, 2026-08-10) | Stack buffer overflow when a word stacks many `SUFX_M` suffixes — the `while (more_suffixes)` loop in `TranslateWord3()` is now capped like the prefix loop | ❌ **Not a crash risk here (Rust), but a feature gap:** the port has **no multi-suffix loop at all** (`SUFX_M`/`more_suffixes` unimplemented; only single-suffix `SUFX_I` stripping). Probe: `s'`×220 → 220 bare `s`. **Re-check 4: the cap is honoured, the phonemes are not** — `s's` → `'Esz` where the oracle record here says `'EsI2z`. See **§3c #57**. |
| 41 ✅ | #2492 (`89fd5d50`, 2026-08-03) | NULL `phoneme_tab` deref in `i_APPEND_IFNEXTVOWEL` (109 of 148 shipped voices segfaulted on `-v de "ईघ"`) | ❌ Crash is N/A (memory-safe), but the **instruction itself is unimplemented**: `src/synthesize/bytecode.rs:498-516` no-ops every group-0 instruction except `RETURN`/`I_IPA_NAME`/`I_CHANGE_PHONEME`, and group-1 `ChangeIf` is skipped entirely (`tr == NULL`). So `i_APPEND_PHONEME`, `i_APPEND_IFNEXTVOWEL`, `i_INSERT_PHONEME`, `i_REPLACE_NEXT_PHONEME`, `i_DELETE_NEXT_PHONEME`, `i_PAUSE_BEFORE/AFTER`, `i_SET_LENGTH`/`i_ADD_LENGTH` and the `CHANGE_IF*` family all silently do nothing. **New gap — phoneme-program instruction coverage.** Probe: `-v de "ईघ"` → empty output (no crash, no speech). **Re-check 4: landed, note was stale.** `I_APPEND_IFNEXTVOWEL` and the rest of the group-0 family are recorded in `PhonemeExtract` (`src/synthesize/bytecode.rs:665-676`) and applied on both paths (`src/translate/mod.rs:2722`, `:8319`); group-1 `ChangeIf` runs in `phoneme_list_mode` (`bytecode.rs:681-692`). The `bytecode.rs:498-516` line numbers in the original note no longer exist. |
| 42 🟡 | #2475 (`503708d3`, **merged 2026-08-02** — §5/§15 tracked it as *open*) | Reverts #2128 as dead code and fixes the real NULL `phoneme_tab` crash under it; also `phonemelist.c` **"keep the word start when dropping an untranslatable phoneme"** and NULL-substitution in `SetWordStress` | 🟡 Crash-safety already verified here (`tests/crash_safety.rs`, §15). **Not ported:** the word-start preservation when an untranslatable phoneme is dropped — relevant once script-switching lands (#46). **Re-check 4:** the word-start preservation is **moot**, not pending — §3b #46 made the port route foreign-script runs instead of dropping them, so there is no dropped-phoneme path whose `sourceix` could be lost. |
| 43 ✅ | #2500 `ku` (2026-08-19), #2422 `fa` (2026-08-19), #2481 `hu` emoji (2026-08-19), #2423 `nl` (2026-08-18) | Dictsource-only language fixes: Kurmanji oblique-plural/preverb/negation stress + unstressed subject pronouns (+ new `docs/languages/ira/ku.md`); Persian exception words; Hungarian emoji + rules exceptions; Dutch `na`+CVC+`ige` | ❌ Inherited automatically by the §3b data refresh; nothing to port in code. |
| — | Android cluster (#2471, #2480, #2483, #2487, #2493, 2026-08-01→08-03) | TTS-framework word boundaries + error reporting, stop actually aborts synthesis, voice name as hint, settings screen off the main thread, launcher alias | N/A to a pure-Rust library. (`#2493`'s analogue — cancel between chunks — already exists in `async_synth.rs`, §7.) |

## 3b. The 1.53.0 data/feature backlog (data refreshed 2026-09-03)

`espeak-ng-data/` **was** eSpeak NG 1.52.0 (2024-12-12) while upstream had moved 432
commits past it, which made upstream's unreleased 1.53.0 ChangeLog this port's parity
backlog. The data was rebuilt from upstream `master` on 2026-09-03 (123 dicts / 151
voices) and re-verified by probe on 2026-10-11, so the rows below are kept for the
**engine** work each item needed; the data half of every one of them is in. Probes ran
against `./target/debug/espeak-ng-rs --path=espeak-ng-data`.

Row prefixes (`44 ✅`) carry the **current** status; the status cell keeps the
2026-09-03 probe that motivated the work, which is why a ✅ row can still read
"❌ Probe …". What landed is listed in the *Executed* table at the end of the
section, and the detail is in [GAPS-LOG.md](GAPS-LOG.md).

| # | Upstream 1.53.0 item | Port status / probe |
|---|---|---|
| 44 ✅ | **6 new languages** — `ab` (Abkhaz), `crh` (Crimean Tatar), `lij` (Ligurian), `mn` (Mongolian), `ps` (Pashto), `rup` (Aromanian); plus restored `fo`/`kaa`/`xex` dictionary builds under CMake | ❌ **9 languages upstream has dictsource for that we ship no dict for**: `ab, crh, fo, kaa, lij, mn, ps, rup, xex` (upstream has 123 dictsource languages, we ship 114 `_dict` files). Worse, `lang/` files for `crh, mn, ps, rup, fo, kaa` **are** bundled, so `--voices` advertises them and then they fail: `-v crh` → `not yet implemented: text_to_ipa: dict not found` (same for mn/ps/rup/fo/kaa/lij). `-v ab` silently returns empty. **Two fixes:** compile the missing dicts (data refresh) *and* make an advertised-but-dictless voice fail with a clear error rather than being listed as available. Supersedes the "blocked (missing data)" note on #2437 (§5) — Pashto dictsource now exists upstream. |
| 45 ✅ | **Emoji overhaul** — CLDR 33.1 → **48.2** (Unicode Emoji 12→16) for 66 languages, emoji/symbol data added for **28 more languages** (he, uk, th, nb, be, uz, kk, fo, …), ZWJ sequences matched against multi-codepoint dict entries, **skin-tone sequences spoken as base name + modifier**, emoji split from adjacent words, subdivision-flag and hyphenated-description fixes | 🟡 Tokenisation largely landed 2026-08-01 (§3 #17) but the **data is CLDR 33.1** and two behaviours are wrong. Probes: `👍🏽` → **empty output** (the *combined* skin-tone sequence is dropped; `👍 🏽` split by a space works → `θˈʌmz ˌʌp mˈiːdiəm skˈɪn tˈəʊn`); `🏴󠁧󠁢󠁥󠁮󠁧󠁿` (England) → `blˈak flˈaɡ` — exactly the subdivision-flag bug upstream fixed; `-v th "👍"` → empty (th is one of the 28 newly covered languages). |
| 46 ✅ | **"Split a word where the script changes to a foreign one"**, so mixed-script tokens are translated per script instead of spelled out (+ "Arabic text spelled letter-by-letter under non-Arabic voices" fixed, + the #2475 crash on a Latin word under a non-Latin voice) | ❌ **The foreign-script run is silently dropped, not spelled and not translated.** Probes: `-v ru "привет hello"` → `prʲivʲˈɛt` (in `--ipa` *and* `-x`); `-v ru "hello"` → empty; `-v en "hello 你好"` → `həlˈəʊ`; `-v en "hello مرحبا"` → `həlˈəʊ`. Silently losing input text is worse than mispronouncing it. Ties to §15 (`phonSWITCH` script-detection switches) and #2479 (hardcoded `en` switch target). |
| 47 ✅ | **Sinhala ZWJ conjunct clusters** read as one conjunct instead of separate letters | ❌ `-v si "ක්‍ර"` → `k ɹˈə` (separate letters) — the bug upstream fixed. |
| 48 ✅ | **Russian decimal fractions** use feminine unit forms and numerators ("одна сотая", "две тысячных") — upstream #2431, already listed as §3 item 18 | ❌ Probes: `-v ru "0,5"` → `nˈojɭʲ ˈi pʲˈʌʲtʲ` ("ноль и пять"), `-v ru "0,25"` → `nˈojɭʲ ˈi dvˈɑ pʲˈʌʲtʲ` — digit-by-digit, no fraction naming at all. Number-grammar gap (§12). |
| 49 | **Slovak number handling** fixed (Samuel Thibault) | 🟡 Not verified against an oracle; `-v sk "21 1000000"` → `dvˈadsatʲjˈeden mˈiliˌoːn` looks plausible. Re-check after the data refresh. |
| 50 ✅ | **New API `espeak_TextToPhonemesWithTerminator`** (Jonathan McDowell) — returns the clause terminator alongside the phonemes | ❌ No equivalent: `EspeakNg::text_to_phonemes{,_with_options,_phonemizer}` (`src/engine.rs:565-593`) return only a `String`. The terminator exists internally (`LookupContext::clause_terminator`, `dictionary/lookup.rs:86`) — surface it. |
| 51 | **Dictionary replacement text is word-segmented like normal input, with nested replacements** (hanzi → pinyin) — fixes silently dropped emoji descriptions in Chinese, Cantonese and Thai | 🟡 The port re-translates TEXTMODE replacement text (§3 #17) but not recursively/segmented. `-v cmn "🇧🇫"` → `ˈuouowʰoˌaiuə` (non-empty but unverified); `-v th "👍"` → empty. Needs an oracle pass after the data refresh. |
| 52 | **`compiledict` no longer leaks bogus entries from over-length list lines** | 🟡 Our compiler (§1.5) has the same class of limitation documented as "remaining: >255-byte entries" — fold the upstream bound in when that's finished. |
| 53 | **IPA/phoneme output no longer ignores `prevPh`/`prevPhW` conditions** (regression since #2082 — restores linking-j suppression after i-vowels, German r-flap, …) | 🟡 The port *does* evaluate `prevPhW` (`bytecode.rs:750`) and `prevPh` end_type matches (`:811`). Add a regression test for linking-j and the German r-flap to lock parity in. |
| 54 ✅ | **14 updated languages** — `de, en, es, fa, fr, he, hu, ko, ku, pt, ru, sv, th, tlh` | ❌ Inherited by the data refresh (#44). Several §4/§5 items collapse into this. |
| — | Build (autotools removal, CMake/GNUInstallDirs/Emscripten/CMake 4.0, PIC, PATH_MAX, macOS CoreAudio), Android app, libsonic big-endian, libpcaudio error reporting, MBROLA fuzzing, `%x` SSML format specifier, Windows voice identifier, "warn when MBROLA/Klatt not compiled in" | N/A to a pure-Rust port. |

**Executed 2026-09-03** (detail and probe output in [GAPS-LOG.md](GAPS-LOG.md)):

| Item | What landed |
|---|---|
| #44 ✅ | Data rebuilt from upstream `master` with CMake, merged over `espeak-ng-data/` (MBROLA files kept), `data-crates/` + Cargo features regenerated. 123 dicts / 151 voices; new test `every_advertised_voice_can_speak` walks every advertised voice and asserts it speaks. |
| #45 ✅ | Skin-tone sequences fall back to base name + modifier names when the pair has no entry (`👍🏽` was **silent**); tag characters stay in the token so subdivision flags name their country. |
| #46 ✅ | 22-script routing table + per-language `alt_alphabet` hook (#2511) + Serbian digraphia + "the dictionary wins over the script". No input text is dropped any more. |
| #47 ✅ | ZWJ/ZWNJ treated as in-word joiners: `si ක්‍ර` → `kɹˈɐ`, byte-identical to upstream. |
| #48 ✅ | Data-driven fraction reading via `_0Z<n>` / `_0Z<n>s` + `_1f`/`_2f`, with the singular/plural and feminine rules; covers ru **and** hu. Structure matches the oracle on all 12 probed cases. |
| #50 ✅ | `Translator::text_to_ipa_with_terminator` / `EspeakNg::text_to_phonemes_with_terminator` + `ClauseTerminator`. |
| #40 ✅ | `SUFX_M` suffix chains (`s's's` → `'EsI2zI2z`, == oracle), bounded at 50. |
| #41 🟡 | `ChangePhoneme` + condition programs now run at translate time on both the IPA and the codes path (so `-x` and synthesis see resolved phonemes), with cross-word `nextPh` context. The rest of the group-0 family (append/insert/delete/length/pause) and group-1 `ChangeIf` are still no-ops. |
| #2470 ✅ | `synth_with_events` applies the tempo/pitch post-pass and rescales event times to the resulting buffer. |
| — ✅ | Fallout fixed on the way: voice `phonemes` directive now outranks the bare tag (European Portuguese was using the *Brazilian* table); `-x` accepts regional locales; composed number entries take end-stress promotion on their last word; Chinese tone digits stay in the word; `[[…]]` keeps word boundaries. |

**Still open after this pass:**

| # | Item | Why it's left |
|---|---|---|
| — | Deliberate divergence: scripts upstream **spells** | Upstream only auto-switches for the `AL_WORDS` alphabets (el, hy, ar, hi, bn, pa, gu, ta, kn, ml, si, ka, ko) and *spells* the rest — `en "привет"` → "pe er ee ve ye te", `en "你好"` → "Chinese letter" ×2. The port speaks them with the script's voice instead (the pre-existing Cyrillic/Greek design here), which never loses text but is not oracle-identical. Implementing the spelling path would need `LookupLetter`-style foreign letter names. |
| 49 | Slovak numbers | Unverified against the oracle; the data refresh may already cover it. |
| 51 🟡 | Word-segmented / nested dictionary replacements | The hanzi→pinyin half now works (§3 #38), so Chinese emoji descriptions read. Still open: segmenting replacement text for languages without spaces (`th 👍`), and multi-character Chinese dictionary words. |
| 52 | `compiledict` over-length list lines | Belongs with the dict-compiler work (§1.5). |
| 53 | `prevPh`/`prevPhW` regression tests | The conditions are evaluated correctly (linking-j, German r-flap verified by hand); dedicated tests not yet added. |
| — | Georgian decimals | `ka` reads the fraction digit by digit and speaks its `_dpt` word; upstream reads it as a number with no point word. Not one of the modes upstream lists for `ka`, so it needs its own investigation. |
| — | Multi-character Chinese words | Re-examined (fifth pass): `谢谢` is **not** in the shipped `cmn` dictionary, so upstream's neutral tone on the second syllable comes from neither a dictionary entry nor the sandhi rules already ported. Mechanism still unidentified. |
| — | Georgian fractions | `ka` ships `_0Z*` entries but upstream reads the numerator *without* the scale word; the port still reads digit-by-digit. Needs its own decimal mode. |

**Suggested execution order for #44–#54:** (1) vendor `dictsource/` from master and run `--compile` for all 123 languages, adding the 9 missing dicts + data-crates; (2) fix the advertised-but-dictless voice error path; (3) skin-tone / subdivision-flag emoji sequences (code, not data); (4) foreign-script split (#46) — the only one that currently *loses* input text; (5) `alt_alphabet` + tone phonemes (§3 #38); (6) the small ones: terminator API (#50), ru fractions (#48), si ZWJ (#47).

## 3c. Divergences found by upstream re-check 4 (2026-10-11)

Differences against upstream `ba90c8e`, each probed on
`./target/debug/espeak-ng-rs --path=espeak-ng-data`. These are *not* unported
commits — the commits are in; these are the places where the port's own output
disagrees with upstream's.

| # | Divergence | Probe | Status |
|---|---|---|---|
| 55 | **`-v ar` + Han: word grouping and stress inside the switched run.** 如果 is one `cmn` dictionary word; the port makes every ideograph its own word, gives each its own `_|` gap and stresses each one | port `?'iDa: (cmn)z.'u35_\| k'uo214_\| n'i35n …(ar)` vs upstream's new `test_phon ar` `?'iDa: (cmn)z.u35k'uo214 n'i35n …(ar)` | ❌ Same root cause as the open "multi-character Chinese words" item at the end of §3b — the switch itself (#2511, §3 #38) works |
| 56 ✅ | **Math symbols read as nothing.** Silently losing input text, the same class of bug as §3b #46 | `-v hu` `−` (U+2212), `∓`, `⊖`, `⁻` → **empty**; `➖` → `v'AStAg m'i:nusjEl` and `-v en "−"` → `m'aIn@s` work. `-v en "∏"` and `€` are empty too | ❌ **Port-side routing, not data staleness:** upstream master's `hu_emoji` carries the same 6 `mínusz` / 4 `mínuszjel` lines the bundled `hu_dict` does. Upstream reads `−` as "mínusz" — the exact entry #2481 changed, so that fix is unreachable here **Fixed 2026-10-11.** Symbol names now come from the dictionary — the `*_emoji`/`*_list` symbol block every shipped language carries — with the old hardcoded 31-character table left as the fallback for the languages whose data names nothing (where upstream is simply silent). `dict_names_symbol()` + `SymbolName::{Inline,Announce,Operator}` in `src/translate/mod.rs`; tests in `tests/dict_symbol_names.rs`. Fallout fixed with it: capitalised replacement text (every German symbol/emoji name) was translated as written, so the rules matched nothing and a one-word reading fell through to being read as raw phoneme codes (`de ∑` → `B'AE,EQ@,aIQqJ^Q"QE2`); clause punctuation (`।`, `।`, `❓`, `❗`, `‼`) is excluded via a transcription of C's `clause_type_from_codepoint`; the synthesised unary minus of `-5` keeps the operator word (`uk` "мінус", not the dictionary's "знак мінуса"). |
| 57 🟡 | **Suffix chains are not byte-identical.** #2495's bound is respected; the first link's phonemes differ | `s's` → `'Esz`, `s's's` → `'EszI2z` vs the `'EsI2z` / `'EsI2zI2z` recorded by §3 #40; at ~110 links output degrades to repeated bare `s` | 🟡 No C binary in this checkout, so the reference is §3 #40's own oracle record rather than a live oracle run **Half fixed 2026-10-11:** `s's` → `'EsI2z` and `x's` → `'EksI2z`, byte-identical to upstream. The letters already spelled now stay available as the rule pre-context (`PosExpect::prev_letters`), which is how `s) 's` → `I2z` outscores the bare `'s` → `z`. **Still open:** the *chained* case `s's's` → `'EszI2z` (upstream `'EsI2zI2z`) — the stem buffers inside the suffix-peeling loop do not carry that pre-context yet, and threading it there also means re-basing the `suffix_start > 1` guards onto `word_start`. |
| 58 ✅ | **Dutch `-ige` drops /ɣ/** — sits exactly on #2423's target words | `nadelige` → `na:d'e:l@` (expected `na:d'e:l@Q@`), `nalatige` → `na:l'a:t@`; `nadelig` → `na:d'e:l@x` and `hoge` → `h'OQ@` are correct, so it is the inflected form only | ❌ The `na`+CVC+`ige` half of #2423 (the `na:` prefix) **is** in the bundled data **Fixed 2026-10-11.** Root cause was the `SUFX_Q` ("don't retranslate") fallback inside a stacked-suffix chain: it threw away both the chain's accumulated suffix phonemes and the stem phonemes its own re-translation had produced, and used the *whole word's* instead. `peel_stacked_suffixes` now returns that stem (`PeeledSuffixes`). All six probed `-ige` words — including #2423's four — are byte-identical to upstream; `tests/suffix_chains.rs`. |
| 59 ✅ | **A language switch from the *rules* stranded the marker inside the word.** `*_rules` files can send a word to another language (`de igh (K  _^_EN`); C discards everything matched so far and returns just the marker so the word is re-translated whole, while the port appended it to the phonemes it had already produced | `de Copyright` → `k'o:py:r_^_t`, `de high` → `h_^_`; ~30 languages use the device (fr 180 rules, af 66, de 60, vi 43, da 39, he/fa 37, lv 33, cmn/yue/th 26-27, sv 19) | **Fixed 2026-10-11** in `translate_rules_ext`: `de Copyright` → `(en)k'0pIr,aIt(de)`, `de high`, `af light`, `fr ©` all byte-identical to upstream. A symbol entry that is *only* a switch now carries the character as its `word_lower` so the target language has something to re-translate. `tests/phonswitch.rs` |
| 60 ✅ | **Multi-word symbol/emoji names differ in stress.** C re-tokenises replacement text into the clause, so the words are stressed as a phrase; the port translates each part in isolation with a default position | `hu →` → `j'ob: _!'oldAlR2A m'utAto: n^'i:l` vs upstream `j,ob: _!,oldAlR2A m'utAto: n^'i:l`; `it 😀` → `kon Un` vs `k'on 'un`; `es ™` → `De` vs `D'e`. The port's *plain-text* reading of the same phrase matches upstream exactly, so the fix is to splice the replacement into the clause rather than translate it per part | ❌ ~30 of the 626 sweep cases; all are non-empty and intelligible, differing only in which words carry primary vs secondary stress **Fixed 2026-10-11 (second pass).** C applies the clause-end stress attributes — `$u+`, `$strend`, `$strend2`, `$unstressend` — to **every** word of a replacement, and only when the symbol itself ends the clause; it does *not* pass clause position down for `$atend` entry selection (`≤` keeps the running-text "tU"). Both halves are now modelled, so `÷`, `👍`, `👍 the`, `a 👍 b`, `es ™`, `it 😀`, `fr ∑`/`→`, `de ÷`, `hu →`/`©` are all byte-identical. A spoken symbol also *occupies* a clause position now, so `a ÷` keeps "a" reduced. |
| 61 ✅ | **Non-ASCII clause terminators don't break the clause.** The port tokenises only `. , ! ? ; :` as clause punctuation; `।` `۔` `።` `。` `…` `❓` are dropped silently (they are no longer *spoken* — §3c #56 — but they also don't end the sentence) | `en "a ❓ b"` → `a# b'i:` vs upstream `'eIb'i:` (the clause break makes "a" clause-final). The same difference already exists for an ASCII `,`, so the gap is really "punctuation doesn't split the clause here", not the character set | ❌ `is_clause_punctuation()` already transcribes C's whole set from `proplist.c`, so the tokenizer has the table it needs **Fixed 2026-10-11 (third pass).** `clause_punctuation_class()` transcribes C's whole table, including the `OPTIONAL_SPACE_AFTER` property that lets a space-less script break at all. The tokenizer ends the clause on those characters when a space, a bracket or end of text follows — `hi "क । ख"`, `cmn "你好。再见"`, `ja "これ。あれ"`, `hy "բարև։ ինչպես"`, `en "a — b"` and `"a … b"` are byte-identical, and an unspaced `a–b` is still dropped, as in C. `tests/clause_punctuation.rs`. |
| 62 🟡 | **ja/de symbol names where upstream is silent.** `ja ™ © €` read "マーク"/"ユーロ" here; upstream produces nothing because its replacement-text path can't carry the language switch the kanji part needs | `ja €` → `j'u:r`o` vs upstream `` | 🟡 Deliberate: the port's stance is that silence loses input text. The kanji half of `ja ©` ("著作権") is still dropped, so the reading is partial **Root cause confirmed 2026-10-11 (second pass)** from upstream's own `-X` trace: the symbol switches to English, English's entry is `$textmode`, and the replacement text is not carried back through the switch — so C prints nothing. Also covers `°` under ja/fi/el (the port reads "degrees"). The port resolves these, which is a deliberate divergence, not a gap: 9 of the 626 sweep cases, all cases where C drops the character. |
| 63 ✅ | **A symbol the language cannot name was dropped.** C's `LookupCharName` names it in the default voice instead (`language = L('e','n')`), wrapped in a language switch | `sv #`, `ja # % ± &` `@`, `he # & @`, `el # & ±`, `fi ±` were **silent**; C reads `(en)h'aS(sv)` etc. — 14 of the sweep cases | **Fixed 2026-10-11:** `lookup_symbol_name` returns a switch marker as its last fallback and the existing switch machinery resolves it. Residual: C names the character with the target's `_<char>` *name* entry, the port with the character's own reading, so 4 cases read English's short form — `he @` → `(en)_'at_(he)` where C says "at sign" **Residual closed (fourth pass):** the switched reading now comes from the target's *character name* (`_@` → "at sign"), with the character's own entry as the second key — the order `LookupCharName` uses, which is also why `#` (no `_#` entry) still reads "hash". `Translator::char_name_phonemes`. |
| 64 ✅ | **Conditional symbol entries were invisible.** A `?n` condition on the entry (Portuguese names `§` once per variety, `?1 §` / `?2 §`) failed because the symbol lookup carried no conditions | `pt §` and `pt-br §` were silent | **Fixed 2026-10-11:** `dict_names_symbol` takes the language's `dict_condition`. `pt §` → `p,&*'ag@-*&fU`, `pt-br §` → `p,a*'ag@-*&fU`, both byte-identical |
| 65 | **`.` between letters.** The port reads the character name where C drops it in some languages | `de "cat.Dog"` → `k'A:t p'UNkt d'o:k` vs C `k'A:t d'o:k`; `en "cat.Dog"` → `k'at d'0t d'0g` matches | ❌ Pre-existing (unchanged by the 2026-10-11 passes); C treats a `.` with no space as a clause terminator that is not spoken, the port as a symbol **Re-measured (third pass):** unchanged and still pre-existing; `en "cat.Dog"` matches. |
| 66 ✅ | **Brackets don't pause.** C announces a bracket with `LOPT_BRACKET_PAUSE` silence and makes the word before it clause-final | `en "a ( b"` → port `a# b'i:` vs C `a#_:_: b'i:`; `"a ("` → port `'eI` vs C `'a#` | ❌ Pre-existing; same family as §3c #61 (punctuation not splitting the clause) **Fixed 2026-10-11 (third pass).** `LOPT_BRACKET_PAUSE` (4) and `LOPT_BRACKET_PAUSE_ANNOUNCED` (2) are read from the voice file's `brackets` / `bracketsAnnounced` — Hungarian sets both to 0 — and spent as C's pre-pause loop does, two `phonPAUSE` each. The pause joins the run already open (`a#_:_: b'i:`) and belongs to the following word, so `(hello)` pauses only on the opening bracket. 17 of the 18 sweep languages were fixed by this one change. |
| 67 ✅ | **The unary minus was read from a hardcoded table, not the data.** C translates the character as a word: Finnish `*_rules` has `__) - (_D  mi:nus`, Greek a `- m'ion` list entry, and a language with neither says nothing | `-5`: the port read `_-`'s *spell-out* name in 4 languages ("tiret", "myślnik"), dropped the sign in `fi`/`el` where C reads it, and read "minus" in 6 where C is silent | **Fixed 2026-10-11:** the operator keeps the ASCII hyphen, is translated as a word with the following number visible as rule context (`next_word_opening`), and stops there — no spell-out name, no cross-language fallback. **17 of 18** sweep languages now match; `pt` differs only in a pre-existing final-vowel detail. A lone-NUL reading was also being accepted as a reading, which is what silenced `fi -5` |
| 68 ✅ | **`groups3` was also indexed by the raw first byte.** C indexes it only by `wc - letter_bits_offset`; the extra fallback made a non-Latin language match a letter's rule on an ASCII byte | `bg "-5"` → `j p'et`: byte 0x2D landed on the Cyrillic group for `ь` | **Fixed 2026-10-11** in `translate_rules_ext`; the first branch now covers `letter_bits_offset == 0` as C does, and the fallback is gone |
| 69 ✅ | **Greek had no `LETTERGP_Y` (or `B`).** `el_rules` palatalises with `κ (Y  c`, so without the front-vowel group the rule never fired | `κόκκινη` → `k'okin,i` instead of `k'ocin,i`; likewise `κύριος`, `όχι`, and four emoji/symbol names | **Fixed 2026-10-11:** C's `el_vowels` / `el_fvowels` / `el_voiceless` / `el_consonants` are transcribed as the same offsets (`build_letter_bits`). `tests/greek.rs` |
| 70 ✅ | **`LOPT_REGRESSIVE_VOICING` was set for `ru`/`uk` only.** `tr_languages.c` sets it for nine arms, and the port's word-final-devoicing list had `sk`/`sl` under a bit C doesn't give them | `pl "minus b"` → `m'inus b'E` (C `m'inuz b'E`); same for `cs`, and `pl "kciuk w górę"` | **Fixed 2026-10-11:** `bg` 0x107, `de`/`nl`/`mt` 0x100, `sl` 0x103, `cs`/`sk`/`sr` 0x03, `pl` 0x09, `ru`/`uk` 0x03. `tests/regressive_voicing.rs` |
| 71 ✅ | **A switch group spanned a clause boundary.** C ends the clause's phoneme list there, so it closes and reopens | `ru "a , b"` → `(en)'eI b'i:(ru)` instead of `(en)'eI(ru) (en)b'i:(ru)` | **Fixed 2026-10-11**; a switch whose target speaks nothing also no longer leaves an empty `(en)(ru)` pair behind |
| 72 ✅ | **Dutch shortens a stem vowel when a suffix is removed.** C keeps the phonemes the whole word produced; the port re-translates the stem, where `a (CK` then matches | `graden` → `Qr'Ad@n` vs C `Qr'a:d@n` (identical rule traces, so the divergence is in the suffix handling, not the match) | ❌ 3 sweep cases, plus `nl °`/`20°` **Fixed 2026-10-11 (fourth pass).** C has an `nl` branch in `RemoveEnding` that *doubles* the stem vowel — Dutch spells a long vowel doubled in a closed syllable, so `graden` − `en` is the stem `graad`. All of `graden`, `straten`, `maken`, `hoge`, `20°` are byte-identical, and `gronden`/`landen` correctly stay short. `tests/suffix_chains.rs`. |
| 73 ✅ | **Italian geminates and open vowels.** `somma` → `s'Om:a` vs C `s'omma`: the port writes one long consonant where C writes two, and picks the open vowel | `sommatoria`, `rosso`, `pollice` | ❌ 5 sweep cases **Fixed 2026-10-11 (fourth pass).** Two causes: (a) `MakePhonemeList` lengthens a fricative, nasal or liquid marked with the lengthener by **doubling** it and dropping the `:` — every other type keeps it, so Finnish `k'uk:a` is unchanged; (b) the open vowel came from a *flags-only* stem entry (`somm $alt2`) whose flags the port dropped, so `ApplySpecialAttribute2` (`LOPT_ALT & 2`, it/pt/sl) never ran. `tests/geminates_and_alt_vowels.rs`. |
| 74 | **Synthesised audio is ~14% shorter than master's.** `tests/oracle_comparison.rs`' duration check fails against an upstream `master` binary (it skips when none is on `PATH`, which is why the default suite is green) | `en "hello"`: 7662 samples here, 8895 in C — ratio 0.861, under the test's 0.9 threshold | ❌ Pre-existing and unrelated to the 2026-10-11 passes (verified by re-running with `src/` stashed). The port's timing was calibrated against 1.52.0; master's phoneme lengths have moved. Run it with `ESPEAK_NG_BIN=<build>/src/espeak-ng ESPEAK_NG_DATA=<build>` |
| 75 ✅ | **A bracket pause sat inside a language-switch group.** C closes the switch, pauses in the base language, and opens a new group | `ru "a ( b"` → `(en)'eI _:_: b'i:(ru)` instead of `(en)'eI(ru)_:_: (en)b'i:(ru)`; same in he/el/uk/ja | **Fixed 2026-10-11 (fourth pass)**: the switch closes at a bracket as well as at a clause boundary, and the pause takes back *every* trailing word boundary (a switched word contributes one on top of the separator's) |
| 76 | **Readings that differ between a word typed directly and the same word as a replacement.** C's number/replacement paths skip some word-final substitutions | `pl @` → C `m'awpa` but C's own `małpa` is `m'awpa#_`; `pt 5` → C `s'i~Nku` but `cinco` is `s'i~NkU`; `es §` → C `sEkTj'on` but `sección` is `sekTj'on`. The port applies the same reduction in both | ❌ 6-8 cases. Probably the voice `replace` table's "only at a word end" bit (bit 0) and where C considers a composed word to end |
| 77 | **Turkish loses the dotless-ı upgrade on a non-final word.** `eksi` alone is `Eks'I` in both, but in a phrase the port gives `Eks'i` | `tr "eksi işareti"` → `Eks'i iSa*et'I` vs C `Eks'I iSa*et'I`; 5 of the sweep's `tr` cases are this | ❌ The vowel comes from the rules via the stress context, so the first word's non-final stress changes it; C's does not |
| 78a ✅ | **An unrecognised letter abandoned the whole word.** C skips the letter its rules don't cover and reads the rest | `ru "његов"` (Serbian `њ`) was **silent** where C reads `'egVf`; under a voice that routes by script the bare `phonSWITCH` leaked (`en "његов"` → `_^_w#U`) | **Fixed 2026-10-11 (fifth pass)**, found while probing upstream #2555. The rules loop notes the letter (`spellword`) and keeps scanning; `ru његов` and `ru ћирилица` are now byte-identical to C and the marker no longer leaks. `tests/upstream_issue_regressions.rs` |
| 79a ✅ | **An unknown voice reported "not yet implemented".** | `-v alex` → `not yet implemented: translate_to_codes: dict not found` | **Fixed 2026-10-11:** `Error::VoiceNotFound` → `voice not found: alex`. (C prints "Unknown phoneme table: ''" and then segfaults — upstream #2469) |
| 80a ✅ | **`$max3` was not implemented.** A symbol entry carrying it is spoken at most three times in a row (C's `phonemes_repeat_count`), and any other word restarts the run | `en "~ ~ ~ ~ ~ ~"` → six tildes here, three in C; `"% % % % % %"` likewise | **Fixed 2026-10-11 (fifth pass)**, from upstream #2551/#2552: symbols are limited, letters are not — which is the position #2552 argues for and which the port already had for letters |
| 81a | **Gujarati inserts a schwa before a medial `r`.** | `gu ગુજરાતી` → `g,uJ@3*'a:ti` vs C `g,uJr'a:ti` | ❌ Found while probing upstream #2563 (which reworks `gu` stress by syllable weight so a medial schwa is *dropped* — the port adds one) |


**Implementation passes (2026-10-11, after re-check 4).** Five passes.
**Fixed:** #56, #58, #59, #60, #61, #63, #64, #66, #67, #68, #69, #70, #71, #72,
#73, #75, #78a, #79a, #80a; **half:** #57; **open:** #55, #62, #65, #74, #76,
#77, #81a.
The fifth pass came out of the live PR/issue sweep recorded in §4 and §5 — three
upstream bugs the port does not have, and one it did (#78a). Measured with
`scripts/oracle_sweep.py` — a 626-case sweep of upstream's own
`tests/language-pronunciation.test` corpus (148 UDHR-style sentences across the
shipped languages) plus 478 symbol/emoji/suffix cases over 18 languages, against
a locally built upstream `espeak-ng` (`ba90c8e`):

| | before | pass 1 | pass 2 | pass 3 | pass 4-5 | pass 6 | pass 7 | pass 8 | pass 9 | pass 10 | pass 11 |
|---|---|---|---|---|---|---|---|---|---|---|---|
| byte-identical to upstream | 291 / 626 | 397 | 421 | 445 | 467 | 469 | 472 | 481 | 485 | 490 | **496 / 626** |
| cases where the port was silent but C speaks | 14 | 14 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | **0** |

(Pass 6 is §3e: `nextVowel` and the Russian fractions.  Pass 7 is §3f: the
`$w_alt`/`$list`/`$p_alt` rule conditions and the Spanish-group stress flags.
Pass 8 is §3g: `isFinalVowel`, `SFLAG_DICTIONARY` and priority stress.  Pass 9
is §3h: Turkish casing, the length mark, and an invented rule removed.  Pass 10
is §3i: a clause boundary is a pause.  Pass 11 is §3k: Serbo-Croatian's second
script and the apostrophe rules; §3l, the transliterating languages, is in the
same column.  §3j — the number grammars — is outside this corpus.)

(The fifth pass's fixes are outside the sweep's corpus — it has no Serbian text
and no unknown-voice case — so the count is unchanged; `tests/upstream_issue_regressions.rs`
pins them instead.)

Of the 626, 159 still differ: **45** are cases where **C** is silent (§3c #62 and
the port's deliberate symbol readings — the largest remaining group, and a
decision rather than a defect), **59** are per-language phoneme details (#76,
#77, Spanish letter-name allophones, Portuguese secondary stress, and the stress
rules of ia/lfn/pap/gn/ta), **29** differ in word count (mostly the same two
causes plus §3c #55), **17** in pauses or spacing and **9** in stress markers.

Every cluster worth chasing turned out to be a single cause: a missing letter
group (Greek, #69), a missing language option (#70, #73b), an extra index
fallback (#68), a table where C reads data (#67), one property table (#61), one
language-specific branch of `RemoveEnding` (#72), and one phoneme-list pass
(#73a).

Suite: **853 tests green**, with new `tests/dict_symbol_names.rs` (9),
`tests/suffix_chains.rs` (4), `tests/clause_punctuation.rs` (5),
`tests/regressive_voicing.rs` (3), `tests/geminates_and_alt_vowels.rs` (3) and
`tests/upstream_issue_regressions.rs` (6),
plus cases added to `tests/phonswitch.rs` and `tests/greek.rs`.  Ten existing
expectations were corrected to upstream's own readings, each re-checked against
the oracle first: `™` is "trade mark", `&` is `_and` with its pause, `it 20°` is
`ɡrˈaːdɪ`, `fr 5×3` is "multiplié par", `el όχι` is `ˈoçi`, `nl 20°` is
`ɣrˈaːdən`, `pt 3,14` is "cat**o**rze", `hi "क । ख"` breaks the clause, `he -5`
and `pl -5` drop the sign (their data has no reading for it), and `fr №5` names
the character through the default voice.  Two debug helpers were added for this
kind of work: `examples/debug_symbol.rs` (what the dictionary holds for a
character) and `examples/debug_suffix.rs` (a word's suffix chain, stem and
ending phonemes).

## 3d. CLI and SSML functionality implemented (2026-10-11)

The option sweep that followed re-check 4: every `espeak-ng.c` option the port
did not have, implemented and verified against upstream `ba90c8e`.  Nothing here
is a language-data gap — it is the command-line and SSML surface.

| Option | What landed |
|---|---|
| `--tie[=<c>]` | A tie *inside* a phoneme name (`tʃ` → `t͡ʃ`), done in the IPA/`-x` writers because only they know where one phoneme's name ends (`tie_phoneme_name`, `tie_unit`).  No value = U+0361, `z` = U+200D. |
| `--sep[=<c>]` | Corrected in three ways: the optional argument only attaches with `=` (a bare `--sep` is a space and the next word stays text), the IPA writer splits *per phoneme* instead of per character, and a stress mark binds to the phoneme it precedes.  `z` = U+200C. |
| `--sep` / `--tie` together | Upstream keeps one separator **character** and a **sticky** tie flag (`phoneme_options \|= espeakPHONEMES_TIE`, never cleared), so the two interact: `--tie --sep=.` ties with a dot, `--sep=. --tie` ties with U+0361.  Both also imply `-x`.  All 22 combinations probed are byte-identical. |
| `--ipa=1\|2\|3` | The deprecated numeric forms — `1` = `_` between phonemes, `2` = a U+0361 tie, `3` = a ZWJ tie, anything else plain IPA — as spellings of the same two slots. |
| `--split[=<minutes>]` | `write_split_wavs`: a new WAV at the first `Sentence` event past each limit, numbered `<name>_01<ext>`, `_02`, … with an extension of up to four characters kept (`out.wav` → `out_01.wav`, `out` → `out_01`), default 30 minutes.  Structurally identical to upstream on the same text (3 parts, cuts within 0.3 s). |
| `--ssml-break=<percentage>` | `Parameter::SsmlBreak` (C `espeakSSML_BREAK_MUL`) + `ssml_to_speech_text_opts`/`process_markup_opts`.  Scales `<break time=…>` and nothing else, in both the `=` and space-separated forms. |
| SSML `<break>` durations | `<break>` used to be a bare clause boundary with the port's flat 200 ms pause.  Now it carries C's own lengths: `strength` picks `{0, 70, 140, 210, 400, 800}` ms and below `medium` is a prepause on the next word rather than a clause break (so the intonation is left alone), `time` overrides with an explicit duration, and `<p>`/`<s>`/`<br>` get C's 700/400/300 ms.  Pause deltas match upstream within 6 ms. |
| `--load` | `voices::load_voice_file` reads a voice definition from a path; its `language`/`dictionary` selects the translation and its `pitch` line reaches the synthesizer through the new `EspeakNg::set_voice_pitch`.  Needed `Cli::voice_raw`, since `-v` is normalized as a language tag (lower-cased, `_` → `-`) and that corrupts a path. |
| `-d <device>` | Parsed and its argument consumed (this port has no playback), so it can no longer turn into spoken text. |
| Word-level IPA pairs | `Translator::text_to_ipa_words` — upstream PR #2533's API, below. |
| `-k <n>` | Two gaps closed: the `-k 1` capital *sound* is part of the word it marks rather than a word of its own, and `n >= 3` (a pitch raise of `n` Hz, 3 meaning C's default 20) was accepted but did nothing.  It now emits C's own pair of relative pitch commands around the word, the restoring one carried on a `phonPAUSE_SHORT` — which is where upstream's trailing `_` in `-x` comes from.  20 `-k`/text combinations byte-identical in `-x` and `--ipa`. |
| `-h` | Was already an alias for `--help`; the help *text* now lists every implemented option — `-m`, `-k`, `-g`, `-z`, `-b`, `-l`, `-d`, `--tie`, `--ipa=N`, `--split`, `--ssml-break`, `--load`, `--compile-debug`. |

**Word-level IPA pairs (upstream PR #2533).**
`Translator::text_to_ipa_words` (and `EspeakNg::text_to_phoneme_words`) returns
one `WordPhonemes { word, offset, length, phonemes }` per source word, with
upstream's own invariant: the phonemes are a *view* of the ordinary rendering,
so joining them with single spaces reproduces `text_to_ipa` byte for byte.  That
is the whole point of putting it in the library rather than leaving callers to
re-tokenize — a number expands to several spoken words under one token
(`2.5` → `tˈuː pɔɪnt fˈaɪv`), an abbreviation becomes a different word, and a
dictionary entry can cover several source words.

The alignment comes from the pipeline, not from a second tokenizer: the tokens
have been through fifteen rewriting passes by translation time, so
`translate_to_codes_tracked` records which token pushed which codes and converts
that to **boundary coordinates** — the number of word/clause boundaries before a
code — because the post-passes (virtual phonemes, list editing, liaison, word
gaps) insert and delete phonemes but never a boundary.  Spans are located by
searching the source text forward for the token, which is exact for text the
tokenizer did not rewrite and `length == 0` where it did.  Verified on 21 texts
in 15 languages, including `phonSWITCH` switching, inline `[[…]]`, `$max3`
repeats and embedded commands (`tests/word_phonemes.rs`).  Where upstream's
example prints `.NET -> dˈɑːt ˌɛnˌiːtˈiː` as one pair, the port reports the
symbol and the word separately, each with its own span.

**How a break duration reaches the synthesizer.** In C a `<break>` becomes part
of the *clause terminator*, which the port's text stream has no equivalent for.
The duration therefore travels as an embedded command, `\x01[+]<ms>Z` — the one
channel that already survives tokenizing, translation and phoneme-code assembly
in order (`ssml::BREAK_LETTER`).  `Z` is not one of upstream's own command
letters (`PSARHTIVYMUBF`), and C drops an unknown one, so text written for
either implementation can never collide.  A `+` marks a *wall-clock* duration:
C deliberately compensates `<break time>` for the speaking rate so the listener
gets what was asked for, while its `strength` pauses are clause units that get
shorter as the voice speeds up — `break_ms` reproduces both.

**Not implemented, with the reason:** `--compile-intonations` stays a no-op
because `espeak_ng_CompileIntonation` compiles `phsource/intonation`, a source
file that ships with upstream's *repository* and not with installed data; there
is nothing to compile against.  `--load` is also only as deep as the port's own
voice plumbing: everything other than `language`/`dictionary`/`pitch` is still
looked up per-language in the data directory, not read from the file.

**One known divergence, found while probing.** `-m "one<break/>two"` reads
`w'0n` here and `w,0n` upstream: C's break terminator carries
`CLAUSE_INTONATION_NONE | CLAUSE_TYPE_NONE`, so the clause it ends is not a
sentence and the word keeps secondary stress, while the port treats an SSML
break as an ordinary clause boundary.  Pre-existing, unrelated to the pause
work, and shared with every other SSML boundary.

**A measurement that nearly became a wrong conclusion.** Upstream's `--split`
appeared to be broken — one 14-minute file at `--split=1` — until the test text
was capitalized.  Neither implementation ends a clause on a `.` followed by a
lower-case word, so `… number 1. hello …` is a single clause with no `Sentence`
event to cut at.  With `Hello …` sentences upstream splits exactly as
documented, and so does the port.

Suite: **872 tests green**, with new `tests/cli_phoneme_separators.rs` (6),
`tests/cli_audio_options.rs` (4) and `tests/word_phonemes.rs` (4), plus
`break_strength_sets_the_pause` and `ssml_break_percentage_scales_time` in
`src/translate/ssml.rs` and two `-k` cases in `tests/capitals.rs`.  The oracle sweep is unchanged at **467 / 626**
byte-identical, as it should be: none of this changes how plain text is
translated.

## 3e. `nextVowel` conditions and Russian fractions (2026-10-11)

Two fixes that came out of probing §3 #18, both measured against upstream
`ba90c8e`.  They move the sweep to **469 / 626** (from 467), with no case
regressing.

**`nextVowel` only looked two phonemes ahead.**  C's `nextVowel` condition
(`synthdata.c`, `InterpretCondition` case 7) scans forward to the next vowel and
gives up only at a word boundary; the port looked at `next` and `next2` and gave
up after that.  Any language whose phoneme programs use it to spot a *following
stressed* vowel therefore lost the rule whenever a consonant cluster stood in
the way.  Russian `V` is `IF nextVowel(isMaxStress) THEN ChangePhoneme(a)` — the
pre-tonic rule — and in `двадцатьпять` the stressed vowel is three phonemes
away, so the port read `dvV_:tsVt;p;'At;` where C reads `dvV_:tsat;p;'At;`.
English `0` reduces the same way: `to convert` is `tə kənvˈɜːt` in C and was
`tə kɒnvˈɜːt` here.

`Neighbours` now carries `next_vowel`/`next_vowel_stress` and
`prev_vowel`/`prev_vowel_stress`, filled by the code-list pass
(`resolve_virtual_phonemes_in_codes`), which is the one stage that can see the
whole word; `prevVowel` was being approximated by "the previous phoneme, if it
is a vowel" and is now C's `worddata->prev_vowel`.  A caller that does not track
them (the synthesis stage) keeps the old two-phoneme window, so nothing else
changed.  Verified on ru/en/bg/hi/gu/bn/lij/tt/ba/as — every language whose
`phsource` uses the condition — plus the twelve Russian numbers above.

Two test expectations were corrected to upstream's own readings, each re-checked
against the oracle first: `to convert` is `kənvˈɜːt` (`tests/homographs.rs`) and
the Russian fractions above (`tests/upstream_1_53_parity.rs`,
`tests/upstream_issue_regressions.rs`).  New `tests/pretonic_vowels.rs` (3)
pins the rule itself, in both languages and both directions — one word reduces,
two words do not.  Suite: **875 tests green**.

**A wrong diagnosis, corrected the same day.**  `en "banana"` (`ba#n'A:n@` in C,
`ban'A:n@` here) looked like the same stress-level question as §3c #76, since
English `a` carries `ChangeIfDiminished(a#)`.  It was not: the `a#` comes from a
*rule* gated on `$w_alt3`, and the port was passing no dict flags into the rule
matcher at all.  See §3f; #76 (the Polish `a#`) stays open on its own merits.

## 3f. Rule conditions and the Spanish-group flags (2026-10-11)

Three fixes found by following §3 #18 further.  Sweep **469 → 472 / 626**, no
case regressing.

**`$w_alt<n>` could never fire.**  C passes the flags from the *list* lookup
into `TranslateRules` (`dict_flags[0]`); the port passed a literal `0`, so a
rule condition asking "does this word carry `$alt<n>`?" always failed.  English
reads `banana` and `cathedral` through exactly that mechanism — `en_list` holds
`banana $alt3` with no phonemes, and `en_rules` has an `a → a#` rule gated on
`$w_alt3` — so the port said `ban'A:n@` where upstream says `ba#n'A:n@`.
Fourteen languages use the condition (ca 89 rules, en 61, nl 8, pt/eu/sl/sr/bs/hr
~5 each).

**`$list` and `$p_alt<n>` were stubbed out.**  C's `DollarRule` looks the word
*up to the current match point* up in the list: `$list` succeeds if that prefix
is an entry (and not `$only`), `$p_alt<n>` if the entry carries `$alt<n>`.  The
port failed both outright, "to avoid false positives".  Danish uses `$p_alt` in
**296 rules** to place the stress of Latinate words its list knows, and all
eleven probed (`fatal`, `biograf`, `biografi`, `monumental`, `specialist`,
`national`, `central`, `total`, `krystalklar`, `regional`, `anale`) were wrong —
`f'?&t?&l` for `f?&t'?&l`.  All eleven are now byte-identical, and the Danish
sweep sentence with them.  `RuleContext` gained `word_start`, since the lookup
text starts at the word's first letter and "after the last space" is not that
when a suffix re-translation has put earlier letters in the buffer.

**Catalan lost two stress flags to a leftover arm.**  `stress_flags_for` has the
transcribed `tr_languages.c` values, but a later match in `for_lang` — a
survivor of the older ad-hoc table — reassigned `es | an | ca | ia | pap` to
Spanish's own flags, dropping Catalan's `S_NO_AUTO_2 | S_FIRST_PRIMARY` and
Papiamento's `S_NO_AUTO_2 | S_FINAL_VOWEL_UNSTRESSED`.  So every Catalan word of
three syllables or more picked up an automatic secondary stress, which then
blocked the vowel reduction that depends on the syllable being unstressed:
`ˌentənˈɛm` for `əntənˈɛm`, `bˌarsəlˈonə` for `bərsəlˈonə`.  Eight Catalan words
and the sweep's Catalan sentence are now byte-identical; es/an/ia are untouched
(the overwrite was their own value) and Papiamento gains its real flags.

One test expectation was corrected to upstream's own reading: `ca "3,14"` says
`kˌomə`, with the secondary stress `NUM_SINGLE_STRESS` gives a word inside a
number group (`tests/french_numbers.rs`).  New `tests/dollar_rule_conditions.rs`
(3) and `tests/spanish_group_stress.rs` (3) pin all three fixes, including the
languages that must *not* change.  Suite: **881 tests green**.

## 3g. `isFinalVowel`, `SFLAG_DICTIONARY` and priority stress (2026-10-11)

Three more mechanisms, found by sorting the sweep's remaining differences by
language (Turkish 11, Polish 8, Portuguese 8 — each a single cause).  Sweep
**472 → 481 / 626**, no case regressing.

**`isFinalVowel` stopped two phonemes ahead instead of at the word boundary.**
C scans to the end of the *word*: its last vowel is final, whatever the next
word begins with.  The port looked at `next`/`next2` and so read `tr "eksi
işareti"` as `Eks'i iSa*et'I` — it saw the following word's vowel and decided
the first `i` was not final.  Turkish turns every word-final `i`/`e`/`o` into
`I`/`E`/`O` this way, so the condition mattered for most of its symbol names.
Now that `Neighbours` carries the word's own next/previous vowel (§3e), the
condition is exact; `vowels_tracked` says whether the caller filled them, so the
synthesis stage keeps the old window and nothing else changed.

**`SFLAG_DICTIONARY` was hardcoded "off" for symbol names.**  C sets it when a
lookup returns `FLAG_FOUND` **and not** `FLAG_TEXTMODE`, and it suppresses the
stress-conditioned phoneme changes.  The distinction is visible in the data:
`pl_list` gives `@` as *phonemes* (`m'awpa`), so Polish's
`ChangeIfDiminished(a#)` must not touch it — while the typed word "małpa", read
by the rules, is `m'awpa#`.  `tr_emoji` gives `−` as *replacement text* ("eksi
işareti"), which is re-translated and does take the changes.  So the flag cannot
be a constant either way: `lookup_symbol_name` now returns it alongside the
phonemes, taken from the lookup that produced them.  (Hardcoding `true` was the
first attempt here, and it broke Turkish and Portuguese in exactly the way the
TEXTMODE rule predicts.)

**`S_PRIORITY_STRESS` was noted but never implemented.**  A priority stress
marker (`''`) replaces any other primary stress in the word; C *unstresses* the
syllable it displaced when the language sets `S_PRIORITY_STRESS` (Italian,
Portuguese) and only demotes it to secondary otherwise.  The port always demoted
to secondary, so `pt "símbolo"` carried two stresses (`s'i~Nb,ulU`) where
upstream has one (`s'i~NbulU`) — and every symbol name whose text contains such
a word inherited it.  `get_vowel_stress` now takes the language's stress flags.

New `tests/translation_given.rs` (4) pins all three, in both directions.
Suite: **885 tests green**.

**Found while probing, still open: the `$brk` pre-pause on a clause's last
word.**  A `$brk` word takes a short pause in front of it, and C omits that
pause when the word is the **last of its clause** — `it "perché no"` is
`_!pe@-*k,e n'O` in both, but `it "no perché"` is `n'O pe@-*k'e` in C and
`n'O_! pe@-*k'e` here; `pl "dom i"` likewise.  C's `FLAG_PAUSE1` branch has no
such test, so the suppression happens somewhere later in its phoneme-list
assembly; the mechanism is not yet identified.  Affects a handful of words per
language (`$brk` entries) and some of the sweep's remaining 17 pause/spacing
differences.

## 3h. Turkish casing, the length mark, and an invented rule removed (2026-10-11)

Turkish was the largest remaining cluster (9 of the sweep's 100 real
differences); all three causes were in shared code.  Sweep **481 → 485 / 626**,
no case regressing.

**The length mark was visible to the phoneme programs.**  C turns a
`phonLENGTHEN` into an `SFLAG_LENGTHEN` flag on the phoneme *before* it and
never puts it in the phoneme list, so a program's `nextPh`/`next2Ph` looks
straight past it.  The port kept it in the code list (it has to, to print `k:`),
and the neighbour window counted it as a phoneme — so Turkish `teşekkür` read
`teSEk:'YR`: the `e` before `kː` saw "two non-vowels" where C sees the vowel
after it.  `resolve_virtual_phonemes_in_codes` now skips it, as C's list does.

**`langopts.dotless_i` was not implemented.**  C's `towlower2` maps `I` to the
*dotless* `ı` for Turkish and Azerbaijani — the only two languages that set the
flag — and `İ` to a bare `i` (the simple Unicode mapping).  The port used Rust's
`char::to_lowercase`, which gives `i` for `I` (so `tr IŞIK` read `iS'Ik` instead
of `@S'@k`) and `i` + a combining dot for `İ` — and that dot broke the next
rule match, so `tr İstanbul` lost its `s` and read `itanb'UL` for `Ist'anbUL`.
New `lowercase_for_lang` is used for the word itself and for the next-word
opening the rules' post-contexts read.

**An invented Turkish rule is gone.**  The port had its own "upgrade e→E, o→O
under primary stress" for Turkish, applied to every word, with a comment saying
C has no `LOPT_ALT` for it.  C's changes live in the `e`/`o`/`i` phoneme
programs and fire on `isFinalVowel` or "the next two phonemes are not vowels" —
conditions the port now evaluates correctly (§3e, §3g and the length mark
above), and which the blanket upgrade could not reproduce: `dokuz` is `d'okUz`
and `teşekkür` is `teSek:'YR`.  Removing it also stopped it from firing on
dictionary pronunciations, where C's `isTranslationGiven` gate keeps it off:
`tr b` (the letter name, `b be` in `tr_list`) is `b'e`, not `b'E`.

28 Turkish words were probed before and after, plus the sweep's Turkish
sentence; `tests/turkish_casing.rs` (3) pins the three mechanisms.
Suite: **888 tests green**.

**Found while probing, still open: the clause-initial pause before a number.**
`hu "5+3"` is `_! 'Yt pl'us h'a:R2om` in C and `_!'Yt …` here — C prints the
vowel-pause as its own unit before a *number*, so the word boundary falls after
it, while a typed `hu "öt"` is `_!'Yt` in both.  Four of the sweep's remaining
17 pause/spacing differences are this, all Hungarian numbers.

## 3i. A clause boundary is a pause, not a gap (2026-10-11)

Two fixes with one cause: in C a clause boundary **is a pause phoneme** in the
phoneme list, while in the port it is a boundary *marker* that the passes walked
straight past.  Anything that asks about neighbours therefore looked through the
comma.  Sweep **485 → 490 / 626**, no case regressing.

**The phoneme programs saw across the boundary.**  Spanish `b` is a stop after a
pause and a fricative after a vowel, so `es "a , b"` is `'a | b'e` in C and was
`'a | B'e` here — the `b` saw the `a` of the previous clause.  Same for
`es "Vamos. Bien"`.  `resolve_virtual_phonemes_in_codes` now treats a clause
boundary between two phonemes as "no neighbour" (code 0, which resolves to a
pause), for `prev`, `next` and `next2` alike; the vowel scans added in §3e
already stopped there.

**Regressive voicing propagated across the boundary.**  C's
`SetRegressiveVoicing` resets the voicing in force at a pause (`type == phPAUSE`
→ `voicing = 0`), which is what devoices a Polish word before a clause that
starts with a voiced consonant: `pl "flag. Zażółć"` is `fl'ak | z'aZuwts;`,
where the port kept `fl'ag` because it could see the `z` across the boundary.
The port's boundary branch now resets the voicing for a clause boundary, leaving
the `0x04`/`0x100` word-boundary bits exactly as they were.  Verified on
pl/ru/cs/de/nl/uk, and the isolated cases that already worked (`pl "minus b"`,
`pl "w prawo"`) are unchanged.  New `tests/clause_boundary_context.rs` (2) pins
both directions.  Suite: **890 tests green**.

## 3j. Numbers: the conjunctions, the year form, and Portuguese (2026-10-11)

Four number-grammar gaps, found by following the sweep's Portuguese cluster.
All four are `tr_languages.c` flags the port either lacked or applied to the
wrong set of languages.  The sweep is unmoved at **490 / 626** — its corpus is
UDHR sentences and symbols, with almost no numbers in it — so everything below
is measured by direct probe against the oracle instead.

**Portuguese cardinals were spelled out as text.**  The note in the code said
the pt dictionary "lacks number keys" — it does not: `pt_list` holds a complete
set behind `?1` (European) and `?2` (Brazilian) conditions, which the port now
honours.  Generating the words instead meant reading them with the
letter-to-sound rules, so every number differed from upstream in vowel quality
and secondary stress (`14` was `k,&t'o*@-zy` for `k&t'o*zy`, `5` `s'i~NkU` for
`s'i~Nku`).  Reading them from the list instead took a 30-number probe from
**10/30 to 24/30**, and with the conjunctions below to **31/32**.

**The conjunction after a *scale* word was missing everywhere.**  C's
`LookupNum3` takes `_0and` when the group has hundreds of its own *or* when it
follows a higher scale (`(value > 100) || (control & 1 && thousandplex == 0)`).
The port only had the first half, so even English was wrong: `en 1001` read
"one thousand one" where upstream says "one thousand **and** one".  Fixed for
both conjunction flavours (unconditional, and C's `NUM_SINGLE_AND`
single-word-remainder variant).

**`NUM_1900` was applied as a universal rule.**  C reads 1900–1999 as "nineteen
hundred (and) …", and only in the ten languages that set the flag (af, da, en,
fo, is, kl, nb/no, nl, sv).  The port read *every* round hundred from 1100 up
that way, in *every* language — `en 1500` was "fifteen hundred" for upstream's
"one thousand five hundred", `de 1100` "elfhundert" for "ein tausend ein
hundert", `fr 1100` "onze cents" for "mil cent" — and, because it only handled
round hundreds, it missed the year reading proper: `en 1984` was "one thousand
nine hundred and eighty four", not "nineteen hundred and eighty four".

**Eleven languages were missing their conjunction flags.**  C's
`NUM_HUNDRED_AND` set is twenty languages; the port configured nine.  Added:
ar, bg, eu, fa, he, io, ku, om, tn (and da/fo/kl/nb/no/is through their own
arms), with `NUM_AND_UNITS` for the tens→units conjunction in ar, bg, fa, he,
io, ka, ku, mk, tn — Arabic also has `NUM_SWAP_TENS`, so its units come first
("arba'a **wa** thalathun").  Also corrected: Kurdish-style `kl` moved out of
the tens-first Finnish/Estonian arm (it is units-first, like Danish), and
`NUM_OMIT_1_HUNDRED` for fo/kl/sl.  Measured on `{21, 101, 1984}` across 41
languages: **92/126 byte-identical**, where the nine languages touched here
were wrong on almost every case before.

Three test expectations were corrected: `tests/portuguese_numbers.rs` (both
tests) and `portuguese_number_words_keep_final_stress` asserted that a digit
reads *identically* to the spelled-out word — upstream itself does not do that
(`pt "1"` is `ˈum` from the list, `pt "um"` is `ˈũm` from the rules), so they
now check upstream's own readings for each form.  The `nl`/`mt` shared-arm
assertion gained `year_1900` to its normalisation list.

## 3k. Apostrophes, and Serbo-Croatian's second script (2026-10-11)

Two input-handling rules from C's `TranslateClause`, both found in the sweep's
word-count bucket.

**Serbo-Croatian is digraphic.**  C gives `hr`, `bs` and `sr` the *same*
dictionary (`strcpy(tr->dictionary_name, "hbs")`) and declares Cyrillic as
`langopts.our_alphabet`, so all three read both scripts.  The port listed only
`sr` as digraphic, so `bs`/`hr` Cyrillic text fell through to the script-switch
machinery: the Serbian pangram came out spelled letter by letter with runs
routed to Russian (`l^'@ U b'@ & z'@ n'@ I (ru)f;i'erdZija#(bs) …`) where C
reads `l^'ub&znI f'En^eRdZ,Ij&`.  Adding bs/hr to the digraphic list fixed both
sweep cases.

**The typographic apostrophes were not mapped to `'`.**  C does this in its
character loop — U+2019, U+00B4, U+2032 and U+0092 all become a plain
apostrophe — and an apostrophe belongs to a word only *between* two letters
(`punct_within_word`), with `LOPT_APOSTROPHE` adding word-initial (bit 0) and
word-final (bit 1, Hebrew only) exceptions.  The port had neither rule, so:

* `en "don’t"` read as **"don" + a pause + "T"** — the smart quote is what
  ordinary prose actually contains, so this was a common failure;
* `hello'`, `dogs'`, `James'` were looked up *with* the apostrophe and fell
  through to the letter rules (`h'El0`, `d'0gs`, `dZ'eImEs` for C's `h@l'oU`,
  `d'0gz`, `dZ'eImz`);
* Irish `D’fhuascail` and Scottish `d’fhàg` split into two words with a pause.

A clause-initial apostrophe also loses its pause in C (`words[0].pre_pause = 0`,
"don't add extra pause at beginning of clause") while a real bracket keeps its
own — `'hello` has none, `‘hello` has `_:_:`.

The normalisation is applied to the **clause's own text only**, which is where C
does it: a `$textmode` replacement is re-translated without passing through that
loop, so Italian's `€ simbolo dell’euro` keeps its curly apostrophe and reads as
two words.  Scoping it that way came out of the sweep — the first, global
version regressed that one case while fixing the rest.

Sweep **492 → 493 / 626** (the two bs/hr cases landed in the 492; the
apostrophe rules move text the corpus barely contains).  New
`tests/apostrophes_and_scripts.rs` (5).  Suite: **895 tests green**.

## 3l. Transliterating languages read their second script (2026-10-11)

C never switches translator by script — its `alphabets[]` table nominates a
language only for *naming* an unknown letter, and the Cyrillic and Latin rows
nominate none at all.  What makes a digraphic voice work in C is the
dictionary's own `.replace` table, applied **per character, before anything
else** in `TranslateClause`: Macedonian maps Latin to Cyrillic (`abc` → `абц`),
Lingua Franca Nova maps Cyrillic to Latin, Uzbek maps both ways (`Ўзбекча` →
`ózbekça`, and the quote in `o‘zbekcha` to the modifier letter `ʻ`).

The port ran that table *inside* `word_to_phonemes`, i.e. after it had already
decided the word was in a foreign script — so those words were spelled out
letter by letter with the language's own letter names (`mk "Dzidarski"` →
`d'@ z'@ I_:…` for C's `dz'id&Rsk,I`) or routed to Russian (`lfn`, `uz`).  Two
changes fix it:

* the script tests (`foreign`, the switch target, and the new
  `rules_know_word`) look at the **replaced** text, while the translation keeps
  the original — Icelandic's `.replace` is `x → xz`, which doubles if the table
  runs twice, and that is exactly what the sweep caught when the first version
  passed the replaced text down;
* `apply_replacements` now case-folds like C's `SubstituteChar`, which looks the
  *lower-case* character up and re-applies the case to the result, so
  `Dzidarski` transliterates as well as `dzidarski`.  Folding in **both**
  directions was the other thing the sweep caught: C folds only an upper-case
  character, and matching a lower-case input against Icelandic's upper-case keys
  inserted a stray phoneme in `öxi`.

New `rules_know_word` also guards the switch in general: a word the language's
own letter rules can read stays in the language.

Sweep **493 → 496 / 626**.  `tests/apostrophes_and_scripts.rs` grew a sixth
case.  Suite: **895 tests green**.

## 4. Open upstream PRs = language data / pronunciation the port would inherit as gaps

**Re-swept 2026-10-11** (re-check 4, live `gh api` listing — 90 open PRs; §5b is the
complete per-item inventory). Everything
opened since the 2026-09-03 sweep is below; the probes ran against the same oracle
build, so "matches C" means the port reproduces upstream's *current* behaviour, which
is the right answer for an unmerged PR.

| # | Ref (opened) | What it changes | Port status |
|---|---|---|---|
| 78 | #2542 (2026-09-11) | `fix(cmn)`: Mandarin tone contours in IPA — the Chao mnemonics (`55`, `35`, `214`) fall through the generic ASCII→IPA map, so different tones print the same or as unrelated vowels | 🟡 The port reproduces C exactly (`cmn 妈麻马骂` → `mˈɑ5 mˈɑɜ mˈɑ2 mˈɑ5` in both), so it will inherit the gap the PR fixes: IPA tone letters for every contour |
| 79 ✅ | #2506 (2026-08-22) | `EncodePhonemes` truncates everything after an unrecognised mnemonic in `[[ ]]` | **The port already behaves as the PR intends:** `[[mʊmbaɪ]]` → `mmb'a` here, `m` in C. Nothing to port |
| 80 | #2539 (2026-09-10) | `--ipa-sbr`: IPA syllable separator, stress on the syllable boundary rather than the nucleus, MOP/SSP syllabification | ❌ No equivalent; the port has no syllabifier |
| 81 ✅ | #2533 (2026-09-09) | New C API exposing word-level IPA phoneme pairs | **Implemented 2026-10-11** as `Translator::text_to_ipa_words` / `EspeakNg::text_to_phoneme_words` → `Vec<WordPhonemes { word, offset, length, phonemes }>`, plus `examples/word_phonemes.rs`.  See §3d |
| 82 | #2545 (2026-09-12) / #2544 | Russian HH:MM time reading | 🟡 The port matches C's *current* reading (`ru 10:30` → `d;'es;It; t@-r;'i_:tsVt;`), so it inherits whatever the PR changes |
| 83 | #2509, #2547, #2510, #2513, #2518, #2520, #2512, #2429, #2419, #2391, #2390, #2349, #2281, #2426, #2466 | Language *data* (hu, sq, ht, ca accents, ja, mn, es, pt, it, sv, fi, he, lv, ar/ur/fa) | ❌ Inherited by a data refresh, as the four merged ones were; nothing to port |
| 84 | #2262, #2277, #2325, #2413, #2447/#2462, #1937, #1589, #1724 | New languages (Toki Pona, Hadrianus, Sinago, Sicilian, Bété, Amazigh, Interlingue, de-ch) | ❌ Data; the port compiles any dictsource it is given (§1.5) |
| 85 | #2261 (2025-08-19) | Dictionaries move to `espeak-ng-data/dicts/` | ❌ Would change data-path resolution here (`resolve_dict_stem`) and the `data-crates/` layout — worth watching, not acting on |
| 86 | #2465, #2530, #2537, #2477/#2529, #2441, #2505, #2519, #2524, #2527, #2516, #2501 | C memory-safety, build and MBROLA/audio work | N/A to a pure-Rust port (the OOB reads and the zero-length-file crash cannot occur); #2524's `pitch=82` default and #2519's WAV-hash precision are synthesis details to re-check if the port's audio is ever re-calibrated |

| # | Ref | Area |
|---|---|---|
| 20 | #2280 | French: dedicated `ɥ` semivowel phoneme (currently mis-uses `y`). |
| 21 | #2268 | Japanese number pronunciation fixes. |
| 22 | #2269 | Bosnian "milijon" → "milion". |
| 23 | #2304 | Sindarin medial preconsonantal F. |
| 24 | #2261 | Dictionaries reorganized into `espeak-ng-data/dicts/` — affects data-path resolution. |
| — | #2426 | Latvian: inflections of ordinal numbers. |
| — | #2429 | Portuguese: language update (2026). |
| — | #2424 | Slovene: pronunciation of phoneme "v". |
| ✅ | #2423 | Dutch: na+CVC+ige mispronunciation — **merged upstream 2026-08-18**, inherited via the §3b data refresh. |
| — | #2419 | Italian: correct pronunciation rules. |
| ✅ | #2422 | Persian: language improvements — **merged upstream 2026-08-19**, inherited via the §3b data refresh. |
| — | #2466 | Arabic/Urdu/Farsi: diacritics + code-switched letters; corpus-attested vocalizations for 763 fallback words. |
| ✅ | #2435 | Normalize stylized Latin letters/digits to plain ASCII — **done 2026-08-01** (`stylized_to_ascii`: math bold/italic/script/fraktur/double-struck/sans/mono + circled + Letterlike holes). |
| — | #2413 | New language: Sicilian. |
| — | #2462 / #2447 | New language: Bété (ISO 639-3 `bof`). |
| — | #2472 | New voice variant "ali" (cf. §11 variants gap). |
| — | #2413 | New language: Sicilian (re-swept 2026-09-03, still open). |
| — | #2262 | New language: Toki Pona (barebones voice). |
| — | #1937 | New language: Amazigh (first implementation). |
| — | #1589 | New language: Interlingue. |
| — | #2325 / #2277 | New languages: Sinago / Hadrianus. |
| — | #2520 | Mongolian voice rebuilt against a measured speech corpus (pairs with the missing `mn` dict, §3b #44). |
| — | #2518 | Japanese pronunciation improvements. |
| — | #2510 | Haitian Creole: pronunciation rewrite (1979 orthography). |
| — | #2513 | Two new Catalan accents (alguerès, rossellonès). |
| — | #2509 | Hungarian: further pronunciation exceptions (follow-on to merged #2481). |
| — | #2391 | Swedish: comprehensive pronunciation improvements. |
| — | #2512 | Spanish pronunciation corrections (filed as an issue). |
| 🔧 | #2524 | **Stale `pitch=82` default in the `formant_factor` calculation.** Directly relevant: `variant_scaled_pitch` (`engine.rs`, §11) hardcodes the same 82 — track and follow upstream's resolution. |
| 🔧 | #2506 | `EncodePhonemes` truncates on an unrecognised phoneme mnemonic — our `[[…]]`/`--sep` mnemonic paths should reject rather than truncate. |
| 🔧 | #2470 | Rescale event sample positions after libsonic compresses the buffer — applies to `with_post_tempo`/`with_post_pitch` (§8) + `synth_with_events` (§7): our post-tempo hook does **not** rescale word/sentence event offsets. |
| 🔧 | #2465 | soundicon: out-of-bounds when loading SSML `<audio src>` files (already tracked in §1.9 — bound the icon table + fuzz test). |
| 🔧 | #2523 / #2485 | Normalize Unicode (NFKC) before synthesis — we do `fullwidth_to_ascii` + `stylized_to_ascii` piecemeal; a general NFKC pass would subsume both. |
| 🔧 | #1466 | Parse language configuration options from a file at runtime — would replace hardcoded per-language `langopts` (cf. §13, #2476). |
| ⚠️ | #2261 | Dictionaries reorganized into `espeak-ng-data/dicts/` — **would break our data-path resolution and the `data-crates/` layout**; watch before refreshing data (§3b #44). |
| — | #2274 | Make libsonic optional (cf. §8, our tempo/pitch is built-in). |
| — | #2501 / #2516 | MBROLA sound quality; `mb-la1` pronunciation (cf. §1.4 — blocked on external DBs here). |
| — | #2441 / #2505 / #2519 | Upstream CI/testing: new dict fuzzer targets, diff-gated clang-tidy, skipping exact WAV hash checks where FP keeps excess precision (the last one is a useful precedent for our oracle-gated PCM comparisons). |
| — | #2497 / #2508 / #2525 / #2474 / #1709 | Android-only (slider dialog, word replacements, API 36, localization, modernization) — N/A. |
| — | #2288 | Run espeak-ng without setting path on Windows; `espeak_ng_Cancel` flush ordering (cf. §7 cancel semantics). |

## 5. Open upstream issues to track (correctness, not "missing feature")

**Re-swept 2026-10-11** (re-check 4, 32 open issues; §5b is the complete per-item
inventory). The ones opened since the
2026-09-03 sweep, each probed against the oracle. Three are upstream bugs the port
**does not have**, and one found a real defect here.

| # | Ref (opened) | Concern | Port status |
|---|---|---|---|
| 87 ✅ | #2551 (2026-09-15) | A one-letter word repeated more than three times loses every repeat after the third (`FLAG_MAX3` catches letters, not just symbols) | **Not reproduced:** `en-us "b b b b b b"` reads all six here, three in C. The port already does what the issue asks |
| 88 ✅ | #2577 (2026-10-04) | Russian decimal fractions with six or more places read only the last four digits | **Not reproduced:** `ru 0,123456` reads the whole fraction here ("сто двадцать три тысячи четыреста пятьдесят шесть миллионных"); C drops the leading digits |
| 89 ✅ | #2437 (2026-06-20), #2438 (2026-06-20) | Pashto `5` read as "carrot"; Nepali digits read in English | **Neither reproduced:** `ps 5` → `p'@ndz@` (C: `g'a:zra`), `ne 5` → `p'a:nc` and `ne 123` → `e:ks'aj@ t'e:Is` (C switches to English) |
| 90 ✅ | #2555 (2026-09-17) | Untagged Serbian Cyrillic is forced through Russian rules | **Found a defect here, now fixed:** an unrecognised letter made the port abandon the *whole word* — `ru "његов"` was silent (C: `'egVf`) and under a switching voice the bare `phonSWITCH` leaked (`en "његов"` → `_^_w#U`). The rules loop now skips the letter and keeps going, as C does, and both cases match or beat C. The issue's actual request — route Serbian-specific letters to `sr` — is a *divergence* from C and is not implemented |
| 91 ✅ | #2469 (2026-07-26) | `-v <unknown voice>` prints "Unknown phoneme table: ''" and segfaults | Crash N/A (memory-safe). The port's message was also unhelpful ("not yet implemented: … dict not found"); it now reports `voice not found: alex` |
| 92 | #2512 (2026-08-26) | Spanish: "scientific", "youtube"/"YouTube" mispronounced | 🟡 Reproduces **identically** (`sTjent'ifik`, `J^owt'uBe`, `J^'ow t'uBe` in both) — a data complaint the port inherits |
| 93 | #2498 (2026-08-12) | The automatic English fallback misses en-us's t-flap and reduced vowels | 🟡 Reproduces identically: `ru "better water"` → `(en)b'Et3 w'O:t3(ru)` in both, where direct `en-us` gives `b'Et#3 w'O:t#3`. Ties to #2479 (the switch target is hardcoded `en`, also identical here) |
| 94 | #2526, #2531, #2442, #2393, #2415/#2416/#2499 | Haitian Creole phonemes, Brazilian Portuguese regression, Vietnamese report, a missing `en_list` word, synthesis-quality proposals | ❌ Data or synthesis-quality work; #2442's Vietnamese tones/numerics is the one with engine content (§3b) |

| # | Ref | Concern |
|---|---|---|
| 25 | #2467 | Segfault on certain Greek letters (`ϖ` U+03D6) in `.replace` groups. ✅ **Defensive test added (2026-08-01)** — `tests/crash_safety.rs` runs `ϖ`/`ϖϖϖ`/`αϖβ` through the `el` translator; Rust is memory-safe so it returns cleanly (no panic). Also covers unknown/empty voice (#2469) + adversarial inputs (emoji, unterminated `[[`/brackets). |
| 26 | #1533 | SSML `<mark>` breaks number pronunciation (moot until SSML lands, §3). |
| 27 ✅ | #228 | CamelCase / mixed-case word splitting — **done 2026-08-01** (both branches). Lower→upper (`caps_word_split`): `CamelCase`→"Camel Case", `iPhone`→"i Phone". Upper→lower (`caps_run_ends_here`): splits at the last uppercase of a run followed by lowercase+letter — `HTMLParser`→"HTML Parser", `JABberwocky`→"JA Bberwocky", `IThink`→"I Think"; pure acronyms (`USA`, `HTML`, `IDs`) stay whole. Irish `ga` exempt (lower→upper); Dutch `nl` "IJ" digraph exempt (upper→lower); Lojban `jbo` uses the capital-stress path. |
| 28 ✅ | #96 | Number-suffix edge cases ("2st" → "secst") — **done 2026-08-01** (`is_ordinal_suffix` now validates a st/nd/rd/th suffix against the number via `english_ordinal_suffix`, so `2st`/`1nd`/`3th` are cardinal + stray word, not a mangled ordinal; genuine `1st`/`2nd`/`11th`/`21st` and other-language ordinals unaffected). |
| 29 ✅ | #257 | Mandarin "9" — **not reproduced 2026-08-01**: `cmn 9` → `tɕˈiou` ("jiǔ", correct); the port is not affected by the C bug. |
| 30 | #367 | English linking-r inconsistency. |
| 31 | #246 | Malayalam chillu-virama Unicode mapping. |
| 32 | #172 | Slow speech interruption — depends on async layer (§7). |
| — | #2476 | Missing `break` in `SelectTranslator()` silently gives one language another's `langopts` — add a fallthrough-proof language-selection test (cf. §13). |
| — | #2479 | Automatic script-based switch target hardcoded to `en` (→ en-gb), not configurable from API/CLI/voice file (cf. §15). |
| — | #2469 | `-v alex` → "Unknown phoneme table: ''" → **segfault**; a missing/empty phoneme table must resolve to a clean error, not a panic (cf. §15). |
| — ✅ | #2438 | Nepali (ne): digits — **not reproduced 2026-08-01**: `ne 123` → "ek saya teis" (Nepali), not English. |
| — 🚫 | #2437 | Pashto (ps): digit "5" — **blocked (missing data) 2026-08-01**: `ps` has a voice file (`lang/ira/ps`, `status immature`) but **no `ps_dict`** ships anywhere (data-crates or `espeak-ng-data/`), so the whole voice fails with "dict not found". Not a code bug — needs the Pashto dictionary data (dictsource not in this checkout). **Update 2026-09-03: unblocked upstream** — `ps` is one of the six new 1.53.0 languages and `dictsource/ps_*` now exists on master, so the data refresh (§3b #44) supplies the missing `ps_dict`. |
| — ✅ | #2464 | ja: emoji/symbol descriptions silent — **root cause was no katakana support; fixed 2026-08-01** (katakana folded to hiragana). Katakana emoji names (⭐"スター", 👍"サムズアップ") now spoken; the one remaining silent case (🔥) has a *kanji* description (火) — a separate kanji-reading gap. See progress log. |
| — | #2393 | "shit" missing from `en_list`. |
| — ✅ | #2340 | Dot kept after expanding an abbreviation — **done 2026-08-01**: a period after a title abbreviation (`Dr.`/`Mr.`/`St.`…) no longer inserts a clause pause (`suppress_abbreviation_periods`, English-gated); real sentence ends and ambiguous/trailing abbreviations keep their break. |
| — | #2517 (2026-08-29) | Upstream: `..` anywhere in the text "makes the speaker go nuts". **Port probe 2026-09-03:** no misbehaviour, but not identical either — `a..b` → `ˈeɪ dˈɒt bˈiː` (speaks "dot"), `hello.. world` → two clauses, `wait....` → `wˈeɪt`. Re-check once upstream lands a fix, then lock in with a test. |
| — | #2499 (2026-08-12) | Robotic/clicking quality in formant-synth consonant transitions (shared `ph_consonants`/`base2` tables). **Directly adjacent to §36** and to our spectral finding that Rust's /s/ region is brighter than the oracle (centroid 3145 vs 2254 Hz) — upstream is chasing the same artifact class. |
| — | #2498 (2026-08-12) | Automatic English fallback via the internal `translator2` doesn't match the direct en-us voice. Related to our just-fixed en-us phoneme-table bug (CHANGELOG "Unreleased") and to the missing script/language fallback (§3b #46). |
| — | #2515 (2026-08-28) | Proposal: a registry of language self-designations — would be a new data file to carry in `espeak-ng-data/`. |
| — | #2504 (2026-08-22) | MBROLA voice file `sw2` not found in any data path (cf. §1.4 `find_database`, our error is `MbrolaVoiceNotFound` — already clean). |
| — | #2490 (2026-08-02) | Feature requests: US/UK accent choice for embedded Latin words, SAPI5, emoji speed — the first ties to §3b #46 (per-script translation) and #2479 (hardcoded `en` switch target). |
| — | #2488 (2026-08-02) | Make emoji processing disableable — worth exposing as an option here too (our emoji path is always-on). |
| — | #2496 / #2489 / #2502 | Android dictionary import, Android stop-reading, Debian install — N/A to this port. |

### Upstream synthesis-quality proposals (align with §36 / §1)

| # | Ref | Direction |
|---|---|---|
| — | #2415 | LF (Liljencrants–Fant) glottal source + formant glissando (sigmoid/linear F1–F5 interpolation to kill micro-clicks) + LPC-calibrated bandwidth tables. Upstream's own statement of the §36 formant-quality gap — track as the reference direction for the coarticulation work. |
| — | #2400 | Per-phoneme pitch/duration embedded in `[[…]]`/SSML (e.g. `[[d(287.4)<40>'eI<860>]]`). Extends the port's already-implemented inline-phoneme support (§1.3) and ties to intonation (§1.1). |
| — | #2416 / #2378 | General TTS audio-quality tracking (context for §36). |

### 🔴 Critical: synthesizer audio is not intelligible

| # | Area | Finding |
|---|---|---|
| 36 | **Formant synthesis quality** | **Update 2026-09-04 — wrong render engine found and fixed.** `flush_run` routed a run through the **Klatt** synthesizer whenever any of its frames carried `FRFLAG_KLATT`. That flag only means "this frame carries the extra parallel-resonator parameters", and **every shipped `en` frame has it**, so the standard voices had been rendered by the wrong engine entirely (C uses Klatt only when the voice sets `klattv[0]`, i.e. `-v en+klatt`). Measured against a locally built upstream binary over an eight-sentence sweep (long-term-average spectral envelope, 400 Hz smoothing, level-normalized): **mean RMS difference 4.19 → 2.79 dB, mean correlation 0.938 → 0.945**; "hello world" alone went 14.17 → 4.61 dB (correlation 0.895 → 0.933). Per-sentence after the fix: hello world 4.61 dB/0.933, quick-brown-fox 1.43/0.981, she-sells-sea-shells 1.60/0.967, peter-piper 2.12/0.975, how-much-wood 5.15/0.793, testing-one-two 2.01/0.982, stitch-in-time 2.37/0.947, rain-in-spain 3.02/0.981. The worst remaining case ("how much wood…", a `w`/`ʊ`-heavy line) is the concrete next lead. Also aligned `frame_to_peaks` with C's `SetSynth` (heights run to peak **7**, not 6 — measurably neutral on the shipped data, where `fheight[7]` is 0) and fixed the `klattp2` index used for the voiced/noise decision (`Fric` is index 2; the port read index 1, which is `AVp`). **Prior status: substantially recovered — ASR 0.00 → ~0.34 mean, "hello world…" 0.00 → 1.00 (= oracle)** after the `InterpretPhoneme` port + `FRFLAG_VOWEL_CENTRE` split + consonant-WAV amplitude fix + VWLEND exit blend + **cross-phoneme run render** (see the ✅ notes below the staged plan). Remaining gap is **fricative/stop-heavy text** (the consonant WAV spectral character), now that coarticulation, the cross-phoneme render, and per-syllable break-aware `SmoothSpect` are all in place. **Originally found via the ASR harness:** Whisper transcribed the C oracle's audio at **0.77 mean word accuracy** (0.56–1.00) but the **Rust engine's audio at 0.00 across every sentence** — e.g. "the quick brown fox…" → *"I'm kidding, I can't do that so bad."* The core formant synthesis (`wavegen`/formant frequencies/`phondata` interpretation) produces the wrong spectral content, so phonemes are unrecognisable. This is the **single biggest synthesis gap** — larger than intonation/duration (which only tune already-wrong audio). Corroborated by the low energy-envelope correlation (~0.24) from `wav-analysis`. **Diagnosis so far** (task #9): (a) *not* caused by the intonation/duration changes — a monotone build is equally unintelligible; (b) the **phoneme codes are correct** (`-x` → `h@l'oU`), so it's purely sample-level rendering; (c) steady-vowel **spectra and spectrograms are roughly right** (formant peaks within ~6%); (d) **not** the HF artifact or pitch — low-passing to 3–6 kHz doesn't recover it. (e) energy envelope over time is also roughly right (the two-word blob structure matches); (f) the Rust vowel is **over-periodic** (autocorr 0.74 vs oracle 0.21). **⇒ ROOT CAUSE (confirmed from C source):** the C synthesizer's `DoSpect2`/`LookupSpect(this_ph, which, fmt_params, plist)` is **neighbour-aware** — it picks formant frames from the surrounding phonemes (`plist`), splits vowels into *start* (`which=1`) vs *body/end* (`which=2`), and carries `last_frame` across phoneme boundaries for continuity. The Rust `synthesize_phoneme_info` renders **each phoneme in isolation** (no neighbour context, no `last_frame`, no start/body split), so **formant transitions / coarticulation are not modelled**. Steady phoneme *bodies* are correct (hence right spectra/envelope) but the **transitions that carry consonant identity are missing** → unintelligible. **Fix (substantial):** port neighbour-aware `LookupSpect` frame selection + cross-phoneme `last_frame` continuity + the vowel `which=1/2` split into the Rust `synthesize` pipeline. **Staged plan below.** |

### Coarticulation port — staged implementation plan (task #9)

Derived from `synthesize.c` / `synthdata.c`. Each stage is a self-contained change; **ASR word-accuracy only recovers near the end**, so verify intermediate stages structurally (frame counts, no-panic, `wav-analysis`) and run the ASR harness after Stage 5.

1. ✅ **Neighbour context** — `PhonemeInfo` now carries `prev_type/prev_code/next_type/next_code` (threaded in `annotate_codes`, reset at pauses). *(done, 390 tests green)*
2. ✅ **`vowel_transition` extraction** — `bytecode::scan_phoneme` now unpacks `i_VOWELIN`/`i_VOWELOUT` into `vowel_transition[0..3]` (C `synthdata.c` formula), merged through `CALLPH`; unit-tested. `pd_FMT/VWLSTART/VWLEND` addresses were already extracted. *(done)* **Remaining for full parity:** `i_CONDITION` branch evaluation against neighbour types (context-dependent FMT selection).
3. **`LookupSpect` frame selection** — split a vowel's `SPECT_SEQ` at the `FRFLAG_VOWEL_CENTRE` frame: `which=1` → onset frames, `which=2` → body; append `fmt2_addr` (VWLEND / next-phoneme) frames; apply the `which=2` length normalisation to `std_length`. *(not yet done — current cut applies transitions to the whole seq without the onset/body split or timing)*
4. 🟡 **`FormantTransition2`** — ported (`unpack_transition`/`adjust_formants`/`set_frame_rms`/`apply_vowel_transition` in `synthesize/mod.rs`); vowel entry (`which=1`) bends `frames[0]`, exit (`which=2`) appends a bent duplicate of the last frame, using the prev/next phoneme's `vowel_transition`. Glottal-stop (`flags&8`) and vowel-colour cases not yet ported. *(done, 391 tests green)*
5. 🟡 **Vowel coarticulation wired** — the vowel path now bends boundary frames toward neighbours. **ASR effect so far: marginal** — some words now emerge (e.g. "…fox" → "Fox") but single words are still unintelligible ("hello"→"So"). **Remaining (the bulk of intelligibility recovery):** the `which=1/2` onset/body split with correct transition *lengths/timing*; and the **consonant cases** (`VSTOP`/`VFRICATIVE`/`LIQUID`/`NASAL` rendering their own frames + transitions) — consonant identity is still largely absent.
6. **Tune & verify** — iterate against the ASR + `wav-analysis` harnesses toward the oracle's ~0.77.

**Methodology note (important).** Stages 1–2 + the `FormantTransition2` port (Stage 4) and vowel wiring (Stage 5, vowels only) are landed and safe (391 tests green), but the audio is **still unintelligible** and a first `last_frame`-continuity attempt (naive prepend) made it *worse* and was reverted. The lesson: this DSP is too intricate to port by reading C and checking only the **coarse** ASR/acoustic harnesses — each change is roughly as likely to hurt as help, so guess-and-check does not converge. Finishing this reliably needs **sample-exact verification**: instrument the C `synthesize.c` (`DoSpect2`/`FormantTransition2`) to dump the generated frame sequence for a test word, and match the Rust frames to it one-for-one. The `last_frame` param is left threaded (unused) as the extension point.

**✅ Frame-dump verification tool built (task #9a).**
- `scripts/dump_c_frames.sh <src> "<text>"` — patches the C `DoSpect2` to append per-phoneme `which`, `nf`, and per-frame `len/rms/ffreq[0..6]` to `$ESPEAK_DUMP_FRAMES` (idempotent, rebuilds in place; env-gated so the oracle is unchanged when unset).
- `ESPEAK_RS_DUMP_FRAMES=<file>` — the Rust synth writes the same format (`synthesize/mod.rs`, env-gated).
- `scripts/compare_frames.py c.txt rust.txt` — side-by-side per-phoneme diff.

**Divergences it immediately revealed for "hello"** (the concrete to-do list for finishing the port):
1. **Two-pass structure missing** — C emits `@ which=1`(onset)+`which=2`(body), `oU which=1/2`; Rust emits one merged block per phoneme (Stage 3).
2. **Formant targets ~5% off** — C `@` F1–F3 `525,1441,2340` vs Rust `552,1441,2400`.
3. **Frame-length scale mismatch** — C `@` `len=50,90` vs Rust `len=3,3,2` (a units divergence in the length field, distinct from the ms↔STEPSIZE fix for total duration).

With this tool the remaining stages become checkable frame-by-frame instead of guessed against fuzzy ASR.

**Tool-verified root blocker (task #9b).** Using the frame dump on "quick" (`k w I k`): the C `I` vowel is a **7-frame** sequence — onset `836,1177,2013` (F2 rising) then body `1940…` — whereas the Rust reads a **body-only 4-frame** FMT (flat F2 ≈ 1940). The onset is **not** from the neighbour (`w` has empty `vowel_transition`) nor a `pd_VWLSTART` (the `I` phoneme has none); it lives in `I`'s own `pd_FMT` SPECT_SEQ *before* `FRFLAG_VOWEL_CENTRE`. **The Rust `scan_phoneme` selects the wrong FMT variant** because it takes the first/unconditional `i_FMT` and does **not evaluate `i_CONDITION` branches** against neighbour context (cf. the earlier `@2`-vs-`@` variant mismatch). ⇒ **The precise next step is a context-evaluating phoneme-program interpreter** (`InterpretPhoneme`): evaluate `i_CONDITION`/`i_JUMP` against prev/next phoneme type & flags to pick the correct `pd_FMT`, then the `FRFLAG_VOWEL_CENTRE` `which=1/2` split renders the onset+body. Building blocks already landed and safe (391 tests): neighbour context, `vowel_transition` extraction, `FormantTransition2`/`adjust_formants`, `pd_VWLSTART` rendering, `last_frame` plumbing.

**Airtight confirmation (program dump of `I`, task #9b).** `I`'s bytecode is: `SET_LENGTH; FMT(default); <i_CONDITION>; JUMP_FALSE; FMT(alt); … ; FMT(alt2)` — i.e. a default FMT **followed by condition-gated alternate FMTs**. The C `InterpretPhoneme` runs the *whole* program, so a true condition's `FMT` **overrides** `sound_addr[pd_FMT]` (last write wins). The Rust `bytecode::scan_phoneme` **`break`s at the first `i_FMT`**, so it always takes the default and never the context-correct override. **The concrete fix is to replace `scan_phoneme` with a faithful `InterpretPhoneme`:** walk the full program; on a condition sequence (`0x2xxx`/`0x3xxx`) evaluate `InterpretCondition` (prev/this/next/next2/nextVowel/prevVowel × mnemonic / start_type / end_type / phoneme-flags) and honour `i_JUMP_FALSE`; keep the last-reached `pd_FMT/WAV/VWLSTART/VWLEND`. That needs the phoneme-*list* context (neighbour `start_type`/`end_type`/flags + word-boundary `sourceix`), most of which `PhonemeInfo` already carries. This is a ~200–300-line but now **fully-specified and frame-verifiable** port — the right dedicated next unit.

**Confound ruled out + gap narrowed (task #9b, matched data).** Running the Rust frame dump against the *same 1.53 data* as the C oracle: the Rust `I` **body frames match C's `which=2` essentially exactly** (`192,408,1940,2600,3420,3960,6496`). So the vowel *body* SPECT_SEQ selection/parse is **correct** — the only missing thing is C's **`which=1` onset pass** (`836→1177→2013`), rendered by C as a separate DoSpect2 call over the vowel's onset frames. ⇒ The port reduces to: in `InterpretPhoneme`, produce the vowel's onset (`which=1`) frame set in addition to the body (`which=2`) — i.e. the `FRFLAG_VOWEL_CENTRE` split (or the correct onset `pd_FMT`) — then render onset+body. Consonant cases follow. (Partial intelligibility already leaks through: "…fox" → "box".) Verify each step with `scripts/dump_c_frames.sh` (use matching data on both sides).

**✅ Vowel onset coarticulation landed (task #9, frame-verified).** Implemented `bytecode::select_vowel_start` — evaluates a consonant's `SwitchNextVowelType` table against the following vowel's `start_type` (C `SwitchOnVowelType`, `phonVOWELTYPES=28`) to pick the onset `VWLSTART` sequence. The vowel path now prepends that onset ahead of the body and gives it its own ms-based duration (excluded from the body's `std_length` scaling). **Frame-exact match to C:** e.g. `quick`'s `I` now renders the onset glide `F2 = 836→1177→2013` (was flat `1940`), matching the oracle's `which=1` pass. 391 tests green, no regressions. **ASR still 0.00, though** — the onset alone doesn't restore intelligibility; the remaining pieces are: **(a)** onset timing not yet exact (Rust `len≈14` vs C `40`, since our raw VWLSTART frame lengths differ from C's post-`LookupSpect` values); **(b)** vowels after non-`SwitchNextVowelType` consonants (e.g. `l`→`oU`) still lack an onset; **(c)** the vowel **exit** (`which=2` end-transition) and **(d)** the **consonant** phonemes' own rendering. Intelligibility will cross zero only once (b)–(d) are also done. All frame-verifiable via the dump tool.

**a-b-c done; d is the remaining unit.** (a) **onset timing** — onset frames now keep their own ms-based duration (excluded from body scaling); duration matches C (dump shows `len=14` vs `40` but `14×64 ≈ 40×22.05` samples). (b) **onset fallback** — vowels without a `SwitchNextVowelType` consonant now get `FormantTransition2(which=1)` bending toward the previous consonant's VOWELIN locus. (c) **vowel exit** — `FormantTransition2(which=2)` toward the next phoneme's VOWELOUT. All landed, 391 tests green. **(d) consonant rendering is the blocker and it reduces to the `InterpretPhoneme` condition port:** dumping `l`'s program shows `condition(2b02) … conditions(3702,3039,203a)+jumps … CALLPH(9100)` with **no plain FMT** — the formants come from a condition-selected procedure call. `scan_phoneme` doesn't evaluate conditions, so it renders the wrong path (`l` F2 `1620` vs C `1380`). **ASR stays 0.00 until (d) lands**, because Whisper needs the consonants right too. (d) = the ~200-line `InterpretPhoneme`/`InterpretCondition` port (spec above) — the single remaining unit; every consonant depends on it.

**✅ FRFLAG_VOWEL_CENTRE split + interpreter-driven onset/exit — ASR 0.13 → 0.19, "hello world" → 0.83 (task #9).** Two follow-ups to the InterpretPhoneme port, both frame-verified against the C oracle on "hello" (matched 1.53 data):
- **Onset through CALLed procedures.** The vowel-onset selection now interprets the *previous* phoneme (with this vowel as its `next`), so a consonant's `NextVowelStarts` switch hidden inside a `CALL` (e.g. `l`'s `CALL vowelstart_l`) resolves — gated on `pd_FORNEXTPH`. `oU`'s onset in "hello" now renders `300,1300,2900 → 440,1300,2900 → 525,1386,2900`, exactly C's `which=1` pass.
- **`FRFLAG_VOWEL_CENTRE` split (C `LookupSpect`).** A vowel's own FMT sequence embeds onset frames before a centre-marked frame; the body (`which=2`) starts at the centre. Rust previously used the whole sequence as the body (an extra pre-centre frame, e.g. `oU`'s stray `550,1397,2420`). Now: `seq_break` = last `FRFLAG_VOWEL_CENTRE` frame; body = `frames[seq_break..]`; pre-centre frames are the intrinsic onset (`use_vowelin` case) when no external VowelStart applies. `oU` body is now `630,1353,2340 … 390,957,2140` — exactly C's `which=2`. The vowel **exit** also now uses `interpret_phoneme(next)` for a context-correct VOWELOUT transition.

**Result:** `oU` in "hello" is frame-exact (3 onset + 5 body, formant values matching C). **ASR 0.13 → 0.19 mean; "hello world how are you today" 0.33 → 0.83** (vs oracle 1.00). 408 tests green, no commit.

**Next blocker (identified, task #9 continued): consonant rendering.** With coarticulation frame-exact, the remaining ASR gap concentrates on **fricative/stop-heavy** sentences — "hello world" (sonorant+vowel) scores 0.83 but "she sells sea shells" (`s ʃ`) and "the quick brown fox…" (`k t p f`) stay ≈0.00 and transcribe as sibilant gibberish, while sonorant-rich text is intelligible. Full-length audio (durations are ≥ C), so it is quality, not truncation. Candidates: (1) the voiceless-stop/fricative WAV path (`sample::parse_wav_sample`) — spectral shaping/amplitude/mixing vs C `DoSample2`; (2) the **vowel-exit `fmt2_addr` (VWLEND) blend** — when the next consonant specifies a `VowelEnding` (e.g. `l`, via `PrevVowelEndings`), C appends its VWLEND SPECT_SEQ to the vowel tail (`interpret_phoneme(next, prev=this).vwlending_addr` already resolves it); Rust currently only applies the `use_vowelin` transition, so vowels before such consonants lack the exit glide; (3) **`SmoothSpect`** (synthesize.c) — the per-syllable formant-rate limiter that caps inter-frame frequency change — is **not ported**, so rapid transitions may chirp across whole utterances. All three are frame/spectrum-verifiable.

**✅✅❌ Attacked (1)(2)(3); (1) and (2) held — ASR 0.19 → 0.32 (task #9).**
- **(1) ✅ Consonant WAV amplitude — FIXED.** `sample::parse_wav_sample` scaled samples as `(raw*89)>>8 ≈ raw*0.35` and ignored the phoneme's WAV amp param; C `PlayWave` uses `raw*consonant_amp(26)*general_amplitude(55)>>10 * amp2/32` where `amp2=(wav_param?wav_param:100)*32/100` (`DoSample3`) — i.e. `≈ raw*1.40` for default params, so voiceless stops/fricatives were **~4× too quiet**. Ported the exact formula and threaded `extract.wav_param` through. RMS of "she sells sea shells" now 3449 vs oracle 2906 (was far lower), peak 18895, **0 % clipping**. ASR "fox" −0.11→0.11, "she sells" 0.00→0.12; **mean 0.19 → 0.23**.
- **(2) ✅ VWLEND `fmt2` exit blend — FIXED (second attempt).** The first attempt regressed because it let the appended VWLEND frames be scaled by the vowel-body duration normalisation.  C `LookupSpect` computes `length1` and applies the which=2 `length_factor` **before** appending `fmt2`, so the VWLEND frames keep their own lengths (only `fmt2[0]` sets the last body frame's length).  Reproduced that: the exit now resolves the VWLEND SPECT_SEQ (vowel's own, else `interpret_phoneme(next, prev=this).vwlending_addr` — the next consonant's `PrevVowelEndings` glide selected by this vowel's end_type), sets the last-body length from `fmt2[0]`, appends `fmt2[1..]` as a `tail_count` region **excluded from body scaling** and given its own ms→STEPSIZE durations.  **ASR 0.23 → 0.32; "hello world how are you today" → 1.00** (= oracle). Synthesis verified deterministic.
- **(3) ❌ `SmoothSpect` — deferred (two attempts).** A per-phoneme formant-rate limiter (`smooth_spect(frames, start)`, retained `#[allow(dead_code)]`) regressed ASR both whole-phoneme (0.32 → −0.08) and body/exit-only (0.32 → 0.14): a per-phoneme pass flattens the intended onset/exit **glides** (their frames are short, so the rate cap fires on them).  C avoids this by smoothing per *syllable*, anchored on the stressed-vowel centre and spanning phoneme boundaries via the wcmdq with final sample-unit lengths.

**Lesson:** (3) genuinely requires the **cross-phoneme (syllable-level) render path** — collect a syllable's frames across phoneme boundaries, render them together threading `last_frame`, and smooth over the whole run.  That path is now the single remaining structural gap; (3) becomes a faithful port once it exists, and it would also fix the boundary formant discontinuities the per-phoneme renderer leaves.

**✅ Cross-phoneme render path built — ASR 0.32 → ~0.34 (task #9).** Refactored the pass-2 loop from per-phoneme PCM to a **run-based** render: `synthesize_phoneme_info` now returns `PhonemeRender::{Frames{frames,amp_factor} | Pcm}`; consecutive formant phonemes accumulate into a `run` (with per-frame amp + per-frame pitch arrays) that a WAV consonant or any silence flushes.  `flush_run` renders the whole run with the new `wavegen::synthesize_frames_seq` (per-frame amp/pitch), so adjacent phonemes' frames **interpolate continuously across the boundary** instead of each phoneme starting/ending flat.  Frame-exactness preserved (`oU` in "hello" still onset `300,1300,2900`→body `630,1353,2340…`); "hello world…" holds 1.00; 408 tests green; synthesis deterministic.  ASR is within the base-model noise band (0.29–0.34 across whisper runs; synthesis itself is byte-identical run-to-run) but shows no regression and the boundary continuity is architecturally correct.

**✅ `SmoothSpect` landed — per-syllable, break-aware (task #9).** The run-wide attempts failed for two reasons, both now fixed:
1. **Runs span multiple syllables.** `flush_run` now splits a run at the **midpoints between adjacent vowel centres** (each vowel's highest-RMS frame, recorded in `run_centres`) and smooths each syllable segment outward from its own centre — mirroring C's per-`syllable_centre` smoothing.
2. **Missing FRFLAG handling.** The bidirectional pass now honours the frame flags exactly like C: it **stops at `FRFLAG_BREAK`** frames (preserving intended discontinuities), skips F1–F3 for `FRFLAG_BREAK_LF`, and widens the allowed rate (`×12/10` back / `×6/5` fwd) for `FRFLAG_FORMANT_RATE`.  Without the break-stop the smoother flattened the onset glides (that was the earlier regression).

Result: **frame-exactness preserved** (`oU` in "hello" still onset `300,1300,2900 → 440,1300,2900 → 525,1386,2900`), "hello world" holds 1.00, 408 tests green.  **ASR is neutral** (0.29–0.31 with vs 0.29–0.34 without — fully overlapping at the base-whisper noise floor): the current intelligibility bottleneck is fricative/stop-heavy content, not chirp, so the rate-limiter (a chirp-reduction quality feature) doesn't move the metric — but it is now the faithful C behavior and is kept enabled.

**✅ (d) InterpretPhoneme condition-evaluation port landed — ASR crosses zero (task #9).** Ported `InterpretPhoneme` + `InterpretCondition` + `SwitchOnVowelType` + `StressCondition` (synthdata.c, synthesis stage `tr==NULL, control==0`) into `bytecode::interpret_phoneme` (`Neighbours` window + a phoneme-table lookup closure). It walks the full program, evaluates `i_CONDITION` sequences (boolean accumulator, `i_NOT`, `i_JUMP_FALSE`, ELSE jumps) against prev/this/next/next2 (phoneme-code / start_type / end_type / phoneme-type / place / phflags / the `isVowel`/`isPause`/`isVoiced`/`isBreak`/`isWordStart` "other" conditions), honours the `CALLPH` return stack, and keeps the last-reached `FMT/WAV/VWLSTART/VWLEND`. Wired into `synthesize_phoneme_info` (replaces `scan_phoneme` for the current phoneme). Word-start flags (`sourceix`) for the `*PhW` variants are computed in `annotate_codes`. **Frame-exact fixes (verified via `scripts/dump_c_frames.sh` on matched 1.53 data):** `l` in "hello" now renders `F1-3 = 360,1380,2680` (was the dark-`l` `1620`) — the interpreter evaluates `nextPh(isNotVowel)=false` and reaches `FMT(l/l)`. **Onset glides through CALLed procedures:** because `interpret_phoneme` follows `CALLPH` and `SwitchOnVowelType(instn_type=2)` (setting `pd_FORNEXTPH`), the previous consonant's VowelStart-for-next now resolves even when hidden in a called procedure (e.g. `l`'s `CALL vowelstart_l`) — `oU`'s onset in "hello" is now `300,1300,2900 → 440,1300,2900 → 525,1386,2900`, matching C's `which=1` pass exactly. The vowel branch mirrors synthesize.c: vowel's own VowelStart unless `pd_FORNEXTPH`, else prev consonant's `pd_FORNEXTPH` VowelStart, else `use_vowelin` transition. **ASR: 0.00 → 0.13 mean word accuracy** (hello 0.00→0.33; "please call stella" 0.00→0.30), vs oracle 0.77. 408 tests green, no regressions. **Remaining gap to 0.77 (broader synthesis fidelity, still task #9):** the vowel **exit** blending toward the next phoneme's `pd_VWLEND` frames (C `DoSpect2 which=2` fmt2 with `FormantTransition2`) is only approximated (vowel_transition only, no fmt2 SPECT_SEQ blend); frame **timing distribution** across onset/body/exit isn't sample-exact; and consonant WAV (stop/fricative) rendering fidelity. These are frame-verifiable via the dump tool.

### Discovered during porting (base-pipeline divergences vs C oracle)

| # | Area | Divergence |
|---|---|---|
| 33 | Article "a" | Standalone `a`/`A` renders as the letter-name `ˈeɪ`; oracle reads the article `ɐ`. Found while verifying SSML entity decoding (`x &#65; y`). **Investigated 2026-08-01:** `lookup("a")` returns a single en_dict entry → phoneme code 147 (citation `eɪ`), whereas `the`/`an` are stored pre-reduced (`D@`, `a#…n`). So this is **not** a spelling-path bug — the fix is unstressed **function-word vowel reduction** (or honouring a `$only`-gated second "a" entry). The reduction machinery flags (`FLAG_ONLY`/`only_form`, `dict_condition`) are already parsed in `lookup.rs`; the gap is applying them / the reduce pass. **Needs the C oracle to confirm the exact reduced vowel (`a#` vs `@`) before landing** — deferred under the best-effort loop. |
| 34 | Symbol names | ✅ **Fixed** (§14) — `&`→"and", `%`→"percent", etc. now read by name, 9/10 byte-identical to oracle. |
| 35 | Sonorant durations | ✅ **Fixed via `wav-analysis`** — nasals/liquids were ~2.9× too long because the non-vowel path multiplied the SPECT_SEQ `length` (which is in **milliseconds**, per C `DoSpect2`) by STEPSIZE 64 instead of `samplerate/1000`. Sonorant-heavy phrases went **2.07→1.12** vs oracle. **Then the residual was addressed:** added the oracle's ~0.30 s **end-of-clause trailing silence** (short clips no longer end abruptly — "cat" 0.52→**1.00**, "hello world" 0.64→**0.88**), reduced the inter-word gap 50→10 ms, and approximated final-lengthening by shortening non-final words. Typical sentences now **1.05–1.16** vs oracle (was up to 1.7×). `synthesize/mod.rs`, 3 harness tests. **Remaining:** true **per-syllable** final-lengthening isn't modelled, so single long words (e.g. "supercalifragilistic" ~1.29) and all-sonorant sequences still run somewhat long — a `CalcLengths` item. |
| 37 ✅ | Rules `.replace` section not applied | **Done 2026-08-01.** The engine parsed `.replace` (`Groups.replace_chars`) but never applied it. Now `decode_replace_table` decodes the `from`→`to` UTF-8 pairs and `Dictionary::apply_replacements` applies them (first-match-wins, per C `SubstituteChar`/`FindReplacementChars`) in `word_to_phonemes_inner` before dict/rule lookup. **Also fixed a latent alignment bug**: the `replace_chars` offset rounded *up* to 4 bytes; C rounds *down* (`(p+4)&~3`), so the table was read 4 bytes (one field) off — never noticed because it was unused. Applies to **59 shipped languages** (ja 190 pairs, ne 359, fa 174, el 139…). Subsumed the ja hand-port (katakana + voiced-kana now go through the real mechanism) and newly fixes e.g. English ligatures (`ﬁsh`→"fish", `encyclopædia`→"encyclopaedia"). Full suite green (no language regressed). `tests/replace_table.rs`. |

---

## 5b. Complete inventory of upstream's open work (2026-10-11)

Every open PR (90) and issue (32) at the re-check, so nothing is tracked only by
memory.  `gh api repos/espeak-ng/espeak-ng/{pulls,issues}?state=open`.  A ✅ means
the port already does what the item asks (or better); 🟡 that it reproduces
upstream's current behaviour and will inherit the change; ❌ that it is a gap or
does not apply.  Items with their own row in §3c/§4/§5 are cross-referenced there.

**Port-relevant** (25) — Code or behaviour the port can act on.

| Ref | Opened | Title | Port |
|---|---|---|---|
| #1466 (PR) | 2022-10-26 | Parse language configuration options from a file at runtime | 🟡 largely done — the port reads `stressRule`, `words`, `replace`, `phonemes`, `dictrules`, `brackets` from the voice file |
| #2261 (PR) | 2025-08-19 | Organize dictionaries in dicts subdirectory | ⚠ would change `resolve_dict_stem` and the `data-crates/` layout here |
| #2280 (PR) | 2025-09-12 | fix(lang-fr): Introduce dedicated phoneme ɥ for French semivowel | ❌ data (`ɥ`); tracked as §4 #20 |
| #2304 (PR) | 2025-10-14 | Fix Sindarin medial preconsonantal F | ❌ data |
| #2435 (PR) | 2026-06-19 | Normalize stylized Latin letters and digits to plain ASCII | ✅ done 2026-08-01 (stylized letters normalise) |
| #2506 (PR) | 2026-08-22 | Stop EncodePhonemes truncating on an unrecognised phoneme mnemonic | ✅ the port already skips a bad mnemonic and keeps going (`[[mʊmbaɪ]]` → `mmb'a`, C `m`) |
| #2533 (PR) | 2026-09-09 | feat: Expose word-level IPA phoneme pairs through the C API | ✅ done 2026-10-11 (`text_to_ipa_words`; §3d) |
| #2539 (PR) | 2026-09-10 | Add option --ipa-sbr for IPA Syllable Separator and added latin IPA test cases | ❌ no syllabifier |
| #2542 (PR) | 2026-09-11 | fix(cmn): preserve Mandarin tone contours in IPA output | 🟡 the port reproduces C exactly, so it inherits the gap (Chao digits, not IPA tone letters) |
| #2545 (PR) | 2026-09-12 | Add Russian HH:MM time pronunciation | 🟡 matches C's current reading; inherits whatever the PR changes |
| #2549 (PR) | 2026-09-14 | Fix stem loss after spelling leading letters | 🟡 the bug doesn't reproduce (`zzmcbugging` → `zmkb'VgIN`, C `z,Edz'Edz'EdIN`), but the port doesn't spell the leading letters either |
| #2552 (PR) | 2026-09-15 | dictionary: do not limit repeats of one-letter words | ✅ letters already unlimited here; **`$max3` now implemented for symbols** (§3c #80a) |
| #2558 (PR) | 2026-09-23 | uk: в as [ʋ]/[w]/[u̯]; я ю є soften a preceding consonant | 🟡 reproduces C (`uk вода` → `B'oda`, `любов` → `l^'uboB`); a rules PR the port inherits |
| #2559 (PR) | 2026-09-23 | te: anusvara takes the place of the following consonant | 🟡 reproduces C (`te సంతోషం` → `s'anto:s.an`) |
| #2560 (PR) | 2026-09-23 | ml: ഴ is the approximant [ɻ]; word-final virama is a short vowel | 🟡 reproduces C (`ml വഴി` → `v'ar.i`) |
| #2561 (PR) | 2026-09-23 | ta: ழ is the approximant [ɻ]; ட் is voiceless before a voiceless stop | 🟡 reproduces C (`ta தமிழ்` → `t'Vmiz.`) |
| #2562 (PR) | 2026-09-23 | hi: anusvara after short i/u before a stop is a nasal consonant | 🟡 reproduces C (`hi हिंदी` → `H'i~di`) |
| #2563 (PR) | 2026-09-23 | gu: stress by syllable weight so medial schwa is dropped; ળ, વ, ઝ | 🟡 reproduces C for stress, but `gu ગુજરાતી` → `g,uJ@3*'a:ti` vs C `g,uJr'a:ti` (§3c #81a) |
| #2564 (PR) | 2026-09-23 | pa: ਹ after a vowel is a tone, not [h] | 🟡 reproduces C (`pa ਪੰਜਾਬੀ` → `p@nJ'abi`) |
| #2565 (PR) | 2026-09-27 | ne: read digits in Nepali instead of switching to English | ✅ already correct (`ne 5` → `p'a:nc`, no English switch) |
| #2566 (PR) | 2026-09-28 | ps: restore emoji keys that had become digits | ✅ already correct (`ps 5` → `p'@ndz@`) |
| #2579 (PR) | 2026-10-04 | Fix Russian decimal fraction pronunciation | ✅ already correct (`ru 0,123456` read in full) |
| #2581 (PR) | 2026-10-05 | ru: keep the soft sign before a non-vowel (#1045) | 🟡 reproduces C (`ru большой` → `bVlS'oj`, `мальчик` → `m'AltS;ik`) |
| #2582 (PR) | 2026-10-08 | bn, as, bpy: handle consonant + nukta written as two characters | 🟡 reproduces C (`bn বাংলা` → `b'aNla`); the two-character nukta case is untested |
| #2583 (PR) | 2026-10-09 | kok: set up Konkani as a Devanagari language | ✅ the port already treats `kok` as Devanagari: `कोंकणी` → `ko~k'Vn.i:0` where C garbles it (`k,@o:k'a:rNk@n.ik'a:r`) |

**Language data** (20) — dictsource/phsource for a language the port already ships — inherited by a data refresh.

| Ref | Opened | Title | Port |
|---|---|---|---|
| #1460 (PR) | 2022-10-24 | Data files linting | ❌ data |
| #1509 (PR) | 2022-11-07 | add exception words with g plosive in Belarusian | ❌ data |
| #1583 (PR) | 2022-12-08 | Phoneme improvements | ❌ data |
| #2281 (PR) | 2025-09-13 | 1769 fix latvian issues | ❌ data |
| #2349 (PR) | 2026-02-05 | eSpeak-NG Hebrew Overhaul – Rules Fix + Lexicon Cleanup + Diacritics-First Processing | ❌ data |
| #2390 (PR) | 2026-03-28 | Finnish: phoneme rules for /h/ allophones before consonants | ❌ data |
| #2391 (PR) | 2026-03-29 | sv: comprehensive Swedish pronunciation improvements | ❌ data |
| #2419 (PR) | 2026-05-15 | fix(it): correct Italian pronunciation rules | ❌ data |
| #2426 (PR) | 2026-06-04 | Fix for inflections of Latvian ordinal numbers | ❌ data |
| #2429 (PR) | 2026-06-14 | Update Portuguese language (2026) | ❌ data |
| #2466 (PR) | 2026-07-25 | Handle diacritics and code-switched Arabic/Urdu/Farsi letters; corpus-attested vocal… | ❌ data |
| #2509 (PR) | 2026-08-24 | dictsource/hu_list, dictsource/hu_rules: added new pronounce exceptions | ❌ data |
| #2510 (PR) | 2026-08-25 | ht: reworking of the Haitian Creole pronunciation (spelling 1979) / refonte de la pr… | ❌ data |
| #2512 (issue) | 2026-08-26 | correct some Spanish pronunciations | 🟡 reproduces identically (§5 #92) |
| #2513 (PR) | 2026-08-28 | Added 2 new catalan accents (alguerès and rossellonès) | ❌ data |
| #2518 (PR) | 2026-08-29 | Improve Japanese pronunciation | ❌ data |
| #2520 (PR) | 2026-08-30 | mn: rebuild the Mongolian voice against a measured speech corpus | ❌ data |
| #2547 (PR) | 2026-09-12 | Albanian (sq) pronunciation improvements | ❌ data |
| #2550 (PR) | 2026-09-14 | Update Afrikaans pronunciation rules and word lists | ❌ data |
| #2554 (PR) | 2026-09-17 | improve Persian pronunciation | ❌ data |

**New languages** (13) — Data only; the port compiles any dictsource it is given (§1.5).

| Ref | Opened | Title | Port |
|---|---|---|---|
| #1589 (PR) | 2022-12-14 | Interlingue: Initial work | ❌ data |
| #1724 (PR) | 2023-05-22 | First contribution of de-ch | ❌ data |
| #1937 (PR) | 2024-06-25 | Amazigh language support (First Implementation) | ❌ data |
| #2262 (PR) | 2025-08-20 | Add barebones voice for Toki Pona | ❌ data |
| #2277 (PR) | 2025-09-09 | Create Hadrianus | ❌ data |
| #2325 (PR) | 2025-11-28 | new art sinago language | ❌ data |
| #2413 (PR) | 2026-05-03 | Added initial Sicilian language support and updated/sorted language list | ❌ data |
| #2447 (PR) | 2026-07-16 | Add Bété language support (ISO 639-3: bof) | ❌ data |
| #2462 (PR) | 2026-07-23 | Add Bété language support (ISO 639-3: bof) | ❌ data |
| #2553 (PR) | 2026-09-17 | Add Indian English (en-in) voice | ❌ data |
| #2567 (PR) | 2026-09-28 | pdc: Add Pennsylvania Dutch (Lancaster, Midwest and Lehigh voices) | ❌ data |
| #2578 (PR) | 2026-10-04 | dv: Add initial support for Dhivehi language | ❌ data |
| #2580 (PR) | 2026-10-05 | Add Pali (pi) | ❌ data |

**C runtime / build** (21) — N/A to a pure-Rust port: memory-safety bounds fixes, CMake, MBROLA, fuzzing, audio.

| Ref | Opened | Title | Port |
|---|---|---|---|
| #2171 (PR) | 2025-04-12 | build docker images | N/A |
| #2274 (PR) | 2025-09-03 | cmake: make libsonic optional | N/A |
| #2288 (PR) | 2025-09-21 | Run espeak-ng without setting path on Windows + espeak_ng_Cancel: audio_object_flush… | N/A |
| #2441 (PR) | 2026-06-24 | OSS-Fuzz: Add new fuzzer targets dict | N/A |
| #2465 (PR) | 2026-07-25 | soundicon: fix out-of-bounds accesses when loading SSML <audio src> files | N/A |
| #2477 (PR) | 2026-07-31 | build: enable -Wimplicit-fallthrough for libespeak-ng | N/A |
| #2501 (PR) | 2026-08-12 | Improve Mbrola sound | N/A |
| #2505 (PR) | 2026-08-22 | Add diff-gated clang-tidy CI check | N/A |
| #2516 (PR) | 2026-08-29 | Resolve MBROLA la1 `mb-la1` Pronunciation Issues and add Mac OS Support | N/A |
| #2519 (PR) | 2026-08-30 | tests: skip exact WAV hash checks where floating point keeps excess precision | N/A |
| #2524 (PR) | 2026-09-02 | Fix stale pitch=82 default in formant_factor calculation | N/A |
| #2527 (PR) | 2026-09-05 | Fix MBROLA support on macOS | N/A |
| #2529 (PR) | 2026-09-07 | build: enable -Wimplicit-fallthrough for libespeak-ng | N/A |
| #2530 (PR) | 2026-09-07 | voices: fix out-of-bounds read when a voice identifier is shorter than the name | N/A |
| #2537 (PR) | 2026-09-09 | synthdata: report a zero-length phoneme data file instead of crashing | N/A |
| #2556 (PR) | 2026-09-18 | readclause: bound buf writes for the embedded B punctuation list | N/A |
| #2569 (PR) | 2026-09-29 | ssml: fix out-of-bounds read in GetSsmlAttribute | N/A |
| #2570 (PR) | 2026-09-29 | bound sscanf %s reads in voice and language file parsers | N/A |
| #2571 (PR) | 2026-09-30 | bound punctuation-list writes to n_buf in ReadClause | N/A |
| #2573 (PR) | 2026-10-02 | fix out-of-bounds read in GetSsmlAttribute attribute scan | N/A |
| #2575 (PR) | 2026-10-03 | translateword: append the prefix phonemes with snprintf | N/A |

**Android app** (14) — N/A.

| Ref | Opened | Title | Port |
|---|---|---|---|
| #1709 (PR) | 2023-04-22 | Android: modernize code | N/A |
| #2398 (issue) | 2026-04-11 | espeak-ng android build being very scratchy | N/A |
| #2430 (issue) | 2026-06-16 | There are potential time-consuming operations on the UI thread. | N/A |
| #2473 (issue) | 2026-07-30 | The proposal to add automatic APK signing to Android Workflow | N/A |
| #2474 (PR) | 2026-07-30 | Russian: Updated Android localization and added some missing strings | N/A |
| #2482 (issue) | 2026-07-31 | Android (Wear): launcher icon absent on a fresh watch install until the app process … | N/A |
| #2488 (issue) | 2026-08-02 | Expose configgerable to be able to disable processing of emojis in espeak android | N/A |
| #2496 (issue) | 2026-08-10 | Android: Dictionary import feature is broken on Android 11+ | N/A |
| #2508 (PR) | 2026-08-24 | Android: add lightweight word replacements | N/A |
| #2535 (PR) | 2026-09-09 | Android: finish the voice settings dialog label cleanup | N/A |
| #2546 (PR) | 2026-09-12 | feat(android): Add Number Reading Mode options to Android TTS settings | N/A |
| #2548 (issue) | 2026-09-13 | Android: TalkBack verbosity issue when toggling "Unicode Normalization" preference | N/A |
| #2572 (PR) | 2026-10-01 | Android: check SynthesisCallback.start() return value | N/A |
| #2587 (PR) | 2026-10-10 | Android: initialize eSpeak once per process | N/A |

**Docs / meta** (9) — N/A.

| Ref | Opened | Title | Port |
|---|---|---|---|
| #1273 (PR) | 2022-08-05 | Tidied up docs/integration.md | N/A |
| #1644 (PR) | 2023-02-05 | Update README.md | N/A |
| #2416 (issue) | 2026-05-06 | [Feature Request] TTS audio quality expectation management | N/A |
| #2417 (PR) | 2026-05-06 | Added text to readme regarding front-end/back-end | N/A |
| #2515 (issue) | 2026-08-28 | Proposal to add a registry of language self‑designations for eSpeak‑NG | ❌ meta (language self-designations registry) |
| #2522 (PR) | 2026-08-31 | build(deps): bump actions/setup-java from 5 to 6 in the github-actions group across … | N/A |
| #2538 (issue) | 2026-09-09 | A question about the Releases page. | N/A |
| #2568 (issue) | 2026-09-29 | A more efficient way of calling eSpeak-ng from my App | N/A |
| #2576 (PR) | 2026-10-04 | Revise platform support table in README | N/A |

**Other issues** (20) — Mostly covered individually in §5 above.

| Ref | Opened | Title | Port |
|---|---|---|---|
| #2393 (issue) | 2026-03-31 | "shit" is missing from en_list | ❌ data (`en_list`) |
| #2400 (issue) | 2026-04-22 | Per-phoneme pitch/duration control | ❌ per-phoneme pitch/duration control — no equivalent |
| #2415 (issue) | 2026-05-06 | Proposal: Enhancing Naturalness through Advanced Formant Modeling and Glottal Source… | ❌ synthesis-quality proposal (§1/§36) |
| #2427 (issue) | 2026-06-09 | Audio Karl | — |
| #2437 (issue) | 2026-06-20 | Pashto (ps): digit "5" read as ګازره ("carrot") instead of پنځه | ✅ not reproduced (§5 #89) |
| #2438 (issue) | 2026-06-20 | Nepali (ne): digits read in English despite ne_list having number entries | ✅ not reproduced (§5 #89) |
| #2442 (issue) | 2026-06-28 | [Vietnamese] Comprehensive Technical Report on Phonetic Nuances, Tone Management, an… | ❌ Vietnamese tones/numerics — engine content, §3b |
| #2464 (issue) | 2026-07-24 | ja: all emoji and symbol descriptions are silent, even when the description is pure … | ✅ fixed 2026-08-01 (katakana support) |
| #2469 (issue) | 2026-07-26 | Unknown phoneme table | ✅ crash N/A; error message fixed (§3c #79a) |
| #2476 (issue) | 2026-07-31 | Enable -Wimplicit-fallthrough: a missing break in SelectTranslator() silently corrup… | N/A — the C fallthrough bug cannot occur here |
| #2479 (issue) | 2026-07-31 | Language for automatic script-based switching is hardcoded to en (resolves to en-gb)… | 🟡 reproduces identically — the port's switch target is `en` too |
| #2490 (issue) | 2026-08-02 | Feature requests: US/UK English accent choice for embedded Latin words, SAPI5 suppor… | ❌ feature requests (accent choice for embedded Latin, SAPI5, emoji toggle) |
| #2498 (issue) | 2026-08-12 | Automatic fallback to English (via internal translator2) doesn't fully match direct … | 🟡 reproduces identically (§5 #93) |
| #2499 (issue) | 2026-08-12 | Robotic/clicking quality in formant-synth consonant transitions (shared ph_consonant… | ❌ synthesis-quality (§1) |
| #2526 (issue) | 2026-09-04 | ht (Haitian Creole): nasal vowels collapsed, r/j/y phonemes incorrect | — |
| #2531 (issue) | 2026-09-08 | I consider this a downgrade regarding eSpeak in Brazilian Portuguese. | — |
| #2544 (issue) | 2026-09-11 | Proposal to improve the processing of time values (hours and minutes) in the Russian… | — |
| #2551 (issue) | 2026-09-15 | One-letter words repeated more than three times lose their phonemes | ✅ not reproduced (§5 #87) |
| #2555 (issue) | 2026-09-17 | Incorrect fallback to Russian phonetic rules for untagged Serbian Cyrillic text | ✅ found and fixed a defect here (§3c #78a) |
| #2577 (issue) | 2026-10-04 | A bug when reading long decimal fractions in Russian | ✅ not reproduced (§5 #88) |
## Priority order (biggest functional gaps first)

Re-measured **2026-09-04**. Every figure comes from a locally built upstream
binary (`f135499`) on this date.

**Where the port stands.** A 32-language sweep of two-word phrases: `-x`
**32/32 byte-identical**, `--ipa` **32/32**. A wider 84-phrase, 50-language
corpus: `-x` **79/84** and `--ipa` **80/84** (from 53 and 46 two passes ago).
The dictionary compiler, compared against upstream's own compiler on the same
`dictsource`: **1144/1154 words across 80 languages**. An eight-sentence English
spectral sweep: mean envelope difference **2.19 dB**, mean correlation
**0.970**. Duration at the default rate is within 5 % of the oracle.

1. ✅ **Secondary-stress placement** — done. `stressOpt` is read from the voice
   file (Finnish, Estonian, Latin and Ligurian configure their stress entirely
   through it), Greek lost the `S_NO_AUTO_2` it never had upstream, Czech and
   Slovak gained `S_FINAL_DIM_ONLY | S_FINAL_NO_2`, Korean gained
   `STRESSPOSN_2LLH`, and the invented Japanese arm is gone.
2. ✅ **`--ipa` cross-word allophones** — done, by rendering `--ipa` from the
   clause's finished code list (C's `GetTranslatedPhonemeString`) instead of
   word by word, and taking each phoneme's name from **running its bytecode
   program** with real neighbours (`WritePhMnemonic`'s `use_ipa` path) instead
   of guessing it from the mnemonic.  That closed all seven divergent languages
   at once and made `-x` and `--ipa` two spellings of one phoneme list by
   construction.  It also surfaced — and fixed — several code-list bugs that
   `-x` had been hiding: `StressCondition`/`ChangeIf` evaluated against the
   syllable's real stress level, C's `LOPT_REDUCE` gate on dictionary-given
   words, a faithful `FLAG_FOUND`, `prevPhW`/`nextPhW` refusing to look across a
   word boundary, and a spoken symbol between two words being its own word.
3. ✅ **Slow-rate duration model** — done. Upstream's 280-entry `speed_lookup`
   is ported, with a fitted affine model (`FIXED 0.502`, `SCALED 0.3546`) for
   the share of an utterance that doesn't scale with it. Wavegen's second,
   separate application of the rate — which was squaring the effect — is gone.
   Measured: 80 wpm 1.07×, 120 wpm 1.03×, 175 wpm 1.05×, 250 wpm 1.15×.
4. ✅ **Per-language details** — done. `ru` no longer upgrades a stressed `е` to
   `E` from stress alone; `uk` final devoicing goes through the ported
   `SetRegressiveVoicing`; `cmn` joins a third-tone sandhi pair into one word.
5. 🟡 **Formant quality, the last stretch** (§36). Mean **2.17 dB / r 0.970**
   over the eight-sentence sweep; the worst sentence is "hello world" at 4.29 dB
   / 0.942. Fixed this pass: the voice **tone curve's shape** was being cancelled
   (the curve was divided by the default curve *per index*, which is exactly the
   identity); dividing by a single constant instead restores the default curve's
   3.8 dB low-to-high tilt. Ruled out this pass, by measurement: the high-frequency
   peaks 6–8 (zeroing them changes "hello world" by 0.03 dB), the trailing-silence
   noise floor (an energy-gated window gives the same answer), diphthong
   interpolation, and durations (1.028 s vs 1.031 s on "hello world"). What is
   left is a *narrow* ~13 dB feature near 7 kHz on short utterances, against a
   broadband agreement of ~2 dB — a spectral-zero placement difference rather
   than a level one.
6. ✅ **The dictionary compiler** — done. Measured by compiling each language's
   real `dictsource` with *both* compilers and speaking the same words through
   each result: **1144/1154 across 80 languages** (English alone 11/19 → 19/19).
   What was missing was the rule-context command set (only seven uppercase
   letters are letter groups; `K` is "not a vowel"), rule conditions, the rule
   and group ordering C's reader depends on, `RULE_PRE_ATSTART`, the reversed
   list-entry order, key lowercasing, `$text`/`$textmode`, the other list files
   (`roots`/`listx`/`emoji`/`extra`), two-byte group-name truncation and the
   `groups3` index for non-Latin scripts. The 10 remaining words are in `da`
   (2/60), `ko` (2/3), `ta` (5/9) and `tn` (1/2).
   The **phoneme** compiler (§1.6) is still 🟡 — condition/jump/IPA-name opcodes
   round-trip only via `Raw`, and no phoneme sources ship with the port.
7. ✅ **`formant_factor`** — done with upstream #2524: the voice file's `pitch`
   line drives the base F0, the pitch range *and* the formant shift.
8. 🟡 **Long tail.** The 84-phrase corpus is at **79/84** (`-x`) and **80/84**
   (`--ipa`).  Five remain:
   - **Clause breaks after an abbreviation.**  Upstream reads "Dr. Smith" as two
     clauses (it breaks after `Dr.`, `St.` and `Jan.` even though those entries
     carry `$dot`); the port suppresses the break, which is a deliberate
     deviation the `abbreviation_periods` tests pin down.
   - **A `_!` after a comma-separated thousands group** ("5,678.90" →
     `f'aIv T'aUz@nd_! s'Iks…`), from `TranslateInteger`'s end-of-integer pause.
   - **Japanese's English switch** for 世界 (`_:(en)tS'aIni:z(ja)l'et@`).
   - **Thai tone placement** on one syllable (`sa2ds` vs `sa3ds`).
   - **Bengali's final inherent vowel** (`b'iSbO` vs `b'iSb`) — the rules
     disagree about the last conjunct, not the port's schwa-deletion pass.
   Plus §4/§5 per-language data that arrives with a future refresh.

**Closed in the 2026-09-04 passes:** `-x` language switches, foreign-script
spelling (`AL_WORDS`), word segmentation in dictionary replacements, `-X`/`-D`,
scoped `<prosody>` and positional embedded commands, the `$atend`/`$allcaps`
dictionary gates, accented letters in letter groups, the wrong-render-engine bug
in §36, all of `stress_flags`/`stressOpt`, `SetRegressiveVoicing`, Mandarin
third-tone sandhi, the `speed_lookup` duration model, the codes-driven `--ipa`
path, and the dictionary compiler.

---

## Big subsystems (user-requested 2026-08-01: Klatt, MBROLA, compilers, async)

Tackled one per loop iteration; each is a multi-day port.
- **Async runtime (§7) — ✅ done, incl. incremental rendering** (`async_synth.rs`, `tokio_synth.rs`, `EspeakNg::synth_streaming{,_with_events}`): cancellable command queue, chunked streaming callback with return-to-stop, set-parameter/set-voice commands, and — new 2026-09-04 — **clause-by-clause rendering**, so the first audio of a long utterance is ready after its first clause instead of its last. Concatenated chunks reproduce `synth()` byte-for-byte (`tests/streaming_synth.rs`). Optional `parallel` (rayon `synth_many`) and `tokio-runtime` (`tokio_synth::synth`/`synth_stream`, bounded channel with backpressure and drop-to-cancel) features. See §7.
- **dict compiler (§1.5 `compiledict.c`)** — 🟡 **end-to-end + CLI: `compile_dictionary(dir,lang,phdata)`**, exposed as **`espeak_cli --compile[=<lang>]`**, reads dictsource `_list`+`_rules` → `_dict`; compiled rules (incl. **pre/post-context** + the **`_` word-boundary** and **`A`–`Z` letter-group** markers, `pre)match(post` DSL) translate for real. Remaining: rule conditions/stress, `@`/`&` context markers; the real `dictsource/` files (drop in to compile real dicts).
- **phoneme compiler (§1.6 `compiledata.c`)** — 🟡 **phontab + program bytecode (FMT/WAV/VWLSTART/VWLENDING/CONTINUE) + SPECT_SEQ + Klatt-frame + WAV writers + source DSL parser done** (`compile_phontab`/`compile_program`/`compile_spect_seq`/`compile_wav_sample`/`parse_phoneme_dsl`, all round-trip; exposed via CLI `--compile-phonemes`). Remaining: condition/jump/IPA-name opcodes (via `Raw`); no real sources ship.
- **Klatt (§1.2 `klatt.c`)** — ✅ **`parwave` ported in full** (cascade + parallel + nasal antiresonator + both glottal sources + tilt/breathiness/skew/flutter); oracle-verified at spectral-envelope correlation 0.997. The `FRFLAG_KLATT` auto-routing was **wrong** and is gone — C routes to Klatt only on `voice->klattv[0]`.
- **MBROLA (§1.4)** — ✅ **everything but the external binary**: `compilembrola.c`, the binary `_phtrans` format, `GetMbrName`, `WritePitch`, `MbrolaTranslate`, `--compile-mbrola`, `--pho`, and `mb-*` voice resolution with a graceful fallback. All 49 shipped tables round-trip byte-for-byte. Diphone *audio* still needs the separately-licensed binary + databases.

**Summary (2026-09-04):** async ✅, Klatt ✅, MBROLA ✅ (bar the external binary), the two compilers 🟡. What is left is gated on **external data that isn't in this checkout** — dictsource/phoneme *sources* for the compilers, and the MBROLA binary + diphone databases — not on further porting.

## Progress log

The pass-by-pass history — 206 entries, every fix with its root cause, plus the
full text of upstream re-checks 1-3 — lives in **[GAPS-LOG.md](GAPS-LOG.md)**.
This document tracks what is still open.
