# Gaps vs. upstream eSpeak NG

Tracking document comparing this pure-Rust port against upstream
[`espeak-ng/espeak-ng`](https://github.com/espeak-ng/espeak-ng)
(`src/libespeak-ng/`, 35 `.c` modules).

**Legend:** ❌ not started · 🟡 partial · ✅ done (listed for context)
**Snapshot date:** 2026-07-31 · upstream `master` around commit `84a37de` (emoji reading, #2463).
**Upstream re-check (2, later 2026-08-01):** `84a37de` still HEAD. Since the first re-check the port has ALSO landed: homographs (verb/noun/past POS triggers), hyphen word-breaks, leading-zero numbers, `vi 10`, `.replace` engine-wide, Korean/Sinhala/Urdu/Sindhi/bpy scripts, and a working dictsource compiler (Bosnian #2269 demoed end-to-end). Fresh probes of newer issues: **#2248 `pt-br` errors** ("phoneme table 'pt-br' not found") — locale should fall back to `pt` (tractable code fix, not yet done); #2243 Hindi "crash" → Rust is memory-safe (no crash); #2295 Polish "zi" → not reproduced; #2196/#2195 en-us weak forms ("was"/"from" not reduced) → data; #2247/#2280 French `ɥ` still `/y/` → data; #2232 "niche" en-us → `niːʃ` (arguable). **Original upstream re-check:** `84a37de` is STILL `master` HEAD (nothing merged since the snapshot). Live GitHub sweep of all open PRs/issues done this date. **Port has since resolved** several tracked refs: #2464 (ja emoji → katakana support), #228 (CamelCase, both branches), #96 (2st ordinal), #2435 (stylized letters), plus non-reproductions #2438/#2340/#2393/#257. **New open items found (not yet in port):** PRs are all language-data (need dictsource): #2391 Swedish, #2390 Finnish /h/, #2349 Hebrew overhaul, #2281 Latvian, #1958 Kurdish, #2481 Hungarian emoji, #2422 Persian, #2352 fa dict, #2429 pt, #2419 it, #2423 nl, #2424 sl, #2426 lv, #2280 fr ɥ, #2269 bs, #2268 ja numbers, #2466 ar/ur/fa diacritics; new languages #2462 Bété, #2413 Sicilian, #2325 Sinago, #2277 Hadrianus, #2262 Toki Pona. Issues: #2442 Vietnamese tones/numerics (big), #2295 Polish "zi", #2314/#2352 language stresses, #2366 word+number split (contested — port already splits at the letter↔digit boundary), #2267 --voices completeness. Android/build/C-runtime issues (#2485/#2483/#2480/#2478/#2477/#2475/#2474/#2473/#2471/#2470/#2288/#2274/#2171/etc.) are N/A to a pure-Rust port. Merged since last snapshot but not separately re-verified: #2431 Russian feminine decimal fractions (§3 #18).

**Upstream re-check (3, 2026-09-03):** master HEAD is now `f135499` (2026-09-01, ar→Mandarin #2511) — **432 commits ahead of the `1.52.0` tag the bundled `espeak-ng-data/` comes from**, with no 1.53.0 release cut yet. 13 PRs merged since 2026-08-01. The delta is recorded in **§3** (merged engine + data commits since `84a37de`), **§3b** (the whole unreleased 1.53.0 changelog vs. our 1.52.0 data — the single biggest parity lever), **§4** (open PRs, re-swept) and **§5** (issues opened since 2026-08-01). Status corrections from this sweep: **#2475 merged 2026-08-02** (§15/§5 still listed it as open) and **#2422/#2423/#2481 merged** (were tracked as open PRs in §4). Every ❌/✅ claim added by this sweep was probed against `./target/debug/espeak-ng-rs --path=espeak-ng-data` on 2026-09-03.

**Implementation pass (2026-09-03, after re-check 3):** the §3b backlog was
**executed**, not just recorded. Bundled data was rebuilt from upstream `master`
(`f135499`) — 123 dicts / 151 voices — and the engine work that the refresh
exposed or that upstream had landed went in with it: full script routing
(§3b #46), program-driven `ChangePhoneme` resolution on both the IPA and codes
paths (§3 #41), cross-word `nextPh` context, skin-tone + tag-sequence emoji
(§3b #45), Sinhala ZWJ (§3b #47), Russian/Hungarian fraction reading (§3b #48),
the terminator API (§3b #50), `SUFX_M` suffix chains (§3 #40) and the #2470 event
rescale. Suite: **696 tests green**. Items below carry ✅ where that is now done;
what remains is listed under "Still open after this pass" at the end of §3b.

The port covers the **text → phonemes/IPA** path and a **working harmonic
synthesizer** that reads real `phondata`. The gaps are concentrated in
**prosody, alternate synthesizers, SSML, the compilers, and the runtime/audio
layer**, plus a long tail of per-language number/rule details.

---

## Verification tooling

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
| 14 | `readclause.c` | 31 KB | 🟡 | Clause splitting works; **`[[…]]` phoneme input and technical-symbol name reading (`# $ % & * + / = @ ~`) now implemented** (`is_spoken_symbol`/`lookup_symbol_name`, 5 tests, 9/10 symbols byte-identical to oracle). **`-x`/synthesis parity (2026-08-01):** `translate_to_codes` had no `Token::Punctuation` arm, so spoken symbols rendered in `--ipa` but were **dropped in `-x`** and silent in audio; added to the word-emission arm (non-spoken punctuation has empty phonemes → no-op) — `-x "a & b"` → `'eI and b'i:`, matching `--ipa`. **Capital-letter announcement (`-k 2` → `_cap` "capital", byte-exact) implemented** (`cap_ipa` in `src/translate/mod.rs`, 3 tests). **`--punct`/`--punct=CHARS` implemented (2026-08-01):** announces punctuation by name — `LangOptions.punct` (`None`/all/subset) + `punct_covers`; a covered `Token::ClauseBoundary`/`Token::Punctuation` becomes a `Word` entry via `lookup_symbol_name` (`,`→"kˈɒmə", `.`→"dˈɒt"), gated so the default path is byte-identical. Works in `--ipa` and `-x` (with END_WORD bracketing for spacing since punctuation absorbs its whitespace). 3 tests. **Reaches audio too (2026-08-01):** `--punct`, `-g` (word gap) and `-k` (capitals) now thread through the engine `synth` path — `make_translator` applies `word_gap`/`capitals`/`punct`; new `EspeakNg::set_punctuation_list` + `Parameter::{Punctuation,Capitals}` handling; the `-k 2` capital announcement was ported into `translate_to_codes` (was `text_to_ipa`-only). Verified: `--punct`/`-g`/`-k` each lengthen the WAV (`runtime_options_audio.rs`). **Remaining:** embedded commands (`\001..`), `-k 1` capital *sound*. |
| 15 | `phonemelist.c` | 19 KB | 🟡 | **`phonSWITCH` mid-clause language switching implemented** (`TranslateEntryKind::LangSwitch` in `src/translate/mod.rs`): `_^_LL` dict entries re-translate the word with the target language's translator and mark it `(target)…(orig)`; bare `phonSWITCH` → default voice `en`. 4 tests, byte-identical to oracle (af→en/de/fr). Also fixed promotion corrupting switch entries. **Remaining:** script-detection switches (non-Latin input), `<voice xml:lang>` wiring (now feasible atop this), `-x` mnemonic path still uses the old `"SW"`-mnemonic check. **Upstream (track):** merged #2456 clears `phoneme_tab` on a table switch so absent codes resolve to NULL (follow-on to #2459/§19); **#2475 (merged 2026-08-02, `503708d3`)** reverts #2128 and fixes a NULL `phoneme_tab` **crash in four shipped voices** underneath it — it also adds `phonemelist.c` "keep the word start when dropping an untranslatable phoneme", which is **not ported** (§3 #42); #2492 (merged 2026-08-03) fixes the same class at `i_APPEND_IFNEXTVOWEL` (§3 #41); open #2469 is a related segfault — `espeak -v alex` → "Unknown phoneme table: ''" → SIGSEGV. **⇒ the port already resolves a missing/empty phoneme table to a clean `Error::InvalidData` (never a panic)** — `find_table`/`select_table_by_name` return errors (test `load.rs:669`), and a `LangSwitch` entry always carries the `PHON_SWITCH` byte so the re-translation path can't index-panic; unknown switch targets go through `if let Ok(Translator::new)`. Verified 2026-08-01; a regression test would lock it in. Open #2479: the automatic script-based switch target is hardcoded to `en` (→ en-gb) upstream and isn't configurable — make it configurable when script-detection switching lands. |
| 16 | CLI (`espeak-ng.c`) | — | 🟡 | `-f` (file), `--stdin`, `-m` (SSML), **`-k` (capitals, byte-exact)**, **`-g` (word gap → `_` pause per word in `-x`, byte-exact)**, **`--voices[=<lang>]` (voice listing)** now wired; **`-z` (no final sentence pause)** suppresses the ~0.30 s end-of-utterance trailing silence (§35) via `VoiceParams.no_final_pause` + `EspeakNg::set_no_final_pause`, verified by WAV sample count (`runtime_options_audio.rs`); **`-b <n>` input encoding (1=UTF-8, 2=Latin-1, 4=UCS-2)** now decodes `-f`/`--stdin` bytes via the `encoding` module (`gather_text` → `decode_to_string`) — a Latin-1 file that fails as UTF-8 reads correctly with `-b 2` (`input_encoding.rs`); `-l` accepted for arg-parity. **`--voices` (2026-08-01):** new `src/voices.rs` walks `<data>/lang/**`, parses `name`/`language <code> [pty]`/`gender`/`age`, and lists all 145 voices sorted by code (filterable by language prefix). Listing-only (identification lines), not the full acoustic voice spec. 2 unit + 2 CLI tests. **`--compile[=<lang>]` implemented (2026-08-01):** compiles `<data>/<lang>_list`+`_rules` → binary `<lang>_dict` via `compile_dictionary` (the `espeak-ng.c` `--compile`/`--compile-debug` path); verified end-to-end (`--compile=en` then `-x hello` → `h@l'oU` from the compiled dict), missing-source fails cleanly. **`--compile-phonemes[=<src>]` implemented (2026-08-01):** parses a phoneme-table source (`parse_phoneme_dsl`) → binary `phontab` (`compile_phontab`); verified end-to-end (source → phontab → `parse_phontab` round-trips), missing-source fails cleanly. Table only (phonindex/phondata not regenerated). **Remaining:** `-b` actual re-decoding, `--compile-mbrola`/`--compile-intonation` (no-ops; no source ships), `--pho`, `-X` trace, and audio playback. |

## 3. Recent upstream commits not reflected here

| # | Ref | Description | Notes |
|---|---|---|---|
| 17 ✅ | #2463 (`84a37de`, 2026-07-25) **+ 2026-07-22 cluster** (#2449, #2451, #2452, #2454, #2455, #2457) | Emoji reading instead of spelling out — flag emoji, ZWJ sequences, skin-tone modifiers (base name + modifier), CLDR 48.2 data for 28+ langs, `$textmode` replacement segmentation | **Core done 2026-08-01** — emoji tokenised as sequences (base + VS16 + skin-tone + ZWJ + flag pairs) and read via the dict's TEXTMODE emoji-name entries (`word_to_phonemes` now re-translates TEXTMODE replacement text). `😀`→"grinning face", `❤️`→"red heart". Data-driven for the shipped langs (en verified); ja follow-up bug #2464 (silent descriptions) **fixed 2026-08-01** — the cause was missing katakana support, now folded to hiragana (see §5 #2464 + progress log). |
| 18 | #2431 | Russian feminine forms for decimal fractions | Number-grammar gap (§12). |
| 19 | #2459 (`121d8dd`) | NULL-deref guard after mid-clause phoneme-table switch | Ties to missing `phonSWITCH` (§15). |
| — | #2456 (2026-07-22) | Clear `phoneme_tab` on table switch so absent codes resolve to NULL | Follow-on to #2459; ties to the open crash cluster #2475/#2469 (§15). |
| — | Android / build / libsonic build commits | N/A to a pure-Rust port | — |
| 38 🟡 | #2511 (`f135499`, 2026-09-01) | **Arabic voice reads Han ideographs with the Mandarin translator** instead of spelling "Chinese letter". Adds `langopts.alt_alphabet` + `alt_alphabet_lang`, the `AL_IDEOGRAPHS` alphabet flag, `ShouldSplitIdeographs()` (per-ideograph word splitting for a *base* language that nominates an ideographic alternate), and `tone_ph_data` on `PHONEME_LIST` + `TonePhoneme()` / `InterpretPhoneme2WithData()` so a tone phoneme resolved under one table survives the switch back | ❌ **Probe 2026-09-03:** `-v ar "إذا 如果您因为翻译 النص"` → `ʔˈiðaː ɣ ʔˈannˈas̪s̪` — the whole Han run collapses to a stray `ɣ`. **Two gaps:** (a) no `alt_alphabet` concept (a language cannot nominate a fallback translator for a script range); (b) **no tone-phoneme support at all** — `tone_ph` appears nowhere in `src/`, which also blocks zh/yue/vi tone rendering generally (cf. §1.1 intonation). |
| 39 | `1f1e3fa3` (2026-09-01) | `TonePhoneme()` accessor replacing five `tone_ph_data`/`phoneme_tab` ternaries; alphabets-table realignment | Refactor, no behaviour change. Adopt the same shape if/when tone phonemes land (#38). |
| 40 ✅ | #2495 (`56f2e9c7`, 2026-08-10) | Stack buffer overflow when a word stacks many `SUFX_M` suffixes — the `while (more_suffixes)` loop in `TranslateWord3()` is now capped like the prefix loop | ❌ **Not a crash risk here (Rust), but a feature gap:** the port has **no multi-suffix loop at all** (`SUFX_M`/`more_suffixes` unimplemented; only single-suffix `SUFX_I` stripping). Probe: `s'`×220 → 220 bare `s`. |
| 41 🟡 | #2492 (`89fd5d50`, 2026-08-03) | NULL `phoneme_tab` deref in `i_APPEND_IFNEXTVOWEL` (109 of 148 shipped voices segfaulted on `-v de "ईघ"`) | ❌ Crash is N/A (memory-safe), but the **instruction itself is unimplemented**: `src/synthesize/bytecode.rs:498-516` no-ops every group-0 instruction except `RETURN`/`I_IPA_NAME`/`I_CHANGE_PHONEME`, and group-1 `ChangeIf` is skipped entirely (`tr == NULL`). So `i_APPEND_PHONEME`, `i_APPEND_IFNEXTVOWEL`, `i_INSERT_PHONEME`, `i_REPLACE_NEXT_PHONEME`, `i_DELETE_NEXT_PHONEME`, `i_PAUSE_BEFORE/AFTER`, `i_SET_LENGTH`/`i_ADD_LENGTH` and the `CHANGE_IF*` family all silently do nothing. **New gap — phoneme-program instruction coverage.** Probe: `-v de "ईघ"` → empty output (no crash, no speech). |
| 42 | #2475 (`503708d3`, **merged 2026-08-02** — §5/§15 tracked it as *open*) | Reverts #2128 as dead code and fixes the real NULL `phoneme_tab` crash under it; also `phonemelist.c` **"keep the word start when dropping an untranslatable phoneme"** and NULL-substitution in `SetWordStress` | 🟡 Crash-safety already verified here (`tests/crash_safety.rs`, §15). **Not ported:** the word-start preservation when an untranslatable phoneme is dropped — relevant once script-switching lands (#46). |
| 43 ✅ | #2500 `ku` (2026-08-19), #2422 `fa` (2026-08-19), #2481 `hu` emoji (2026-08-19), #2423 `nl` (2026-08-18) | Dictsource-only language fixes: Kurmanji oblique-plural/preverb/negation stress + unstressed subject pronouns (+ new `docs/languages/ira/ku.md`); Persian exception words; Hungarian emoji + rules exceptions; Dutch `na`+CVC+`ige` | ❌ Inherited automatically by the §3b data refresh; nothing to port in code. |
| — | Android cluster (#2471, #2480, #2483, #2487, #2493, 2026-08-01→08-03) | TTS-framework word boundaries + error reporting, stop actually aborts synthesis, voice name as hint, settings screen off the main thread, launcher alias | N/A to a pure-Rust library. (`#2493`'s analogue — cancel between chunks — already exists in `async_synth.rs`, §7.) |

## 3b. Data staleness — bundled data is 1.52.0, upstream is 432 commits ahead

`espeak-ng-data/` in this repo is eSpeak NG **1.52.0** (2024-12-12) plus a few 1.52.0.1
`lang/` files. Upstream's **unreleased 1.53.0** ChangeLog is therefore, almost verbatim,
our parity backlog. **This is the single biggest lever in the whole document:** refreshing
the data closes the missing languages, all 14 updated ones and the entire emoji block at
once, and `compile_dictionary` (§1.5, CLI `--compile`) already does dictsource → `_dict`.
All probes below run against `./target/debug/espeak-ng-rs --path=espeak-ng-data` (2026-09-03).

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

**Executed 2026-09-03** (see the progress log for detail and probe output):

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
| 38 ✅ | Tone phonemes (`tone_ph`) | **Done 2026-09-03 (second pass).** Root cause was `langopts.textmode`: cmn/yue/zh compile their dictionaries as replacement *text*, which **reverses** `FLAG_TEXTMODE`, so `你`'s reading (`ni3`) was being read as phoneme codes (`qr`). With that fixed plus tone phonemes (phSTRESS entries carrying a program — previously dropped as stress markers), tone-after-vowel placement, the `_|` word gap, tone digits kept inside pinyin words, and a port of `CalcPitches_Tone`'s Mandarin sandhi, `-x -v cmn 中文` / `北京` / `请问` and `yue 你好` are **byte-identical to upstream**. Remaining: multi-character dictionary words (谢谢 keeps two full tones where upstream neutralises the second), and Vietnamese tone selection (its rules emit no tone phoneme at all for `xin`, so nothing reaches the tone pass). |
| — | Foreign-script **run grouping** | Upstream emits one switch per run (`(cmn)w1 w2(ar)`); the port emits one per word (`(cmn)w1(ar) (cmn)w2(ar)`). Same words spoken, different marker placement. |
| — | Deliberate divergence: scripts upstream **spells** | Upstream only auto-switches for the `AL_WORDS` alphabets (el, hy, ar, hi, bn, pa, gu, ta, kn, ml, si, ka, ko) and *spells* the rest — `en "привет"` → "pe er ee ve ye te", `en "你好"` → "Chinese letter" ×2. The port speaks them with the script's voice instead (the pre-existing Cyrillic/Greek design here), which never loses text but is not oracle-identical. Implementing the spelling path would need `LookupLetter`-style foreign letter names. |
| 49 | Slovak numbers | Unverified against the oracle; the data refresh may already cover it. |
| 51 🟡 | Word-segmented / nested dictionary replacements | The hanzi→pinyin half now works (§3 #38), so Chinese emoji descriptions read. Still open: segmenting replacement text for languages without spaces (`th 👍`), and multi-character Chinese dictionary words. |
| 52 | `compiledict` over-length list lines | Belongs with the dict-compiler work (§1.5). |
| 53 | `prevPh`/`prevPhW` regression tests | The conditions are evaluated correctly (linking-j, German r-flap verified by hand); dedicated tests not yet added. |
| ✅ | German/Hungarian vowel break (`_!`/`_|`) | **Done (third pass), corrected (eleventh).** The two pause phonemes were swapped and the stress was read before the placement rule rather than after; instrumenting both engines showed C picks `_!` when `vowel_stress[1] >= 4` and `_|` otherwise. German, Hungarian and Dutch now match upstream byte-for-byte. Original note: **Done (third pass).** It wasn't `PauseBefore` at all but `langopts.vowel_pause`, parsed into `StressOpts` and then ignored. Implemented in `set_word_stress` for the 0x30/0x20/0x20c languages; `de beachten` is byte-identical to upstream. Residual: the choice between `_|` (no-link) and `_!` (very short) still differs for *dictionary-stressed* words — upstream reads `vowel_stress[1]` at a point in `SetWordStress` this port doesn't reproduce exactly, so `Ich`/`einen` get `_!` where upstream gives `_|`. Both are breaks; the audible difference is small. |
| ✅ | Foreign-script run grouping | **Done (third pass).** Consecutive words in the same foreign script now share one `(target)…(base)` group, as upstream does. |
| ✅ | Short decimal fractions as one number | **Done (third pass).** `NUM_DFRACTION_2`/`_4` implemented for the ~25 languages upstream lists, including the Portuguese text path; `cs 0,25` and `fr 3,14` are byte-identical to upstream. |
| ✅ | Group-0 list-editing instructions | **Done (fourth pass).** `InsertPhoneme`, `AppendPhoneme`, `IfNextVowelAppend` and `ChangeNextPhoneme` are applied on both the IPA and the codes path: English linking r (`sofa area` → `sˈəʊfəɹ ˈeəɹiə`) and the `;` glide (`maI; 'ap@L`) now match upstream, and the mnemonic-sniffing approximations they replace are gone. `ChangePhoneme(NULL)` deletes the phoneme, as in C. Still no-ops: `PauseBefore`/`PauseAfter` and the length modifiers (`SetLength`/`AddLength`/`LengthMod`), which only affect synthesis timing. |
| ✅ | Context-dependent vowels + `isTranslationGiven` | **Done (fourth pass).** Program-driven `ChangePhoneme` now applies to vowels as well as the `#` class — `the apple` → `ðɪ ˈapəl`, Hindi `भारत` → `bʰˈaːɾət`, both byte-identical to upstream — and `isTranslationGiven` keeps dictionary/number readings unreduced in tr/ba/tt (`tr 100` = `jˈyz`, `yüz` = `jˈøz`). Consonant changes stay deferred, matching upstream's `-x` (which keeps a plain `l` in "world"). |
| — | Georgian decimals | `ka` reads the fraction digit by digit and speaks its `_dpt` word; upstream reads it as a number with no point word. Not one of the modes upstream lists for `ka`, so it needs its own investigation. |
| ✅ | `-x` allophone spelling | **Done (twelfth pass).** Not a phoneme-choice problem at all: a `/` in a mnemonic is a *variant indicator* that C drops when printing (`WritePhMnemonic`), so `a/` prints as `a` and `l/2` as `l`. The port printed the full mnemonic. Fixing the display (and dropping the earlier "consonant changes are deferred" theory, which this disproves) took the `-x` sweep from 27/32 to 31/32. Superseded note: | Several languages print an allophone variant where upstream prints the base mnemonic (`es 'ola/` vs `'ola`, `it bUo/n…`), while `--ipa` agrees with upstream. **Investigated (fifth pass):** C gates `ChangeIfNotStressed` inside `StressCondition` on `SFLAG_DICTIONARY`, unless the language sets `LOPT_REDUCE & 1` (only Italian) — "don't change phonemes which are given for the word in the dictionary". Wiring that gate to this port's notion of a dictionary-given word made Russian **lose** its reduction (`molok'o`) while Spanish kept its allophone, dropping the `-x` sweep from 18/24 to 15/23, so it was reverted; `LangOptions::reduce_dictionary_vowels` is parsed and carried for a future attempt with a faithful `SFLAG_DICTIONARY`. Not a pronunciation difference — the IPA is identical either way. |
| ✅ | `PauseBefore` / `PauseAfter` | **Done (eighth pass).** Both render paths honour them: the formant path ends its run at the pause and emits the silence, the WAV path splices it inline. Verified against `ph_german`'s /r/ — 30 ms after /t/, 15 ms after the other stops, none after a vowel. |
| ✅ | Length modifiers (`length` / `LengthAdd` inside a program) | **Done (eleventh pass).** Captured in `PhonemeExtract` and used in place of the declared `std_length` when computing a vowel's duration, as C does by seeding `pd_param[i_SET_LENGTH]` with `std_length`. German raises /a/ 110 → 125 before a rhotic and uses the pattern on most of its vowels. |
| ✅ | Thai tone placement | **Done (sixth pass).** Tones are attached one per syllable vowel — pulled back over a coda (Mandarin), or forward from ahead of the syllable (Thai) — and extras are dropped. `สวัสดี`, `ขอบคุณ` and `ไทย` are byte-identical to upstream; Mandarin/Cantonese unchanged. Residual: `กิน` still picks the leading tone (`k'i3n`) where upstream takes the trailing one (`k'i2n`) — with two tones around one vowel, which wins isn't captured by "first wins", and the two Thai samples that need opposite answers don't disambiguate it. |
| ✅ | Vietnamese tones | **Done (seventh pass).** The tone wasn't missing from the rules — `CalcPitches_Tone` *adds* it: a toneless syllable takes `phonDEFAULTTONE`, and Vietnamese turns the clause-final one into tone 7. With that plus the `langopts.word_gap` `_|`, `xin chào`, `tôi` and `Việt Nam` are byte-identical to upstream. |
| ✅ | Decimal separators for af/ka/kk | **Done (seventh pass).** All three read "3,14" as two numbers and use the period; the port had them as comma-decimal. Verified against the oracle for all 18 languages the port configures. |
| ✅ | Sentence-end detection | **Done (seventh pass).** A period only ends a sentence before a capital (or at end of text) — `one. two` is one clause, `one. Two` is two — and a comma between two numbers is a digit separator. The port broke on every period, adding a spurious clause break to "etc. and", "5. item" and decimal text. |
| ✅ | Explicit stress vs the placement rule | **Done (ninth pass).** An explicit primary-stress marker from the dictionary or rules now fixes the stressed syllable for every language (composed number words exempt), matching `GetVowelStress`; Crimean Tatar is byte-identical to upstream. The default rule was also corrected to C's (`STRESSPOSN_2R`, `unstressed_wd2 = 3`). |
| ✅ | Voice-file `stressRule` | Parsed and applied (ninth pass) — `stressRule <rule> [wd1] [wd2]`, used by ~15 languages. |
| ✅ | Per-language `stress_flags` | **Done (tenth pass).** They were set in only seven language arms, three of which disagreed with upstream; the rest ran with none. Upstream's whole `tr_languages.c` table is now transcribed as the *base*, with the port's tuned per-arm values still overriding where it has them (those compensate for differences in its own stress fill, and replacing them wholesale regressed nl/pt/ru). Basque, which was the symptom, is byte-identical: `kaixo` → `kˈaɪʃo`, `etxe` → `ˈetʃe`. Diagnosis came from instrumenting *both* engines and diffing their `vowel_stress[]` arrays. |
| — | Multi-character Chinese words | Re-examined (fifth pass): `谢谢` is **not** in the shipped `cmn` dictionary, so upstream's neutral tone on the second syllable comes from neither a dictionary entry nor the sandhi rules already ported. Mechanism still unidentified. |
| — | Georgian fractions | `ka` ships `_0Z*` entries but upstream reads the numerator *without* the scale word; the port still reads digit-by-digit. Needs its own decimal mode. |

**Suggested execution order for #44–#54:** (1) vendor `dictsource/` from master and run `--compile` for all 123 languages, adding the 9 missing dicts + data-crates; (2) fix the advertised-but-dictless voice error path; (3) skin-tone / subdivision-flag emoji sequences (code, not data); (4) foreign-script split (#46) — the only one that currently *loses* input text; (5) `alt_alphabet` + tone phonemes (§3 #38); (6) the small ones: terminator API (#50), ru fractions (#48), si ZWJ (#47).

## 4. Open upstream PRs = language data / pronunciation the port would inherit as gaps

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

## Autonomous loop progress log

- **2026-09-05 — 📦 Publish script, stale data crates fixed, whole workspace to 0.2.0 (user: "create a publish shell script that will honor rate limits and delays" / "address data crates issue" / "change all of them to 0.2.0").**
  `scripts/publish.sh` publishes the workspace paced against both crates.io
  rate-limit buckets (new crate: burst 10, 1 per 10 min; new version: burst 30,
  1 per min) using a GCRA token bucket, so build and upload time counts against
  the interval instead of being added to it.  A 429 is answered by parsing the
  `try again after <timestamp>` out of the error and sleeping until exactly
  then.  Every crate is checked against the sparse index first and skipped if
  that `name@version` exists, so a failed run resumes for free; after each
  upload it polls the index until the version is visible, because the next
  crate may depend on it.  `plan` / `package` / `publish` modes, `--only`,
  `--exclude`, `--from`, `--limit`, `--assume-drained`.
  Its data-freshness preflight then caught a real one:
  - **`espeak-ng-data-dicts`, `espeak-ng-data-phonemes` and
    `espeak-ng-data-dict-ru` were pre-refresh snapshots** — the old 168K
    `en_dict`, the old `phondata`/`phontab`/`phonindex`, an `ru_dict` half a
    megabyte short.  All still carried their old version numbers, so a publish
    run would have skipped them as "already published" and shipped stale
    dictionaries under a fresh release.  Silently.  That is why the check is a
    hard failure and not a warning.
  - **Six `lang/` voice files were never added to the phonemes crate**
    (`lang/cau/ab`, `lang/roa/lij`, `lang/xgn/mn-f`,
    `lang/ira/ps-x-{northwest,southeast,yusufzai}`), so those dictionary crates
    had no voice to load.  Two superseded hand-written Pashto voices
    (`voices/ps`, `voices/asia/ps`) that upstream does not have are removed.
  - **Faroese and Pashto move out of the aggregate**, joining Russian.  The
    refreshed dictionaries are much bigger; all three inside would have taken
    the aggregate to 8.9 MB against a 10 MB hard limit.  It is 4.80 MB with 120
    languages; `bundled-data` depends on all four crates so callers see no
    difference.  `tests/bundled_data_complete.rs` walks the actual crate set to
    prove no language is lost on either side of the split — matching voices the
    way eSpeak resolves them, since `lang/gmq/nb` is what declares
    `dictionary no` and `lang/iro/chr` declares `chr-US-Qaaa-x-west`.
  - **The `install_bundled_data` doc example never compiled** — its
    `with_data_dir` arguments were reversed, and it is only compiled with
    `bundled-data` on.
  Then the whole workspace moved to **0.2.0** — library, all 125 data crates,
  all 125 dependency requirements, and the `generate_dict_crates.py` templates
  — so one version number covers one set of bytes and no crate can be missed.
  824 tests green with `bundled-data` (820 default), 0 warnings, preflight
  clean, ETA 1h27m for the 126 publishes.  Nothing committed or published.

- **2026-09-05 — 🏷️ Patch release 0.1.4 prepared (user: "bump patch version and prep for release").**
  `espeak-ng` `0.1.3` → **`0.1.4`** (Cargo.toml + Cargo.lock), and the
  accumulated unreleased work cut into a dated `[0.1.4]` CHANGELOG section (its
  two `### Fixed` blocks merged, sections ordered Added / Changed / Fixed).
  Two things the release surfaced:
  - **The data crates had lost their `authors`/`repository`/`homepage`** in the
    working tree — 122 of them, from the data refresh.  Restored to match the
    committed format, so the published crates keep their repository link.
  - **The bundled data changed, so the data crates can't be republished at
    0.1.0.**  crates.io has `espeak-ng-data-dict-*` at 0.1.0 only, and 93 of
    them now carry different `*_dict` bytes, so those are bumped to **0.1.1**;
    the 9 crates that are new in this release (`ab`, `crh`, `fo`, `kaa`, `lij`,
    `mn`, `ps`, `rup`, `xex`) stay at 0.1.0 for their first publish.  The
    dependency requirements stay at `0.1.0`, which `^0.1.0` already satisfies.
    `cargo package` confirms the ordering: it refuses `espeak-ng 0.1.4` until
    the nine new crates exist on the index.
  Verified at the new version: 820 tests green (827 with all features), 0
  warnings, `-x` and `--ipa` sweeps 32/32, `cargo package --list` clean, and all
  nine new crates package with their data.  Nothing committed or published.

- **2026-09-05 (nineteenth pass) — the last of the corpus tail.**
  `-x` 75 → **79/84** and `--ipa` 77 → **80/84**, sweep still 32/32, compiler
  parity unchanged at 1144/1154.  Six more upstream mechanisms the port lacked:
  - **Voice-file `replace`** (14 shipped voices): Russian's `replace 03 a a#` is
    what gives a word-final unstressed `a` its `a#`.
  - **Stress conditions ask about the phoneme they name** — `nextVowel(isMaxStress)`
    was reading *this* phoneme's level, so Russian "мама" was `m'ama#`.
  - **Post-contexts reach past the word boundary** (`_) o (_C`), which the port
    couldn't do because it translates a word at a time; the next word's opening
    is now carried into the rules buffer, and composed numbers get it too.
  - **`$abbrev` and vowel-less words are spelled out**, with upstream's spelling
    stress and letter-name lookup order: "USA" → `j,u:,Es'eI`, "BBC" →
    `b,i:b,i:s'i:`, bare "km" → `k,eI'Em`.  This also exposed an inverted gate in
    the unpron test that made every rule invisible to it.
  - **Combining digits before a scale word** (`_<n>a`): German 100 is
    "einhundert", 1000 "ein tausend", 1 "eins" — and German keeps the "ein" the
    port used to omit.
  - **`phonX1` is a marker**, not a sound.
  - 820 tests green (827 with all features), 0 warnings.

- **2026-09-04 (eighteenth pass) — the long tail of `-x`/`--ipa` differences.**
  A wider 84-phrase, 50-language corpus went **`-x` 53 → 75/84** and
  **`--ipa` 46 → 77/84**, with the 32-language sweep holding at 32/32
  throughout.  Almost all of it was language *configuration* that had been
  transcribed loosely:
  - **The stress tables are now a straight transcription of
    `tr_languages.c`.** The old per-language match ended in a catch-all that
    reset `stress_flags` to zero, so the flags table below it was nearly dead
    code.  Eight languages had the wrong rule outright and five more lost their
    flags.  `unstressed_wd1`/`wd2` and Russian's `S_NO_AUTO_2` came with it.
  - **`SUFX_Q`** ("don't retranslate") — Italian `casa` matches `@C) a (_S1q`
    and the retranslation was losing the intervocalic `s` → `z`.
  - **Inter-word pauses**: `vowel_pause`, `word_gap`, the `words` voice
    directive, and the `$pause`/`$brk` dictionary flags.
  - **Hyphen word-join**: "well-known" is `wˈɛlnˈəʊn`, one word with two
    stresses.
  - **`LOPT_IT_LENGTHEN`** (parsed but never applied), **`isFirstVowel` /
    `isSecondVowel`** (always false), **`phonX1`** (a marker, spoken as a
    phoneme), and the **Cyrillic `Y` group** (Russian's additions were being
    applied to every Cyrillic language).
  - The port's exception for composed entries' own stress marks is gone; C's
    `GetVowelStress` always reports the primary it finds.
  - 808 tests green, 0 warnings.  What is left in the corpus is 9 phrases:
    abbreviation clause breaks, all-caps spelling ("USA"), two number-composition
    details, Japanese's English switch, Thai tone placement, and three per-language
    vowel choices (`pt` `U`/`u`, `ru` word-final `a#`, `bn` final inherent vowel).

- **2026-09-04 (seventeenth pass) — ✅ items 2 and 6; item 5 measured.**
  - **`--ipa` (item 2) is closed.** It now renders from the clause's finished
    phoneme-code list, the way C's `GetTranslatedPhonemeString` does, with each
    phoneme's name taken from *running its bytecode program* with real
    neighbours rather than guessed from its mnemonic. 32-language sweep
    **20/32 → 32/32**; an 84-phrase corpus 46 → 58. That one change fixed all
    seven divergent languages, and forced out a run of code-list bugs `-x` had
    been hiding: `StressCondition`/`ChangeIf` ported properly (English "unseen"
    keeps its `ʌ` because that syllable is *unstressed*, not *diminished*),
    C's `LOPT_REDUCE` gate, a faithful `FLAG_FOUND`, `prevPhW`/`nextPhW` no
    longer looking across a word boundary (Dutch "twintig graden" kept its `ɣ`,
    "1km" stopped assimilating across its internal `END_WORD`), spoken symbols
    as separate words ("cat.Dog" → `kˈat dˈɒt dˈɒɡ`), `dictrules` honoured in
    number lookups, and markers dropped before synthesis' look-ahead window.
  - **The dictionary compiler (item 6) is closed.** Measured properly for the
    first time — compile the same `dictsource` with *both* compilers and speak
    the same words through each — it went from 11/19 English words to
    **1144/1154 across 80 languages**. The fixes, in rough order of how much
    they mattered: the rule-context command set (only `A B C F G H Y` are letter
    groups; `K` is "not a vowel"), the reversed list-entry order, group sorting
    and merging (without it Czech `ý` and Polish `ę` were simply unreachable),
    `RULE_PH_COMMON` sharing, `RULE_PRE_ATSTART`, `?n` rule conditions, key
    lowercasing and `U+xxxx`, `$text`/`$textmode`, the `roots`/`listx`/`emoji`/
    `extra` files, two-byte group-name truncation, and `groups3` headers for
    non-Latin scripts.
  - **Formants (item 5).** One faithful fix: the voice tone curve was divided by
    the *default curve per index*, which cancels it exactly, so the default
    curve's 3.8 dB low-to-high tilt was never applied. Dividing by a single
    constant restores the shape at the levels the port was tuned at:
    2.22 → **2.17 dB**, r 0.968 → **0.970**. Ruled out by measurement: the HF
    peaks 6–8 (zeroing them moves "hello world" by 0.03 dB), the silence noise
    floor, diphthong interpolation and durations. The residual is a *narrow*
    ~13 dB feature near 7 kHz on short utterances — a spectral-zero placement
    difference, not a level one.
  - 808 tests green (815 with all features), 0 warnings.

- **2026-09-04 (sixteenth pass) — ✅ the refreshed priority list, items 1–6.**
  `-x` parity went **25/32 → 32/32** and `--ipa` **20/32 → 25/32**.
  - **Stress (item 1).** `stressOpt <bit…>` is now read from the voice file —
    four languages configure their stress *entirely* through it and the port
    ignored it. Greek's `S_NO_AUTO_2` was invented, as was the whole Japanese
    arm; Czech/Slovak were missing `S_FINAL_DIM_ONLY | S_FINAL_NO_2`; Korean had
    no arm and fell through to the default instead of `STRESSPOSN_2LLH`.
  - **Regressive voicing (item 4).** `SetRegressiveVoicing` ported — the
    backwards walk where a cluster takes its last member's voicing, with
    `end_type` naming the switch partner. Fixes Ukrainian, and crosses word
    boundaries when the language clears the boundary bit.
  - **Mandarin sandhi (item 4).** A third-tone pair joins into one word, so
    你好 reads `ni35X'Au214_|` — one gap, no stress on the first syllable.
  - **Russian stressed vowels (item 4).** The codes path resolved a *stressed*
    allophone from stress alone; the choice is contextual and the
    context-aware pass already had it. Only the *reduction* is stress-driven
    now. (Making the same change in the IPA path broke Czech/Russian numbers
    and was reverted — see the comment in `phonemes_to_ipa_full`.)
  - **Speed (item 3).** Upstream's `speed_lookup` table replaces the linear
    `175/wpm` model, and wavegen's *second* application of the rate — which
    squared it — is gone. 80 wpm went 2.42× → 1.07× vs the oracle.
  - **Compilers (item 6).** `.Lnn` letter groups compile; ten are used by
    `en_rules`, and a rule whose group is undefined never matches. Probe still
    12/19 — the rest is rule conditions.
  - **Investigated, still open.** Item 2 (`--ipa` allophones): the clause-pass
    shortcut regresses within-word allophones and was reverted; the fix is to
    render IPA from the code list. Item 5 (formants): the tone-curve scale was
    the big win (r 0.938 → 0.963), the diphthong hypothesis was disproved, and
    the residual 4.39 dB on "hello world" has no single identified cause.
  - 793 tests green, 0 warnings.

- **2026-09-04 (fifteenth pass) — ✅ the upstream-PR survey, implemented.**
  Seven open upstream PRs judged relevant to a pure-Rust port, all landed or
  answered:

  **#2524 (`formant_factor`'s stale `pitch=82`).** The port had the deeper
  version of the same bug: it hardcoded `pitch 82 118` for *every* voice, never
  read a voice file's `pitch` line, and pinned `formant_factor` to 1.0. Now the
  line is read, and the reference is derived from C's `VoiceReset`
  (`pitch_base = 0x47000` → 80) rather than repeated — which is exactly the
  PR's point. Implementing it exposed a second bug: a vowel's pitch was **flat**
  across its syllable where C glides through the envelope every `STEPSIZE`
  samples. Fixing that moved the eight-sentence sweep to 2.47 dB / 0.963 and
  improved every voice with a wide declared pitch range (af 6.70 → 6.08 dB,
  sv 5.24 → 3.89, vi 4.55 → 3.94, ko 1.87 → 1.40).

  **#1466 (language config from data).** `intonation <n>` (22 languages) and
  `stressLength <l0…l7>` (20) are now read from the voice file. Wiring the
  latter surfaced a third bug: `-v en-GB-scotland` returned `VoiceNotFound` from
  the *audio* path, because it used `select_table_by_name` (no locale fallback)
  where the phoneme paths use `select_phoneme_table`.

  **#2523 (NFKC).** Folded, but only for the 183 characters the port was
  **dropping** — `㎏`, `㎐`, `㎞` and friends. The 139 it already reads better
  than their NFKC form (`™` → "trademark", not "T M") are deliberately excluded;
  a first attempt that folded everything regressed exactly those.

  **#2261 (`dicts/` subdirectory)**, **#2288 (cancel ordering** — the port did
  leak a whole clause after `cancel()`; now checked before any chunk is
  delivered**)**, **#2501/#2516 (MBROLA `.pho` now follows the voice's own
  pitch)**, **#2441 (fuzz-style robustness tests over every binary parser —
  random bytes, every truncation, single-byte corruptions; no panics found).**

  Checked and already fine: #2506, #2470, #2465, #2435.

  791 tests green, 0 warnings.

- **2026-09-04 (fourteenth pass) — ✅ the eight open items.** Worked the priority
  list end to end; every claim below was checked against the locally built
  upstream binary.

  **The find of the pass: accented letters never matched a letter group.** C's
  `IsLetter` folds a letter through `remove_accent` before testing membership,
  so `í` is a vowel because `i` is. The port indexed `letter_bits` by the raw
  code point, so *every* `A`/`C`/`Y` rule silently failed on an accented letter.
  Spanish `día` came out as `d` — truncated after the first letter — and French
  intervocalic `s` didn't voice before an accented vowel. One 414-entry table
  fixed both, and it had been mispronouncing accented words in every Latin-script
  language.

  **`-x` and language switching.** `-x` printed the raw `_^_` marker where
  upstream prints `(en)h@l'oU(ru)`; `translate_to_codes` now resolves switches
  and `format_x` loads the target's phoneme table. Separately, the clause-level
  post-passes were rewriting switched codes with the *clause's* table, which is
  why Arabic `H` printed as `d`. Both are byte-identical to upstream now.

  **Foreign scripts are spelled, not switched**, when upstream spells them —
  the `AL_WORDS` distinction from `tr_languages.c`. `en "хорошо"` reads out the
  Cyrillic letter names, byte-identical. Scripts with no letter names here still
  fall back to their own voice rather than dropping the text.

  **Conditional dictionary entries.** `word_to_phonemes` passed
  `lookup_symbol: true`, which disables the `$atend` gate, and never set the
  capitalisation flags — so English "a" was the letter name everywhere, "to"
  never reduced, and "I" kept primary stress. *Lesson: the flag was named for a
  symbol lookup and had been set to `true` "because it worked"; it silently
  disabled a whole class of conditional entry.*

  **`-X` and `-D`** landed (ports of `DecodeRule` / `DecodePhonemes` /
  `print_dictionary_flags`), byte-identical for rule words and dictionary hits.

  **Positional embedded commands → scoped `<prosody>`.** `\x01<n>S` now applies
  from where it appears, threaded as a marker through the code stream and the
  render loop; the SSML element emits the same commands upstream writes.

  **Compilers, now measurable.** Compiling upstream's real `en_list`+`en_rules`
  and speaking through the result: 12 of 19 probe words identical, up from 10,
  after fixing silent rules (`c (q`) and compiling `$flags`.

  **Formant quality.** The eight-sentence sweep improved to 2.52 dB / 0.961
  (from 2.79 / 0.945), mostly because "how much wood…" stopped mispronouncing
  the article. Added the voice `tone` curve (`SetToneAdjust`), applied relative
  to the default so the standard voices are unchanged and `+variant` `tone`
  lines finally do something.

  771 tests green, 0 warnings.

- **2026-09-04 (thirteenth pass) — ✅ klatt.c, MBROLA, incremental async,
  and the wrong-render-engine bug.** Four subsystems in one pass, all
  oracle-checked where an oracle exists.

  **`klatt.c` (§1.2) — ported in full.** `synthesize_frames_klatt` was a
  placeholder (a bare cascade off a phase counter); it is now a direct port of
  `parwave()` + `Wavegen_Klatt()` + `SetSynth_Klatt()` + `frame_init()` +
  `pitch_synch_par_reset()`. Cascade *and* parallel branches, the nasal
  antiresonator, both glottal source models at 4× oversampling, spectral tilt,
  breathiness, skew, `DBtoLIN`, flutter, fades — plus `espeak_rand` reproduced
  exactly (upstream's `- min` sign bug included, so noise stays comparable
  sample-for-sample; worth reporting upstream, since it biases the aspiration
  noise positive instead of centring it). Verified through `-v en+klatt` against
  the C binary on a sustained vowel: spectral-envelope **correlation 0.997**,
  RMS difference 2.4 dB.

  **⚠️ The finding that mattered most (§36):** with the Klatt engine now correct,
  a spectral sweep still showed `en` far from the oracle — because `flush_run`
  was routing *every* standard voice through Klatt. It keyed on `FRFLAG_KLATT`,
  which merely marks a frame as carrying the extra parallel-resonator
  parameters; every shipped `en` frame has it. C routes to Klatt only on
  `voice->klattv[0]`. Fixing the routing moved the eight-sentence sweep from
  **4.19 → 2.79 dB** mean spectral-envelope RMS difference (correlation 0.938 →
  0.945), and "hello world" from 14.17 → 4.61 dB. *Lesson: the earlier note
  "voices with no Klatt frames (e.g. `en`) are unaffected" was never checked
  against the data — a one-line probe would have caught it.*

  **MBROLA (§1.4) — everything but the external binary.** `compilembrola.c`, the
  binary `_phtrans` format, `GetMbrName` (all seven control bits), `WritePitch`,
  `MbrolaTranslate`, `--compile-mbrola`, `--pho`, `mb-*` voice resolution, and a
  graceful fallback to the built-in synthesizer when the (unshipped) `mbrola`
  binary is absent. All **49 shipped `_phtrans` tables round-trip byte-for-byte**
  and the compiler reproduces `en1_phtrans` exactly — a real data oracle even
  though the audio path can't run here (the C binary can't produce `.pho`
  without the databases either).

  **Async core (§7) — incremental rendering.** `Synthesizer::render` now accepts
  a sink and flushes each clause as it completes; `EspeakNg::synth_streaming{,_with_events}`
  expose it and `AsyncSynth` uses it, so audio starts flowing after the first
  clause instead of the last. Rendering order is unchanged, so the concatenated
  chunks equal `synth()` byte-for-byte. Added optional `parallel` (rayon
  `synth_many`) and `tokio-runtime` (`tokio_synth::synth`/`synth_stream`)
  features; the crate still has no required dependencies beyond `bitflags` and
  `thiserror`.

  **Also fixed:** `frame_to_peaks` now sets peak 7's height (C's `SetSynth` loops
  to 8 for heights, 7 for frequencies) — neutral on the shipped data, where
  `fheight[7]` is 0; and the voiced/noise decision read `klattp2[1]` (`AVp`) as
  `Fric`, which is index 2.

  753 tests green (760 with `parallel` + `tokio-runtime`), 0 warnings.

- **2026-09-04 (twelfth pass) — ✅ variant indicators, liaison codes, default
  tone in IPA.** Instrumented upstream's `ChangeIf` to settle the `-x` allophone
  question and found the change *does* fire for Spanish — the difference was
  purely in printing: a `/` in a mnemonic is a variant indicator that
  `WritePhMnemonic` discards, so `a/` prints `a` and `l/2` prints `l`. That also
  disproves the fifth-pass "consonant changes are deferred" theory and the
  `SFLAG_DICTIONARY` gating I had reverted; consonant `ChangePhoneme`s are now
  applied again (invisible in `-x`, audible in synthesis). Two follow-ons from
  the same sweep: French liaison phonemes that don't surface are now dropped from
  the codes path (`-x -v fr "3,14"` → `trwa`), and code 17 — a control code in
  most tables, `phonDEFAULTTONE` in a tone language — is no longer skipped as a
  pause, restoring Vietnamese's tone in IPA. Suite **717 tests green**; sweep
  `-x` **31/32**, `--ipa` 29/32.

- **2026-09-04 (eleventh pass) — ✅ vowel-break phonemes, length modifiers.**
  Reused the both-engines instrumentation trick on the `_|` vs `_!` residual and
  found the mapping simply **inverted**: C emits `_!` when the first vowel has
  primary stress and `_|` otherwise, and reads that stress *after* the placement
  rule (the port had a "pre-rules snapshot" workaround built on the wrong
  assumption). Removing the workaround and swapping the mnemonics makes German
  byte-identical on `Ich`, `achten`, `einen`, `Achtung`, `beachten` and a full
  sentence, plus Hungarian `alma`. Then implemented the last group-0
  instructions: a `length <n>` inside a program overrides the declared
  `std_length` (and `LengthAdd` adjusts it), which German uses on most of its
  vowels. Suite **717 tests green**; sweep `-x` 27/32, `--ipa` 28/32.

- **2026-09-03 (tenth pass) — ✅ per-language stress flags.** Finished the Basque
  residual from the ninth pass by instrumenting **both** engines (added an
  `fprintf` to the local upstream build) and diffing their `vowel_stress[]`
  state: identical arrays, but upstream's `stressflags` were `0x10100` where the
  port had `0`. Basque *does* have a `tr_languages.c` entry
  (`S_FINAL_VOWEL_UNSTRESSED | S_MID_DIM`) — the port simply set `stress_flags`
  in only seven arms, three of which also disagreed with upstream. Transcribed
  the full table as a base layer. A first attempt made it authoritative and
  regressed nl/pt/ru (the port's tuned values compensate for its own stress
  fill), so the port's arms still override; `eu kaixo`/`etxe` are now
  byte-identical and nothing else moved. Suite **716 tests green**; sweep
  unchanged at `-x` 25/30, `--ipa` 26/30.

- **2026-09-03 (ninth pass) — ✅ stress placement.** Chasing Crimean Tatar's
  doubled stress mark turned up three things, in increasing generality: the port
  honoured an explicit stress marker only for `caps_mark_stress` languages, where
  C honours it everywhere (composed number words excepted, which is why the
  restriction was there); the default stress rule was first-syllable where C uses
  **penultimate**, with `unstressed_wd2` 2 instead of 3; and `stressRule` in a
  voice file was never read, though ~15 languages are configured that way. All
  three fixed — `crh selâm/merhaba/kitap` are byte-identical to upstream and the
  major languages are unchanged. Residual: Basque words of ≤2 syllables still
  differ (rule 15 only fires above two, and the fallback then picks a different
  syllable). Suite **713 tests green**; on a 30-sample sweep `-x` 25/30 and
  `--ipa` 26/30 exact.

- **2026-09-03 (eighth pass) — ✅ `PauseBefore` on the formant path.** The fifth
  pass had only wired the pause instructions into the WAV branch, which never
  fires for German's /r/ (a liquid → formant path). `PhonemeRender::Frames` now
  carries the lead/trail silence, and the run accumulator flushes the current
  formant run before emitting it — a real gap, so ending the run there is
  correct.  Confirmed against `ph_german`: "trotz"/"trinken" (t + r) get 30 ms,
  "Preis" (p + r) 15 ms, word-initial "Rose" none. Covered by a unit test that
  drives the interpreter directly rather than measuring audio. Suite: **713 tests
  green**.

- **2026-09-03 (seventh pass) — ✅ default tones, decimal separators, clause
  rules.** Vietnamese turned out not to be a rules gap after all: upstream *adds*
  the tone in `CalcPitches_Tone` (`phonDEFAULTTONE` for any toneless syllable,
  falling tone 7 clause-finally). Implementing that, plus the `word_gap` `_|` for
  vi and mn, makes `xin chào`, `tôi` and `Việt Nam` byte-identical. Checked the
  decimal separator of every language the port configures against the oracle and
  found three wrong — **af, ka, kk** read "3,14" as two numbers and use the
  period. Finally, matched upstream's sentence detection: a period ends a
  sentence only before a capital, and a comma between digits is a separator, so
  `-x` no longer adds a clause break to "one. two", "etc. and", "5. item" or
  "3,14". Three test expectations that encoded the old break behaviour were
  updated (oracle-verified). Suite **709 tests green**; on a 28-sample sweep
  `--ipa` 24/28 and `-x` 22/28 exact.

- **2026-09-03 (sixth pass) — ✅ tone placement (Thai).** Replaced the positional
  tone shuffle with C's model: a tone is a property of its syllable's vowel, so
  each vowel takes exactly one — pulled back over a coda where the rules wrote it
  after (Mandarin `w'u@35n`), carried forward where they wrote it ahead (Thai
  `kh,a5?…`), extras dropped. Thai `สวัสดี` went from `s55'a2wms55a3ds` to
  `s'a5wmsa3ds` — byte-identical to upstream — as did `ขอบคุณ` and `ไทย`, while
  Mandarin/Cantonese stayed exact (10/10 on a tone-language sample). Residual:
  `กิน` picks the leading tone where upstream takes the trailing one, and
  Vietnamese emits no tone phoneme at all for `xin` (a rules gap). Suite: **709
  tests green**; sweeps `--ipa` 21/24, `-x` 18/24.

- **2026-09-03 (fifth pass) — 🟡 pause instructions, and a reverted experiment.**
  `PauseBefore`/`PauseAfter` are now captured from the phoneme program and applied
  to WAV-rendered phonemes; the formant path still ignores them (documented in
  §3b). Also chased the `-x`-only allophone spelling (`es 'ola/`, `it bUo/n…`):
  the C rule lives in `StressCondition` — `ChangeIfNotStressed` is refused for
  dictionary-given words unless `LOPT_REDUCE & 1` (Italian only). Implementing
  that gate with this port's `translation_given` flag **regressed** the sweep
  (Russian lost its reduction, Spanish kept its allophone; 18/24 → 15/23), so it
  was reverted and the finding recorded instead. Multi-character Chinese words
  were re-examined: `谢谢` is not in the shipped cmn dictionary, so upstream's
  neutral second tone has some other source — still unexplained. Suite: **707
  tests green**; sweeps unchanged (`--ipa` 21/24, `-x` 18/24).

- **2026-09-03 (fourth pass) — ✅ phoneme-program list edits, vowel changes,
  `isTranslationGiven`.** The group-0 instructions that were still no-ops now run
  on both surfaces: English gets its **linking r** from `IfNextVowelAppend(r-)`
  ("sofa area" and "father and son" are byte-identical to upstream) and the `;`
  glide from `AppendPhoneme`; two hand-rolled approximations (a mnemonic check
  for rhotic liaison, a special case for the word "or") were deleted in favour of
  the real mechanism. Program-driven `ChangePhoneme` was extended from the `#`
  class to **vowels**, which fixes `the apple` → `ðɪ ˈapəl` and Hindi `भारत` →
  `bʰˈaːɾət`; consonant changes stay deferred because upstream's `-x` keeps a
  plain `l` in "world" (they happen later, in `MakePhonemeList`).
  `ChangePhoneme(NULL)` now deletes the phoneme instead of printing `_` inside
  Russian words, and `isTranslationGiven` is implemented for tr/ba/tt. Eight test
  expectations that encoded the older, further-from-upstream values were updated,
  each re-verified against the oracle. Sweeps: **`--ipa` 21/24 exact** (from
  17/24 at the start of the pass) and **`-x` 18/24**; suite **707 tests green**.

- **2026-09-03 (third pass) — ✅ vowel_pause, decimal fractions, switch grouping.**
  Continued down the §3b "still open" list, oracle-gated as before. **`vowel_pause`**
  (`langopts`) was parsed into `StressOpts` but never applied: implemented in
  `set_word_stress` for de/nl/af (0x30), hu (0x20) and jbo (0x20c), which is what
  puts the break in front of a vowel-initial word or prefix-stripped stem —
  `de beachten` → `b@_!'axt@n`, byte-identical to upstream, and German IPA is
  unchanged. **Short decimal fractions** now read as one number
  (`NUM_DFRACTION_2`/`_4`, ~25 languages, plus the Portuguese text path):
  `cs 0,25` and `fr 3,14` are byte-identical; pl/fi/tr/es differ only by
  pre-existing phoneme details. **Foreign-script runs** are wrapped in a single
  `(target)…(base)` group like upstream. A 24-sample cross-language `-x` sweep
  went from **15/24 to 17/24 exact**; suite **704 tests green**. What's left is
  listed at the end of §3b — chiefly the remaining group-0 instructions
  (AppendPhoneme/InsertPhoneme/length), the `_|` vs `_!` choice for
  dictionary-stressed words, `-x`-only allophone spelling, Thai tone selection
  and multi-character Chinese words.

- **2026-09-03 (second pass) — ✅ Chinese/tones, plus two engine bugs the oracle
  exposed.** Continuing the same session's implementation pass, still oracle-gated
  against the locally built upstream `f135499`. **Chinese now speaks:** the root
  cause was `langopts.textmode` — cmn/yue/zh compile their dictionaries as
  replacement *text*, reversing `FLAG_TEXTMODE`, so `你`'s reading `ni3` was being
  interpreted as phoneme codes (`qr`). Fixed, then added tone phonemes (phSTRESS
  entries with a program, previously dropped), tone-after-vowel placement, the
  `_|` inter-word gap, tone digits inside pinyin words, and a port of
  `CalcPitches_Tone`'s Mandarin sandhi (default tone, 214+214→35, 51+51→53,
  neutral-tone levelling). `-x -v cmn 中文`/`北京`/`请问` and `yue 你好` are now
  **byte-identical to upstream**. **German prefix bug (pre-existing, severe):**
  the rules scan must stop at a standard prefix so the caller can re-translate the
  stem; the port kept scanning, a later suffix overwrote `end_type`, and
  `end_phonemes` still held the prefix's phonemes — every `be-`/`ge-` verb with an
  ending read `b'@b@`. `bezahlen` is now `bətsˈɑːlən`, == upstream. **Codes-path
  allophones:** `ChangeIfDiminished` was IPA-only, so `-x` and the synthesizer
  used citation vowels (ru `molok'o` → now `mVlVk'o`, == upstream). Suite: **701
  tests green**; a broad 24-sample cross-language `-x` sweep matches upstream
  exactly on 15, with the remainder being known per-language details listed in
  §3b.

- **2026-09-03 — ✅ Upstream-parity implementation pass (data + 12 engine fixes).**
  Built upstream `master` (`f135499`) from source with CMake and used the
  resulting binary as a **live oracle** for every change below; refreshed
  `espeak-ng-data/` from that build (123 dicts / 151 voices, MBROLA files kept
  from the old bundle) and regenerated `data-crates/` + the Cargo feature lists.
  Engine work, each verified against the oracle: **script routing** for 22 scripts
  (no more silently dropped `-v ru "привет hello"` / `-v en "hello مرحبا"`), the
  `alt_alphabet` hook (#2511), Serbian digraphia, dictionary-beats-script;
  **program-driven `ChangePhoneme`** in a new `interpret_phoneme_ctl` PhonemeList
  mode (first change wins, as in `synthdata.c`) applied on the IPA *and* codes
  paths — this fixed `pt dente` (`tˈẽŋty` → `dˈẽŋty`), gave pt-br its affricates
  (`n'oItSy`, == oracle) and made German `zwanzig` end in `ç` like upstream;
  **cross-word `nextPh` context** (pt `os amigos` → `uz …`, `três` → `…ʃ`);
  **voice `phonemes` directive outranks the tag** (European Portuguese had been
  using the Brazilian table); **`-x` regional-locale fallback**; **skin-tone and
  tag-sequence emoji** (`👍🏽` was silent, England flag said "black flag");
  **Sinhala ZWJ** (`kɹˈɐ`, byte-identical); **fraction reading** for ru/hu driven
  by `_0Z<n>`/`_1f` data; **composed-number end stress** (pt `101` == "cento e
  um"); **`SUFX_M` chains** (`s's's` → `'EsI2zI2z`, == oracle); **tone digits**
  kept in Chinese words; **`[[…]]` word boundaries**; **event rescaling after the
  tempo/pitch post-pass** (#2470). New `tests/upstream_1_53_parity.rs` (13 tests)
  plus updated expectations in 5 existing tests where the refreshed data or the
  now-correct phoneme programs changed the right answer — each re-verified
  against the oracle. **Suite: 696 tests, 0 failures.** Remaining items and the
  deliberate divergences are listed at the end of §3b.

- **2026-09-03 — 📋 Upstream sweep (no code changes).** master HEAD `f135499`
  (2026-09-01), **432 commits ahead of the `1.52.0` tag our data comes from**; 13 PRs merged
  since 2026-08-01. Recorded as §3 items **38–43** (merged engine work: #2511 `alt_alphabet` +
  tone-phoneme carry-over, #2495 SUFX_M suffix chains, #2492 `i_APPEND_IFNEXTVOWEL`, #2475
  merged, ku/fa/hu/nl data), a new **§3b** (items **44–54** — the whole unreleased 1.53.0
  changelog vs. our 1.52.0 data), a re-swept §4, and §5 issues opened since 2026-08-01.
  Status corrections: **#2475 merged 2026-08-02** and **#2422/#2423/#2481 merged** — all four
  were tracked here as open. Behavioural probes run against `./target/debug/espeak-ng-rs`:
  `-v ar "…如果您因为翻译…"` → Han run collapses to a stray `ɣ`; `-v ru "привет hello"` →
  the Latin word is **silently dropped** (both `--ipa` and `-x`); `-v en "hello 你好"` /
  `"hello مرحبا"` → same; `👍🏽` → **empty**; `🏴󠁧󠁢󠁥󠁮󠁧󠁿` → "black flag"; `-v si "ක්‍ර"` → `k ɹˈə`;
  `-v ru "0,25"` → digit-by-digit, no fraction naming; `-v crh|mn|ps|rup|fo|kaa|lij` →
  "dict not found"; `-v ab` → silently empty. Priority order re-ranked: **data refresh is
  now priority 0**, silently-dropped input is priority 1.

- **2026-08-01 — ✅ Dotted version numbers no longer gain a spurious "zero"
  (`1.2.3`→"one point two point three").**  A dotted number with 2+ decimal points
  (a version string) had its second `.3` parsed as a fresh leading decimal
  (`0.3`), so `1.2.3` read "one point two **zero** point three".  espeak reads
  each `.` as "point".  **Fix:** the leading-decimal tokenizer branch now checks
  whether the `.` immediately follows a `Number` — if so it's a *continued*
  decimal and takes an **empty** integer (rendered as just "point three", no
  "zero"); a standalone `.5` (nothing before it) keeps `integer:"0"` → "zero point
  five".  Extended the `Decimal` renderer to skip the leading cardinal for an
  empty integer.  Verified: `1.2.3`→"one point two point three", `1.2.3.4`,
  `192.168.0.1`, `v1.2.3` all lose the stray "zero"; `.5`/`0.5`/`3.14` unchanged.
  en-specific (only `.`-decimal languages hit this branch; de `1.2.3` already went
  through the `.`→"punkt" path).  Full suite green (**677 passed / 0 failed**).
  `tests/version_numbers.rs` (2 tests).  *(Minor residual: the continued "point"
  gets a space the oracle omits — "two·point" — cosmetic, words identical.)*
- **2026-08-01 — ✅ Non-time colon read/dropped, not a clause break (`3:2`→"three
  colon two").**  Closes the last deferred `:` sub-case.  A `:` between numbers
  that isn't a valid time (`3:2`, `10:3`, `10:300`) was breaking the clause;
  espeak instead reads it as the ":" symbol in English ("colon") and drops it
  silently everywhere else (de/fr `3:2`→"drei zwei"/"trois deux") — never a break.
  Mirrored the earlier `.`→"dot" work: the tokenizer now emits `Punctuation(':')`
  for `:`+digit (was `ClauseBoundary`), `apply_time_reading` matches that (valid
  times still consumed, colon silent + "uhr"/"heures"/"e" connector), and its
  copy-through disposes of the surviving non-time colon — English inserts a
  "colon" word, others drop it.  A `:` before whitespace ("Note: this") stays a
  clause boundary.  Verified against the oracle (en reads "colon", de/fr drop, all
  times unchanged); full suite green (**676 passed / 0 failed**).
  `tests/time_reading.rs` (updated 1, +1 → 5 tests).  *(Aside: found while sweeping
  English number formats — also noted `1.2.3` version strings get a spurious
  "zero" (the leading-decimal parse fires on the second `.3`), and `mp3`/`4WD`
  spell-vs-pronounce; left as separate items.  The hr/bs/sr `tri`-rhotic bug is a
  deeper phoneme-rendering issue, still deferred.)*
- **2026-08-01 — ✅ Croatian scale-word declension (Slavic VAR5) — all 5 variants
  done.**  The last of espeak's `NUM2_THOUSANDS_VAR*` groups.  Croatian's scale
  word takes `_1M{n}` ("tisuću") for count ≡ 1 mod 10, `_0MA{n}` ("tisuće") for
  2–4, and `_0M{n}` ("tisuća") for 5+/teens, with a feminine "thousand" ("dvije",
  any count, like Russian).  The hbs (Serbo-Croatian) number data isn't in the
  `hr_list` source, so I added `SlavicThousands::Hr` (VAR5 rule + Ru-style
  feminine) and **verified empirically** the compiled dict *does* carry
  `_1M1`/`_0MA1`/`_0M1` — `2000`→"dvije tisuće", `21000`→"… tisuću", `5000`/
  `11000`→"tisuća", `22000`→"dvadeset **dvije** tisuće" (fem compound), `2000000`→
  "dva milijuna" — all structurally exact vs. the oracle (residual diffs are the
  pre-existing `tri`→"t ɪ" rhotic-cluster rendering and vowel/voicing rendering).
  Full suite green (**675 passed / 0 failed**).  `tests/russian_thousands.rs`
  (+1 → 7).  **Slavic thousands declension is now complete for all five espeak
  variants (ru, cs/pl, lt, sk, hr).**  *(bs/sr also use VAR5 but are blocked on a
  **data** issue — the port speaks Croatian "tisuću" where they want "hiljadu";
  and the `tri`-cluster rhotic-rendering bug remains as a separate synthesis-layer
  item, affecting hr/bs/sr 3000/300/etc.)*
- **2026-08-01 — ✅ Slovak scale-word declension (VAR2 + narrow feminine).**
  Slovak shares Czech's `M_Variant` (VAR2: `_0MA{n}` for count exactly 2–4) but
  differs in gender: "thousand" is feminine only for a **bare single-digit** count
  — `2000`→"**dve** tisíc", yet `22000`→"dvadsať **dva** tisíc" and millions stay
  masculine ("dva milióny").  Added `SlavicThousands::Sk` (VAR2 rule shared with
  `Cs`) and generalized the branch's feminine flag per-variant: `Ru` → any count
  (compound propagates, "двадцать две"), `Sk` → `count < 10` only.  Also fixes sk
  millions, which were using the 5–9 form ("dva miliónou" → "dva **milióny**" via
  `_0MA2`).  Verified structurally exact vs. the oracle (residual diffs are
  Slovak's pre-existing `tri`→təɾ epenthesis and voicing-assimilation rendering),
  and cs stays correct (it shares the VAR2 arm).  Full suite green (**674 passed
  / 0 failed**).  `tests/russian_thousands.rs` (+1 → 6 tests).  **Slavic status:
  ru/cs/pl/lt/sk done; only VAR5 (bs/hr/sr) remains** — hr is tractable (feminine
  "dvije" + VAR5), but bs/sr have a **data** mismatch (the port speaks Croatian
  "tisuću" for "hiljadu") and the pre-existing `tri`-cluster rhotic-rendering bug.
- **2026-08-01 — ✅ Lithuanian scale-word declension (Slavic VAR4).**  Fourth of
  espeak's five `NUM2_THOUSANDS_VAR*` groups.  Lithuanian's "tūkstantis" has three
  plural forms by count that the port collapsed to one ("tūkstančiai"): `10000`/
  `11000`/`20000`/`100000` should be the genitive plural "tūkstančių" (`_0MB`),
  and `21000` the singular "tūkstantis" (`_0MA`), while `2000`–`9000`/`22000` keep
  "tūkstančiai" (`_0M`).  Added `SlavicThousands::Lt` with its `M_Variant` rule
  (`_0MB` for a teen or count ≡ 0 mod 10, `_0MA` for ≡ 1 mod 10, else `_0M`);
  count 1 already used the dedicated `_1M1`.  No gender.  Verified **7/7**
  structurally exact vs. the oracle (residual diffs are lt's pervasive unstressed
  `ɪ`≈`i` and stress-mark rendering).  Full suite green (**673 passed / 0
  failed**).  `tests/russian_thousands.rs` (+1 → 5 tests).  *(Aside: lt `10000000`
  is an **oracle** bug — espeak reads 10 million as "ten thousand"; the port is
  correct.  Remaining Slavic: VAR5 bs/hr/sr — feminine "dvije/dve" plus the bs/sr
  "hiljadu" **data** mismatch and the `tri`-cluster rhotic rendering; and sk —
  VAR2 but feminine, no `_0MA1`.)*
- **2026-08-01 — ✅ Czech & Polish scale-word declension (Slavic VAR2/VAR3).**
  Extended last iteration's Russian declension to two more of espeak's
  `NUM2_THOUSANDS_VAR*` groups.  Czech `2000`→"dva tisíc" (should be "dva
  **tisíce**"), `2000000`→"dva miliónů" (→ "dva **milióny**"); Polish `2000`→"dwa
  tysiące", correctly keeping the gen-plural for 5+ ("pięć tysięcy").  Added
  `SlavicThousands::{Cs, Pl}` with their `M_Variant` rules (Cs: `_0MA{n}` only for
  count *exactly* 2–4; Pl: for count ≡ 2–4 mod 10, teens excluded) and refined the
  shared branch to check a **complete `_{count}M{scale}` word first** (Czech/Polish
  `_1M1`="tisíc"/"tysiąc" bakes in the "one" — cleaner than an omit flag, and it
  matches espeak's `LookupThousands` order).  That refactor let Russian drop its
  now-moot omit tweak while staying correct ("одна тысяча", "две тысячи").  cs/pl
  verified **structurally exact** for `2000`/`3000`/`5000`/`22000`/`2000000`/
  `5000000` (residual diffs are pre-existing voicing-assimilation / `tɕ` rendering);
  full suite green (**672 passed / 0 failed**).  `tests/russian_thousands.rs`
  (+1 → 4 tests).  *Still deferred: VAR4 (lt — mostly works via `_1M1`, needs the
  `_0MB` gen-plural for teens/×10), VAR5 (bs/hr/sr — feminine "dvije/dve", plus a
  bs/sr **data** issue where the port uses Croatian "tisuću" for "hiljadu" and the
  pre-existing `tri`-cluster rhotic rendering); and sk (VAR2 but feminine "dve",
  no `_0MA1`).*
- **2026-08-01 — ✅ Russian scale-word declension (gender + case).**  Took on the
  Slavic pass flagged last iteration.  Russian "тысяча"/"миллион" decline by the
  count before them and the count's trailing "one"/"two" agrees in gender — the
  port dropped the "one" and always used the gen-plural ("тысяч", "два тысяч").
  Ported espeak's `M_Variant` / `NUM2_THOUSANDS_VAR1`: scale word `_1MA{n}` for
  count≡1, `_0MA{n}` for 2–4, `_0M{n}` for 5+/teens (mod-10, 11–19 excluded);
  "thousand" is feminine (одна/две via `_1f`/`_2f`), the higher scales masculine;
  the "one" is spoken.  Added a `SlavicThousands` enum + `thousands_variant` field
  (ru only), a Slavic branch in `append_scale_word`, and gender threading through
  `num3_phonemes` (new `num3_phonemes_g`/`unit_ph`; the old signature is a
  wrapper, so no other call site changed) so a **compound** count keeps its gender
  ("двадцать **одна** тысяча", "двадцать **две** тысячи").  Verified **structurally
  exact** for `1000`/`2000`/`3000`/`5000`/`11000`/`21000`/`22000`/`25000`/
  `100000`/`1000000`/`2000000`/`5000000`/`1234` (the residual diffs are Russian's
  pre-existing a/ɑ, e/ɛ vowel rendering).  Gated on the `Ru` variant, so bg/uk/be
  and all non-Slavic languages are untouched (spot-checked bg/en/de).  Full suite
  green (**669 passed / 0 failed**).  `tests/russian_thousands.rs` (3 tests).
  *(Deferrable extensions: the other `NUM2_THOUSANDS_VAR2–5` variants — cs/sk,
  pl, lt, bs/hr/sr — follow the same shape with different M_Variant rules; uk/be
  may want their own; and the uk-millions **oracle** bug (`1000000`→"тисяча") is
  an upstream espeak issue where the port is already correct.)*
- **2026-08-01 — ✅ Exact-thousand variant `_0M{n}x` (7 langs) + omit-one-thousand.**
  Generalized last iteration's ml-hundred fix to the thousands scale.  A *round*
  N000 uses a distinct scale word from one spoken before more digits — Tamil
  "ஆயிரம்" (āyiram) for `1000`/`2000` vs "ஆயிரத்தி" (āyiratti) in `1001` — encoded
  as `_0M{n}x` (exact) vs `_0M{n}` (combining).  The port always used `_0M{n}`.
  Read espeak's `LookupThousands` to confirm, then threaded an `exact` flag (whole
  number has no non-zero digit below this scale) through `append_scale_word`/
  `append_cardinal_group`/their 3 call sites (`scale_group_is_exact` helper;
  ordinal-path groups are never exact).  `append_scale_word` now prefers
  `_0M{n}x` for an exact group.  Purely additive — only the 7 `_0M{n}x` languages
  (fa/ka/kn/ml/si/te/ta) are affected, and only on exact groups; en/es/fr/ru/de
  `1000`/`1234` verified unchanged.  Also set `thousands.omit_one_prefix` for the
  three that drop the "one" before "thousand" (ta/ml/si — kn/te/fa correctly keep
  it, verified).  Now `ta`/`ml`/`si` `1000`→"āyiram"/"āyiram"/"ahas" and
  `1001`→"āyiratti onru" etc.  Full suite green (**668 passed / 0 failed**).
  `tests/dravidian_hundreds.rs` (+1 → 4 tests).  *(Aside: `ru 1000` wants "одна
  тысяча" — feminine "one" agreement the port drops — pre-existing, unrelated to
  this change; noted for a future Slavic pass.)*
- **2026-08-01 — ✅ Malayalam bare 1-hundred ("നൂറ്", was "ഒരു നൂറ്").**  Finished
  the item deferred last iteration.  Malayalam is unique among the `_0C0`
  languages: it has both the bare hundred word (`_0C0`="നൂറ്" / `_0C`="നൂറ്റി")
  and a dedicated `_1C0`/`_1C` that bakes in "ഒരു" (oru = one); espeak's
  `NUM2_OMIT_1_HUNDRED_ONLY` makes it drop the "one" *and* skip the dedicated word
  for a bare `100`, using `_0C0` instead — Spanish/French/Portuguese, by contrast,
  have no bare `_0C0` so their `_1C0` ("cien"/"cent"/"cem") is correct and must be
  kept.  Added a **separate** `HundredsGrammar.omit_one_hundred_word` flag (set
  for `ml` only): for a bare 1-hundred it skips `_1C`/`_1C0` and takes `_0C0`
  (exact) / `_0C` (compound).  The bare-`_0C0` path is gated on this flag, so ka/
  kn/te (the other `_0C0` languages) and es/fr/pt/en are provably untouched —
  verified `100`/`101` unchanged for all of them.  Now `ml 100`→"നൂറ്", `101`→
  "നൂറ്റി ഒന്ന്", `200`→"ഇരുന്നൂറ്" (dedicated, kept).  Full suite green (**667
  passed / 0 failed**).  `tests/dravidian_hundreds.rs` (+1 → 3 tests).
  *(Still open: `ml 1000` is the same shape one scale up — "ആയിരം" bare vs the
  port's "ഒന്ന് ആയിരത്തി"; a thousands-level `_0M1`-standalone analog, deferred.
  Georgian `ka` remains a separate vigesimal item.)*
- **2026-08-01 — ✅ Standalone vs. combining hundred form (Tamil/Kannada/Telugu).**
  Picked up a deferred item.  These languages have a distinct word for a *round*
  hundred vs. one *before a remainder* — Tamil `100`="நூறு" (nūru) but `101`=
  "நூற்று ஒன்று" (nūṭṭri ondru) — encoded in the dict as `_{n}C0` (exact) vs
  `_{n}C` (combining).  The port checked `_{n}C` **first** and only fell back to
  `_{n}C0`, so every bare `100`/`200`/… took the wrong combining form.  Read
  espeak's `LookupNum3` to confirm the priority, then reordered the hundreds logic
  to prefer `_{n}C0` for an exact hundred (`tensunits == 0`), `_{n}C` otherwise —
  a minimal change that only affects languages defining *both* keys.  Verified
  `ta`/`kn`/`te` fixed (`ta 100`→"nūru", `200`→"irunūru", `101` still "nūṭṭri")
  with **no regression** to the other 11 `_NC0` languages (es/fr/pt/el/en `100`–
  `300` unchanged).  Full suite green (**665 passed / 0 failed**).
  `tests/dravidian_hundreds.rs` (2 tests).  *Deferred:* Malayalam needs more — its
  `100` is the bare `_0C0` "nūṟṟu" (it omits the "one" *and* uses the bare-hundred
  word, espeak's `NUM_OMIT_1_HUNDRED` path), which the minimal reorder doesn't
  cover (it still emits `_1C0` "oru nūṟu").
- **2026-08-01 — ✅ Conditional hundreds-conjunction + Swahili teens ("na").**
  Implemented the feature deferred last iteration.  Afrikaans/Swahili put a
  conjunction before the remainder for `101`–`120` ("honderd **en** een", "mia
  moja **na** ishirini") but drop it before a compound that already carries its
  own ("honderd vier-en-dertig", "…thelethini na nne").  Found the clean unifying
  rule: **add it only when the remainder is a single word** — `remainder % 10 == 0`
  (round ten) or a direct `_{n}` dict entry (unit / single-word teen).  This one
  test covers both languages despite their teens differing (af "vyftien" is a word
  → gets "en"; sw "kumi na tano" is compound → doesn't).  Added
  `HundredsGrammar.conjunction_before_simple_remainder` (set for af/sw), kept
  English's unconditional flag separate.  **15/16 probed cases exact.**  The lone
  miss exposed a *separate* pre-existing bug — Swahili teens were composed without
  their "na" (`15`→"kumi tano", should be "kumi **na** tano") — so I also fixed the
  compositional-teen path to insert `_0and` when `tens == WithConjunction`, gated
  so it only reaches Swahili (es/ro/is/sq teens are dedicated single words that
  never hit that path; Turkish/Hungarian are `Standard`/`UnitsThenConjunction`,
  untouched — verified).  Now af `101`–`120`/`134` and sw `11`–`19`/`101`–`134`
  are oracle-exact.  Full suite green (**664 passed / 0 failed**).
  `tests/number_conjunctions.rs` (+1 → 9 tests).
- **2026-08-01 — ✅ Amharic omit-one, Swahili "na" conjunction.**  Swept 15 more
  (mostly Dravidian/African/SE-Asian) languages vs. the oracle.  Two more clean
  config fixes: Amharic `100`→"አንድ መቶ" (should be "መቶ"; `1000`→"አንድ ሺ" keeps "አንድ")
  → hundreds `omit_one_prefix`; Swahili dropped the "na" that joins tens and units
  (`21`→"ishirini moja", should be "ishirini **na** moja") → `WithConjunction`.
  Both verified segment-exact (Swahili `134`="mia moja thelethini na nne" is
  byte-exact — the tens-part "na" appears with no spurious "na" after the
  hundred).  Full suite green (**662 passed / 0 failed**).
  `tests/number_conjunctions.rs` (+2 → 8 tests).
  *Deferred (needs its own careful pass — the condition is language-specific):*
  a **conditional hundreds-conjunction** — Afrikaans/Swahili say "honderd EN
  een"/"mia moja NA moja" for `101` but drop it before a compound remainder
  (`134`); the rule is "add unless the remainder already carries its own
  conjunction", which depends on whether a language's teens are single words
  (af "vyftien") or compounds (sw "kumi na tano").  *Also deferred:* Tamil/
  Kannada/Malayalam/Marathi "standalone vs combining hundred" forms
  (`ta 100`="nūru" alone but "nūṭṭri" in `101`), and Khmer/Lao/Burmese which have
  no number data (the port emits wrong-language fallbacks; the oracle is empty).
- **2026-08-01 — ✅ More compound-number fixes: kk/az omit-one, Afrikaans
  units-first.**  Continued the oracle sweep across 14 more languages.  Three
  clean bugs: Kazakh `100`→"бір жүз" (should be "жүз"; but `1000`→"бір мың" keeps
  "бір"), Azerbaijani `100`/`1000`→"bir yüz"/"bir min" (both should drop "bir"),
  and **Afrikaans read tens-first** (`21`→"twintig een", `34`→"dertig vier") when
  it is Germanic **units-first** like Dutch (`21`→"een-en-twintig",
  `34`→"vier-en-dertig") — but unlike Dutch it keeps "een honderd"/"een duisend".
  All pure config: `kk`→hundreds `omit_one_prefix`; `az`→hundreds+thousands
  `omit_one_prefix`; `af`→`TensGrammar::UnitsThenConjunction` (no omit-one),
  split out of the comma-decimal catch-all.  **14/14 probed cases now segment-
  exact vs. the oracle** (Afrikaans byte-exact), with `200`/`300` multipliers and
  round tens correctly unaffected.  Full suite green (**660 passed / 0 failed**).
  `tests/number_conjunctions.rs` (+2 → 6 tests).  *(Deferred as deeper/rendering
  issues, not clean config: Basque `eu` & Maltese `mt` vigesimal + word order,
  Georgian `ka` vigesimal, and a bs/hr `tri`→"t ɪ" cluster-rendering bug.)*
- **2026-08-01 — ✅ Compound-number conjunctions/prefixes for ro/is/sq/cy/hy.**
  Swept number composition across 11 less-verified languages vs. the oracle;
  found five with missing joining words: Romanian dropped "și" (`21`→"douăzeci
  unu", should be "douăzeci **și** unu"), Icelandic dropped "og" (`34`), Albanian
  dropped "e" (in both tens→unit and hundreds→remainder), and Welsh/Armenian
  emitted a spurious "one" before "hundred" (`100`→"**un** cant"/"**mek** harjur"
  vs "cant"/"harjur").  All reuse existing grammar machinery (the conjunction is
  the data-driven `_0and` dict word; `WithConjunction` / `use_conjunction_with_remainder`
  / `hundreds.omit_one_prefix` flags already existed) — so the fix is pure config:
  gave `ro`/`is` `TensGrammar::WithConjunction`, `sq` that plus
  `use_conjunction_with_remainder`, and `hy`/`cy` `omit_one_prefix`, splitting
  them out of the comma-decimal catch-all (cy stays period-decimal).  Verified
  against the oracle — structurally exact for all (sq byte-exact; the rest differ
  only in pre-existing rendering: Romanian tr-epenthesis, Armenian ɹ/r, Icelandic
  length marks).  Round tens (`ro 20`) and 200/300 (`cy`/`hy`, multiplier kept)
  are correctly unaffected.  Full suite green (**656 passed / 0 failed**).
  `tests/number_conjunctions.rs` (4 tests).  *(Georgian `ka` also has gaps — a
  complex vigesimal system for 99 and a wrong `100` — left as a larger separate
  item.)*
- **2026-08-01 — ✅ Hyphen between numbers reads as "dash"/"Strich" (`5-10`→"five
  dash ten", was "five ten").**  Swept common English patterns vs. the oracle;
  most diffs were cosmetic hyphen-spacing or cases where the port is *better* than
  espeak (`1990s`→"nineteen nineties", `vs.`→"versus", `60s`→"sixties").  The one
  clear gap: an ASCII hyphen-minus `-` directly between two numbers is read by
  espeak as the `-` symbol — English "dash", German "Strich" — but the port
  dropped it, giving the ambiguous "five ten".  Mapped the rule precisely: only
  between two digits (`x-5`/`5-x` drop, letter-adjacent), only the ASCII `-`
  (en/em dashes `5–10`/`5—10` stay silent), and only en/de voice a word (fr/es/
  it/ru and the rest drop it).  **Fix:** a new `apply_dash_reading` token-rewrite
  pass (after `apply_time_reading`) rewrites `Number '-' Number` into number ·
  `Word("dash"|"Strich")` · number for en/de, chaining for `3-4-5`.  Purely
  additive — the port already dropped the hyphen for every language, so non-en/de
  behaviour is unchanged and there is no regression surface.  No date conflict:
  espeak doesn't parse `3-4-2020`/`2020-01-15` as dates (nor does the port; its
  `dateize_english` is SSML-`say-as`-only).  Verified against the oracle (en/de
  dash exact, fr/es/ru drop, letter/en-dash cases untouched); full suite green
  (**652 passed / 0 failed**).  `tests/dash_reading.rs` (4 tests).  *(Noted as
  intentional divergences, not bugs: the port reads `1990s`/`60s` as decades and
  `vs.` as "versus" — nicer than espeak's literal spellings.)*
- **2026-08-01 — ✅ `№` (numero sign) readings reconciled with the oracle across
  languages.**  Last small deferred punctuation item.  Probing `№5` across 16
  languages showed the port diverged several ways: it **dropped** `№` for
  de/it/pt/nl/cs (oracle reads a word), **added** it for uk (oracle drops), and
  read en as "number" (oracle: "numero").  espeak reads `№` as each language's own
  numero word only where one is defined, else drops it.  **Fix** (`lookup_symbol_name`):
  en → "numero" (was "number"); added de/it → "numero", pt → "número", nl →
  "nummer", cs → "číslo"; removed uk (now drops); ru/bg keep "номер"; fr/es/pl/sv/
  nb/da/tr still drop.  Now structurally oracle-matched for all 16 probed langs
  (residual diffs are pre-existing word rendering — the en "numero" `ɹ`-drop is in
  the word itself, plus uk/bg/pl palatalization).  Note the oracle actually reads
  "No. 5" → "no five" (the port's "No."→"number" is a deliberate port-only
  convenience, left as-is but decoupled from `№` in the tests).  Full suite green
  (**652 passed / 0 failed**); updated 2 assertions in `tests/symbol_reading.rs`
  that had coupled `№` to "No."/"number".  *(All three deferred `.`/`:`/`№`
  punctuation items from the last few iterations are now done; the non-time `:`
  disposition — en `10:3`→"ten colon three" — remains the one open sub-case.)*
- **2026-08-01 — ✅ Time reading `H:MM[:SS]` — colon is a silent time separator,
  not a clause break (`10:30`→"ten thirty", was "tˈɛn⏎θˈɜːti").**  Third deferred
  `:`/`.`-punctuation item.  The port broke the clause at every `:`; espeak reads
  a colon-separated time as consecutive numbers with the colon silent.  Mapped
  the full rule against the oracle: a field must be **exactly two digits** to be a
  minute/second (`10:3`/`10:300` are not times); the hour drops a leading zero
  (`02:30`→"two thirty", `00:30`→"zero thirty") while the minute keeps it
  (`10:05`→"…zero five"); and de/fr/it insert a **connector** between hour and
  minute for a valid hour **0–23** only (`10:30`→de "zehn **uhr** dreißig", fr
  "dix **heures** trente", it "dieci **e** trenta"; `24:30`/`100:30` get none).
  **Fix:** a new `apply_time_reading` token-rewrite pass (run after
  `suppress_abbreviation_periods` in both the `text_to_ipa` and `translate_to_codes`
  pipelines) rewrites `Number : 2-digit-Number [: 2-digit-Number]` into
  hour · [connector Word] · minute · [second] with the `:` boundaries dropped;
  the connector is inserted as a literal `Word("uhr"|"heures"|"e")` the dictionary
  translates (French "dix heures" even gets the correct `/z/` liaison via the
  earlier liaison work).  Verified against the oracle — 11/19 segment-exact and
  the other 8 differ only in pre-existing rendering (de `r`/`ɾ` & `-ig`→`ç`,
  `hundert`/`einhundert`, it `tr`/`təɾ`, en "at" aspiration), never in time
  structure.  Full suite green (**648 passed / 0 failed**).
  `tests/time_reading.rs` (4 tests).  *(Still deferred: the non-time `:`
  disposition — en `10:3`→"ten **colon** three" reads `:` as a symbol — and the
  `№` symbol name for German.)*
- **2026-08-01 — ✅ Period between words/digits reads as the "dot"/"point"/"punkt"
  symbol (`cat.Dog`→"cat dot dog", de `3.14`→"drei punkt vierzehn").**  Second
  half of the `.`-not-before-whitespace disposition (the acronym case landed last
  iteration).  espeak treats a `.` immediately followed by a letter/digit not as
  a full stop but as the `.` character, read by its per-language name.  **Fix:**
  after the single-letter-acronym check, a `.` followed by an alphanumeric now
  emits `Token::Punctuation('.')` (not a `ClauseBoundary`), and `.` was added to
  `is_spoken_symbol` so that token is spoken by default via the existing
  `lookup_symbol_name('.')` (already returns en "dˈɒt" / de "pˈʊŋkt", used by
  `--punct`).  Low-risk: `Punctuation('.')` was previously never constructed, and
  `is_spoken_symbol` feeds exactly one render arm, so only these new tokens are
  affected; `ClauseBoundary('.')` (real sentence ends) is untouched.  English
  `3.14` never reaches it (its `.` is the decimal separator, consumed by the
  number tokenizer first), so `3.14`→"three point one four" is unchanged; comma-
  decimal locales (de/es/…) now read `3.14`→"drei punkt vierzehn".  Verified
  against the oracle (`cat.Dog`/`5.and`/de `3.14`/de `3.5` all match); full suite
  green (**646 passed / 0 failed**).  `tests/acronym_periods.rs` (+2 → 5).
  *(Aside: the stale GAPS note "#2248 pt-br → phoneme table not found" no longer
  reproduces — `pt-br`/`en-us`/`es-419`/`fr-ca`/… all resolve and match the
  oracle; the regional-variant fallback works.  Still deferred: `:` in times
  `10:30`→"…uhr…", and the `№` symbol name for German.)*
- **2026-08-01 — ✅ Single-letter acronym periods no longer break the clause
  (`U.S.A.`, `e.g.`, `i.e.`).**  Broadened the oracle diff to symbols/punctuation
  across en/de and found the port treats **every** `.`/`:` as a clause boundary,
  so acronyms shattered into separate clauses: `U.S.A.`→"juː⏎ɛs⏎eɪ" (three
  sentence breaks), `e.g.`→"iː⏎dʒiː".  Confirmed espeak's rule with `-x`: a `.`
  terminates a sentence only when followed by whitespace/end; tight between single
  letters it is a silent acronym separator ("U.S.A."→"jˌuːˌɛsˈeɪ", no breaks).
  **Fix** (narrow, low-risk — the clause splitter is central): in the tokenizer's
  boundary branch, a `.` that immediately follows a one-letter word *and* is
  immediately followed by a letter is dropped instead of emitting a
  `ClauseBoundary` (`is_single_letter_word` helper, ignoring a Lojban stress
  mark).  Trailing periods (`end.`, `5.`), decimals (`3.14`→"three point one
  four"), list markers (`A. First` still breaks, matching the oracle), and
  `cat. Dog` (space after `.`) are all untouched.  Verified against the oracle;
  full suite green (**643 passed / 0 failed**).  `tests/acronym_periods.rs` (3
  tests).  *Deferred (noted for a future pass): the broader `.`-not-before-space
  dispositions — `cat.Dog`→"cat dot dog", de `3.14`→"drei **punkt** vierzehn"
  (period-between-digits in comma-decimal locales), `:` in times (`10:30`), and
  the `№` symbol name for German — each needs context-specific "dot"/"point"/
  "uhr" handling rather than a simple break-suppression.*
- **2026-08-01 — ✅ French `3`-liaison citation form before a pause (`six`→"sis",
  was "si").**  Follow-up to the number-liaison fix, closing the 2 residual
  oracle diffs it noted.  `six`/`dix`/`huit` carry `ph_french`'s `z3`/`t3`
  liaison, which has **three** realizations by following context — the port only
  had two: `z3` = [z] before a vowel ("six ans"→"siz"), silent before a consonant
  ("six chats"→"si"), and a *citation* [s] **before a pause** ("six"→"sis",
  `26`→"vɛ̃tsis").  The port dropped the pre-pause case → "si"/"di"/"yi".  **Fix**
  (from the exact `ph_french` `ChangePhoneme` rules): generalized the liaison
  resolver in `phonemes_to_ipa_full` to a `LiaisonCtx { resolve, next_starts_vowel,
  next_is_pause }`.  A liaison phoneme's mnemonic level (`2` vs `3`) now matters:
  at token end a `3`-liaison emits its citation consonant (z3→[s], t3→[t]) when a
  pause follows, while a `2`-liaison (and `z3` before a consonant-initial word)
  stays silent.  `next_is_pause` is derived in the caller by scanning for the
  first `ClauseBoundary`-or-`Word` after the token (also making `next_starts_vowel`
  correctly stop at a clause boundary).  Verified **23/23** fr cases now match the
  oracle segmentally: citation (`6`/`10`/`8`/`26`/`36`/"six."), liaison ("six
  ans"→"siz", "huit ans"→"yit"), silent ("six chats"→"si"), plus every prior
  fix (`200`→"døsɑ̃", `2000`→"dø mil", `21`, "deux enfants").  Full suite green
  (**641 passed / 0 failed**).  `tests/french_number_liaison.rs` (+2 tests → 5).
  *(Fixed en route: `then_some(b1 - b'0')` eagerly underflowed on non-liaison
  phonemes — switched to lazy `then(|| …)`.)*
- **2026-08-01 — ✅ French liaison inside composed number tokens (`200`→"døsɑ̃",
  was "døzsɑ̃").**  **Found via the C oracle** by diffing port vs. oracle IPA for
  compound numbers across fr/es/pt/nl/ca: French hundreds/thousands leaked an
  erroneous liaison consonant — `200`→"dø**z**sɑ̃", `300`→"tʁwa**z**sɑ̃",
  `600`→"si**z**sɑ̃", `2000`→"dø**z** mil".  Word-final liaison already worked
  across *separate* tokens ("deux enfants" keeps /z/, "deux chats" drops it) via
  a per-word `next_starts_vowel` flag, but a number is a *single* token whose
  parts are composed with no word boundary between them, so the liaison /z/ of
  "deux" was neither word-final nor dropped.  **Fix:** replaced the word-final
  liaison flag in `phonemes_to_ipa_full` with a proper resolver — for a French
  liaison phoneme (2-char consonant mnemonic ending in `2`/`3`), scan forward to
  the next real phoneme (skipping stress/pause markers *and* internal `END_WORD`
  separators): keep before a vowel, drop before a consonant; at true end-of-token
  the caller's `next_token_starts_vowel` decides.  This fixes both the no-boundary
  case (`200`=deux·cent) and the boundary case (`2000`=deux·mille).  Still
  French-only (the comment's Hungarian "öt"=ö+`t2` caveat stands).  Verified 24/26
  fr number/liaison cases now segmentally match the oracle (the 2 residual diffs
  — standalone `six`/`dix`→"si"/"di" vs oracle "sis"/"dis" — are the separate,
  pre-existing pre-pause /s/ realization, not caused or fixed here).  Full suite
  green (**641 passed / 0 failed**); updated two fr characterization tests that
  had baked in the old "døzsɑ̃".  `tests/french_number_liaison.rs` (3 tests).
- **2026-08-01 — ✅ Italian compound elision (`31`→"trentuno", was "trenta uno").**
  Italian tens elide their final vowel before a vowel-initial unit
  (uno=1, otto=8): trenta+uno→"trentuno", ottanta+otto→"ottantotto".  **Confirmed
  a real bug via the C oracle** (`espeak -v it 31`→"trˈeːntˈuno", `81`→"ottantˈuno")
  — the port only elided the 20s (via dedicated `_21`/`_28` dict words) and read
  `31`→"trenta uno", `48`→"quaranta otto".  Added `NumberGrammar.elide_tens_vowel`
  (set for `it`) + `Pronunciation::drop_final_phoneme`: in the `Standard` tens
  compose, when the unit is 1 or 8, the tens word's final vowel byte is dropped
  before appending the unit (no `phdata` needed — Italian tens all end in a
  vowel).  Now `31`→"trentuno", `41`/`81`/`91`→"…untuno", `48`/`88`/`98`→
  "…antotto", `131`→"cento trentuno"; consonant-initial units (`32`→"trentadue")
  and other Standard-tens languages (en "thirty one", es "treinta y uno", tr
  "otuz bir") unchanged.  Full suite green (**635 passed / 0 failed**, 61
  binaries).  `tests/italian_elision.rs` (3 tests).
- **2026-08-01 — ✅ Default speaking-rate calibrated to the oracle (SetSpeed/durations; user-chosen).**
  Investigated the #1 acoustic gap with the built C oracle.  **Found the exact
  cause:** C's `CalcLengths` vowel length is `table × len_speeds[pos]/128 …
  × stress/128`, but the Rust's `calc_vowel_length_mod` **omits the
  `× len_speeds[pos]/128` factor** (len_speeds[0]=82 at 175 wpm from
  `speed_lookup`+`SetSpeedFactors`, speedf1/2/3=256/238/232), so it runs long.
  **Established it's a *coupled* pipeline:** the Rust's `length_mod` /
  `length_mod_to_samples` use a different parameterisation than C's `p->length`,
  so C's formula can't be dropped in, and the overshoot is **systemic** (vowels
  *and* sonorants: "hello world" 1.28×, "many money lemons" 1.34×, "stop that
  cat" 1.05×) — a genuine multi-day faithful port.  **Measured** the geometric-
  mean duration ratio over a 10-word corpus vs the C reference: **1.196**.
  Applied the calibration `speed_factor ×= 0.836` (`RATE_CALIBRATION`, = 1/1.196)
  — a legitimate default-rate offset, not a per-phoneme fudge.  **Result:**
  "hello world" 1.28→1.13, "computer science" 1.13→0.99, "quick brown fox"
  1.14→1.02; corpus gm 1.20→~1.05 (fixed pauses dilute it below 1.0).  All
  **655 tests pass** (incl. the `wav-analysis` duration checks — the new
  durations stay within their tolerant ranges, closer to target).
  `synthesize/mod.rs`.  **Remaining (the byte-exact fix):** port `CalcLengths` +
  `SetSpeed` per-phoneme (`len_speeds` by syllable position, `length_mods`
  tables, `pd_DONTLENGTHEN` stops, per-syllable final-lengthening) — now
  frame-verifiable against the built oracle.
- **2026-08-01 — ✅ Inline embedded `\001` commands (3rd of the user's 5-feature list).**
  Raw embedded commands **garbled** the text — `\x01+10P hello` read "ten pee
  hello" (the value+letter tokenised as "10"+"P").  Ported C's `EmbeddedCommand`
  format (`CTRL_EMBEDDED`=0x01, optional `+`/`-`, digits, a letter from
  `"PSARHTIVYMUBF"`): `parse_embedded_commands(text) -> (stripped, Vec<EmbeddedCmd>)`,
  wired into `tokenize_opts` so **all** paths strip them (fast-path no-op without
  `\x01`; an invalid/stray sequence is consumed and dropped, matching C).  The
  engine then **applies** the pitch/speed/volume (`P`/`S`/`A`) levels to the
  utterance in `synth`/`synth_with_events` (`embedded_levels` +
  `make_voice_params_levels`).  **Verified:** `\x01 10P hello`→"hello" (no
  garble); `\x01 90S`→2.82 s (slow), plain→1.72 s, `\x01 400S`→1.08 s (fast).
  Full suite green (**635 passed / 0 failed**, 61 binaries).  `translate/mod.rs`,
  `engine.rs`, `tests/embedded_commands.rs` (3 tests).  **Limitation
  (documented):** commands apply at the **utterance** level (a leading command,
  the common case, is exact; mid-text positional changes aren't modelled), and
  `\x01V`/`\x01B` (voice/punctuation, which take word arguments) aren't parsed.
  **Remaining from the list:** `-X` trace, `--compile-mbrola`/`--compile-intonation`.
- **2026-08-01 — ✅ `-k 1` capital *sound* (2nd of the user's 5-feature list).**
  The port had `-k 2` (speak "capital") but not `-k 1`.  From the C
  (`translate.c:511`): `option_capitals==1` inserts `phonPAUSE_SHORT`(10) +
  `phonCAPITAL`(18) before a first-upper word, **doubled** for an all-caps
  multi-letter word (`FLAG_ALL_UPPER && IsAlpha(word[1])`).  Wired into
  `translate_to_codes`'s capital-indication path: `cap_codes` now maps `1`→
  `[PHON_PAUSE_SHORT, PHON_CAPITAL]`, `2`→the `_cap` word; an all-caps word
  repeats the sound.  **Verified vs the C oracle:** `Hello`→1 capital sound,
  `NASA`→2, lowercase→0, `iPhone`→(split "i Phone")→1; audio "Hello NASA"
  `-k0`=1.15 s, **`-k1`=1.38 s** (brief sound), `-k2`=2.21 s (spoken word).  Full
  suite green (**631 passed / 0 failed**, 60 binaries).  `translate/mod.rs`,
  `tests/capitals.rs` (+1).  **Remaining from the list:** embedded `\001`
  commands, `-X` trace, `--compile-mbrola`/`--compile-intonation` (the intonation
  source `phsource/intonation` is present in the built C tree → compilable).
- **2026-08-01 — ✅ SSML `<mark>` (first of the user's 5-feature list).**
  `<mark name="…"/>` was parsed but a **no-op**.  Now the SSML walker records each
  mark's name + character offset into the stripped text (`Mark` struct,
  `process_markup_with_marks`, `output_len()`), and `EspeakNg::synth_with_events`
  emits a new **`EventKind::Mark(name)`** for each (mirroring `espeak_EVENT_MARK`)
  — text position exact, audio time interpolated proportionally from the mark's
  text position (exact per-word timing isn't tracked through the markup path,
  documented).  `<mark>` carries no text and doesn't disturb the stripped output.
  Verified end-to-end (`hello <mark name="m1"/> world <mark name="done"/>` →
  Mark("m1")@587ms, Mark("done")@1272ms).  Full suite green (**629 passed / 0
  failed**, 60 binaries).  `translate/ssml.rs` (+1 unit test),
  `tests/ssml_integration.rs` (+1).  **Remaining from the user's list (sequenced
  for following iterations):** embedded `\001` commands, `-k 1` capital *sound*
  (vs the existing `-k 2` name), `-X` translation trace, `--compile-mbrola` /
  `--compile-intonation` (the latter compilable against the downloaded
  `phsource/intonation`).
- **2026-08-01 — ✅ Consonant WAV spectral character: loop the noise middle instead of resampling (§36, user-requested; built the C oracle).**
  Attacked the #1 fidelity gap.  **Built the C oracle** (`cargo build --features
  bundled-espeak,c-oracle` downloads + CMake-builds espeak-ng 1.52.0 →
  `…/install/bin/espeak-ng` + source), enabling sample-exact verification the
  GAPS notes said was essential.  Read the real C `PlayWave`/`DoSample2`/
  `DoSample3` (wavegen.c/synthesize.c): a noise sample is **never resampled** —
  when a fricative must be longer than its stored sample, C plays the first 3/4,
  **loops the steady-state middle half** (`[len4, 3·len4)`) to sustain, then
  tails from the end; shorter → truncate.  The Rust `parse_wav_sample` instead
  did **nearest-neighbour linear resampling**, which stretches/aliases the noise
  and lowers its high-frequency content (blurring `s`/`ʃ`/`f`).  Ported the exact
  chunk-and-loop.  **Verified vs the C oracle** ("she sells sea shells",
  `wav-analysis` spectra): flatness **0.309→0.215** (C 0.160), centroid
  **3570→3390 Hz** (C 3159), roll-off90 **7687→6912 Hz** (C 7063) — all moved
  toward the oracle; time-domain duration/energy unchanged (the length
  computation is a separate `SetSpeed`/`std_length` item, still ~1.28× long).
  Full suite green (**627 passed / 0 failed**, +2 sample tests: looping-not-
  resampling, short-target-truncates).  `src/synthesize/sample.rs`.
  **Remaining synthesis gaps (documented):** the C `DoSample2` *length*
  computation (`std_length`·`length_mod`·`wav_factor`, `pd_DONTLENGTHEN` for
  stops) needs a `SetSpeed` port (WPM→`wav_factor`/`min_sample_len`), not yet
  present — that's the fricative-too-long fix; plus `CalcPitches` intonation and
  per-syllable final-lengthening.
- **2026-08-01 — ✅ German "einundzwanzig" (`de 21`→"ein…", was "eins und zwanzig").**
  German compound tens 21–91 read the unit 1 as the standalone **"eins"**, but
  before "und" it apocopates to **"ein"** ("einundzwanzig", not
  "einsundzwanzig") — while standalone `1`="eins" and final-position `101`=
  "hunderteins" correctly keep the "s".  Added `NumberGrammar.combining_one`
  (`Some("ein")` for `de`): in the `UnitsThenConjunction` compound branch, unit 1
  is looked up as the combining word ("ein") instead of `_1` ("eins").  Now
  `de 21`→"einundzwanzig", `91`→"einundneunzig", `121`→"hunderteinundzwanzig",
  `21.`→"einundzwanzigste"; `101`/`1001`/`1000001` keep "eins".  Danish/Slovenian/
  Dutch (also units-first) already use their correct unit-1 form ("en"/"ena"/
  "een"), so `combining_one` stays `None` for them.  Updated the two German-21
  characterization tests.  Full suite green (**626 passed / 0 failed**, 60
  binaries).  `tests/german_ordinals.rs` (+1 test).
- **2026-08-01 — ✅ English "-ire" words dropped their offglide (`fire`→"faɪə", was "faɪ").**
  Probing emoji found 🔥→"faɪ"; the emoji correctly maps to "fire", so the bug
  was in the *word*.  Traced: `fire` = `f` + code 154 = mnemonic **"aI3"** (the
  rhotic diphthong); its phondata `i_IPA_NAME` isn't extractable (→None), so the
  fallback `mnemonic_to_ipa("aI3")` dropped the "3" variant marker → bare "aɪ".
  (Sibling "aI@" = code 153 correctly gives "aɪə"; working words like `hour`/
  `higher` use a two-code vowel+ə sequence, not the single rhotic vowel.)  Added
  `EN_IPA_OVERRIDES` entry **154 → "aɪə"** (British non-rhotic offglide, like
  "aI@").  Now `fire`/`tire`/`wire`/`hire`/`choir`/`desire`/`empire`/`liar`→
  "…aɪə", 🔥→"faɪə"; the existing word-medial linking-ɹ (`fiery`→"faɪəɹɹi") and
  non-rhotic words (`hour`→"aʊə", `here`→"hiə") are unchanged.  Full suite green
  (**624 passed / 0 failed**, 59 binaries).  `tests/rhotic_diphthong.rs` (2
  tests).  **Emoji coverage otherwise excellent** (flags 🇺🇸→"united states",
  ZWJ 👨‍👩‍👧→"family man woman girl", all read).  Confirmed via a 1–10 number
  scan that the liaison fix left no other truncated readings (only Galician —
  a missing-`gl_dict` data gap).
- **2026-08-01 — ✅ French liaison suppression was misapplied to all languages → Hungarian numbers lost final consonants (`hu 5`→"öt", was "ö").**
  A decimal-reading scan found Hungarian digits ending in a consonant dropped it
  word-finally: `hu 5`→"ö" (öt), `6`→"ha", `7`→"hé", `15`/`25` too — but the
  *word* `hu öt` and the non-final decimal integer (`5,5`→"öt egész…") kept the
  t.  Traced by dumping codes: `_5`="ö"+**"t2"** (code 121), a `2`-suffixed
  phoneme.  The word-final **liaison suppression** (drop a consonant whose
  mnemonic ends in 2/3 before a non-vowel — a *French* feature) was gated only on
  `!next_starts_vowel`, i.e. **applied to every language**, so Hungarian's real
  final "t2" was wrongly dropped.  Scoped `suppress_liaison` to
  `primary_bcp47_subtag(lang) == "fr"`.  Now `hu 5`→"öt", `15`→"tizenöt",
  `3,5`→"három egész öt"; **French liaison intact** (`les amis`→"lez ami",
  `les chats`→"le cha", `deux ans`→"døz ɑ̃", `deux chats`→"dø cha").  This also
  protects any other language using `2`/`3`-suffixed phonemes for real
  consonants.  Full suite green (**622 passed / 0 failed**, 58 binaries).
  `tests/liaison_scope.rs` (2 tests).  **Also seen (deferred — data/plumbing):**
  `uk 3,5`→"три пять" drops the decimal separator (uk_dict lacks `_dpt`);
  `ru 3,5`→"три и пять" reads "и"; proper Slavic decimals ("целых…десятих") are a
  bigger number-grammar item.
- **2026-08-01 — ✅ Danish/Slovenian/Faroese units-first cardinals (`da 21`→"enogtyve", was "tyve en").**
  Fixed the Danish cardinal-order bug noted last iteration: the arm
  `da|et|fi|fo|kl|lt|nb|no|sl` set only separators (no tens grammar → default
  **tens-first**), but Danish/Faroese/Slovenian are **units-first** ("enogtyve",
  "enaindvajset"), so `da 21`→"tyve en" (twenty-one, wrong).  Split them into
  their own arm with `TensGrammar::UnitsThenConjunction` — the conjunction is the
  dict's own `_0and` ("og"/"in"), so it's language-correct automatically — plus
  `compound_cardinal_suffix`.  Now `da 21`→"enogtyve", `33`→"treogtredive",
  `99`→"nioghalvfems", `da 21.`→"enogtyvende" (ordinal too); `sl 21`→
  "enaindvajset".  **Tens-first stays put:** fi "kaksikymmentäyksi", et "kaks
  kümmend üks", nb "tjue en" (modern Norwegian), lt — all unchanged (kept in the
  tens-first arm).  Faroese has no dict in this checkout (config harmless);
  Slovenian's ordinal *suffix* is a separate data gap (dict lacks `_#.`), but its
  cardinal order is now right.  Updated `lang_number_grammars_are_independent`
  (da now legitimately differs from fi).  Full suite green (**619 passed / 0
  failed**, 57 binaries).  `tests/units_first_cardinals.rs` (3 tests).
- **2026-08-01 — ✅ Dutch compound ordinals (`21e`→"eenentwintigste"), extending the German fix.**
  Verified the German compound-ordinal bug also hit the other units-first
  Germanic languages.  **Dutch** matched exactly: units-first cardinal
  ("eenentwintig") + the same two-word breakage (`21e`→"twintig eerste").  Set
  `compound_cardinal_suffix` for `nl` (gated off the shared `nl|mt` arm —
  Maltese ordinals differ), so `21e`→"eenentwintigste", `33e`→"drieëndertigste",
  `de 21e`→"de eenentwintigste"; round/small (1e/20e/100e) and Maltese
  unchanged.  Updated `lang_number_grammars_are_independent` to normalise the new
  Dutch-only flag (like `dutch_ij`) before the nl==mt compare.  **Danish** is
  *not* a clean match — its cardinal order is itself wrong (`da 21`→"tyve en"
  instead of units-first "enogtyve"), so its ordinal needs the cardinal fixed
  first; **Norwegian/Swedish** are English-style (tens + units-ordinal:
  "tjueførste"), a different composition.  Full suite green (**618 passed / 0
  failed**, 57 binaries).  `tests/german_ordinals.rs` (+1 Dutch test).
- **2026-08-01 — ✅ German compound ordinals (`21.`→"einundzwanzigste", was "zwanzig erste").**
  A cross-language ordinal scan showed German compound ordinals (21., 22., 33.,
  …) read the wrong **two-word** form — `ordinal_sub_thousand_pronunciation`
  composed tens-ordinal + units-ordinal (English "twenty-first" order), giving
  "zwanzig erste".  German instead suffixes the **whole cardinal**
  ("einundzwanzig" + suffix).  Added `OrdinalGrammar.compound_cardinal_suffix`
  (set for `de`): for a compound tensunits it now renders `num3(tensunits)` +
  the ordinal suffix (`_#.`, else `_ord`) — the same suffix the round-ten
  ordinals use, so 21. = "einund" + the exact `20.` ending.  Now `21.`→
  "einundzwanzigste", `33.`→"dreiunddreißigste", `99.`→"neunundneunzigste",
  `der 21. Mai`→"der einundzwanzigste Mai", `am 24. Dezember`→"am
  vierundzwanzigste Dezember".  Round/small German (1./3./20./100./1000.) and
  **English "twenty-first"** are unchanged (flag-gated).  Updated the
  characterization test that had locked in the old buggy `21.`→"zwanzig erste".
  Full suite green (**616 passed / 0 failed**, 56 binaries).  `tests/german_
  ordinals.rs` (2 tests).  **Broader ordinal gap noted (not taken — bigger
  feature):** Russian `1-й`→"один и-краткое" (reads the letter-name of й);
  Polish/Finnish/Czech/Hungarian/Turkish read the *cardinal* not the ordinal —
  each needs per-language ordinal words/morphology.  Dutch/Nordic likely want
  `compound_cardinal_suffix` too (unverified).
- **2026-08-01 — ✅ Degree sign `°` completed (sv/da/nb/ru/bg).**
  Finished the degree-sign coverage started with uk/he: `°` was still dropped in
  **sv, da, nb, ru, bg** (`sv 20°`→"tjugo", the ° silent).  Added the "degrees"
  word — sv/da/nb "grader", ru "градусов", bg "градуса" (uk "градусів" already).
  A fixed plural form is used; temperatures are almost always > 1°, so it's right
  in the common case and beats the silent drop.  Now `sv 20°`→"tjugo grader",
  `ru 20°`→"dvadtsat gradusov", and negatives compose with the minus fix
  (`sv -40°`→"minus förti grader", `ru -40°`→"minus sorok gradusov").  Dict-based
  languages (de/fr/es/it/nl/pl/cs/ro/tr/pt) unchanged.  Full suite green (**615
  passed / 0 failed**, 56 binaries).  `tests/symbol_reading.rs` (+1 test).  The
  four basic arithmetic operators (+ − × ÷) plus ± ° % now read across the
  common European-language set + Hebrew.
- **2026-08-01 — ✅ Plus-minus `±` broadly + Hebrew/Ukrainian operators (user: "add ukrainian and french and hebrew").**
  A symbol-drop scan showed `±` was silent outside en/de/fr/nl; added a non-
  English `±` arm (es "más menos", it "più meno", pt "mais menos", pl/cs/ro/sv/da
  "plus minus", nb "pluss minus", tr "artı eksi", ru/bg "плюс минус", uk "плюс
  мінус") — de/fr/nl left to their dicts so French keeps "plus ou moins".  Then,
  per the user: **French** was already complete (+ − × ÷ ± ° % all read).
  **Ukrainian** only lacked `°` → added "градусів" (`20°`→"dvadsyat hradusiv").
  **Hebrew** dropped `+ − × ÷ °` (only ±/% came from its dict); added them —
  but unpointed Hebrew reads garbled (`פלוס`→"flvs"), so used **niqqud-pointed**
  forms (פְּלוּס/מִנוּס/כָּפוּל/חֶלְקֵי/מַעֲלוֹת): `2+3`→"ʃtajim flus ʃaloʃ",
  `10÷2`→"eser χelkej ʃtajim", `20°`→"esrim maʔalot", `-5`→"minus χameʃ".  The
  `p→f`/`k→χ` approximations ("flus", "khaful") are espeak-Hebrew's inherent
  no-dagesh limitation (applies to all Hebrew), and every case is strictly better
  than dropping the symbol.  Full suite green (**613 passed / 0 failed**, 56
  binaries).  `tests/symbol_reading.rs` (+2 tests).
- **2026-08-01 — ✅ Multiply/divide signs `× ÷` read in 11 more languages.**
  A cross-language symbol scan showed `×` and `÷` were **dropped** (silent)
  outside en/de/fr/es/it/nl — `pt 2×3`→"dois três" (no "vezes"), `tr 10÷2`→"on
  iki" (no "bölü"), etc.  (`+`/`−`/`%` already had broad coverage.)  Since the
  symbol was silent, any standard reading is strictly better; added the operator
  words for **pt, pl, cs, ro, sv, da, nb, tr, ru, uk, bg** to
  `lookup_symbol_name` (× = vezes/razy/krát/ori/gånger/gange/ganger/çarpı/
  умножить на/помножити на/по; ÷ = dividido por/przez/děleno/împărțit la/delat
  med/divideret med/delt på/bölü/разделить на/поділити на/разделено на), each
  pronounced by that language's own rules.  Now `tr 2×3`→"iki çarpı üç",
  `ru 10÷2`→"десять разделить на два", `sv 2×3`→"två gånger tre".  The existing
  five languages are unchanged.  Full suite green (**612 passed / 0 failed**, 56
  binaries).  `tests/symbol_reading.rs` (+1 test).  **Also probed (not taken):**
  `°` still dropped in ru/uk/bg/sv/da/nb (degree words decline by number — riskier);
  pt "three"/"threes" → empty is an isolated pt dict quirk (all real loanwords
  read fine); the article "a"→"eɪ" remains §33 (oracle-deferred).
- **2026-08-01 — ✅ French clitic schwa stray dash (`le`→"lˈə", was "lˈə-").**
  Probing found every short French clitic ending in the reducible "e caduc"
  schwa carried a trailing dash — `le`→"lˈə-", `la`/`les`/`du`/`je`/`de`/`ce`/…
  (`que`→"kˈə" was the odd one out).  Traced it: the phoneme (fr code 14) is the
  schwa variant with mnemonic **`@-`** (type=vowel); its phondata `i_IPA_NAME`
  isn't extractable (`phoneme_ipa_string`→None, blocked by a conditional), so the
  fallback `mnemonic_to_ipa("@-")` mapped `@`→"ə" and `-`→"-" literally.  The `-`
  is a **variant marker** on the mnemonic, not IPA (like the already-skipped
  `/` and vowel `#`).  Added: skip `-` in a **vowel** mnemonic — now `le`→"lˈə",
  `du`→"dˈy", `je le vois`→"ʒə lə vwˈa".  Scoped to vowels, so English syllabic
  consonants (`l-`→`l̩`, via `EN_IPA_OVERRIDES`) and everything else are
  byte-unchanged (610 tests, all languages green).  `tests/french_schwa.rs`
  (2 tests).
- **2026-08-01 — ✅ Romance dotted ordinals (`es 3.º`→"tercero", was "tres").**
  The RAE-recommended Spanish ordinal form writes a dot before the gender
  indicator (`1.º`, `2.ª`, `3.º`; also pt/it/fr).  The tokenizer's ordinal-suffix
  lookahead broke at the `.`, so `3.º`→cardinal "tres" (and `pt 1.º`→"um ordinal
  masculino", the º read by name).  Added a dot-before-indicator skip: when the
  digits are followed by `.º`/`.ª`, the `.` is consumed with the indicator so it
  parses as the ordinal — now `es 3.º`→"tercero", `2.ª`→"segunda", `10.º`→
  "décimo"; pt/it/fr benefit too (`fr 1.º`→"premier").  **Gated off for
  `dot_marks_ordinal` languages** so German's own dot-ordinal is untouched
  (`de 3.`/`de 3.º`→"dritte", `5. Mai`→"fünfte Mai" — not empty).  A bare `3.`
  stays cardinal and decimals (`3.5`) are unaffected.  Full suite green (**607
  passed / 0 failed**, 54 binaries).  `tests/dotted_ordinals.rs` (3 tests).
- **2026-08-01 — ✅ Numero sign `№` read in Cyrillic (`№5`→"номер пять").**
  Probing multi-language symbols found `№` (U+2116, the numero sign — ubiquitous
  in Russian/Ukrainian/Bulgarian text, `дом №3`) was **dropped**: `ru №5`→"пять"
  (five, no "номер"), because `lookup_symbol_name`'s non-English branch had no
  `№` arm (English already reads "number").  Added `'№' => "номер"` for
  ru/uk/bg (mirroring the existing `%`→процент/відсоток arm), so `ru №5`→"номер
  пять", `дом №3`→"дом номер три", `uk`/`bg` likewise.  English unchanged
  ("number five"); languages that don't use `№` (de …) still return `None` and
  are byte-unchanged.  Full suite green (**606 passed / 0 failed**, 54 binaries).
  `tests/symbol_reading.rs` (+1 test).  **Also seen this iteration (not taken):**
  the article "a" reads as the letter name (`a dog`→"eɪ dˈɒɡ") — item §33,
  deliberately deferred pending the C oracle for the reduction conditions;
  German vulgar-fraction words (`½`→"eins schrägstrich zwei") and Spanish
  abbreviated ordinals (`3.º`→"tres") are per-language data follow-ups.
- **2026-08-01 — ✅ Title-abbreviation periods no longer pause (`Dr. Smith`, §5 #2340).**
  A period after a title abbreviation was tokenised as a clause boundary, so
  `Dr. Smith`→"doctor **[pause]** smith" (likewise `Mr.`/`Mrs.`/`Prof.`/`St.`/
  `Gen.`…).  Added `suppress_abbreviation_periods` (an English-gated post-pass
  beside `apply_roman_numerals`): a `ClauseBoundary('.')` whose previous token is
  a title abbreviation (`is_period_abbreviation`: dr/mr/mrs/ms/prof/rev/st/gen/
  col/sgt/capt/…) is downgraded to a plain word break — adjacency is implicit
  (a space before the dot would make the previous token a `Space`).  Now
  `Dr. Smith`→"dˈɒktə smˈɪθ", `Mr. and Mrs. Smith`→no pauses, `St. Louis`→"sənt
  lˈuːi".  **Real sentence ends untouched:** `the end. Next one.` still breaks,
  and ambiguous/trailing abbreviations (`etc.`, `Sr.`, `U.S.`, a number-dot
  `5.`) deliberately keep their break so no genuine full-stop is swallowed.
  English-only.  Full suite green (**604 passed / 0 failed**, 53 binaries).
  `tests/abbreviation_periods.rs` (2 tests).
- **2026-08-01 — ✅ Minus before a currency amount (`-$5`→"minus five dollars").**
  Probing found `-$5` read **"five dollars"** — the sign dropped.  Last
  iteration's minus fix only converted an ASCII `-` before a *digit*; `-$5` has
  the `-` before the `$`, so it fell through to the dropped-punctuation path
  (`+$5` already worked, since `+` is a spoken symbol).  Generalised the
  tokenizer branch via `minus_precedes_number` — a leading `-` (clause start or
  after a space) fires when followed by a digit **or** a currency symbol
  (`$ € £ ¥ ¢`) then a digit, allowing one optional space (`-$ 5`).  It emits the
  U+2212 minus, which the currency rewrite leaves untouched, so `-$5`→"minus five
  dollars", `-€10`→"minus ten euros", `-$0.99`→"minus ninety nine cents",
  `-$1234.56`→"minus … dollars … cents"; language-neutral (de "minus fünf
  dollar", fr "moins cinq dollar").  Gated so a hyphen after a word/number
  (`US-$5`, `5-$3`, `price-$5`) stays a separator, and `+$5`/`$5`/`5-10`/
  `COVID-19` are unchanged.  Full suite green (**603 passed / 0 failed**, 53
  binaries).  `tests/negative_numbers.rs` (+1 test, 5 total).  **Also checked
  (no change):** Roman numerals after regnal names (`Louis XIV`) are
  *deliberately* left unparsed (documented as ambiguous — "Apollo XI" is eleven,
  not the eleventh); `chapter/vol/war …` contexts already parse (`chapter
  XLII`→"forty two", `MMXXIV`→"two thousand twenty four").
- **2026-08-01 — ✅ Large numbers ≥ 10¹² (`1000000000000`→"one trillion", was "ten hundred billion").**
  Probing found the scale split (`split_scale_groups`) hard-capped at 4 groups,
  the top being `value / 1_000_000_000` (billion) with **no modulo** — so
  everything ≥ 10⁹ piled into the billion group, and at 10¹² that group became
  1000, rendered by `num3(1000)` as "ten hundred" → **"ten hundred billion"**.
  Generalised the split to emit three-digit groups for every scale up to 10¹⁸
  (the largest that fits in `u64`), keyed `_0M1`.._0M6`. Now **en** reads
  trillion / quadrillion / quintillion, **de** the correct long scale ("eine
  Billion" 10¹², "eine Trillion" 10¹⁸), **ru** триллион/квадриллион/квинтиллион,
  **it** likewise, and **es** `10¹²`→"un billón". **Regression guard:** a
  language that doesn't define a scale word (French has none past "milliard")
  would otherwise collapse 10¹² to a bare "un" — added a fold step that carries
  a word-less group down into the next scale (×1000), so fr 10¹² still reads
  "dix cent milliards" (conveys magnitude, matching the prior behaviour).
  Numbers < 10¹² are byte-unchanged; full suite green (**599 passed / 0
  failed**, 52 binaries). `tests/large_numbers.rs` (4 tests). **Remaining:**
  proper long-scale composition for es/fr at 10⁹/10¹⁵⁺ ("mil millones", "mille
  milliards") is a separate, deeper number-grammar item (both were already
  approximate before this change).
- **2026-08-01 — 🏷️ Patch release 0.1.3 + `+`/`−` reading extended to more languages (user: "bump patch version" / "run tests").**
  Bumped the package version `0.1.2`→**`0.1.3`** (Cargo.toml + Cargo.lock) and
  cut the accumulated unreleased work into a dated `[0.1.3]` `CHANGELOG.md`
  section (dropped the earlier 0.2.0-minor recommendation per the user's patch
  choice). **Feature (extends last iteration's minus fix):** the `+`/`−` symbol
  readings in `lookup_symbol_name` only covered de/fr/es/it/nl, so `-5`/`+5`
  **dropped the sign** in every other language (pl `-5`→"pięć"). Added
  high-confidence readings for **pl, pt, ru, uk, bg, cs, ro, sv, da, nb, tr** —
  each reads its own word (Turkish `-5`→"eksi beş", `+5`→"artı beş"; Russian
  "минус пять"; Polish "minus pięć"; nb "pluss"), pronounced by that language's
  own rules. Verified all 17 tested languages now speak the sign (differs from
  the bare number). Full suite green: **598 passed / 0 failed** across 52
  binaries, warning-free build. `tests/negative_numbers.rs` (+1 test, 4 total).
- **2026-08-01 — ✅ Leading minus sign before a number read as "minus" (`-5`→"minus five").**
  Probing common English inputs found a genuine drop: `-5` read **"five"** — the
  leading ASCII hyphen fell through to the dropped-punctuation path, silently
  losing the sign (while `minus 5` correctly read "mˈaɪnəs fˈaɪv", and the
  Unicode `−` U+2212 was already handled). Added a tokenizer branch: an ASCII
  `-` immediately before a digit, **at a clause start or just after a space**,
  is emitted as the U+2212 minus symbol so it reuses the existing per-language
  symbol reading (`lookup_symbol_name`). Now `-5`→"minus five", `-5.5`→"minus
  five point five", `was -5 degrees`/`= -3`/`temperature: -40` all carry the
  sign; **German `-5`→"minus fünf"** (language-neutral, via de's own symbol
  word). Gated so a hyphen **after** a word/number stays a separator: `5-10`,
  `COVID-19`, `2024-01-15`, `1-800-flowers`, spaced `3 - 5`, and the
  `well-known` letter-hyphen word-break are all unchanged (no false "minus").
  No regressions (595 tests, 51 binaries). `tests/negative_numbers.rs` (3
  tests). **Also verified-and-corrected two stale notes this iteration:** `© ®
  ™` are already read (copyright/registered trademark/trademark, done earlier)
  and **half-width katakana** (U+FF66–U+FF9F, incl. combining ﾞ/ﾟ) already reads
  identically to full-width via the engine-wide `.replace` — both subsumed by
  later broad fixes; notes below updated.
- **2026-08-01 — 📝 Release-prep: `CHANGELOG.md` + warning-free build (user: "prepare changelog and improve").**
  Prepared a first `CHANGELOG.md` (Keep-a-Changelog / SemVer format) covering
  everything accumulated since the last release `0.1.2`, organised under
  Added / Changed / Fixed with sub-themes (public API & engine, synthesis,
  text/numbers/markup, languages & scripts, dictionary tooling, CLI). The
  `[Unreleased]` section recommends a **0.2.0** minor bump — the cycle adds new
  public modules (`soundicon`, `voices`, `async_synth`, `mbrola`, `analysis`)
  and engine surface (`Builder`, `VoiceSpec`, `SynthEvent`/`EventKind`,
  `OutputMode`, `TextToPhonemesOptions`) plus ~40 language/script/number fixes
  and the dict-source compiler — all backward-compatible additions. Left
  `Cargo.toml` at `0.1.2` (version bump is the maintainer's release call; noted
  the recommendation in the changelog). **Improve:** cleared the one build
  warning — `is_emoji_char` had a redundant `0x1F1E6..=0x1F1FF` (regional-
  indicator flags) arm that is a strict subset of `0x1F000..=0x1FAFF`, so it was
  unreachable; folded the note into the containing range's comment. Build is now
  warning-free; full suite green (346 lib tests + all integration binaries,
  0 failed). No behaviour change. `CHANGELOG.md`, `src/translate/mod.rs`.
- **2026-08-01 — ✅ Voice `dictrules` → dict_condition: Brazilian Portuguese now reads Brazilian (§5 #2248 follow-up).**
  Completed the pt-br fix by wiring the `?N` conditional dict/rule mechanism
  (which the compiler already emitted, and the reader already checked) into the
  runtime.  A voice's `dictrules <n…>` directive sets bit `n` of the dict
  condition (base `pt` = `dictrules 1` → European `?1`; `pt-BR` = `dictrules 2` →
  Brazilian `?2`).  Parsed it into `VoiceEntry.dict_rules` +
  `voice_dict_condition`, stored the mask in `LangOptions.dict_condition` (set
  from the voice in `Translator::new`), and threaded it into every dict lookup
  **and** rules-translation call (previously hard-coded `0`).  Now
  `pt-br dente`→"deɪŋtʃy" (the Brazilian ti/di→tʃi/dʒi palatalisation),
  `tia`→"tʃia", `leite`→"leɪtʃy" — distinct from European `pt` "denty"/"tia"; and
  European `pt` itself is now more accurate (its `?1` rules fire: `olá`→"ɔlá"
  open-o).  This also activates every other language's `?N` variants (bs gender
  forms, etc.).  No regressions. `tests/regional_locales.rs` (+1 test).
- **2026-08-01 — ✅ Regional locales fall back to the base phoneme table (§5 #2248).**
  `-v pt-br` errored "phoneme table 'pt-br' not found": the `pt-BR` voice
  declares `language pt-br` + `language pt 6` but **no `phonemes` directive**, so
  `resolve_phoneme_table` returned "pt-br" (no such table) and selection failed.
  Same for `en-gb`, `fr-fr`, `fr-ca`, `es-mx`, `de-at`, …  Added
  `select_phoneme_table` (tag → `phonemes` directive → **primary BCP-47
  subtag**) and used it at both engine call sites, so a regional locale that
  ships only as a base table now reads like its base: `pt-br olá`→"olá",
  `fr-ca bonjour`→ fr, `en-gb hello`→ en.  Base/working locales
  (en/en-us/pt-pt/fr) unchanged.  (Brazilian-specific `dictrules 2` rule
  variants aren't applied yet — a refinement.)  No regressions.
  `tests/regional_locales.rs`.
- **2026-08-01 — ✅ Part-of-speech homograph selection (`to live`→"lɪv", `have read`→"rɛd").**
  English homographs were never disambiguated — `to live`→"laɪv", `to
  record`→noun "ˈɛkɔːd", `to use`→"juːs". The dict already encodes the
  `$verb`/`$past` alternates and the lookup already had the `expect_verb`/
  `expect_past` guards (verified: `word_to_phonemes_pos("live", verb)` returns
  the verb form) — but the translator always passed `expect=false`. Added a
  `PosExpect` threaded through a new `word_to_phonemes_pos`, and
  `build_translate_entries` now sets it from the **previous word**: after a
  verb-trigger ("to", modals) → `$verb`; after a perfect/passive auxiliary
  ("have"/"had"/"was"…) → `$past`; a **subject pronoun** ("I"/"we"/"they"/…)
  also triggers the verb ("I lˈɪv", "we jˈuːz"); and a **determiner**
  ("a"/"the"/"this"/…) triggers the `$noun` form for the ~15 words whose default
  is the *verb* ("a ˈkɒnvɜːt" not "kɒnvˈɜːt", "a ˈriːfʌnd", "the lˈɜːnid
  professor"). Detection is a conservative text-trigger list, not the dict
  `$verbf`/`$nounf` flags — those occupy the same low flags1 bits as the stress
  position in the shipped encoding, so they don't survive as readable flags.
  Now `to record`→"ɹɪkˈɔːd", `to close`→"kləʊz", `to present`→"prɪzˈɛnt",
  `have read`→"rɛd"; defaults unchanged (`a record`→noun, `live show`→adj). Reset
  at clause boundaries. No regressions. `tests/homographs.rs` (2 tests).
- **2026-08-01 — ✅ Hyphen between letters is a word break (`well-known`→"well known").**
  The tokenizer kept a hyphen inside the word, joining the parts
  (`well-known`→"wɛlnəʊn", `X-ray`→"ksreɪ"), but espeak's `TranslateClause`
  treats a hyphen between two letters as a **word break** ("hyphen between two
  letters: the clause tokenizer turns it into a word break, so … drop the
  hyphen"). Changed the tokenizer to consume+drop the hyphen and emit a
  `Space` (renamed the CamelCase `caps_split` flag to the general `word_break`).
  Now `well-known`→"well known", `X-ray`→"eks ray", `co-op`→"ko op",
  `mother-in-law`→"mother in law", `state-of-the-art`→"state of the art". Also
  made the **French ordinal generator split on `-` as well as spaces** so a
  generated compound (`80e`) matches the typed word (`quatre-vingtième`) — both
  now "katʁ vɛ̃tiɛm". (French *cardinals* keep their dict-composition liaison
  form when generated, splitting only on typed input — matching espeak's
  generated-vs-typed paths.) No regressions. `tests/hyphen_words.rs`.
- **2026-08-01 — ✅ Dictsource compiler: `?N` condition prefixes in `_list`.**
  `_list` entries may be guarded by a leading `?N` / `?!N` dict-condition (bs's
  gendered/case number forms, homographs) — the entry is used only when
  condition N is (un)set.  `parse_list_dsl` was mis-reading `?4 _0M2 m'ilijO:na`
  with the word `?4` (a bogus entry).  Now the leading `?N`/`?!N` tokens are
  parsed into flag bytes `100+N` (require) / `132+N` (forbid), which the reader
  already honours against `LookupCtx::dict_condition`.  Recompiling bs is
  unchanged for the default forms (`1000000`→milijun, `1000000000`→milijardu)
  and the conditional variants are now faithfully encoded (used when the context
  sets the condition) instead of dropped. No regressions. `compile.rs` +1 test.
  **Number-scan aside:** swept 0/10/100/1000/1000000 across ~45 languages —
  only `th 10` still empty (Thai `th_list` defines only digits 0–9, no
  tens/hundreds structure words — an upstream data gap).
- **2026-08-01 — ✅ Vietnamese "ten" was silent (§5 #2442).**
  `vi 10` produced **no output at all**: Vietnamese has no dedicated `_10`
  number word (it uses the `_1X` ten-prefix "mười"), but the teens path only
  composed ten+unit for `tensunits > 10`, so *exactly* 10 matched neither the
  dedicated-word branch nor the compose branch and emitted nothing. Added an
  `== 10` case that uses the `_10`-else-`_1X` ten word. Now `vi 10`→"mười",
  `110`→"một trăm mười", `210`→"hai trăm mười"; en/de/fr/tr/ja/ko 10 unchanged.
  (The separate Vietnamese `lăm` morphology — `15`→"mười năm" should be "mười
  lăm", selected via a dict `?`-condition on the units after a tens — remains, a
  data/condition item.) No regressions. `tests/vietnamese_ten.rs` (2 tests).
- **2026-08-01 — ✅ Dictsource compiler: voice phoneme-table resolution → Bosnian #2269 fix demoed end-to-end.**
  Tested the compiler on a second, non-trivial language (Bosnian, the #2269
  "milijon→milion" target) and found a high-impact bug: `--compile` selected the
  phoneme table by the **lang code** (`bs`) and, when that didn't exist, fell
  back to **en** — so every mnemonic resolved against the wrong table (`U:`→"ts",
  `1000000`→"mˈilɪjtsːn"; `dobar` lost its ɑ). But bs's voice declares `phonemes
  hr`. Fixed `run_compile` to resolve the table via `resolve_phoneme_table`
  (honouring the `phonemes <name>` directive: bs→hr, nb→no, …), falling back to
  the lang code then en. **Result: bs now compiles 9/10 sampled words identical
  to the shipped dict** (`dobar`→"dobɑr", `1000000`→"milijun"; only `grad`, a
  rule quirk, differs). **Then demonstrated the real fix end-to-end**: edited
  `bs_list`'s `_1M2`/`_0M2` million entries `milijun`→`milion`, recompiled, and
  `bs 1000000` → "mˈilɪˌon" (milion) — the #2269 upstream fix, applied through
  the port. So the "edit dictsource → --compile → fixed dict" loop now works on
  a real language. `tests/voice_phoneme_table.rs`. **Remaining:** `$`-flag bit
  encoding, `.L` emission, rule conditions (`?N`), the `grad`/`reĝo` context
  edge cases.
- **2026-08-01 — ✅ Dictsource compiler: `.replace` emission + `$`-flag list entries → 15/15 eo faithful.**
  Continued making the dictsource compiler reproduce real sources faithfully.
  Two more fixes, both found by diffing compiled-eo vs shipped-eo: (1)
  **`.replace` emission** — `parse_replace_dsl` reads the `.replace` section, and
  `compile_dict_full`/`compile_replace_section` emit the `RULE_REPLACEMENTS`
  table **correctly 4-byte-aligned** to the reader's `(rules_offset+relpos+4)&~3`
  calc, so eo's `cx`→ĉ etc. now compile (`cxapelo`→"tʃapelo"; reuses the
  runtime `.replace` application landed earlier). (2) **`$`-flags in `_list`** —
  `parse_list_dsl` parsed a flag token like `$u+` as a phoneme mnemonic (→ bogus
  "u"), so `ĝis  $u+ $pause` compiled to "u" instead of falling through to the
  rules; now `$`-tokens are treated as flags and a flags-only entry has empty
  phonemes (→ rules give "dʒis"). **Result: all 15 sampled eo words now match
  the shipped eo_dict exactly** (was 10/11); the compiler faithfully round-trips
  a real language, so language-data fixes can be applied by editing dictsource +
  `--compile`. 2 tests. **Remaining:** `$`-flag *bit* encoding (parsed but not
  yet applied), `.L` letter-group emission, rule conditions, and the `K)`
  letter-group edge case (`reĝo`).
- **2026-08-01 — ✅ Dictsource compiler works on real espeak sources (user: "implement dictsource").**
  The dict compiler existed but had never been run against real `<lang>_rules`
  — no dictsource ships in this checkout. Fetched a real one (Esperanto) and
  compiled it; found and fixed two blocking bugs. (1) **Reader crash**:
  `rules.rs is_letter/is_letter_wc` did `letter_bits[idx] >> group` which
  **panics** for `group ≥ 8` (u8 shift overflow) on a malformed/misread rule
  byte — guarded (only 8 built-in groups exist), so a bad dict degrades instead
  of crashing. (2) **Rule parser** (`parse_rules_dsl`): espeak rule lines are
  whitespace-separated *columns* `[pre)] match [(post]  <phonemes>`, but the
  parser only read the first two tokens — so it took the **match letter as the
  phonemes** (`A) r (A  R` used `r`→ɹ instead of the `R` column). Fixed to take
  the **last** token as phonemes and join the rest for the match+context.
  **Result: compiling real eo_rules now yields 10/11 sampled words byte-identical
  to the shipped eo_dict** (saluton/mondo/esperanto/lingvo/… ✓); the lone diff
  (`reĝo`) is a `K)r(A` letter-group-context edge case. So dropping real
  dictsource files in + `--compile` now produces working dicts — the path to
  applying upstream language-data PRs. `compile.rs` +1 end-to-end test.
  **Remaining:** `.replace`/`.L` directive emission (4-byte-aligned table),
  rule conditions/`$`-flags, the `K)`-style letter-group edge case.
- **2026-08-01 — ✅ Leading-zero numbers spoken as codes (`007`→"zero zero seven").**
  A number with a leading zero was read as a bare quantity ("007"→"seven",
  dropping the zeros). Ported C `TranslateNumber`'s rule into
  `cardinal_pronunciation`: a leading-zero number ≤3 digits speaks its leading
  zeros then the number (`007`→"zero zero seven", `05`→"zero five",
  `000`→"zero zero zero"); >3 digits is spoken digit-by-digit (`0800`→"zero
  eight zero zero"). A lone `0`, ordinary numbers, and decimals (`0.5`→"zero
  point five") are unaffected; works per-language (de "null null sieben", fr
  "zéro zéro sept"). No regressions. `tests/leading_zero_numbers.rs`.
- **2026-08-01 — ✅ Bishnupriya (bpy) now reads; systematic missing-offset scan.**
  Instead of guessing which languages were broken, wrote a scan over every
  shipped `_dict`: flag any with **`groups3` populated but `letter_bits_offset =
  0`** (non-Latin rules that can't be indexed). It found exactly **one**
  remaining: `bpy` (Bishnupriya Manipuri, 60 groups3 entries) — an Indo-Aryan
  language in the Bengali/Eastern-Nagari script. Added it to the **0x980
  (Bengali) offset** + the ISCII letter-groups arm (same as bn/as). Now
  `bpy নমস্কার`→"nomoshkar", `ভাষা`→"bhasha", `মানুষ`→"manush(ə)",
  `বাংলা`→"bangla"; bn/as unchanged. (Its word-final schwa isn't the `V`
  phoneme `delete_final_schwa` targets, so that refinement is a no-op — left
  off.) The scan confirms **no other missing-offset languages remain**; the
  rest of the empties are dict-not-found (dv/ckb/sah/mn/kaa/ps) or need real
  script subsystems (Thai/Khmer/Lao/Tibetan/Burmese) or a syllabary→roman table
  (Cherokee). One-line fix, no regressions. `tests/bishnupriya.rs`.
- **2026-08-01 — ✅ Urdu (ur) + Sindhi (sd) now read (were entirely empty).**
  Same missing-offset pattern, found in the broken-language sweep: both use the
  Arabic script and ship substantial rule sets (ur 62 groups, sd 73) but —
  unlike `ar`/`fa` — had `letter_bits_offset = 0` so groups3 never found their
  rules → empty. Added them to the **0x600 (Arabic) offset** arm. Now
  `ur اردو`→"urdu", `ur کتاب`→"kitab", `ur پاکستان`→"pakistan",
  `sd سنڌي`→"sindhi", `sd ڪتاب`→"kitab"; `ar`/`fa` unchanged. (Uyghur `ug`
  also uses Arabic script but resolves via groups2 and already worked without
  the offset, so it was left off.) One-line fix, no regressions.
  `tests/urdu_sindhi.rs` (2 tests).
- **2026-08-01 — ✅ Sinhala (si) now reads (previously entirely empty).**
  Continuing the broken-script sweep: `si` produced empty output despite
  si_rules having 300+ Sinhala-keyed groups — the same missing-offset pattern as
  Korean/Devanagari. Sinhala is an abugida in its own (non-ISCII) block, so it
  was explicitly excluded from the earlier ISCII offset work. Two pieces: (1)
  `letter_bits_offset = 0x0D80` so groups3 finds the si_rules groups; (2) the
  built-in **letter group B** on the dependent vowel signs (U+0DCF–U+0DDF, +
  vocalic-sign U+0DF2–3) and the virama/al-lakuna (U+0DCA) — exactly Devanagari's
  mechanism, since si_rules keys the inherent-vowel drop on group B (`ක (B → k`).
  The Sinhala virama was already in `is_indic_combining` (kept in-word). A first
  cut with only the offset read the vowel signs aloud as their *names*
  ("පිල්ල"/pilla, "හල් කිරීම"/hal-kirima); group B fixed it. Now
  `කම`→"kama", `කි`→"ki", `ක්`→"k" (virama drops the vowel), `ආයුබෝවන්`→
  "ayubowan" (hello), `ලංකා`→"lanka", `ස්තූතියි`→"stutiyi" (thanks). No
  regressions. `tests/sinhala.rs`.
- **2026-08-01 — ✅ Korean (ko) Hangul now reads (previously entirely empty).**
  Earlier entries listed Korean as blocked on "a real script-processing
  subsystem"; it turned out to need only two well-defined pieces. (1) **Hangul
  syllable decomposition** — each precomposed syllable (U+AC00–U+D7A3) breaks
  algorithmically into conjoining jamo: leading (U+1100+L), vowel (U+1161+V),
  optional trailing (U+11A7+T), mirroring C's `TranslateChar`
  (`decompose_hangul`, applied for every word like the C does). (2)
  **`letter_bits_offset = 0x1100`** so groups3 finds ko_rules' jamo groups
  (`.group ᄀ`, `.group ᄋ`, letter groups L01–L03). The ko rules already exist
  and carry the word-initial/final context (`_)ᄀ`→g, `L01)ᄀ`→k-). One tweak:
  the **leading ㅇ (choseong ieung, index 11) is a silent vowel-syllable
  placeholder** (the "ng" sound is only the *trailing* ᆼ), so it's omitted
  rather than emitting the jamo's letter-name — otherwise `안` came out
  "iɯŋan". Now `한`→"han", `안`→"an", `안녕하세요`→"annyeonghaseyo",
  `사랑`→"sarang", `감사합니다`→"gamsahamnida" (voiced word-initial ㄱ),
  `김치`→"gimchi". No regressions (non-Korean words have no Hangul, so the
  decomposition is a no-op). `tests/korean.rs`. **Remaining:** intervocalic
  ㄱ/ㄷ/ㅂ voicing across syllable boundaries (`한국`→"hanquk" keeps the plain
  phoneme) and the `q`-phoneme IPA rendering are refinements, not blockers.
- **2026-08-01 — ✅ Rules-engine `.replace` section now applied (gap #37, general).**
  The rules engine parsed each dict's `.replace` table (`replace_chars`) but
  **never applied** it — a general gap that the previous two iterations worked
  around per-language in code for Japanese. Fetched the C (`SubstituteChar` /
  `FindReplacementChars` in translate.c): the table is UTF-8 `from\0to\0…` pairs
  ending at 4 NUL bytes; matching is first-entry-wins at each input position,
  emitting the 1–2-char `to`. Implemented `decode_replace_table` +
  `Dictionary::apply_replacements`, wired into `word_to_phonemes_inner` before
  lookup. **Fixed a latent alignment bug** while at it: the stored
  `replace_chars` offset rounded *up* to a 4-byte boundary but C rounds *down*
  (`((intptr_t)p+4)&~3`), so decoding was one field off (harmless while
  unapplied; the ja probe showed `が`→`ぎ` until fixed → `が`→`か`+゙). Now
  applies to **59 languages** with a `.replace` table (ja 190, ne 359, fa 174,
  el 139, grc 159, …). **Subsumed the two ja hand-ports** — katakana fold + the
  voiced-kana decomposition now go through the real mechanism (helpers removed);
  ja output identical. **Newly fixes other languages too**, e.g. English
  typographic ligatures (`ﬁsh`→"fish", `encyclopædia`→"encyclopaedia", æ/œ/ﬂ/…)
  and each language's own char normalisation. **No regressions** across the
  whole suite (the `.replace` tables are legitimate normalisations the port
  should always have applied). `tests/replace_table.rs` (2 tests); the existing
  `tests/japanese_katakana.rs` now exercises the general path.

Sequential feature work driven by the `/loop` task (5-min cadence, best-effort,
regression-guarded by the test suite — no C oracle installed). Newest first.
Each fresh iteration should read this log + the section notes, pick the next
tractable item, implement + test, then append here. **Do not commit** (per user).

- **2026-08-01 — ✅ Japanese voiced/semi-voiced kana (が だ ば ぱ …) now read.**
  Probing the katakana emoji further exposed a **far bigger** pre-existing bug:
  *every* precomposed voiced/semi-voiced kana (が ざ だ ば ぱ rows — ~⅓ of the
  syllabary) produced **empty** output (`だ`→∅, `カナダ`→"kana"), so most real
  Japanese was broken. Root cause (confirmed from upstream `ja_rules`): the
  pronunciation rules match the **decomposed** form (base kana + combining
  dakuten U+3099 / handakuten U+309A), and a `.replace` section maps precomposed
  `だ`→`た`+゙. **The port parses the `.replace` section (`replace_chars`) but
  never applies it** (general gap — noted below). Reproduced the ja-relevant
  part in code: `normalize_japanese_kana` (already folding katakana→hiragana for
  #2464) now also decomposes precomposed voiced kana → base + combining mark,
  the form the ja rules match. Now `が`→"ga", `だ`→"da", `ぱ`→"pa",
  `カナダ`→"kanada", `だいがく`→"daigaku", `ございます`→"gozaimasu",
  `サムズアップ`→full. Unvoiced unaffected; scoped to `ja`. No regressions.
  `tests/japanese_katakana.rs` (+1 test). **General follow-up:** apply the
  rules-engine `.replace` section for all languages (would subsume this + the
  katakana fold + other languages' char-normalisation); half-width katakana
  (U+FF66–U+FF9F, incl. combining half-width dakuten) now reads via the
  engine-wide `.replace` — `ｶﾀｶﾅ`→"katakana", `ｶﾞ`→"ga", `ﾊﾟ`→"pa" (verified).
- **2026-08-01 — ✅ Japanese katakana support (fixes §5 #2464 root cause).**
  Investigating #2464 (ja emoji descriptions silent) found the real gap:
  **katakana was entirely unhandled** — `カタカナ`→∅ while the identical-sound
  `かたかな`→"katakana". Full-width katakana (U+30A1–U+30F6) is phonetically the
  same as hiragana (カ=か, offset 0x60), and the ja rules/dict are keyed on
  hiragana, so `word_to_phonemes_inner` now folds katakana→hiragana for `ja`
  (before the dict lookup, so it also covers emoji TEXTMODE re-translation).
  Fixes katakana loanwords (`アメリカ`→"amerika", `コンピューター`→"konpyuːtaː" —
  the ー prolonged-sound mark is handled by the ja rules post-fold, `ハート`→
  "haːto") **and** the katakana emoji descriptions #2464 is about (⭐"スター",
  👍"サムズアップ" now spoken). Scoped to `ja`; `en カタカナ` stays empty (non-ja
  unaffected). The `ー`/middle-dot/half-width katakana are intentionally not
  folded. **Remaining:** 🔥's description is the *kanji* 火 — kanji reading is a
  separate (much larger) gap, so that one emoji stays silent. No regressions.
  `tests/japanese_katakana.rs` (2 tests). Also corrects the §3 #17 note ("ja
  follow-up #2464 not checked").
- **2026-08-01 — ✅ Ordinal-suffix validation ("2st" → "secst" fixed, §5 #96).**
  `is_ordinal_suffix` accepted **any** of st/nd/rd/th after a number regardless
  of correctness, so `2st` parsed as an ordinal and rendered the mangled
  "secst". Now the English suffix must equal `english_ordinal_suffix(digits)`
  (the existing 1→st/2→nd/3→rd/else-th generator, with the 11/12/13→th case), so
  `2st`/`1nd`/`3th`/`11st` are a cardinal + a stray word ("2st" → "two" + "st"),
  while genuine `1st`/`2nd`/`3rd`/`4th`/`11th`/`21st`/`22nd` still parse. The
  Iberian `º`/`ª` (gender, number-independent), Dutch `e` indicator, German
  `.`-dot, and French letter-ordinals are untouched (verified `fr 2nd`→seconde,
  `nl 1e`→eerste, `de 3.`→dritte, `es 1º`→primero). No regressions.
  `non_ordinal_suffix_keeps_the_number` extended (+4 mismatch cases + 11th).
  **Also probed this iteration (no code needed):** Mandarin `cmn 9`→"jiǔ"
  (§5 #257, correct — not reproduced); Nepali digits read in Nepali (#2438,
  not reproduced); Pashto `ps` fails to load — **missing `ps_dict` data**, a
  blocker not a bug (#2437). English linking-r (§5 #367) deferred: needs
  cross-word phoneme context (coarticulation-class) and is contested upstream.
- **2026-08-01 — ✅ CamelCase upper→lower split — Branch C (§5 #27, #228 complete).**
  Completed the mixed-case splitting by porting espeak's *third* uppercase
  branch (the one the two prior entries flagged as missing): at the **last
  uppercase of a run** that's followed by a lowercase letter **and** another
  letter, insert a break so an acronym glued to a capitalised word separates.
  Re-fetched the exact C to confirm there's **no numeric length guard** (the "3
  or more letters" comment is informal — the real conditions are `prev_out !=
  ' '` + `IsAlpha(next2)`), so `caps_run_ends_here` splits on `prev` upper,
  `next` upper, `next+1` lower, `next+2` alpha. Results: `HTMLParser`→"HTML
  Parser", `JABberwocky`→"JA Bberwocky", `IThink`→"I Think", `XMLHttp`→"XML
  Http"; pure acronyms with no trailing lowercase run (`USA`, `HTML`) and
  `IDs` (no letter after the `s`) stay whole. Ported the **Dutch-IJ exception**
  (`NumberGrammar.dutch_ij`, `nl` only): `en IJssel`→"I Jssel" but `nl
  IJssel`→"IJssel" whole. Refactored the tokenizer's case tracking from
  `prev_lower` to `prev_char` to serve both branches. Updated the §2476
  independence test (nl now legitimately differs from mt by the `dutch_ij`
  flag — normalised out before the number-config compare). No regressions.
  `tests/camelcase.rs` (+2 tests: upper→lower split, Dutch IJ).
- **2026-08-01 — ✅ Lojban capital-stress notation (`LOPT_CAPS_IN_WORD`).**
  Follow-on to the CamelCase work: Lojban (`jbo`, which **ships** — `jbo_dict`,
  `lang/art/jbo`) writes the stressed syllable with capitals, so the CamelCase
  split from the previous entry was **actively wrong** for it (`bAstu`→"bə
  ˈastu", `spagEti`→"spˈaɡ ˈeti"). Confirmed from upstream `translate.c` that
  `LOPT_CAPS_IN_WORD` (set only for `jbo`) is **mutually exclusive** with the
  split (`if (LOPT_CAPS_IN_WORD) {stress marker} else {split}`): the first
  mid-word capital becomes a primary-stress mark `ˈ` and the word stays whole.
  Implemented via `NumberGrammar.caps_mark_stress` (tokenizer inserts `ˈ`
  before the first capital, incl. a leading one, and keeps the letter — lowered
  downstream). **Also fixed a latent stress bug it exposed:** an explicit `ˈ`
  in a rule-translated word did **not** suppress the default placement rule
  (`set_word_stress` discarded `get_vowel_stress`'s detected primary position),
  so `spAgeti` double-stressed "spˈaɡˈeti". Mirrored C's `GetVowelStress`
  (detected explicit primary → skip the default rule), but **scoped it to
  `caps_mark_stress` languages** via a new `StressOpts.caps_mark_stress`: the
  broad version rippled into *composed* numbers (Turkish "on"+"bir" lost its
  final stress, es/it ordinals changed) because the port's number composition
  bakes citation stresses into composed words — so the default rule must still
  re-stress those. Result: `spAgeti`→"spˈaɡetˌi", `spagetI`→"spˌaɡetˈi" (one
  primary, on the capital); es/it/tr byte-unchanged. No regressions.
  `tests/lojban.rs` (2 tests). **Follow-up:** the broad `set_word_stress` fix
  matches the oracle for es/it (`primero`/`siete`/`undicesimo` are truly
  single-stressed) — landing it generally needs the number-composition path to
  stop embedding citation stresses first.
- **2026-08-01 — ✅ CamelCase / mixed-case word splitting (§5 #27, issue #228).**
  Ported espeak's `TranslateClause` case-split: verified from upstream source
  that a **lowercase→uppercase boundary inserts a word break** (`c = ' '`) for
  **all** languages, with the sole exception of Irish (`ga`), whose
  `UpperCaseInWord` keeps lenition prefixes (`nGael`) whole. Implemented in the
  tokenizer word loop (`prev_lower` case tracking + a new `NumberGrammar.
  caps_word_split` flag, `true` except `ga`): on the boundary it stops the word
  **and emits a `Token::Space`**, so the split is byte-identical to a literal
  space across every surface (`--ipa`, `-x`, audio) — `CamelCase`→"Camel Case"
  (`kˈaməl kˈeɪs`), `iPhone`→"i Phone" (`ˈaɪ fˈəʊn`), `fooBar`==`foo bar`,
  `McDonald`==`Mc Donald`. All-caps runs (`HTML`, `USA`) and the upper-prefix
  `JABberwocky` shape stay whole; `I'm` (apostrophe) and word-start capitals
  (`Hello World`) are untouched; Irish `nGael` preserved. **Correction (later
  the same day):** espeak actually *does* split the `JABberwocky`/`HTMLParser`
  shape via a **third, upper→lower branch** (`iswupper(prev)&&iswlower(next)`,
  3+ letters), which this port does **not** yet implement — so those stay whole
  here for now (tracked in §5 #27). **Also not ported then, done next
  iteration:** the `LOPT_CAPS_IN_WORD` stress-marker variant (Lojban) — see the
  Lojban entry above. No regressions. `tests/camelcase.rs` (3 tests).
- **2026-08-01 — ✅ Word-final schwa deletion extended to Indo-Aryan Indic (§13).**
  Disproved the prior entry's parenthetical: a `-x` probe showed Bengali's
  inherent vowel *is* the `V` phoneme (`bn নমস্কার`→`n,VmVSk'arV`) and its
  `bn_rules` uses the same `ক→kV` / `ক(B)→k` mechanism as Hindi — so the
  Hindi schwa-deletion path applies unchanged. Extended `delete_final_schwa`'s
  gate from `hi/mr/ne` to add **sa/kok/bn/as/gu/pa** (Indo-Aryan, which delete
  the final schwa); left the **Dravidian ta/te/kn/ml and Oriya** out (they keep
  the inherent vowel). Also fixed the deletion guard: it required "another vowel
  that isn't `V`", so all-inherent-vowel words like `घर`/`कर`/`मन` never
  deleted (their only other vowel *is* `V`) — now any earlier vowel (incl.
  another inherent `V`) licenses deletion, leaving the first schwa and dropping
  the last: `घर`→"ghʌr", `कर`→"kʌr", `मन`→"mʌn", `bn ঘর`→"ghʌr",
  `bn নমস্কার`→"nʌmʌʃkar", `pa ਪੰਜਾਬ`→"pʌnɟab". Lone `क` still keeps its
  schwa; Dravidian unaffected. No regressions. `tests/devanagari.rs` +3 asserts.
- **2026-08-01 — ✅ All ISCII Indic scripts read (Bengali/Tamil/Telugu/… ) (§13).**
  Generalised the Devanagari fix: every ISCII-layout Indic script has the same
  *relative* positions (consonants 0x15–0x39, vowels 0x05–0x14, signs 0x3E–0x4C,
  virama 0x4D within its block), so **one** offset-relative letter-group table +
  the per-script block-start offset makes them all work. Added **bn/as (Bengali,
  0x980), pa (Gurmukhi, 0xA00), gu (Gujarati, 0xA80), or (Oriya, 0xB00), ta
  (Tamil, 0xB80), te (Telugu, 0xC00), kn (Kannada, 0xC80), ml (Malayalam, 0xD00),
  sa/kok (Devanagari)**. All now read with correct conjuncts: `bn নমস্কার`→
  "namoshkar", `ta வணக்கம்`→"vaɳakkam", `te నమస్తే`→"namaste", `kn ನಮಸ್ಕಾರ`→
  "namaskara", `ml നമസ്കാരം`→"namaskaram", `gu નમસ્તે`→"namaste", `pa ਸਤਿ ਸ੍ਰੀ`→
  "sat sri", `or ନମସ୍କାର`→"namaskar". At this point word-final schwa deletion
  stayed hi/mr/ne only — later extended to the Indo-Aryan set (see the
  2026-08-01 schwa entry above; the "their phoneme isn't `V`" hunch was wrong).
  Sinhala (si) was excluded here (non-ISCII layout) — **later done**, see the
  Sinhala progress entry. No regressions. `tests/devanagari.rs` (3 tests).
- **2026-08-01 — ✅ Hindi / Marathi / Nepali (Devanagari) now read (§13, "fix it").**
  Turned the earlier negative into a real fix by finding the **three** missing
  pieces (fetched espeak's `hi_rules` to confirm the mechanism): (1) `OFFSET_
  DEVANAGARI = 0x900` + Devanagari letter groups **including group B = vowel signs
  + virama ्** (the earlier attempt set A/C/VOWEL2 but never B — the `क(B)` rule
  that drops a consonant's inherent vowel could never match); (2) the **tokenizer
  was splitting words at the virama** (a non-alphabetic combining mark), breaking
  every conjunct — added `is_indic_combining` so virama/nukta stay in the word;
  (3) **word-final schwa deletion** (`delete_final_schwa`) — the inherent-vowel `V`
  after a consonant is dropped word-finally when the word has another vowel
  (espeak does the full positional version in the `V` phoneme program). Result:
  `नमस्ते`→"nʌmˈʌsteː" (namaste), `भारत`→"bʰˈaːrʌt", `किताब`→"kɪtˈaːb",
  `स्कूल`→"skˈuːl", `धन्यवाद`→"dhanyavaːd", sentence `मेरा नाम राहुल है`→"mera nam
  rahul hai"; `क` (lone consonant) keeps its schwa. Marathi & Nepali share it.
  No regressions (the tokenizer/schwa changes are gated to Indic/Devanagari).
  `tests/devanagari.rs`. **Remaining (refinement): full positional (medial) schwa
  deletion, and the same 3-part fix for the other Indic scripts (Bengali/Tamil/…),
  which use the same group-B + combining-mark + schwa pattern.**
- **2026-08-01 — 🔬 Devanagari abugida experiment (§13, definitive negative).**
  Tried offset 0x900 + Devanagari letter-bits (consonants, independent vowels,
  matras, virama-as-vowel). Result: rule chains fire and the "χ" garbage clears,
  but the output is **badly wrong** — every consonant keeps its inherent schwa and
  conjuncts don't collapse (`नमस्ते`→"namasa tae" not "nəməste"; `स्कूल`→"sə kʌuːlʌ"
  not "skul"). Confirms Devanagari needs the real Indic pipeline (**inherent-vowel
  suppression at the virama + Hindi schwa-deletion**), which the rules+letter-bits
  can't provide — worse than silence, so reverted. **Indic is a proper subsystem
  port** (the `LOPT`/`TranslateLetter` Indic branch), the single biggest remaining
  language-coverage feature. Kept the two clean wins below.
- **2026-08-01 — ✅ Hex-literal dimension guard; Indic subsystem confirmed absent
  (§14/§13).** Confirmed the port has **no Indic inherent-vowel/virama/LOPT
  processing at all** (grep: no `LOPT_`/virama/inherent handling) — so Indic
  abugidas, Korean Hangul (jamo), Thai/Burmese, and Chinese all need a real
  script-processing subsystem, not an offset. The script-restoration vein is
  closed. Clean fix: `apply_dimensions` now requires a **non-zero** number on each
  side, so a hex literal (`0x1F`) no longer reads "zero by one F" — it stays "zero
  ex one F"; real dimensions (`8x10`→"eight by ten") unchanged. Test added.
- **2026-08-01 — ✅ Per-mille `‰`/`‱`; Indic/Cherokee offset probe (§14/§13).**
  Continued the offset vein and mapped its boundary: **Indic (Devanagari hi/mr/ne)**
  gets rule chains with `OFFSET_DEVANAGARI=0x900` but the output is garbled — the
  virama/conjunct → inherent-vowel suppression isn't done (abugidas need the Indic
  vowel-handling pass, a bigger feature; reverted rather than ship garble).
  **Cherokee (chr)** stays empty even with 0x13A0 (dict has no letter-to-sound
  rules). **nn (Nynorsk)** isn't a shipped voice (no bug). Clean win landed: added
  `‰`→"per thousand" and `‱`→"per ten thousand" to the symbol readings (were
  dropped). `µ` (U+00B5) can't go through the symbol path — Unicode classifies it
  as a *letter*, so `µg` tokenises as a word; needs unit-prefix handling instead.
  1 test. **Scripts still needing complex processing (noted): Indic abugidas,
  Korean Hangul (jamo decomposition), Thai/Burmese/Chinese.**
- **2026-08-01 — ✅ Armenian + Georgian voices fixed (§13, follow-up).** Same
  missing-offset bug: `hy` (Armenian) and `ka` (Georgian) produced empty/garbage
  for words. Added `OFFSET_ARMENIAN = 0x530` (hy/hyw) and `OFFSET_GEORGIAN = 0x10A0`
  (ka) — found the Georgian value by trying candidates (0x1080 gave garbage
  "jh"; 0x10A0 is correct). The offset alone sufficed (it's what locates the
  multi-byte rule chains via `groups3`; no custom letter-bits needed for these).
  Now `hy "բարեւ"`→"bˈaɹev", `hy "Հայաստան"`→"haɪˈastan", `ka "გამარჯობა"`→
  "ɡˈamardʒˌoba", `ka "საქართველო"`→"sˈakʰartʰvˌelo". Updated the offset doc table.
  Test added. **Remaining broken non-Latin: Devanagari (hi) and other Indic** —
  those are abugidas needing per-script offset *and* special vowel-sign handling.
- **2026-08-01 — ✅ Cyrillic offset for mk/kk/ky/ba/… (§13, follow-up).** The Greek
  fix exposed the same class of bug in Cyrillic: `letter_bits_offset` (0x420) + the
  Cyrillic `build_letter_bits` were only set for **ru/bg/uk/be/tt**, so the other
  Cyrillic-primary voices produced empty words — `mk "здраво"`→empty, `kk`/`ky`/
  `ba` likewise. Added mk, mn, kk, ky, ba, cv, tg, os, cu to both lists; now
  `mk "здраво"`→"zdravo", `kk "сәлем"`→"sælem", `ky "салам"`→"salam",
  `ba "һаумы"`→"hawmy". **Serbian (`sr`) is Latin-primary in espeak** (Serbo-
  Croatian) — it reads Latin fine but not Cyrillic — so it's deliberately *not*
  in the Cyrillic list (its Latin reading verified intact). mn has no dict shipped
  (separate). Devanagari/Georgian/Armenian still empty — each needs its own
  offset + letter-bits (follow-ups). Test added.
- **2026-08-01 — ✅ Greek voice fixed — `OFFSET_GREEK` was missing (§13).**
  Discovered while adding script-switching: the **entire `el` (Greek) voice was
  non-functional for words** — `γεια`/`Αθήνα`/`Ελλάδα` all produced empty output
  (numbers and single letter-names worked). Root cause: `letter_bits_offset` was
  set for Cyrillic (0x420) and Arabic (0x600) but **not Greek** (should be
  `OFFSET_GREEK = 0x380`), so the letter-to-sound rules couldn't classify Greek
  letters for their vowel/consonant context conditions → no rule matched. Added
  the offset + a Greek `build_letter_bits` arm (vowels incl. accented, consonants,
  upper/lowercase). Now `γεια`→"jˈa", `καλημέρα`→"kalimˈera", `Αθήνα`→"aθˈina",
  `ευχαριστώ`→"evxarˈistˈo". Also re-enabled Greek→el in the script switcher, so
  `en "the word Αθήνα"`→"…(el)aθˈina(en)". Cyrillic/en unaffected. `tests/greek.rs`.
- **2026-08-01 — ✅ Automatic script switching: Cyrillic → ru (§2.15 / #2436, #2479).**
  From the GitHub review: a foreign-script word in a Latin clause was **dropped**
  (`en "hello мир"`→"hello"). `foreign_script_voice` detects a word's script
  (`char_script`) and, when the voice can't read it (`native_script`), emits the
  existing `phonSWITCH`/`LangSwitch` marker so it's re-translated: `en "hello мир"`
  →"hello (ru)mʲˈir(en)", `de "ist хорошо"`→"…(ru)xʌrʌʃˈo(de)". No switch when the
  voice reads the script (ru/uk read Cyrillic) or for Latin words. **Cyrillic→ru
  and Greek→el** (Greek enabled once the `el` voice itself was fixed — see the
  newer OFFSET_GREEK entry above); CJK/Arabic deferred (voice-choice ambiguity).
  Works on `--ipa`; the audio path emits the
  switch marker like existing phonSWITCH (produces a valid WAV; full cross-voice
  audio splicing is the known phonSWITCH-audio gap). 2 tests (`tests/script_switch.rs`).
  From the GitHub review: espeak reads emoji by CLDR name; the port **dropped them
  all**. Root cause found: the emoji-name data **is already in the shipped
  `en_dict`** (`😀`→"grinning face"), but (a) the tokenizer dropped emoji as
  `Punctuation` before lookup, and (b) the entries are **TEXTMODE replacements**
  (the value is replacement *text*, not phonemes) which `word_to_phonemes` never
  handled — so they'd have rendered as garbage. Fixed both: `is_emoji_char`/
  `is_emoji_modifier` tokenize an emoji **sequence** (base + variation selector +
  skin-tone + ZWJ-joined parts + regional-indicator flag pairs) as one word;
  `word_to_phonemes` now detects `TEXTMODE` and **re-translates the replacement
  text** as words (guarded against recursion by `FLAG_IN_TEXTMODE`). The
  presentation selector U+FE0F is dropped so `❤️`==`❤`. Now `😀`→"grinning face",
  `👍`→"thumbs up", `🎉`→"party popper", `💯`→"hundred points", `❤`→"red heart";
  in-sentence `hello 😀`→"hello grinning face". Reaches audio. Also fixes any other
  `$text`/TEXTMODE dict entries. 1 unit + `tests/emoji.rs`. (ZWJ family/profession
  sequences depend on the shipped dict having the multi-codepoint key.)
- **2026-08-01 — ✅ Stylized Latin normalization (§4 #2435).** From the GitHub
  review: espeak PR #2435 normalizes stylized Latin/digits to ASCII; the port
  **dropped them all** (`𝐇𝐞𝐥𝐥𝐨`→empty). Added `stylized_to_ascii` to the
  normalization pass: the Mathematical Alphanumeric Symbols block (bold/italic/
  script/fraktur/double-struck/sans-serif/monospace, U+1D400–1D7FF via offset),
  circled letters (Ⓐ/ⓐ), and the Letterlike holes (ℝ ℋ ℎ …). Every style of
  `Hello` now reads "həlˈəʊ"; `𝟏𝟐𝟑`→"123", `ℝℤℕ`→"R Z N". Plain ASCII is
  untouched (returned borrowed). 2 unit + 1 integration test.
- **2026-08-01 — ✅ `No.` before a number → "number" (§14).** `No. 5`→"no five";
  now `apply_number_abbrev` reads it "number five" (matching the `№` sign).
  Capital "No" + period + a following number required, so the word "no" (`No way`,
  `No. I refuse`, "say no") is untouched. 1 test.
- **2026-08-01 — ✅ Dimension `8x10` → "eight by ten" (§14).** A lowercase `x`
  between two numbers is a dimension separator, read "by": `8x10`→"eight by ten",
  `1920x1080`→"…by…", `2x4`→"two by four", `5 x 7`→"five by seven". English-gated,
  requires a **number on both sides** — so algebra (`2x=4`), multipliers
  (`3x faster`, `10x`), and a bare variable (`x=5`) keep the letter "x" (they have
  no number after the `x`). (Multiplication with ASCII `x` is rare; dimensions
  dominate.) es/pt ordinals verified fine (dict has them; the earlier "XX" was a
  stress-mark artifact). 1 test.
- **2026-08-01 — ✅ Metric units for de/fr/es/it (§14, follow-up).** Units were
  English-only, so `de 5km`→"fünf km" and `fr 5km`→garbage. Made `unit_words`
  language-aware for the regular metric set (km/cm/mm/kg/mg/ml) in de/fr/es/it —
  `de 5km`→"fünf kilometer" (invariable), `fr 10kg`→"dix kilogrammes",
  `es 5km`→"cinco kilómetros", `it 5km`→"cinque chilometri" (plural per language).
  Data/frequency units (byte↔octet differ) stay English-only. English unchanged;
  unsupported languages untouched. 1 test.
- **2026-08-01 — ✅ Non-English `× ÷ %` readings (§14).** Follow-up to the queued
  discoveries: `×`/`÷` were dropped for de/fr/es/it/nl (`de 5×3`→"fünf drei"), and
  `%` came out as garbage "f" for the Cyrillic voices ru/uk/bg (dict lacks the
  char-name). Extended the per-language symbol block: `×`→mal/fois/por/per/keer,
  `÷`→geteilt durch/divisé par/dividido por/diviso/gedeeld door, `%`→процент(ru/bg)/
  відсоток(uk). Now `de 5×3`→"fünf mal drei", `fr 6÷2`→"six divisé par deux",
  `ru 50%`→"pyatdesyat protsent". English (`5×3`→"five times three") and the
  dict-backed `%` (de "prozent") unchanged. Test extended.
- **2026-08-01 — ✅ Trademark/copyright symbols `™ ® ©` (§14).** These were
  dropped entirely. Added to `is_spoken_symbol` + the English symbol readings:
  `™`→"trademark", `®`→"registered trademark", `©`→"copyright". Works in context
  (`Acme™ shoes`→"acme trademark shoes", `Windows®`→"windows registered
  trademark", `© 2024`→"copyright two thousand twenty four") and reaches the audio
  path. (Discovered but not fixed: Russian `%`→garbage "f"; `×` dropped for
  de/fr; plural acronyms `CDs`→"kdz".) 1 test.
- **2026-08-01 — ✅ Common abbreviation expansion (§14).** Several abbreviations
  read as unpronounceable clusters: `vs`→"vz", `govt`→"govt", `Blvd`→"blvd",
  `Sgt`→"sgt", `Rd`→"rd". Added `apply_abbreviations` (English-gated) for an
  unambiguous, non-word set: vs→versus, govt→government, blvd→boulevard,
  sgt→sergeant, capt→captain, lt→lieutenant, rd→road, inc→incorporated,
  prof→professor, approx→approximately, misc→miscellaneous. Case-insensitive,
  period-tolerant (`vs.`, `SGT`). Ambiguous ones excluded (`St`=saint/street,
  `No`=number/no). Ordinals ending in "rd" (`3rd`→"third") are unaffected (they
  tokenise as ordinals, not `Word("rd")`); real words and non-English untouched.
  1 test.
- **2026-08-01 — ✅ Superscript powers `²`/`³` → squared/cubed (§14).**
  `normalize_number_symbols` turned superscript `²`/`³` into the digit ("5²"→"five
  two"). They're powers: now `²`→"squared", `³`→"cubed" — `5²`→"five squared",
  `x³`→"x cubed", `E=mc²`→"E equals mc squared", and they compose with units
  (`5 km²`→"five kilometres squared"). **Subscripts are untouched** (different
  codepoints): `H₂O`→"H two O", `CO₂`→"C O two". (Superscripts `⁴`–`⁹`/`¹`/`⁰` stay
  digits — "to the power of N" would need the exponent path; rare.) 1 unit + 1
  integration test.
- **2026-08-01 — ✅ Two-char temperature `20°C`/`72°F` (§14).** The single-char
  `20℃` read "twenty degrees Celsius", but the two-char `20°C` (degree + letter)
  read "twenty degrees **C**" (the letter). Added `apply_temperature`: a `°`
  followed by `C`/`F` expands the letter to "celsius"/"fahrenheit" (the `°` already
  reads "degrees"/"grad"/"degrés"). Now `20°C`==`20℃` and `72°F`==`72℉` exactly,
  spaced (`20 °C`) or not, and it works per-language (`de 20°C`→"zwanzig grad
  celsius", `fr`→"vingt degrés celsius"). A bare `90°` angle stays "degrees". 1 test.
- **2026-08-01 — ✅ Unit abbreviations `5km`→"five kilometres" (§14).** Units after
  a number read as letters/garbage: `5km`→"five kum", `10kg`→"ten kg", `20MB`→
  "twenty m". Added `apply_units` (English-gated): a number followed by a known
  **multi-letter** unit (km/cm/mm/kg/mg/ml/kb/mb/gb/tb/kw/hz/khz/mhz/ghz/mph)
  reads the full word, pluralised unless the amount is one — `5km`→"five
  kilometres", `1km`→"one kilometre", `20MB`→"twenty megabytes", `100Hz`→"one
  hundred hertz". British spelling (metre/litre) matching the default en-gb; the
  unit may abut the number (`5km`) or be spaced (`5 km`). Single-letter units
  (m/g/s/W) are excluded as ambiguous; non-unit words (`5 cats`) and bare units
  untouched. 1 test.
- **2026-08-01 — ✅ English decades `1980s`/`'90s` (§14).** A bare "s" after a
  round-ten number read the "s" as a letter: `1980s`→"…eighty ess", `'90s`→"ninety
  ess". Added `apply_decades` (English-gated): `1980s`→"nineteen eighties",
  `1900s`→"nineteen hundreds", `1850s`→"eighteen fifties", `80s`/`'90s`→
  "eighties"/"nineties", `20s`→"twenties" (all byte-equal to the written-out
  phrase; the century reads as its teen word). 2000s are ambiguous ("two
  thousands" vs "twenty twenties") and left alone; non-round numbers (`1985`,
  `5s`) untouched. Also handles the possessive-apostrophe spelling (`1990's`,
  `80's` → same decade); genuine possessives (`cat's`) and non-decades (`5's`,
  `1985's`) unaffected. 1 test.
- **2026-08-01 — ✅ Currency "one" apocope de/es/it (§14).** `de 1€`→"eins euro",
  `es`/`it 1€`→"uno euro" used the counting cardinal instead of the article form.
  Added `currency_one_word` — amount exactly 1 now emits the apocopated "one"
  before the currency noun: `de 1€`→"ein euro", `de 1$`→"ein dollar", `es 1€`/
  `it 1€`→"un euro" (`£` is feminine in es/it → "una"). fr/pt/en already correct
  (un/um/one), plurals and cents unaffected. Test added.
  **German ordinals 20+ investigated, left as a gap:** they should be cardinal +
  "-ste" (`21.`→"einundzwanzigste"), but (a) generating the text fails — the de
  L2P mangles the "einund…" X1 compounds ("ein ein"), and (b) the renderer can't
  cheaply source the "-ste" suffix (dict `_ord`="te"; 20+ needs "ste"). Round tens
  also wrongly use "-te" (`20.`→"zwanzigte"). Needs dict data or a de speller +
  suffix phonemes. (Also noted: Russian `-й` ordinals read the letter name.)
- **2026-08-01 — ✅ Italian irregular ordinals 1–10 (§12).** Probing ordinals
  across languages found Italian `1º`–`9º` wrong: the it dict lacks the irregular
  ordinal words, so the regular `-esimo` fallback produced non-words
  (`3º`→"treesimo" instead of "terzo", `1º`→"unesimo" instead of "primo"). Added
  `italian_ordinal_word` (masculine primo…decimo / feminine prima…decima, gender
  from the `º`/`ª` indicator) intercepted in `build_translate_entries` like the fr
  ordinals; 11+ keep the regular `-esimo` rule (which is correct: `undicesimo`,
  `ventesimo`). Now `1º`–`10º` match the typed words exactly (masc + fem). 1 test.
  (Discovered but not fixed: German compositional ordinals `21.`→"zwanzig erste";
  Russian `-й` suffix reads the letter name; Italian `21º`→"venti unesimo".)
- **2026-08-01 — ✅ Compositional teens (11–19) for 12 languages (§12).** Probing
  the broken teens found it wasn't just data: **Turkish `15`→empty** despite
  `10`→"on". `num3_phonemes` only looked up whole teen words (`_15`), which don't
  exist for languages whose teens are "ten"+unit ("on beş", "tizenöt"). Added a
  fallback: when the whole-teen lookup is empty, compose the teen prefix
  (`_1X`, else `_10`) + the unit. Fixes **az, vi, kk, uz, ba, nog, tr, tk, hy, hu,
  ky, tt** — `tr 15`→"on beş", `hu 11`→"tizenegy" (correct — `_1X` is "tizen"),
  `hy 11`→"tasnamek". Cascades to compounds: `tr 2015`→"iki bin on beş",
  `tr 115`→"yüz on beş". **No regression** — languages with dedicated teen words
  (en "fifteen", de "fünfzehn", fr "quinze", fi) skip the fallback. (cv still
  empty — no `_1X`/`_10` shipped; vi `15`→"mười năm" not "mười lăm", a phonological
  irregularity.) 1 integration test.
- **2026-08-01 — ✅ Comma decimals for 5 more (vi/az/ka/hy/kk) (§12, follow-up).**
  These dropped the fraction entirely (`vi 3,14`→"ba", `az`→"üç", `hy`→"yerek",
  `kk`→"üş") because their teen cardinals are *also* broken (bare `14`→empty).
  Adding comma-decimal config makes the fraction read digit-by-digit, which
  **works around** the broken teens: `vi 3,14`→"ba phẩy một bốn", `az`→"üç tam bir
  dörd", `ka`→"sami mteli erti otxi", `hy`→"yerek storaket mek ch'vors",
  `kk`→"üş … bir tört". 18 comma-decimal languages now configured total.
- **2026-08-01 — ✅ Comma decimals for 13 more languages (§12).** Probing decimals
  across languages found many comma-decimal locales unconfigured, so `pl 3,14`→
  "three [pause] fourteen", `sv`/`cs`/`ro`/`el`/`lv`/`is`/`ca`/`eu`/`sq`/`af`/`bs`/
  `id` all read the comma as a clause break and the fraction as a cardinal. Added
  them to a comma-decimal / period-thousands arm (decimal + group only — the
  ordinal-dot marker varies, so it's left off). Now `pl 3,14`→"trzy przecinek
  jeden cztery", `ro`→"trei virgulă unu patru", `ca`→"tres coma u quatre", etc.
  (integer + separator word + digit-by-digit fraction); space grouping is enabled
  too (bonus). English/de/nl and the already-configured set unchanged. 1 unit + 1
  integration test.
- **2026-08-01 — ✅ Currency + scale word `$5 million` (§14, follow-up).** The
  currency word was inserted between the amount and its scale: `$5 million`→"five
  dollars million", `$1.2 billion`→"one dollar twenty cents billion",
  `$100 thousand`→"one hundred dollars thousand". Now a scale word
  (thousand/million/billion/trillion/…) after a currency amount is emitted
  *before* the currency word (and the amount isn't cents-split): `$5 million`→
  "five million dollars", `$1.2 billion`→"one point two billion dollars",
  `$1 million`→"one million dollars" (plural). Plain currency (`$5`, `$5.99`,
  `$5 each`) unchanged. English-only (scale words are English). Test added.
- **2026-08-01 — ✅ Non-English currency symbols (§14).** Probing found the euro
  **dropped** for every non-English voice: `de 5€`→"fünf", `fr 5€`→"cinq",
  `es 5€`→"cinco", `pt 5€`→"cinco" (`apply_currency` was English-only). Made
  `currency_words(c, lang)` language-aware for the eurozone/continental set
  (de/fr/es/it/pt/nl × €/$/£, invariable nouns like German "Euro" handled) and
  ran `apply_currency` for those languages, reading `<amount> <word>` after the
  figure (cents-splitting stays English-only). Now `de 5€`→"fünf euro",
  `fr 5€`→"cinq euros", `it 5€`→"cinque euro", `de 5$`→"fünf dollar",
  `fr 5£`→"cinq livres" (each == the written-out amount). English unchanged
  (`$5.99`→"five dollars ninety nine cents"). Currencies with no word for the
  language stay as the bare symbol. 1 test (symbol_reading, 10 total). *(Minor
  known nit: German `1€`→"eins euro" not "ein euro" — cardinal-vs-article
  agreement, a broader number issue.)*
- **2026-08-01 — ✅ Keyword-gated Roman numerals (§14).** Probing found Roman
  numerals spelled as letters: `Chapter IV`→"chapter i-v", `World War II`→"world
  war i-i", `Section III`→"…i-i-i". Added `apply_roman_numerals` (English-gated):
  an **uppercase** valid Roman numeral immediately after an enumeration keyword
  (chapter/part/section/act/scene/volume/book/war/type/class/grade/level/mark/…)
  becomes a cardinal — `Chapter IV`→"four", `World War II`→"two", `Type II`→"two",
  `Chapter I`→"one". `roman_value` validates by round-tripping through `to_roman`,
  so malformed numerals (`IIII`, `VX`) and Roman-lettered words reject. Keyword
  gating + uppercase-only avoids false positives: `MIX tape`, `Vitamin D`,
  `plan C`, `CIVIC`, `DVD`, `a MI` all untouched. **Regnal ordinals deliberately
  left alone** (`Louis XIV`, `Elizabeth II`) — they're ambiguous (`Apollo XI` is
  *eleven*, not *the eleventh*). Non-English unaffected. 1 unit + 2 integration
  (`tests/roman_numerals.rs`).
- **2026-08-01 — ✅ Voice `dictionary`/`phonemes` directives honoured (§11).**
  Probing cardinals across languages exposed `-v nb` (Norwegian Bokmål) failing
  with **"dict not found"** — there's a `no_dict` but no `nb_dict`, and the nb
  voice file declares `dictionary no` / `phonemes no`, which the port ignored
  (`resolve_dict_stem` only tried `<tag>_dict` prefixes; the phoneme table used
  the raw code). Parsed the two directives into `VoiceEntry`
  (`voice_data_overrides`), added them as a **fallback** to `resolve_dict_stem`
  and a new `resolve_phoneme_table`, and made the phoneme-table selection retry
  via the directive when the code-named table is absent. Now `nb` reads correctly
  and byte-matches `no` (`hei`→"hˈaɪ", `1000`→"et tusen"); Western Armenian `hyw`
  (declares `dictionary hy`) also resolves (`hyw 2000`→"jeɹkˈu hazˈaɹ") — both were
  "dict not found" before. **Common languages are untouched** (the fallback only
  runs when `<tag>_dict`/the code-named table is missing, so no voice-file walk on
  the hot path). fo/nn still fail — genuinely no data/directive shipped. 2 tests
  (`tests/voice_alias.rs`).
- **2026-08-01 — ✅ Portuguese decimals + thousands separator (§12, follow-up).**
  `pt "1,5"` read "um" [pause] "cinco" (comma treated as a clause break) and
  `3,14`→"três, catorze" (14 as a cardinal). Set the pt grammar to comma-decimal /
  period-thousands (like the rest of the Romance set) and extended the generator
  (`portuguese_number_word`) to the `Decimal` token: integer word + "vírgula" +
  each fraction digit read individually. Now `1,5`→"um vírgula cinco",
  `3,14`→"três vírgula um quatro", `2,50`→"dois vírgula cinco zero" (all byte-equal
  to the typed words). Bonus: the period thousands separator now composes
  (`1.234.567`→full reading). Non-pt decimals unaffected. Test extended.
- **2026-08-01 — ✅ Portuguese cardinals generated as text (§12).** Probing found
  pt cardinals broadly **broken**: `1`→empty, `5`→empty, `100`→"cento" (should be
  "cem"), `200`→"dois", `1000`→empty, `2000`→"dois", `5000000`→"milhões" (drops
  the multiplier) — the shipped pt dict lacks number keys (`_1`/`_5`/`_0M1`/…),
  though every number *word* translates fine. Added a European-Portuguese speller
  (`pt_group` 0–999 + `portuguese_cardinal_word` 0–999,999,999) and a
  `NumberGrammar.portuguese_cardinals` flag; `build_translate_entries` generates
  the word and pronounces it via the rules (same mechanism as the fr ordinals).
  Handles the Portuguese "e" rule (joins the final group only when it is <100 or a
  round hundred: `1500`→"mil **e** quinhentos" but `1120`→"mil cento e vinte"),
  cem/cento, and milhão/milhões. **All of 0–999,999,999 verified byte-identical to
  the typed word** (`2024`→"dois mil e vinte e quatro", `1234567`→"um milhão
  duzentos e trinta e quatro mil quinhentos e sessenta e sete"). Non-pt languages
  unaffected. 2 unit + 1 integration (`tests/portuguese_numbers.rs`). Remaining:
  ≥10⁹ (mil milhões / bilião — region-dependent); pt-BR forms
  (dezesseis vs dezasseis). *(pt decimals + thousands separator done — see the
  newer follow-up entry above.)*
- **2026-08-01 — ✅ Round thousands ≥2000 no longer read as "N hundred" (§12).**
  The `is_year_form` heuristic in `cardinal_pronunciation` applied the "N hundred"
  year reading to *every* round hundred in 1100–9900 (`value/100 >= 11`), so
  `2000`→"vingt cent" (20×100), `5000`→"cinquante cent", en `2000`→"twenty
  hundred". Narrowed the range to **1100–1900** (`(1100..=1999) && %100==0`): 2000+
  now use the standard thousands form — `fr 2000`→"deux mille", `de 2000`→
  "zweitausend", `es 2000`→"dos mil", `en 2000`→"two thousand", `5000`→"five
  thousand". The oracle-verified year form is preserved (`en 1900`→"nineteen
  hundred", `fr 1900`→"dix-neuf cents"). Also fixes the fr ordinal ≥1001 fallback
  (`2000e`→"deux mille"). Applies to all languages (the bug was language-agnostic).
  Test added (6 in french_numbers). *(Minor pre-existing nit, unrelated: fr
  "deux"/"trois" gain a spurious liaison /z/ before mille/cent.)*
- **2026-08-01 — ✅ French letter-suffix ordinals `1er`/`2e`/`21e` (§12).**
  Probing found French ordinals badly broken: `1er`→"un e", `2e`→"deux e" (the
  suffix leaked as a spoken letter). The fr dict ships **no `_No` ordinal words**,
  so the number path can't produce "premier"/"deuxième" — but the ordinal *words*
  translate cleanly through the French rules (`deuxième`→døziˈɛm). Implemented a
  generator: `OrdinalGrammar.french` recognises the written markers (er/ers, re/
  ère, e/es/ème/eme, nd/nde…, gender- & second-aware); `fr_cardinal_text` (0–999
  vigesimal speller: "soixante et onze", "quatre-vingts") + `ordinalize_fr_word`
  ("ième" with cinq→cinquième/neuf→neuvième/silent-e drop) build the ordinal word,
  which `build_translate_entries` translates (one entry, word-boundary-joined —
  entries stay 1:1 with tokens). **All of 1..=1000 verified byte-identical to the
  typed word** (`21e`==="vingt et unième", `71e`==="soixante et onzième",
  `80e`==="quatre-vingtième", `100e`==="centième", `1000e`==="millième");
  in-sentence `le 1er janvier`==="le premier janvier". ≥1001 falls back to the
  cardinal (rare). Minor known nit: 200–999 add a spurious liaison /z/ ("deuz
  centième"). Non-French ordinals (nl `2e`, de `3.`, en `3rd`/`21st`) unchanged.
  3 unit + 1 integration test. **Discovered (separate, unfixed): the fr *thousands*
  cardinal is wrong — `2000`→"vingt-cent", `5000`→"cinquante-cent" (should be
  "deux mille"/"cinq mille"); worth a follow-up.**
- **2026-08-01 — ✅ Grouped decimals `1 234,56` (§12, follow-up).** Extended
  `apply_space_grouping` so a space run may end in a decimal group: if the final
  `⎵<group>` is a `Decimal` with a 3-digit integer part, its integer merges into
  the whole number and the fraction carries over — `fr "1 234,56"` now byte-equals
  `fr "1234,56"` ("mille deux cent trente-quatre virgule cinq six"), and
  `1 234 567,89` == `1234567,89` (cardinal groups + trailing decimal). Non-3-digit
  integer parts (`12 34,5`) still don't merge. Test extended (equality assertions).
- **2026-08-01 — ✅ Space-grouped thousands for continental Europe (§12).**
  Probing revealed `fr "1 000"`→"un zéro" and `fr "1 234 567"` read as *three*
  separate numbers (space-grouped thousands weren't composed; `fi "1 000"`→"yksi
  nolla"). Added `NumberGrammar.space_group` (auto-set for the comma-decimal
  languages, where a space separator is unambiguous) + `apply_space_grouping`
  token-rewrite: a 1–3 digit lead group followed by one or more `⎵<exactly-3
  digits>` groups merges into one number. Now `fr "1 234 567"`→"un million deux
  cent trente-quatre mille cinq cent soixante-sept", `fi "1 000"`→"tuhat",
  `de "1 234 567"` and `es "12 500"` compose fully. English keeps a space as an
  ordinary separator (byte-unchanged); guards prevent false merges (`12 34`,
  `5 pommes` stay split). `space_grouped_thousands_compose` test (4 in
  french_numbers). Runs before `apply_currency` so grouped amounts compose.
- **2026-08-01 — ✅ Per-language `+ − °` readings for de/fr/es/it/nl (§14).**
  Probing non-English symbol reading: `%` reads fine everywhere (the dicts have
  `_%` → "Prozent"/"pourcent"/"porciento"), but `+` and `°` fell to the char-name
  spell-out, which produced **garbage letters** for German (`5+3` → "fünf ˈɪːɜˌaʊɐ
  drei", `5°` → "fünf r.ˈʊʑç") — worse than dropping them. Added confident
  readings for the three arithmetic/degree symbols in the five languages:
  `de 20°`→"zwanzig grad", `fr 20°`→"vingt degrés", `es 20°`→"veinte grados",
  `it 20°`→"venti gradi", `nl 20°`→"twintig graden", `de 5+3`→"fünf plus drei",
  `de 5−2`→"fünf minus zwei". English readings unchanged; other symbols still
  fall to the dict. (Same capitalisation quirk as ℃: the symbol path calls
  `word_to_phonemes` directly and drops a capitalised "Grad", so the reading is
  lowercase "grad".) `symbol_reading` test added (9 tests).
- **2026-08-01 — ✅ Temperature symbols `℃`/`℉` (§14).** Probing letterlike symbols:
  `℃` (U+2103) and `℉` (U+2109) were dropped (`20℃`→"twenty"). Added them to the symbol
  readings: `20℃`→"twenty degrees celsius", `72℉`→"seventy two degrees fahrenheit".
  (Used lowercase "celsius"/"fahrenheit" — a capitalised word wasn't resolving through
  the symbol-reading path, though it reads fine as a normal lowercased token.)
  English-gated. `symbol_reading` test extended (337 lib).
- **2026-08-01 — ✅ Circled numbers & Roman-numeral codepoints (§12).** Probing enclosed
  forms: circled `①⑩⑳`, parenthesised `⑴`, and Roman-numeral codepoints `ⅣⅫⅿ` were all
  dropped. Added `enclosed_number_value` (circled ①–⑳/⓪, parenthesised ⑴–⒇, Roman
  Ⅰ–Ⅻ + Ⅼ/Ⅽ/Ⅾ/Ⅿ upper+lower) into `normalize_number_symbols`, mapping each to its
  spaced numeric value. Now `①`→"one", `⑩`→"ten", `Ⅳ`→"four", `Ⅿ`→"one thousand",
  `chapter Ⅳ`→"chapter four", `①item`→"one item". 1 unit test (337 lib).
- **2026-08-01 — ✅ Full-width (CJK-compatibility) characters (§12/§14).** Probing CJK-mixed
  text: full-width forms `５ＡＢＣＨｅｌｌｏ＄％` (U+FF01–FF5E) and the ideographic space were
  **all dropped** (they're neither ASCII nor letters to the tokenizer). Added
  `fullwidth_to_ascii` (subtract 0xFEE0; U+3000→space) into `normalize_number_symbols`.
  Now `５`→"five", `１２３`→"one hundred and twenty three", `Ｈｅｌｌｏ`→"hello", `＄５`→"five
  dollars" (composes through currency!), `５％`→"five percent", full-width space splits
  words. A whole class of CJK-mixed text (very common in Japanese/Chinese) now reads.
  1 unit test (337 lib).
- **2026-08-01 — ✅ Arabic native separators & percent (§12).** Follow-up to native
  digits: Arabic/Persian decimal `٫` (U+066B), thousands `٬` (U+066C) and percent `٪`
  (U+066A) were dropped (`١٫٥`→"one five", `١٬٠٠٠`→"one zero", `٥٪`→"five"). Normalised
  them to `.`/`,`/`%` in `normalize_number_symbols`, and gave `ar`/`fa`/`ur` a comma
  group separator so `٬` actually groups. Now `١٫٥`→"waːħid faːsˤil xamsa" (one point
  five), `١٬٠٠٠`→"ʔalf" (thousand), `٥٪`→"xamsa bilmiʔah" (five percent). 1 unit test
  (337 lib). RTL/Arabic-script number text is now well covered.
- **2026-08-01 — ✅ Native-script digits read (§12).** Probing non-Latin scripts:
  Arabic-Indic `٥`, Devanagari `५`, Persian `۵`, Bengali `৫`, Thai `๕` etc. were
  **dropped entirely** (the tokenizer only recognised `is_ascii_digit`), while ASCII
  digits read fine (`ar 5`→"khamsa"). Added `native_digit_to_ascii` (14 scripts:
  Arabic-Indic, Extended Arabic-Indic, Devanagari, Bengali, Gurmukhi, Gujarati, Oriya,
  Tamil, Telugu, Kannada, Malayalam, Thai, Lao, Tibetan) into `normalize_number_symbols`,
  mapping consecutive native digits to adjacent ASCII so they form one number. Now
  `ar ٥`→"khamsa", `ar ١٢٣`→"one hundred twenty three" (Arabic), `hi ५`→"panch",
  `th ๕`→"ha". Language-neutral (the resulting number reads in the target language).
  1 unit test (336 lib). A high-impact real-world gap for RTL/Indic/SE-Asian text.
- **2026-08-01 — ✅ Numeric exponent `2^10` (§14).** The caret between two numbers was
  dropped (`2^10`→"two ten"). Added `apply_exponents`: `<number>^<number>` → "<base> to
  the power of <exp>" (`2^10`→"two to the power of ten", `10^6`→"ten to the power of
  six"). Only numeric base+exponent (unambiguous); a variable base `x^2` is left alone.
  English-gated. 1 unit test (336 lib).
- **2026-08-01 — ✅ Fraction reading `1/2`→"one half" (§12).** The vulgar fractions I
  normalize to `n/m` (and typed `1/2`) were reading "one slash two". Added `apply_fractions`,
  a token-rewrite (after `apply_currency`): `<num>/<den>` for a small denominator (2–10)
  → numerator + the ordinal fraction word (`half`/`quarters`/`thirds`/…), pluralised
  unless the numerator is one. Now `1/2`→"one half", `3/4`→"three quarters", `½`→"one
  half", `22/7`→"twenty two sevenths". Guarded: a date-like `3/4/2024` (trailing `/`) and
  a large denominator `1/12` stay "slash". English-gated (German `1/2` → "schrägstrich").
  1 unit test. **Mixed numbers added (2026-08-01):** a whole number before a fraction
  (`1½` = `1 1/2`, `5 3/4`) reads "<whole> and <num> <fraction>" — `1½`→"one and one
  half", `2¾`→"two and three quarters". (Used "one and *one* half" not "…and *a* half":
  a bare "a" reads as the letter name, not the article schwa.) 335 lib.
- **2026-08-01 — ✅ Math relation/operator symbols (§14).** Probing scientific text:
  Greek letters already read (α→alpha, π→pi, Ω→omega, μ→mu), but `∞ √ ∑ ≈ ≠ ≤ ≥` were
  dropped. Added them to the symbol readings: `∞`→"infinity", `√4`→"square root four",
  `∑`→"sum", `≈`→"approximately equal to", `≠`→"not equal to", `≤`→"less than or equal
  to", `≥`→"greater than or equal to" (multi-word via `PHON_END_WORD`). English-gated.
  Extended `symbol_reading` test (334 lib). (© ® ™ are now read
  copyright/registered trademark/trademark; `′ ″ | ← ↑ →` stay silent —
  genuinely ambiguous, matching espeak.)
- **2026-08-01 — ✅ Currency after the number + `¢` (§12/§14).** My earlier currency
  fix only handled symbol-*before* (`$5`); continental `10€`/`5$`/`99¢` were dropped or
  mishandled. Refactored `apply_currency` to handle **both orders** (symbol before *or*
  after the number, optional space) via a shared `emit_currency_amount` helper, and
  added the cent symbol `¢`. Now `10€`→"ten euros", `5$`→"five dollars", `99¢`→"ninety
  nine cents", `5.99€`→"five euros ninety nine cents" (decimal split works after-symbol
  too); `$5`/`$5.99` (before) still correct; bare `$` still "dollar". English-gated. Test
  extended (334 lib). Non-English currency words remain future polish.
- **2026-08-01 — ✅ Croatian/Serbian/Macedonian "one hundred/thousand" prefix (§12).**
  Broad 100/1000 sweep across ~15 more languages: Croatian/Serbian `100`→"jedansto",
  Macedonian `100`→"еденсто"/`1000`→"еден илјада" (→"sto"/"сто"/"илјада"). Added arms
  (`hr`/`sr` hundreds-only, `mk` both) with comma decimals. fi/et/lt/lv/sl/ga/cy/eu
  already correct; da/is/af left as-is ("et hundrede"/"eitt hundrað"/"eenhonderd" are
  legitimate); Albanian `njëqind` is correct (it genuinely says "one hundred"). 3 test
  assertions (334 lib). The omit-one-prefix family now spans de/it/ru/bg/uk/be/tr/hu/
  sk/hr/sr/mk (+ fr/es/nl/mt via their own arms).
- **2026-08-01 — ✅ Phoneme bytecode compiler: CALLPH opcode (§6).** Diversifying from
  the number sweep: added `ProgInstn::CallPh { program }` (`i_CALLPH` 0x9100 + callee
  index) to `compile_program`. **Round-trip verified across the call boundary**: a main
  program `[CALLPH sub, RETURN]` placed before a sub `[FMT, RETURN]` → `scan_phoneme`
  follows the call and surfaces the callee's `fmt_addr`/`fmt_param`. 1 new test (334 lib).
  The program compiler now covers FMT/WAV/VWLSTART/VWLENDING/CONTINUE/CALLPH/RETURN;
  only condition/jump/IPA-name opcodes still go through `Raw`.
- **2026-08-01 — ✅ Slovak/Turkish/Hungarian "one hundred/thousand" prefix (§12).**
  Continued the 100-scan: Slovak `100`→"jedensto" (→"sto"), Turkish `100`→"biryüz"/`1000`
  →"bir bin" (→"yüz"/"bin"), Hungarian `100`→"egyszáz"/`1000`→"egy ezer" (→"száz"/"ezer").
  Added arms: `tr`/`hu` omit both hundreds+thousands, `sk` hundreds only (its 1000 is
  already "tisíc"); all get comma decimals. Czech/Catalan/Greek/Romanian already correct
  (their dicts have the compound or legitimately keep the "one"). 3 test assertions
  (333 lib). Portuguese numbers stay deferred (dict-level: `_1`/`_5`/`_0`/`_1000` entries
  missing though the words `um`/`cinco`/`zero`/`mil` read fine — not a port bug).
- **2026-08-01 — ✅ Italian "cento" prefix (§12).** Multi-language probe found Italian
  `100`→"unocento" (should be "cento" — the "uno" prefix must drop, like German/Russian;
  1000 is already "mille"). Added an `it` arm: `hundreds.omit_one_prefix` + comma
  decimal / period grouping. Now `it 100`→"cento", `101`→"cento uno", `3,14`→"tre
  virgola…"; 200/1000 unchanged. Dutch/Polish already correct (nl/mt arm; Polish dict).
  1 test assertion (333 lib). **Spotted (dict-level, deferred): Portuguese `21`/`1000`/`0`
  → *empty*** — the pt number path/dict entries look broken, a bigger investigation.
- **2026-08-01 — ✅ Ordinal zero ("0th"→"zeroth") (§12).** `0th` had rendered as a bare
  "θ" — `try_ordinal_number` bailed out on an all-zero value (`rposition(...)?`→None),
  so it fell back to letter rules and dropped the number. Fixed: the "no non-zero
  group" case now falls back to the **units** group (not the highest scale — an
  intermediate attempt gave "zero billionth"), and `ordinal_sub_thousand` renders the
  cardinal "zero" for a zero value. Now `0th`→"zeroth"; `1st`/`100th`/`1000000th`
  (scale ordinals) unchanged. 1 integration test (333 lib). **(Also attempted Spanish
  `cien` apocope but reverted — "cien" is produced by letter-rules, not a dict entry,
  so the dict-only number lookup can't reach it without threading `phdata`.)**
- **2026-08-01 — ✅ German/Russian "one hundred/thousand" prefix dropped (§12).**
  Follow-up on the numbers probe: German `100`→"eins hundert", `1000`→"eins tausend";
  Russian `100`→"один сто", `1000`→"один тысяч" — all wrongly kept the "one" (German
  says "hundert"/"tausend", Russian "сто"/"тысяча"). Set `hundreds.omit_one_prefix`
  + `thousands.omit_one_prefix` for `de` and a new `ru`/`bg`/`uk`/`be` arm (plus
  `decimal_separator=','`). Now `de 100`→"hundert", `de 1000`→"tausend", `ru 100`→"сто",
  `ru 1000`→"тысяч"; 200/2000 unchanged (only the "one" is dropped); **English keeps
  "one hundred"/"one thousand"**. Updated the `lang_number_grammars_are_independent`
  guard to match. 1 integration test (333 lib). (Spanish `100`→"ciento"/should be "cien"
  apocope + Russian "тысяча" case declension remain.)
- **2026-08-01 — ✅ French vigesimal 70s/90s numbers (§12).** Probing found France-French
  70–99 **badly broken**: `70`/`90`→empty, `72`→"deux", `99`→"neuf" (only the unit).
  Root cause: the generic tens path looked up a `_7X`/`_9X` word that doesn't exist in
  French (70=soixante-dix, 90=quatre-vingt-dix), then fell through to units only. Added
  `NumberGrammar.vigesimal_70_90` (set for `fr`): 70–79 = `_6X` (soixante) + 10–19,
  90–99 = `_8X` (quatre-vingt) + 10–19. Now `70`→"soixante-dix", `72`→"soixante-douze",
  `99`→"quatre-vingt-dix-neuf", `1999`→"mille neuf cent quatre-vingt-dix-neuf"; `71`/`91`
  keep their special dict entries; 60/80 unchanged. Default false → no effect on other
  languages. 1 integration test. **Also spotted (future):** Russian `100`→"один сто"
  (should be "сто") and Spanish `100`→"ciento" (should be "cien").
- **2026-08-01 — ✅ Minus sign + section/numero symbols (§14).** Probing found the
  Unicode `−` (U+2212 MINUS SIGN, not ASCII hyphen) dropped — `−5`→"five". Added
  `−`→"minus", `§`→"section", `№`→"number" to the symbol readings: `−5`→"minus five",
  `3−5`→"three minus five", `§5`→"section five", `№7`→"number seven", `temp −10°`→"temp
  minus ten degrees". English-gated. Extended `symbol_reading` test. (© ® ™ ′ ″ left
  dropped — usually silent/ambiguous.)
- **2026-08-01 — ✅ Currency symbols + `±` (§12/§14).** Probing found `$5`→"dollar
  five" (**wrong order**), `€10`/`£20`→dropped, `±5`→"five". Added `apply_currency`,
  a token-rewrite (before `build_translate_entries` in both `text_to_ipa` and
  `translate_to_codes`) that turns `<symbol><number>` into `<number> <currency word>`,
  pluralised unless the amount is exactly one: `$5`→"five dollars", `$1`→"one dollar",
  `€10`→"ten euros", `£20`→"twenty pounds", `¥100`→"one hundred yen", `$5.99`→"five
  point nine nine dollars"; handles an optional space (`$ 5`), and a bare `$` still
  reads "dollar". Also `±`→"plus or minus". English-gated (German `$5` unchanged → no
  regression). 1 unit + `-x`. **Cents split added (2026-08-01):** a decimal amount
  becomes "<dollars> dollars <cents> cents" — `$5.99`→"five dollars ninety nine cents",
  `$1.01`→"one dollar one cent" (singular), `$1.00`→"one dollar" (no cents), `$0.99`→
  "ninety nine cents" (no dollars), `$5.5`→"five dollars fifty cents"; cents padded/
  truncated to two digits. Non-English currency words remain future polish.
- **2026-08-01 — ✅ Leading decimal point read (§12).** `.5` had rendered as "five"
  (the `.` was consumed as a clause boundary before the number). Added a tokenizer
  branch: a bare decimal separator followed by a digit becomes a decimal with a `0`
  integer part → `.5`→"zero point five", `.25`→"zero point two five", consistent with
  `0.5`. Language-neutral via `grammar.decimal_separator` (German `,5`→"null komma
  fünf"). Non-decimal dots unaffected — `5.`/`end. next`/`a.b`/`3.14` unchanged.
  1 unit test (332 lib).
- **2026-08-01 — ✅ Math/degree symbols `× ÷ °` spoken (§14).** Probing found these
  dropped (`2×3`→"two three", `10÷2`→"ten two", `50°`→"fifty"). Added them to
  `is_spoken_symbol` + English readings in `lookup_symbol_name` (`×`→"times",
  `÷`→"divided by", `°`→"degrees"); the reader now handles **multi-word** readings by
  translating each part and joining with `PHON_END_WORD`. Now `2×3`→"two times three",
  `10÷2`→"ten divided by two", `50°`→"fifty degrees", `3 × 4 = 12`→"three times four
  equals twelve". English-gated (German `2×3` unchanged → no regression). 1 test (331
  lib + 8 symbol tests). `.5` (leading decimal → "point five") remains a minor gap.
- **2026-08-01 — ✅ Unicode number symbols no longer dropped (§12/§14).** Probing found
  vulgar fractions and super/subscript digits **silently vanished** (`½`→"", `x²`→"ex",
  `H₂O`→"aitch oh", `¼ cup`→"cup") — they're neither letters nor ASCII digits, so the
  tokenizer discarded them. Added `normalize_number_symbols` (a borrow-if-unchanged
  pre-pass in `tokenize_opts`): super/subscript digits → the digit, vulgar fractions →
  `n/m`, spaced so they don't merge with an adjacent number (`10³`→`10 3`). Now `x²`→"ex
  two", `H₂O`→"aitch two oh", `½`→"one slash two"; **language-neutral** (German `x²`→"iks
  zwei"). 1 unit test + `-x` (331 lib, no regressions). Fraction-*word* reading ("one
  half") and "squared"/exponent semantics remain future polish.
- **2026-08-01 — ✅ Phoneme bytecode compiler: VWLSTART/VWLENDING/CONTINUE opcodes (§6).**
  Extended `compile_program`/`ProgInstn` beyond FMT/WAV/RETURN/Raw with
  `i_VWLSTART` (0xd000) + `i_VWLENDING` (0xe000) — the vowel onset/exit transition
  frame addresses (same addr bit-packing as FMT) — and `INSTN_CONTINUE`.
  **Round-trip verified**: `[VwlStart, VwlEnding, Continue, Fmt, Return]` → `scan_phoneme`
  reads back `vwlstart_addr`/`vwlending_addr`/`fmt_addr` exactly, and CONTINUE doesn't
  halt the scan (the FMT after it is still reached). 1 new test (330 lib). The program
  compiler now covers the full frame-address opcode set; conditions/jumps/CALLPH/IPA-name
  still go through `Raw`.
- **2026-08-01 — ✅ Bogus ordinal-suffix parsing fixed (§12/§14).** Probing found
  `the 1990s` → **"the z"** — the number was swallowed: the tokenizer treated *any*
  trailing letters as an ordinal suffix (`1990s`→`Ordinal{1990,"s"}`→garbage, `5km`
  too). Added `is_ordinal_suffix` (only `st`/`nd`/`rd`/`th`, `º`/`ª`, or the
  language's `ordinals.indicator` count) and made the tokenizer peek-validate before
  consuming, so a non-ordinal suffix leaves the number intact and the letters as a
  separate word. Now `1990s`→"…ninety ess", `5km`→"five k m" (number spoken), while
  `3rd`/`21st`/`22nd`/`100th`/`3º`/Dutch `1e` still parse as ordinals. 1 unit test +
  `-x` verification (329 lib, no regressions). Year-plural "nineteen nineties" is a
  separate future enhancement.
- **2026-08-01 — ✅ European decimal/grouping swap (§12).** Fixed clearly-broken
  continental number parsing: `de "3,14"` had rendered as just "drei" (comma dropped)
  and `de "1.000"` as "eins komma null null null" (`.` read as decimal). Added
  `NumberGrammar.decimal_separator` (default `'.'`); the tokenizer's decimal branch
  now uses it, and de/es/fr/nl/mt + the Nordic arm are set to `decimal ','` +
  `group '.'` (the standard European convention). Now `de "3,14"`→"drei komma eins
  vier", `de "1.000"`→"eintausend", `de "1.000.000"`→"eine million", `de "1.000,5"`→
  grouped+decimal, and `de "3."`→"dritte" (dot-ordinal still works — group needs 3
  digits, ordinal needs a non-digit after). **English is the mirror image and
  byte-unchanged** (`3.14`→decimal, `1,000`→thousand). Oracle-safe (binary absent →
  those tests skip; no decimal oracle case existed). 1 unit test + `-x` on de/es/fr.
- **2026-08-01 — ✅ Comma-grouped thousands in numbers (§12).** Found via probing:
  English `1,000` rendered as just "one" (the `,000` was dropped — the comma split
  it into separate clauses). Added a `NumberGrammar.group_separator: Option<char>`
  (default `None`; `Some(',')` for `en`); the tokenizer's digit loop now absorbs a
  separator **only** for a genuine group (exactly three following digits not
  themselves followed by a digit), so `1,000`→"one thousand", `1,234,567` reads
  fully, `1,000.5`→ decimal, while malformed `1,0000` and non-grouping languages
  (German `1,000`→"eins") keep the comma as punctuation. Default `None` → **no change
  for any language except `en`** (327 lib green, no regressions). 1 unit test +
  manual `-x` verification.
- **2026-08-01 — ✅ Sonic tempo/pitch wired into the engine (opt-in) (§8).**
  `EspeakNg::with_post_tempo(f)` / `with_post_pitch(f)` add a post-synthesis
  libsonic hook: `synth` runs the final PCM through `change_tempo`/`change_pitch`
  (both fast and icon/markup paths, via `apply_post_fx`). Default factors `1.0` →
  no-op → **byte-identical** to before. Kept as a *post* hook (not a replacement for
  the base-F0 pitch / frame-duration rate, which are better for normal voice) — the
  pitch hook resamples, so it's a formant-shifting effect voice. **Verified end-to-end**
  (`tests/post_fx.rs`, 3 tests): 1.0/1.0 == default byte-for-byte; tempo 1.6 →
  shorter (ratio 0.4–0.85); pitch 1.4 → same length, altered audio. Suite green (326
  lib + 29 bins). Closes the §8 "wire into engine" follow-up additively.
- **2026-08-01 — ✅ Pitch-shift (duration-preserving) — libsonic pitch scaling (§8).**
  `synthesize::tempo::change_pitch(pcm, factor, sample_rate)` completes the `sonic`
  pair: it shifts pitch **without changing duration** (`factor>1` higher, `<1`
  lower). Method = the standard sonic pitch-shift: `change_tempo` to `factor×`
  length (pitch-preserving), then linear `resample_linear` back to the original
  length so the stretched waveform played faster comes out `factor×` higher at the
  same duration. Standalone; no existing path touched. **Verified oracle-independently**:
  200 Hz ×1.5 → ~300 Hz same length; 300 Hz ×0.5 → ~150 Hz same length; `factor==1`
  pass-through (3 unit tests). Tempo + pitch scaling now both available (326 lib).
- **2026-08-01 — ✅ Pitch-preserving tempo scaling — libsonic role (§8).** New
  `synthesize::tempo::change_tempo(pcm, speed, sample_rate)` time-scales a PCM
  buffer **without shifting pitch** — the job `libsonic` does in upstream. It's a
  pitch-synchronous overlap-add (the core of Bill Cox's `sonic`): AMDF pitch-period
  detection (`find_pitch_period`), then whole periods are **merged** (speed-up) or
  **repeated** (slow-down) with a crossfade, so local waveform shape (pitch) is
  preserved while the period count (duration) changes. Standalone DSP utility — no
  `phondata`, no change to any existing path (zero regression). **Verified
  oracle-independently** (4 unit tests on synthetic sines): 2× → ~0.5× length with
  pitch held at ~200 Hz (not doubled as naive resampling would); 0.5× → ~2× length,
  pitch ~150 Hz; `speed==1`/too-short/non-positive → pass-through. 4 tests (323 lib).
  Not yet wired into the engine rate control (which still scales frame durations);
  exposed for callers to post-process audio.
- **2026-08-01 — ✅ CLI `--compile-phonemes[=<src>]` (§16/§6).** Wired the phoneme
  compiler to the CLI, completing the compiler-CLI story alongside `--compile`:
  `run_compile_phonemes` reads a phoneme-table source (default `<data>/phonemes`),
  parses it with `parse_phoneme_dsl`, and writes the binary `phontab` via
  `compile_phontab`. Refuses a missing source (clean `NotFound`), and prints a note
  that only the phoneme *table* is produced (phonindex/phondata are compiled
  separately). **Verified end-to-end through the built binary**
  (`tests/compile_phonemes_cli.rs`): a synthetic `phonemes` source → `phontab` that
  `parse_phontab` reads back to the same tables/mnemonics/lengths; missing source
  exits non-zero, writes nothing. Help text updated. 2 CLI tests; suite green (319
  lib + 28 bins). (Chose this over `sourceix` threading, which needs a ~150-line
  position-aware tokenizer refactor of a core function — deferred as higher-risk.)
- **2026-08-01 — ✅ Per-clause Sentence events (§7).** Extends the event stream to
  multi-clause text: `synth_with_events` now emits a `Sentence` event at every
  clause boundary (not just the start). A new `AnnCode::ClauseBoundary` (emitted for
  the `code=0 & is_boundary` clause pause, replacing the anonymous `Pause(200)`)
  records a sentence *and* word mark at the next clause's start;
  `synthesize_codes_with_marks` now returns `(pcm, word_marks, sentence_marks)`, and
  the engine interleaves Sentence + Word events in audio order. **Byte-identical
  audio**: `ClauseBoundary` emits the same 200 ms silence and leaves `word_idx`
  (which drives non-final lengthening) untouched — only the *marks* are new (319 lib
  + oracle/synthesis tests green). Verified: "Hello there. How are you?" → ≥2
  Sentence events, first at 0, all events in audio order (4th `word_events` test).
  Remaining: exact word `text_position` through number/abbrev expansion (`sourceix`).
- **2026-08-01 — ✅ Word/sentence boundary events (§7).** `synth_with_events` now
  emits a real event stream instead of only `MsgTerminated`: a `Sentence` event,
  one `EventKind::Word(i)` per spoken word — carrying its **start time in the
  audio** and a best-effort source `text_position` — then `End` + `MsgTerminated`.
  Timing comes from a new `Synthesizer::synthesize_codes_with_marks`, which records
  `output_i16.len()` at each `AnnCode::WordBoundary` (word 0 at sample 0); the public
  `synthesize_codes` is unchanged (delegates, discards marks) so nothing regressed.
  Source offsets come from `word_char_offsets` (scan for word starts) paired by
  index — exact audio timing, best-effort text position (drifts when a token expands
  to several spoken words, e.g. numbers; documented). Icon/markup paths fall back to
  message-level events. 3 integration tests (`tests/word_events.rs`): 2 words →
  ordered events at text-pos 0/6, single word → 1 event, four words → monotonic
  times. Suite green (319 lib + 27 bins). Remaining: source-position threading
  (`sourceix`) for exact text offsets through number/abbrev expansion; per-clause
  `Sentence` events in multi-clause text.
- **2026-08-01 — ✅ Language number-grammar independence test (§13); breath ruled out.**
  Added `lang_number_grammars_are_independent` — a regression guard for the upstream
  #2476 "missing `break`" bug class: asserts each language's `NumberGrammar` is its own
  (en/es/fr/de each carry only their distinguishing config, don't inherit a neighbour's
  tens/ordinal/hundreds rules), that same-arm languages (`nl`/`mt`, `da`/`fi`) share
  intentionally, and that unknown langs fall back to `default` — never a neighbour's
  config (what the C bug produced). The Rust `match` makes the fallthrough structurally
  impossible, so this locks in that no arm was mis-assigned. 1 test (319 lib).
  **Also — variant `breath` investigated and confirmed oracle-gated (backed out):**
  wiring `breath` to the existing voiced-segment `noise_frac` adds only ~0.01 % energy —
  that noise path is calibrated for *unvoiced* content (where noise is the whole signal);
  layered on loud voiced output it's inaudible. Real breath needs per-formant aspiration
  injected into the resonators (`parwav`), which is genuinely oracle-gated. Reverted to
  keep the default path clean. Remaining variant acoustics (breath, roughness) stay
  oracle-gated.
- **2026-08-01 — ✅ Variant per-stress amplitude (`stressAmp`) (§11).** A `+variant`'s
  `stressAmp <a0>…<a7>` (i.e. `voice.stress_amps[]`) now overrides the built-in
  per-stress-level amplitude. `VoiceParams.stress_amps[8]` (populated in
  `make_voice_params` from `VariantParams.stress_amp`) drives
  `VoiceParams::stress_amp_ratio(level)`, which rescales the phoneme `amp_factor`
  in `synthesize_codes` by `stress_amps[level] / STRESS_AMPS_EN[level]` (the
  baseline `amp_factor` already bakes in). A `0` entry = unset → ratio exactly 1.0,
  so the all-zero default is **byte-identical** (316 pre-existing tests green).
  **Verified**: unset-is-identity across all 8 levels, doubling primary stress →
  ratio 2.0, out-of-range level clamps (2 unit tests). 2 new tests (318 lib). Exact
  values + the level-0/1 stress swap are oracle-gated. Remaining variant acoustics:
  breath, roughness (both genuinely oracle-gated — noise mixing / jitter).
- **2026-08-01 — ✅ Variant pitch flutter (`flutter <n>`) (§11).** A `+variant`'s
  flutter now de-robotises the pitch: `VoiceParams.flutter` (populated in
  `make_voice_params` from `VariantParams.flutter`) drives `synthesize::flutter_factor`,
  a per-phoneme F0 multiplier (two incommensurate sinusoids → a smooth non-repeating
  deviation in [-1,1], scaled by `flutter/1000`), applied to `ph_pitch` in
  `synthesize_codes`. Directional approximation of `wavegen.c`'s `Flutter()` (exact
  per-cycle table is oracle-gated). `flutter <= 0` (the default) returns exactly 1.0
  → default path **byte-identical** (314 pre-existing tests green). **Verified**:
  identity-when-off, bounded-by-`flutter/1000`, raises *and* lowers pitch,
  deterministic (2 unit tests). 2 new tests (316 lib). Remaining variant acoustics:
  breath, roughness, stressAmp.
- **2026-08-01 — ✅ Variant echo (`echo <delay_ms> <amp>`) (§11).** A `+variant`'s
  echo now applies: `VoiceParams` gained `echo_delay_samples`/`echo_amp`, populated
  in `make_voice_params` from `VariantParams.echo` (delay → samples via the sample
  rate), and `synthesize::apply_echo` runs as a post-pass at the end of
  `synthesize_codes` (after the trailing silence, so the tail rings into it).
  Faithful to `wavegen.c`: `echo = (delayed * amp) >> 8`, **feedback** (reads the
  already-echoed sample) with **i16-saturating** storage — which is why the shipped
  amps span 15…10000 without diverging (they saturate, as the C stores into a
  `short` buffer). Default (delay/amp 0) is a no-op → the standard path is
  byte-identical (311 pre-existing tests green). **Verified**: exact decay-hop +
  saturation + no-op unit tests, and end-to-end `en+robosoft` (`echo 30 1000`)
  differs from `en` through the full engine. 3 new tests (314 lib). Amp scaling is
  directional/oracle-gated like pitch/formant. Remaining variant acoustics:
  flutter/breath/roughness/stressAmp.
- **2026-08-01 — ✅ Variant per-formant height + width scaling (§11).** Extends the
  frequency scaling from the previous iteration to the other two `formant <i>
  <freq%> <height%> <width%>` fields (`voice.height[i]`/`voice.width[i]`).
  `VoiceParams` gained `formant_height_pct`/`formant_width_pct` (`[i32;7]`, default
  100); `scale_frame_formants` now takes the whole `VoiceParams` and scales
  `fheight`/`fwidth`/`fright` alongside `ffreq`; `formant_scaling_active` covers all
  three arrays; `engine::variant_formant_pct` fills all three from the variant.
  Default path still **byte-identical** (100 % is exact integer identity; 310
  pre-existing tests green). **Verified**: doubling F2's height raises the raw
  Goertzel power at 1400 Hz by >1.5×; exact freq/height/width scaling + 3-way
  active-flag + engine-mapping tests. 2 net new tests (311 lib). The variant
  `formant` line is now **fully applied** (freq+height+width); remaining variant
  acoustics: breath/echo/flutter/roughness/stressAmp (oracle-gated).
- **2026-08-01 — ✅ Variant per-formant frequency scaling (§11).** `+variant`
  voices now apply their `formant <i> <freq%>` lines (`voices.c` `voice.freq[i]`),
  not just pitch. `VoiceParams` gained `formant_freq_pct: [i32;7]`; the harmonic
  path (`synthesize_frames`/`synthesize_frames_seq`) pre-scales each frame's
  formant frequencies via `scale_frame_formants` **only when a variant is active**
  — the all-100 default skips it entirely, so the standard en path is **byte-for-
  byte unchanged** (309 pre-existing tests still green). `engine::variant_formant_pct`
  reads the parsed `VariantParams.formants` into the array. **Verified**: an FFT-free
  Goertzel probe shows raising formants 60 % pushes spectral energy to the higher
  band by >2× (`formant_scaling_shifts_energy_upward`); exact scaling + active-flag +
  engine-mapping unit tests. 4 new tests. **Debugging note:** the first test frame
  left F4–F6 zero, which forced `hmax`=0 (derived from the F5 peak) so mid formants
  never synthesised — a *fully-populated* frame was needed to observe the shift.
  Remaining: per-formant **height/width** scaling, breath/echo/flutter (oracle-gated).
- **2026-08-01 — ✅ Phoneme-source DSL parser (§1.6/§6).** `phoneme/source.rs`
  `parse_phoneme_dsl(source)` — the text front-end of the phoneme compiler,
  symmetric with the dict `parse_rules_dsl`. Parses `phonemetable <name> [<base>]`
  (base → `includes` inheritance) and `phoneme … endphoneme` blocks: mnemonic +
  sequential code, manner/place/voicing via the **existing `PhonemeFeature`/
  `apply_feature` machinery** (`vwl`/`stp`/`blb`/`vcd`/…), the non-feature type
  words (`pause`/`stress`/`vowel`/`liquid`), and `length`/`lengthmod`; program-body
  lines (`FMT`/`WAV`/`IF`) pass through harmlessly. **Round-trip verified** (parse →
  `compile_phontab` → `parse_phontab` preserves mnemonics/types/place/voicing/length
  + `includes`); made `parse_phontab` `pub` as the documented inverse. 2 tests, suite
  green (306). Table-level slice — the synthesis *program* (phonindex/phondata) has
  its own writers (`compile_program`/`compile_spect_seq`) but no source parser yet.
- **2026-08-01 — ✅ CLI `--compile[=<lang>]` — dict compiler exposed (§16/§1.5).**
  Wired the `espeak-ng.c` `--compile`/`--compile-debug` path: `run_compile` reads
  `<data>/<lang>_list`+`_rules`, resolves mnemonics against the `<lang>` phoneme
  table (fallback `en`), and writes the binary `<lang>_dict` via
  `compile_dictionary`. Default `<lang>` = the `-v` voice's base language. Picks
  the right script compression through a new **`TransposeConfig::for_lang`** helper
  (refactored `Dictionary::from_bytes` to share it — DRY, no behaviour change).
  Refuses to silently emit an empty dict when no source exists (clean `NotFound`).
  **Verified end-to-end through the built binary** (`tests/compile_dict_cli.rs`):
  `--compile=en` on a synthetic dictsource → `en_dict`, and `-x hello` then returns
  `h@l'oU` *from that compiled dict*; missing-source exits non-zero, writes nothing.
  Help text updated. 2 CLI tests; full suite green (304 lib + integration). This is
  the first user-facing surface for the whole dict-compiler pipeline.
- **2026-08-01 — ✅ Rule `A`–`Z` letter-group context markers (§1.5).**
  Context encoding is now token-based (`encode_context`): an uppercase `A`–`Z`
  compiles to a two-byte `RULE_LETTERGP <letter>` reference (reader computes
  `group = letter − 'A'`; `A` = vowels, `C` = consonants in the English
  `letter_bits`), so a context char matches any group member. Pre-context reverses
  the *token list* (nearest-match first) while keeping each `RULE_LETTERGP` pair's
  internal order. **Verified through the real engine**: a `A)b` rule (b after a
  vowel) fires on "ab" but not "kb". 1 new test, suite green (304). Remaining
  markers: `@`/`&` (syllable/stress) context.
- **2026-08-01 — ✅ Rule `_` word-boundary context marker (§1.5).**
  `compile_rules` now encodes context chars via `encode_context_byte`: `_` →
  `RULE_SPACE` (the marker real rules use for word start/end, e.g. `_)a`, `a(_`).
  The word buffer brackets each word with spaces, and the reader's PRE/POST literal
  cases reward matching that boundary byte — so a pre-`_` rule fires at word start,
  not mid-word. **Verified through the real engine**: `_)a` fires on "a" but not
  "ba" (helper 'b' group so the engine reaches the non-initial 'a'). 1 new test,
  suite green (303). Remaining context markers: `A`–`Z` letter-groups, `@`/`&` stress.
- **2026-08-01 — ✅ Rule pre/post-context, DSL + compiler + fires (§1.5).**
  `Rule` gained `pre_context`/`post_context`; `compile_rules` emits `RULE_PRE`
  (context reversed, per the reader's backward walk) and `RULE_POST` around the
  match. `parse_rules_dsl` now parses the `pre)match(post` letter-token syntax
  (plain letters; `_`/`#` markers still TODO). **Both directions verified through
  the real engine** via `word_to_phonemes`: a post-'b' rule fires on "ab" not "ax";
  a pre-'k' rule fires on "ka" not "ba" (helper 'k'/'b' groups so the engine
  advances to the target letter). 3 new tests. Suite green (302). Remaining in the
  rule compiler: conditions/stress, `_`/`#` context markers.
- **2026-08-01 — ✅ Dictsource driver + rule-fires verification (§1.5).** (User: "add
  dictsource".) `compile_dictionary(dir, lang, phdata, transpose)` — the top-level
  `compiledict` entry point: reads `<lang>_list`+`_rules` source files → DSL parsers →
  `_dict` binary. **Dict compiler now end-to-end** (synthetic dictsource → compile →
  read → lookup). Also proved a compiled no-context rule **fires through the real
  engine** (`word_to_phonemes` emits /s/). 7 compile tests. Suite green. To compile
  *real* dicts, drop the espeak-ng `dictsource/` tree into a dir.
- **2026-08-01 — ✅ Rules-DSL parser (§1.5).** `parse_rules_dsl(source, phdata)` reads
  `<lang>_rules` source (`.group` + `match phonemes`, mnemonics→codes, group letter
  stripped) → `RuleGroup`s; en-data-verified (parse → `compile_rules` → reader registers
  groups). Both source DSLs (`_list`+`_rules`) now parse to compiler input. Remaining:
  `(`/`)` context + conditions. Suite green.
- **2026-08-01 — ✅ List-DSL parser + Klatt interpolation (§1.5/§1.2).** (User: "add
  dsl".) `parse_list_dsl(source, phdata)` reads `<lang>_list` source text → `ListEntry`s
  (mnemonics→codes via the phoneme table) — **verified end-to-end on real en data**:
  source → compile → read → lookup returns the codes (first compiler slice validated
  with shipped data). Also: `synthesize_frames_klatt` now interpolates formants
  intra-frame (gliding transitions). Suite green. Remaining: the `_rules` DSL parser.
- **2026-08-01 — ✅ Phondata WAV-sample writer (§1.6).** `sample::compile_wav_sample`
  emits the consonant-WAV format (header + 16-bit LE data); round-trip verified
  (structural + `parse_wav_sample` count). Acoustic-data compiler now covers both
  FMT frames and WAV samples. 1 test.
- **2026-08-01 — ✅ Klatt `frame_t` (64 B) writer (§1.6).** `to_bytes_t` +
  `compile_spect_seq` auto-selecting frame size from `FRFLAG_KLATT` → both non-Klatt
  and Klatt frame sequences round-trip byte-identically. 1 test. Frame-data compiler
  now covers both frame types.
- **2026-08-01 — ✅ Phondata SPECT_SEQ frame-data compiler (§1.6).**
  `SpectFrame::to_bytes_t2` + `compile_spect_seq` — inverse of `SpectSeq::parse`
  (header + 44-byte frame_t2 entries). **Round-trip verified** (compile → parse →
  byte-identical frames; 1 test). The phoneme-*data* path now round-trips end-to-end:
  `compile_program` (bytecode) → `compile_spect_seq` (FMT frames). Remaining: Klatt-
  frame + WAV writers, more opcodes.
- **2026-08-01 — ✅ Phondata program-bytecode compiler (§1.6).** (Multi-day item
  3/3.) `synthesize/bytecode.rs` `compile_program`/`ProgInstn` — encodes phoneme
  programs to `phonindex` bytecode (`i_FMT`/`i_WAV` exact bit-packing, `RETURN`,
  raw), inverse of `scan_phoneme`. **Round-trip verified:** `[i_FMT(addr,param),
  RETURN]` → `scan_phoneme` reads `fmt_addr`+`fmt_param` back exactly. 1 test.
  Remaining: more opcodes + the `phondata` data writer. **All 3 multi-day items
  (async streaming, rules-DSL, phondata program) now have a landed tested slice.**
- **2026-08-01 — ✅ Dict rules-DSL compiler, group/rule format (§1.5).** (Multi-day
  item 2/3.) `compile_rules` + `compile_dict_with_rules` emit the binary rule-group
  format — inverse of `build_groups`/`match_rule`. **Round-trip verified:** compile a
  group → reader registers `groups1['a']` + correct rule bytes (`RULE_PHONEMES`+phon+`0`).
  Minimal (no-context rules). 3 tests. Remaining: rule context/conditions, the DSL text
  parser. **Next: phondata program compiler.**
- **2026-08-01 — ✅ Async streaming + command queue (§7).** (User asked for the
  multi-day pieces: rules-DSL compiler, phondata program compiler, async streaming.)
  `AsyncSynth` now streams audio in `CHUNK_SAMPLES` chunks (`AsyncChunk`, callback
  returns `true` to stop, cancel checked between chunks) and the queue is a command
  list — `set_parameter`/`set_voice` apply to later utterances. 3 tests (chunking +
  total==direct synth, callback-stop drains queue, queued slow-rate lengthens next
  utterance). Suite green. **Next: rules-DSL compiler → phondata program compiler.**
- **2026-08-01 — ✅ Phoneme compiler, phontab section (§1.6).** `phoneme/table.rs`
  `compile_phontab` — inverse of `parse_phontab`: `[n_tables]` header + per-table
  header + 32-byte name + 16-byte entries. **Round-trip verified** (compile → parse
  → identical names/includes/mnemonics/codes + full entries; 1 test). Remaining:
  `phonindex` + the `phondata` program compiler. Now all of Klatt/dict/phoneme
  compilers/async/MBROLA have a landed slice.
- **2026-08-01 — ✅ Klatt per-phoneme auto-select (§1.2).** `flush_run` now routes
  runs whose frames carry `FRFLAG_KLATT` through the cascade automatically (not
  just the manual `VoiceParams.klatt` flag) — Klatt frames use Klatt synthesis.
  en has no Klatt frames → byte-unchanged (suite green). 1 test. Also **corrected a
  stale GAPS claim**: `is_klatt` is *not* hardcoded false in production — the parser
  sets it from the frame flag; the `false` is only a test literal.
- **2026-08-01 — ✅ Klatt wired into synthesis (§1.2).** `synthesize_frames_klatt`
  renders real `SpectFrame` runs through the cascade (formants + bandwidths + per-
  frame pitch, resonator state carried across frames); routed from `flush_run` via
  an opt-in `VoiceParams.klatt` flag → **Klatt is now usable**, default harmonic
  path byte-unchanged. Integration test: Klatt output audible + differs from
  harmonic. Suite green. Remaining: per-phoneme auto-select, parallel/nasal branches.
- **2026-08-01 — ✅ MBROLA interface — big-subsystem 4/4 (§1.4).** `src/mbrola.rs`:
  `write_pho` (`.pho` handoff format), `mbrola_database` (`mb-en1`→`en1`),
  `find_database`, and `synthesize` (pipes to the external `mbrola` binary → raw
  PCM, `MbrolaNotFound` when absent). 3 tests. Diphone audio blocked on the
  unshipped binary + databases. **All 4 user-requested subsystems now have a
  landed tested slice; completion is gated on multi-day work or external data.**
- **2026-08-01 — ✅ Klatt cascade formant synth — big-subsystem 3/4 (§1.2).**
  `src/synthesize/klatt.rs`: 2nd-order `Resonator` + `synthesize` — glottal impulse
  train (+aspiration noise) through a cascade of per-formant resonators.
  **Spectrally verified** (`wav-analysis`): steady /ɑ/ at 120 Hz → F0≈120 + F1/F2
  energy peaks >2× the above-formant band. 3 tests. Standalone (no pipeline
  regression). Remaining: parallel branch, nasal, LF glottal, wiring into wavegen.
- **2026-08-01 — ✅ Dict compiler, list section — big-subsystem 2/4 (§1.5).**
  `src/dictionary/compile.rs` `compile_dict` — the inverse of the `_dict` reader:
  builds the header + 1024 hash buckets with the exact entry encoding (transpose-
  compressed word, null-term phonemes, flag bytes) + empty rules section.
  **Round-trip verified** (compile → `from_bytes` → `lookup` returns the same
  phonemes/flags; 2 tests). No `_rules`/`_list` sources ship, so synthetic-only.
  Remaining: the rules-DSL compiler (bulk of `compiledict.c`). Suite green (282 lib).
- **2026-08-01 — ✅ Async runtime — big-subsystem 1/4 (§7).** User asked to add
  the big subsystems (Klatt/MBROLA/compilers/async). Started with async (most
  tractable + verifiable in Rust): `src/async_synth.rs` `AsyncSynth` — background
  worker owning an `EspeakNg`, cancellable `Mutex<VecDeque>`+`Condvar` queue,
  per-utterance callback (PCM+events), `speak`/`cancel`/`synchronize`/`is_speaking`,
  `Drop` stops+joins. Unblocks #172 (utterance-granular cancel). 2 tests. Suite green.
  **Next big items:** dict compiler → phoneme compiler → Klatt → MBROLA.
- **2026-08-01 — ✅ `-x -m` processes SSML (§1.3/§16).** The `-x` mnemonic path
  fed raw text to `translate_to_codes`, so `-m` markup was spoken literally
  ("emphasis hello slash emphasis"). Now runs `ssml_to_speech_text` first →
  `-x -m <emphasis>hello</emphasis>` == `-x hello`, say-as/entities applied.
  All three surfaces (`--ipa`/`-x`/audio) now handle SSML consistently. 1 CLI test.
- **2026-08-01 — ✅ SSML `<voice name>` resolution (§1.3).** `<voice name="French
  (France)">` now resolves to French via `find_voice` (== `xml:lang="fr"`); an
  unresolvable selector falls back to the base voice instead of **erroring**
  (previously propagated "dict not found"). `resolve_voice_lang` in translate. 1 test.
- **2026-08-01 — ✅ CLI `-v` voice-name resolution (§16/§11).** `-v "French
  (France)"`/`-v "Afrikaans"` now work — `resolve_voice` matches names via
  `find_voice` only when the value isn't already a language tag (no regression for
  `-v en`/`-v fr`), then maps through `resolve_dict_stem` (made `pub`) so a locale
  primary (`fr-fr`) resolves to the `fr` phoneme table. 1 CLI test. Suite green.
- **2026-08-01 — ✅ Property-based voice matching (§11).** `voices::find_voice` +
  `VoiceQuery` score voices by language (exact/locale-prefix/family × per-language
  priority), name (human name or code), and gender; `EspeakNg::set_voice_by_query`
  = `SetVoiceByProperties`. Resolves names ("Afrikaans"→af) + locale fallback
  ("en"→en-gb) that `set_voice` can't. Added `VoiceEntry.languages: Vec<(code,prio)>`
  for correct tie-breaking. 2 tests. Suite green (278 lib).
- **2026-08-01 — ✅ Say-as interpret modes in audio (§1.3).** The audio path
  applied `<say-as>` *spelling* (via strip_markup) but not the *interpret* modes
  (ordinal/date/time) — those were `-x`/`--ipa`-only. Added
  `crate::translate::ssml_to_speech_text` (process_markup + `interpret_segment_text`)
  and swapped it in for the engine's strip_markup. Now `-m` audio of
  `<say-as ordinal>3</say-as>` is **byte-identical** to plain "3rd" (reads "third",
  not "three"); date likewise. Single synth → no boundary artifacts. 1 test. Suite green.
- **2026-08-01 — ✅ Audio-path SSML fix + document `<prosody>` (§1.3).** Found &
  fixed a real bug: the engine `synth` markup path spoke every non-`<audio>` SSML
  tag literally (only sound icons were handled). Now `strip_markup`s each text run
  → emphasis/prosody/say-as/`<sub>`/entities work in audio. Then added
  document-level `<prosody rate/pitch/volume>` (`ssml::document_prosody`, applied
  only when one prosody wraps the whole utterance → no scoped-case regression):
  rate 50%→longer / 200%→shorter WAV, exactly matching `-s 88`/`-s 350`. 4 tests
  (unit + engine + CLI). Suite green (277 lib).
- **2026-08-01 — ✅ SSML `<say-as date>` + `<say-as time>` (§1.3).** Reused the
  `Segment.interpret` hook (added `SayAs::Date`/`Time`, generalized `SayAsKind` +
  `interpret_mode`). `date`: ISO `YYYY-MM-DD` → "Month Dayth Year"; `time`: `HH:MM`
  → hour+minute with "oh"/"o'clock". English best-effort. Say-as family now complete
  (characters/digits/telephone/ordinal/date/time/cardinal). 3 tests. Suite green.
- **2026-08-01 — ✅ `<say-as telephone>` + `-b` input encoding (§1.3/§16).**
  `telephone`/`tel`/`phone-number` route to the spell path (digits individual,
  `+`→"plus", `-` silent). `-b <n>` decodes `-f`/`--stdin` bytes via the `encoding`
  module: `1`=UTF-8 (strict), `2`=Latin-1, `4`=UCS-2; a Latin-1 "café" file that
  errors as UTF-8 reads correctly with `-b 2`. 2 tests. Suite green.
- **2026-08-01 — ✅ SSML `<say-as ordinal>` + per-span interpret infra (§1.3).**
  Added `Segment.interpret` (`ssml::SayAs::{Normal,Ordinal}`) — the flagged
  prerequisite: a per-span attribute threaded from the SSML walker (flush+tag the
  span) through `text_to_ipa_with_options`. First use: ordinal reading, done at the
  translate layer where the language is known (`english_ordinal_suffix` → "3"→"third",
  "11"→"eleventh", "21"→"twenty first", "100"→"one hundredth"); inline stays one
  clause; non-English unchanged. 4 tests. **Reusable for date/time/telephone next.**
- **2026-08-01 — ✅ `-z` no-final-pause (§16).** Suppresses the ~0.30 s
  end-of-utterance trailing silence (§35). New `VoiceParams.no_final_pause` gates
  the silence in `synthesize_codes`; `EspeakNg::set_no_final_pause` +
  `make_voice_params` thread it; CLI `-z` (was a no-op stub). Verified: `-z`
  removes ≈6615 samples (~300 ms) from the WAV. 1 test. Suite green.
- **2026-08-01 — ✅ SSML `<say-as digits>` + crash-safety tests (§1.3/§5/§15).**
  `digits`/`tts:digits` route to the spell mechanism → each digit read separately
  ("42"→"four two", was "forty two"; "007"→"zero zero seven"). 3 tests. Also added
  `tests/crash_safety.rs`: Greek `ϖ` (#2467) through `el`, unknown/empty voice
  (#2469), and adversarial inputs (emoji, unterminated `[[`) — all return cleanly,
  no panic (Rust is memory-safe; locks that in). `ordinal`/`date`/`time` deferred
  (need number-grammar hints past the language-agnostic SSML layer).
- **2026-08-01 — ✅ Runtime flags reach the audio path (§14/§16).** `--punct`,
  `-g` (word gap), `-k` (capitals) previously only affected `-x`/`--ipa` — the
  engine `EspeakNg` built its own translator without them. `make_translator` now
  applies `word_gap`/`capitals`/`punct`; added `set_punctuation_list` +
  `Parameter::{Punctuation,Capitals}` get/set; **ported the `-k 2` capital
  announcement into `translate_to_codes`** (was `text_to_ipa`-only, so `-x -k2`
  and audio now announce "capital"). 3 engine-audio tests (each flag lengthens
  the WAV). Suite green.
- **2026-08-01 — ✅ `--punct`/`--punct=CHARS` (§14).** Announce punctuation by
  name. `LangOptions.punct` + `punct_covers`; punct-covered `ClauseBoundary`/
  `Punctuation` tokens become `Word` entries with the char's name phonemes
  (`,`→"kˈɒmə", `.`→"dˈɒt"), gated so default output is byte-identical. `text_to_ipa`
  speaks them via its Word arm for free; `translate_to_codes` ClauseBoundary arm
  emits the name (END_WORD-bracketed). CLI `--punct`/`--punct=","`. 3 tests.
  Phoneme-output path only (WAV/`synth` not yet — engine builds its own options).
- **2026-08-01 — ✅ Spoken symbols in `-x`/synthesis (§14).** `translate_to_codes`
  had no `Token::Punctuation` arm (same class as the inline-`[[…]]` bug), so
  `&`→"and", `%`→"percent" etc. rendered in `--ipa` but were dropped in `-x` and
  silent in audio. Added `Token::Punctuation(_)` to the word-emission arm
  (non-spoken punctuation → empty phonemes → no-op). `-x "a & b"` → `'eI and b'i:`.
  1 regression test (`symbol_reading.rs`). Suite green.
- **2026-08-01 — ✅ Variant PITCH applied to synthesis (§11).**
  `make_voice_params` scales `pitch_hz` by the variant's `pitch` base ÷ 82
  (eSpeak default) — `variant_scaled_pitch`, variant-only path so the default
  voice is byte-unchanged. **End-to-end verified** (not just directional-by-code):
  `wav-analysis` YIN shows `en+f3` mean F0 > `en` by >1.2× (`synthesis_analysis.rs`,
  run with `--features wav-analysis`). 1 unit + 1 e2e test. Not oracle-exact.
  **Next:** apply formant/breath/echo/flutter (the harder, more oracle-gated ones).
- **2026-08-01 — ✅ Variant-file parsing + validation (§11).**
  `voices::VariantParams` + `VariantFormant` + `load_variant(data_dir, name)`
  parse `voices/!v/<name>` (pitch, formant lines, flutter, voicing, consonants,
  roughness, echo, stressAmp/Add, breath/breathw, name/gender/age). CLI now warns
  on a missing `+variant` (`-v en+nope` → stderr) and still speaks the base.
  3 unit + 1 CLI test. **Next:** apply `VariantParams` to synthesis (oracle-gated;
  start with pitch — directionally verifiable via `wav-analysis` YIN: f3 > base).
- **2026-08-01 — ✅ Voice-variant name parsing `en+f3` (§11).**
  `split_voice_variant` splits `base+variant`; `Translator::new` +
  `VoiceSpec::by_name`/`builder().language` now resolve the base language for
  dict/phontab (was: `-v en+f3` → "dict not found") and store the variant in the
  new `VoiceSpec::variant` field. `en+f3 --ipa hello` now == `en`, and `en+f3`
  synth produces audio. 3 unit tests. **Not yet:** applying the variant's
  acoustic params (`voices/!v/<name>`: pitch/formant/stressAmp/breath/echo) —
  oracle-gated (the file parse + effect are frame/spectrum-verifiable later).
- **2026-08-01 — ✅ CLI `--voices[=<lang>]` (§16).** New `src/voices.rs`
  (`list_voices` + `VoiceEntry`) walks `<data>/lang/**`, parses the
  `name`/`language`/`gender`/`age` identification lines, and lists all 145
  voices sorted by code, filterable by language prefix. Replaces the no-op
  `--voices` arg stub; help text updated. 2 module unit + 2 CLI smoke tests.
  Listing only (not the full acoustic voice spec / variant matching, §11).
- **2026-08-01 — ✅ Inline `[[…]]` in the `-x`/synthesis path (§1.3).**
  `translate_to_codes` built the inline-phoneme entry but its emission `match`
  swallowed `Token::InlinePhonemes` (`_ => {}`), so `-x "[[k]]"` was empty and
  `[[…]]` synthesised silently. Added the token to the word-emission arm — now
  `-x "[[h@l'oU]]"` → `h@l'oU` (exact round-trip) and audio renders. Parity with
  the `--ipa` path. 1 regression test (`inline_phonemes.rs`). Suite green.
- **2026-08-01 — ✅ SSML `<phoneme ph="…">` (§1.3).** `ph` attribute (eSpeak
  mnemonics) bridged to the always-on `[[…]]` inline-phoneme span, element
  content discarded; `<phoneme>` without `ph` strips like an unknown tag.
  End-to-end IPA byte-identical to `[[ph]]` (the oracle-matched path). 2 unit
  (`ssml.rs`) + 1 integration (`ssml_integration.rs`) test. Full suite green
  (262 lib + integration). Not yet: `alphabet="ipa"` transliteration.
- **2026-08-01 — verified (no change) #2469/#2475 crash-safety (§15).** The port
  already returns clean errors for missing/empty phoneme tables and can't panic
  on the `phonSWITCH` path. Documented in §15.
- **2026-08-01 — deferred #33 article "a".** Root-caused (see §33 note); needs
  the oracle to confirm the target vowel. Good candidate once an oracle exists.

**Next tractable candidates (no oracle needed, self/suite-verifiable):**
The clean, self-contained CLI/SSML backlog is largely cleared (13 features landed
2026-08-01). What remains is either **larger** (needs new infra) or **oracle-gated**:
- **Scoped** `<prosody>` (document-level done 2026-08-01) — per-*span* acoustic
  params in the engine `synth` (the audio path now strips SSML but renders one
  voice/rate; per-span needs the engine to iterate segments). Verifiable by WAV.
- `<break time="Xms">` variable pause (encode a duration on the break sentinel →
  proportional pause; verifiable by WAV length).
- `-X`/`--pho` phoneme trace/timing (needs per-phoneme durations surfaced from the
  synth timing model).
**Oracle-gated (defer):** variant amplitude/formant/breath (per-stress table, deep
in synth); exact `--pho` durations; phonSWITCH `-x` format; digit-stress `[[…]]`;
`<phoneme alphabet="ipa">`; #33 article "a" reduction. **Big subsystems:** Klatt,
MBROLA, dict/phoneme compilers, async/audio runtime (§1.2–1.7) — multi-day each.
**Oracle-gated (defer):** phonSWITCH `-x` output format (needs a known trigger
word + oracle); in-bracket stress-digit notation for `[[…]]` (numeric-stress
semantics unclear); `<phoneme alphabet="ipa">`; #33 article "a" reduction.
**Won't-reproduce:** the Greek `.replace` segfault (#2467) and empty-phoneme-table
crash (#2469) are memory-unsafe C bugs; Rust returns clean errors — verified, no
panic. A regression test would be near-trivial (asserts no panic).
