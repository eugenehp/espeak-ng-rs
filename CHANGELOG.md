# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.4] - 2026-09-05

### Added

- **The dictionary compiler reproduces upstream's.** `--compile` was measured by
  compiling each language's real `dictsource` with *both* compilers and speaking
  the same words through each result: **1144 of 1154 words across 80 languages**
  now come out identical (English alone went from 11/19 to 19/19). What was
  missing:
  - **Rule-context commands.** Every uppercase letter was encoded as a letter
    group; only `A B C F G H` and `Y` are groups (and through C's own
    `lettergp_letters[]` mapping, not alphabetically). `K` is "not a vowel",
    `D` a digit, `V` "if verb", `Z` non-alpha, `N` no-suffix, `W` spelling,
    `X` no-vowels, `J` skip-chars, `T` `$w_alt1`; `$unpr`/`$noprefix`/`$list`/
    `$w_altN`/`$p_altN` and the `P`/`S` prefix/suffix endings are encoded too.
    English "world" needs `w) or (K` → `3:`.
  - **Rule conditions.** `?n` / `?!n` compile to `RULE_CONDITION`; English uses
    225 of them.
  - **Rule and group ordering.** A group's rules are sorted by their phoneme
    string and identical pronunciations share one copy via `RULE_PH_COMMON`;
    groups are sorted long-name-first and same-named groups merge. The group
    order is load-bearing — the reader expects every multi-byte group sharing a
    first byte to be contiguous, and without it Czech `ý` and Polish `ę` were
    unreachable (`dobrý` → `d'obR`).
  - **`_)` becomes `RULE_PRE_ATSTART`**, which matches the true start of a word
    where a literal space does not.
  - **List-entry order.** C builds each hash chain by *prepending*, so a word's
    entries are written in reverse source order and the file's *last* definition
    wins. English `a` (`$nounf` then `a eI $atend`) and `to` depend on it.
  - **List keys** are lowercased (with `$allcaps` noted) and `U+xxxx` names a
    character, so `Louis` and `I` are found at all.
  - **`$text` / `$textmode` / `$phonememode`.** An entry marked `$text` gives
    replacement *text*, not phonemes — French `aujourd'hui  aujourdui $text` was
    being read as phoneme mnemonics — and a lone `$textmode` line switches the
    rest of the file. `roots`, `listx`, `emoji` and `extra` are read alongside
    `list`, in C's order.
  - **Group names** are truncated to two bytes (Danish `.group øs` *is* the `ø`
    group) and `0xNNNN` names a character code; non-Latin scripts write their
    groups by `groups3` index, as C does.
  - Over-long entries degrade to flags-only instead of failing the compile, and
    `phonSWITCH`'s language name is lowercased and dropped when it is the
    default voice.
- **`.Lnn` letter groups compile.** Ten of them are used by the shipped English
  rules (`_) c (L01Y` keeps `cl`/`cr` hard), and a rule whose group is undefined
  never matches. Definitions are emitted longest-item-first, as C does, and
  `Lnn` in a rule context encodes to `RULE_LETTERGP2` with the pre-context's
  reversed byte order.

- **Language configuration is read from the voice file** (the direction of
  upstream #1466, which proposes replacing the transcribed `tr_languages.c`
  tables with data). `pitch`, `intonation <n>` (22 shipped languages set it, and
  it selects which tune each clause terminator maps to) and `stressLength
  <l0…l7>` (20 languages) are now honoured. English (Scotland) is measurably
  shorter than the default voice and Lancashire longer, matching upstream's
  direction.
- **`dicts/` dictionary layout accepted** (upstream #2261). `dict_path` tries
  the flat `<data>/<lang>_dict` first and then `<data>/dicts/<lang>_dict`, so a
  data refresh that adopts the new layout doesn't break every voice.
- **Unicode compatibility (NFKC) folding of input** (upstream #2523), applied
  only to the 183 characters this port would otherwise **drop** — squared units
  (`㎏` → `kg`, `5㎏` → "five kilograms"), the remaining letterlike symbols,
  enclosed forms and ligatures. Characters it already reads better than their
  NFKC form (`™` → "trademark", `№` → "numero", circled digits, superscripts,
  full-width letters) are deliberately not in the table.
- **Fuzz-style parser robustness tests** (upstream #2441, which adds OSS-Fuzz
  dictionary targets). Every binary parser — `phontab`, `_dict`, `_phtrans`,
  `phondata` spectrum sequences — is fed random bytes, every truncation of a
  real file, and single-byte corruptions, and must not panic. Deterministic, so
  a failure is reproducible from its seed.

- **Foreign scripts are spelled, not switched, when upstream spells them.**
  A script without `AL_WORDS` in `tr_languages.c`'s `alphabets[]` (Cyrillic,
  Hebrew, Thai, Han, …) is read out with the current language's letter names:
  `en "хорошо"` → `x'a: 'o: 'Er 'o: S'a: 'o:`, byte-identical to upstream, where
  the port used to switch to Russian. `AL_WORDS` scripts (Arabic, Devanagari,
  Korean, Georgian, Tamil, …) still switch. A script with no letter names in the
  current language falls back to its own voice rather than being dropped —
  a deliberate divergence, since upstream says "Chinese letter".
- **`-X` (translation-rule trace) and `-D` (deterministic random mode).** `-X`
  ports `DecodeRule` and `DecodePhonemes`, printing `Translate '<word>'` or
  `Found: '<word>' [<phonemes>]  <flags>` and then `<score>\t<rule> [<phonemes>]`
  per matching rule, with a blank line between letter positions and the
  resulting phonemes last. Byte-identical to upstream for words translated by
  rules and for dictionary hits. `-D` pins the noise seed (`espeak_ng_SetRandSeed`,
  also exposed as `EspeakNg::set_rand_seed`); the port never seeds from the
  clock, so its output is reproducible either way.
- **Inline embedded commands are positional.** `\x01<n>S`, `\x01<n>P` and
  `\x01<n>A` now take effect from where they appear instead of being folded into
  the whole utterance, which is what makes **scoped `<prosody>`** work:
  `<prosody rate="50%">slow</prosody> fast` slows only "slow", as upstream does.
  The SSML element emits the same embedded commands upstream writes, and the
  values are clamped to the `-s`/`-p`/`-a` ranges.
- **`$flags` are compiled.** `dollar_flag_byte` maps every `mnem_flags` name
  (`$atend`, `$u+`, `$allcaps`, `$only`, …) to the flag byte the `_dict` format
  stores, so a self-compiled dictionary keeps its conditional entries.
- **The voice `tone` curve.** `set_tone_adjust` ports `SetToneAdjust`, and a
  `tone <freq> <height> …` line in a voice or `+variant` file now shapes the
  harmonic spectrum. It is applied *relative to the default curve*, so the
  standard voices keep the levels they were tuned at (see the code comment).

- **`klatt.c` fully ported** (`src/synthesize/klatt.rs`). The frame renderer is
  now a direct port of upstream's `parwave()` / `Wavegen_Klatt()` /
  `SetSynth_Klatt()` / `frame_init()` / `pitch_synch_par_reset()`, replacing a
  placeholder that ran a bare cascade off a phase counter. Cascade **and**
  parallel branches, the nasal antiresonator (`setzeroabc`, with C's `a == 0`
  guard), both glottal source models (impulsive doublet through `RGL`, and the
  "natural" quadratic pulse) at 4× oversampling, spectral tilt, open-phase
  breathiness, glottal skew, `DBtoLIN`, F0 flutter and the fade-in/out ramps.
  Verified against the C oracle on a sustained vowel through `-v en+klatt`:
  spectral-envelope **correlation 0.997**, RMS difference 2.4 dB. Ten unit tests.
- **MBROLA: the phoneme-translation compiler and the `.pho` writer.**
  `MbrolaTable::compile` ports `compilembrola.c` (the `phsource/mbrola/<db>`
  source format), `parse`/`to_bytes` the binary `<db>_phtrans` layout,
  `mbr_name` ports `GetMbrName` including all seven `control` bits (skip-next,
  match-previous, word-initial, no-cross-word, name-prefix, stressed-only,
  word-final), `write_pitch` ports `WritePitch`, and `translate` ports
  `MbrolaTranslate`. All 49 shipped `_phtrans` tables round-trip byte-for-byte,
  and the compiler reproduces the shipped `en1_phtrans` rows exactly.
- **`--compile-mbrola=<source>` and `--pho`.** The first compiles an MBROLA
  source into `<data>/mbrola_ph/<name>_phtrans`; the second writes the `.pho`
  phoneme/duration/pitch script instead of speaking. `Synthesizer::mbrola_phonemes`
  builds the phoneme list (mnemonics, types, stress, durations, and each
  syllable's pitch envelope) that `translate` consumes.
- **`mb-*` voices speak.** `voices::load_mbrola_voice` reads
  `voices/mb/mb-<db>` (its `mbrola <db> <phtrans>` directive and the
  best-priority `language` line), the CLI hands the `.pho` to the external
  `mbrola` binary, and — since neither that binary nor the diphone databases are
  shipped — falls back to the built-in synthesizer on the voice's own language
  with a clear warning instead of failing.
- **Incremental synthesis: `EspeakNg::synth_streaming` and
  `synth_streaming_with_events`.** Audio is delivered one clause at a time, as
  soon as that clause is rendered, so the first samples of a long utterance are
  available after its first clause rather than its last. Concatenating the chunks
  reproduces `synth()` exactly (asserted in `tests/streaming_synth.rs`); a
  callback returning `true` stops synthesis; sound icons, SSML and the tempo/pitch
  post-pass fall back to a single final chunk. `AsyncSynth` now renders this way,
  so its queue produces audio sooner.
- **`parallel` feature (rayon): `EspeakNg::synth_many`** renders a slice of
  utterances across the rayon pool, returning per-item results in input order.
- **`tokio-runtime` feature: `espeak_ng::tokio_synth`** — `synth` awaits a whole
  utterance and `synth_stream` returns a bounded `mpsc::Receiver` fed clause by
  clause, both off the executor via `spawn_blocking`, with backpressure and
  drop-to-cancel.

- **Chinese and Cantonese now speak.** Their dictionaries are compiled as
  replacement *text* (`你` → `ni3`), which reverses the meaning of
  `FLAG_TEXTMODE` (upstream `langopts.textmode`); reading the entries as phoneme
  codes produced gibberish (`你` → `qr`). With that fixed, plus **tone phonemes**
  (phSTRESS-type entries that carry a program — they were being dropped as stress
  markers), tone placement after the syllable's vowel, the `_|` inter-word gap
  (`langopts.word_gap`), tone digits staying inside pinyin words, and **Mandarin
  tone sandhi** (a port of `CalcPitches_Tone`'s Mandarin branch: default tones,
  `214`+`214` → `35`, `51`+`51` → `53`, neutral-tone levelling), `-x -v cmn 中文`
  is now byte-identical to upstream, as are 北京 and 请问; `yue 你好` matches too.
- **`Translator::text_to_ipa_with_terminator` / `EspeakNg::text_to_phonemes_with_terminator`**
  and the `ClauseTerminator` type — the Rust form of upstream's new
  `espeak_TextToPhonemesWithTerminator` (1.53.0). The phoneme string alone
  doesn't say whether the text ended a sentence, asked a question, or simply ran
  out; callers driving a phonemizer or prosody model need that.
- **Foreign-script text is spoken instead of dropped.** A run written in a script
  the voice doesn't read is translated by a voice that does — 22 scripts are
  routed (Latin, Cyrillic, Greek, Arabic, Hebrew, Armenian, Georgian, the Indic
  set, Sinhala, Thai, Myanmar, Ethiopic, Cherokee, Hangul, Kana, Han). `-v ru
  "привет hello"` had silently lost `hello`, in `--ipa` *and* `-x`; likewise
  `-v en "hello 你好"` and `-v en "hello مرحبا"`. Serbian is treated as digraphic
  (Cyrillic *and* Latin), and a word the current dictionary knows is never
  switched away. Includes upstream #2511's `alt_alphabet` hook, by which Arabic
  nominates Mandarin for Han ideographs.
- **Skin-tone emoji sequences** (`👍🏽`) are spoken as the base name plus the
  modifier name when the sequence has no dictionary entry of its own — they
  produced *no output at all* before. Matches upstream 1.53.0.
- **Subdivision-flag emoji** (`🏴󠁧󠁢󠁥󠁮󠁧󠁿`, `🏴󠁧󠁢󠁳󠁣󠁴󠁿`): the tag sequence is kept in the
  token, so they read "England" / "Scotland" instead of "black flag".
- **Stacked suffixes** (`SUFX_M`): `s's's` now keeps its stem (`ˈɛsɪz ɪz`), where
  the chain used to swallow it. Bounded at 50 iterations — upstream left this
  loop uncapped, which is how a 440-byte `s's's…` smashed its stack (#2495).

### Changed

- **Standard voices no longer render through the Klatt engine.** The run
  dispatcher routed to Klatt whenever a frame carried `FRFLAG_KLATT` — but that
  flag only means "this frame carries the extra parallel-resonator parameters",
  and *every* shipped `en` frame has it, so all standard voices were being
  synthesized by the wrong engine. C routes to Klatt only when the voice sets
  `klattv[0]` (`-v en+klatt`), and the port now does the same. Measured against a
  locally built upstream binary over an eight-sentence sweep, the spectral
  envelope moved **4.19 → 2.79 dB RMS difference** (correlation 0.938 → 0.945);
  on "hello world" alone, 14.17 → 4.61 dB.
- **`frame_to_peaks` sets peak 7's height.** C's `SetSynth` runs
  `for (ix = 0; ix < 8; ix++)` for `height` while frequencies stop at peak 6; the
  port stopped both at 6, leaving the fixed 7800 Hz peak silent. Measurably
  neutral on the shipped data (`fheight[7]` is 0 there), but the loop now matches.
- **`klattp2` is indexed correctly.** It continues `klattp` from index 5 —
  `Kopen, AVp, Fric, FricBP, Turb` — so frication amplitude is `klattp2[2]`; the
  wavegen voiced/noise decision was reading `klattp2[1]` (AVp) as `Fric`.

- **Bundled data refreshed from eSpeak NG 1.52.0 (Dec 2024) to upstream `master`
  `f135499` (1.53.0-dev, 2026-09-01)** — 432 commits of language data. The data
  directory now carries **123 dictionaries** (was 114) and **151 voices**, adding
  `ab, crh, fo, kaa, lij, mn, ps, rup, xex`; six of those already had voice files
  bundled, so `--voices` advertised them while `-v crh` failed with "dict not
  found". Every advertised voice now speaks (regression test:
  `every_advertised_voice_can_speak`). Also brings updated `de, en, es, fa, fr,
  he, hu, ko, ku, pt, ru, sv, th, tlh` and the CLDR 33.1 → 48.2 emoji/symbol data
  (66 languages updated, 28 newly covered). The `data-crates/` sub-crates and the
  generated feature list were regenerated to match.

  Two test expectations moved with the data and were re-verified against a
  locally built upstream binary of the same revision: German `zwanzig` is
  `tsvˈantsɪç` (was `…ɡʰ`), Portuguese `primeiro` is `pɾimˈeɪɾˈʊ` (tap).

### Fixed

- **The voice file's `replace <flags> <old> <new>` substitutions run.** Fourteen
  shipped voices use them; Russian's `replace 03 a a#` is what makes a
  word-final unstressed `a` into `a#` (`дела` → `d;'ela#`). The flags select
  word-end only, word-start only, and "not in a stressed syllable", and a `NULL`
  replacement deletes the phoneme.
- **A stress condition asks about the phoneme it names.** `nextVowel(isMaxStress)`
  was being evaluated against *this* phoneme's stress, so Russian "мама" came out
  `m'ama#` where upstream has `m'Ama#`. `Neighbours` now carries the previous and
  next-but-one syllables' levels too.
- **A rule's post-context can reach past the word boundary.** C's matcher walks
  the whole clause, so `_) o (_C` reads the Portuguese article "o" as `U` before
  a consonant-initial word; the port translates a word at a time, and now carries
  the next word's opening into the rules buffer. Composed multi-word numbers get
  it too, so French "vingt et unième" keeps the liaison `t`.
- **`$abbrev` with no pronunciation spells the word out**, as C does — English
  `usa  $abbrev` is `j,u:,Es'eI` — with upstream's spelling stress (every primary
  but the last demoted to secondary) and its letter-name lookup order (`_a`
  before the word "a").
- **A word with no vowel is spelled letter by letter** (C's `Unpronouncable`
  path): "BBC" is `b,i:b,i:s'i:` and a bare "km" is `k,eI'Em`, while "street",
  "brown" and German "schnell" are read normally. `LOPT_UNPRONOUNCABLE` and
  `max_initial_consonants` are transcribed from `tr_languages.c`.
- **The unpron test's own gate was inverted**: it read the outer flag rather than
  the copy `RULE_PRE_ATSTART` clears, so every rule was skipped and the test
  could never see a match.
- **A digit takes its combining `_<n>a` form before a scale word.** German reads
  100 as "einhundert" and 1000 as "ein tausend" but 1 as "eins"; it also keeps
  the "ein" that the port used to omit (upstream sets neither
  `NUM_OMIT_1_HUNDRED` nor `NUM_OMIT_1_THOUSAND` for German).
- **`phonX1` is a marker, not a sound** — C reads it as "double the next word's
  initial consonant" and keeps it out of the phoneme list, where the port spoke
  it and printed a stray `_X1` (Finnish `k'aikIlle_X1`).

- **Per-language stress placement is transcribed from `tr_languages.c` instead
  of guessed.** The old table was one ad-hoc match whose catch-all reset
  `stress_flags` to zero, so the flags table below it reached almost nothing.
  Bulgarian, Esperanto, Indonesian, Malay, Macedonian, Romanian, Swahili and
  Azerbaijani had the wrong rule outright (Latvian stressed the second syllable
  of "pasaule", Armenian the first of "բարեւ"), and Croatian, Hungarian,
  Georgian, Serbian and Bosnian silently lost their flags. `unstressed_wd1` /
  `unstressed_wd2` are now set for the eleven languages that override them, and
  Russian/Ukrainian get the `S_NO_AUTO_2` that `Translator_Russian` sets.
- **`SUFX_Q` — "don't retranslate".** When a suffix rule carries `q` and the
  stem isn't in the dictionary, C keeps the phonemes the *whole word* produced
  rather than re-running the rules on the stem. Italian "casa" matches
  `@C) a (_S1q`, and retranslating lost both the intervocalic `A) s (A → z` and
  the open/close `o` distinction (`k'asa m'Olto` for `k'aza m'olto`).
- **Inter-word pauses** (`MakePhonemeList`'s `sourceix` block): `langopts.
  vowel_pause` inserts a break between two vowels across a word boundary
  (Italian `bUondZ'O@-*no_| a`), before any vowel-initial word, or after a word
  ending in a consonant; `word_gap & 7` then sets a minimum for every boundary,
  including the one before the clause's end. The `words <gap> [vowel_pause]`
  voice directive is read too — Latin's `words 2` is what puts a short pause
  after every word (`s'alwE_ m'UndE_`).
- **`$pause` and `$brk`** put a pause *before* a word, as C's `pre_pause` does:
  Swahili's `za  $u $brk` gives `hab'aRi_! za`.
- **A hyphen between two letters joins the words it separates.** Each part is
  still looked up on its own — so "well" and "known" keep their own stress — but
  they are spoken as one word: `wˈɛlnˈəʊn`, not `wˈɛl nˈəʊn`. Likewise
  "twenty-two" (`tw'Entit'u:`), "state-of-the-art" and French "allez-vous".
- **`LOPT_IT_LENGTHEN`**, which was parsed and never applied: the `:` length
  mark is dropped from a syllable that isn't stressed enough to keep it (bit 0)
  or that isn't the word's most stressed (bit 4). Slovenian's rules give every
  `o` a length mark and only the stressed one keeps it — `d'O:bRO`, not
  `d'O:bRO:`.
- **`isFirstVowel` / `isSecondVowel`** are evaluated instead of always being
  false. Malayalam's `a` keeps its full quality in a word's first syllable
  (`hal'o:`, not `h@l'o:`).
- **`phonX1` is a marker, not a sound.** C reads it as "double the next word's
  initial consonant" and keeps it out of the phoneme list; the port spoke it and
  printed a stray `_X1` (Finnish `k'aikIlle_X1`).
- **The Cyrillic `Y` letter group** no longer carries Russian's additions in
  every Cyrillic language. `SetCyrillicLetters` puts only `ь ю я ё` in it;
  `Translator_Russian` adds `е и є ї` for Russian and Ukrainian alone, and
  applying those everywhere palatalised Bulgarian's `в` before `е`
  (`zdr'av;ej` for `zdr'avej`).
- **A composed word's own stress marks are honoured** (C's `GetVowelStress`
  always reports the primary it finds). The port had an exception for composed
  entries that gave Albanian "21" and Spanish "1º" a second primary stress.

- **`--ipa` is rendered from the clause's phoneme list, as upstream does.** It
  used to render each word on its own, so it could not see a cross-word
  allophone and had to guess a phoneme's IPA name from its mnemonic. It now
  ports `GetTranslatedPhonemeString(espeakPHONEMES_IPA)` and the `use_ipa` half
  of `WritePhMnemonic`: the same list `-x` prints, with each phoneme's name
  taken from **running its bytecode program** with real neighbours. A
  32-language sweep went **20/32 → 32/32**, and a wider 84-phrase corpus
  46 → 58. Concretely:
  - cross-word allophones appear (`es` `bwˈenos ðˈias`, `pt` `bˈõn dˈiɐ`,
    `uk` `prɪβˈiːd̪ zβˈiːt̪`);
  - a name can depend on context (`it` `*` is `r` between vowels, `ɾ`
    elsewhere) or simply differ from the mnemonic (`tr` `R` is `r`, not `ʀ`;
    `es` `i;` is `i`, not `iʲ`);
  - the mnemonic fallback is C's, character for character, so French keeps the
    `-` of its reducible schwa (`lˈə-`).
- **Stress-conditioned phoneme changes** (`ChangeIfDiminished` and friends) are
  now evaluated by the interpreter against the syllable's real stress level, a
  port of `StressCondition`. Previously any non-primary vowel was reduced, which
  is too coarse in both directions: English "unseen" kept its unreduced `ʌ`
  upstream (that syllable is *unstressed*, not *diminished*) while Russian
  "молоко" does reduce to `mʌlʌkˈo`. The gate C applies alongside it is ported
  too — a word the dictionary *list* pronounces keeps the phonemes it was given
  unless the language sets `LOPT_REDUCE & 1` (only Italian does), which needs a
  faithful `FLAG_FOUND`: a flags-only `$` entry leaves the rules to pronounce
  the word and does **not** count.
- **`prevPhW` / `nextPhW` no longer see across a word boundary.** The
  code-list passes never set "this phoneme starts a word", so Dutch's
  `IF prevPhW(x) THEN ChangePhoneme(NULL)` deleted the `ɣ` of "twintig graden",
  and a composed number assimilated across its internal `END_WORD` ("1km" →
  `w'0N k'Il@m,i:t3`).
- **A spoken symbol between two words is its own word**: "cat.Dog" reads
  `kˈat dˈɒt dˈɒɡ`, and German "3.14" `dɾˈaɪ pˈʊŋkt fˈɪɾtseːn`, where the three
  words used to run together.
- **Number words honour the voice's `dictrules`**, so European and Brazilian
  Portuguese pick their own `?1`/`?2` variants of `_14`.
- **Non-phoneme markers are dropped before synthesis' look-ahead window**, where
  one used to sit between two real phonemes as a spurious `code == 0` and break
  their coarticulation.
- **The voice tone curve's shape is applied.** The curve was being divided by
  the default curve *per index*, which cancels it exactly; it is now divided by
  a single constant, so the default curve's 3.8 dB tilt between its
  low-frequency and high-frequency values survives. An eight-sentence spectral
  sweep against the oracle: 2.22 → **2.17 dB** mean envelope difference,
  correlation 0.968 → **0.970**.
- **Every language's `-x` output now matches upstream.** A 32-language sweep of
  two-word phrases went from 23/32 to **32/32 byte-identical**:
  - **Secondary-stress placement.** Greek had `S_NO_AUTO_2`, which upstream does
    not set (`kalim'era` for `k,alim'era`); Czech and Slovak were missing
    `S_FINAL_DIM_ONLY | S_FINAL_NO_2`; Korean had no arm at all and fell to the
    default rule instead of `STRESSPOSN_2LLH`; the Japanese arm invented settings
    upstream doesn't have. `stressOpt <bit…>` is now read from the voice file,
    which is how Finnish, Estonian, Latin and Ligurian configure their stress
    entirely — Finnish stopped putting a secondary stress on its final syllable.
  - **Regressive voicing assimilation** (`SetRegressiveVoicing`) is ported, so a
    consonant cluster takes the voicing of its last member: Ukrainian `світ` is
    `zB'i:t[`, and with the word-boundary bit unset the assimilation crosses
    words (`привіт світ` → `pRIB'i:d[ zB'i:t[`).
  - **Russian stressed vowels.** The code path resolved a *stressed* vowel from
    stress alone, upgrading `е` to `E` everywhere; the allophone depends on the
    neighbours (`n;'et` after a soft consonant but `'EtV` word-initially), which
    the context-aware pass already gets right.
  - **Mandarin third-tone sandhi joins its pair.** 你好 reads as `ni35X'Au214_|`
    — one inter-word gap, no stress on the first syllable — where the port
    emitted two separate words.

- **The speech-rate model.** `-s` mapped linearly (`175 / wpm`), so slow rates
  over-stretched badly: `-s 80` ran 2.4× upstream's duration and `-s 300` 0.55×.
  Upstream's `speed_lookup` table is now ported, with a fitted affine model for
  the share of an utterance that doesn't follow it (recorded consonants, fixed
  pauses). Measured against the C binary: 80 wpm 1.07×, 120 wpm 1.03×, 175 wpm
  1.05×, 250 wpm 1.15× — and at the default rate the port is now within 5 % on
  a five-sentence sample instead of ~14 % long. The old `RATE_CALIBRATION`
  constant and wavegen's second, separate application of the rate are both gone.
  Inline `\x01<n>S` and scoped `<prosody rate>` use the same curve.

- **A voice's `pitch` line was ignored, and `formant_factor` was hardcoded**
  (upstream #2524). Every voice used a literal `pitch 82 118` for its base F0
  and never shifted its formants, so `lang/gmw/af`'s `pitch 63 120` and the
  MBROLA voices' own pitch lines did nothing. The voice file is now read, and
  the reference for `formant_factor` is *derived* from C's `VoiceReset`
  (`pitch_base = 0x47000` → 80) rather than repeated as a literal — which is
  #2524's point: C's own `(pitch1 - 82)/82` no longer matches its default.
- **A vowel's pitch was flat across the syllable.** C advances `wdata.pitch`
  every `STEPSIZE` samples, so its contour glides (97→117→105→95→85 Hz over a
  sentence); the port held one value per syllable and produced plateaus. Vowels
  now glide through their syllable's envelope, and the following consonants
  carry the syllable's *end* pitch. Eight-sentence spectral sweep: 2.52 → 2.47 dB
  mean envelope difference, correlation 0.961 → 0.963.
- **A regional voice with no phoneme table of its own failed to speak.**
  `EspeakNg::with_data_dir("en-GB-scotland", …).synth(…)` returned
  `VoiceNotFound`: the audio path used `select_table_by_name`, which has no
  locale fallback, where the phoneme paths use `select_phoneme_table`.
- **`AsyncSynth::cancel` could deliver a whole further clause.** The cancel flag
  was only checked between `CHUNK_SAMPLES` pieces, so a clause already rendered
  went out anyway. It is now checked before any chunk is handed to the callback
  — the guarantee upstream #2288 makes for the audio device.
- **MBROLA `.pho` used the default male pitch for every database.** `mb-de1`
  declares `pitch 140 220` and `mb-la1` `pitch 82 117`; both got the same
  targets. The voice file's own pitch now drives them (upstream #2501/#2516 are
  about MBROLA output quality, and this is the first thing that makes it wrong).

- **Accented letters never matched a letter group.** C's `IsLetter` folds a
  letter through `remove_accent` before testing group membership, so `í` counts
  as a vowel because `i` does. The port indexed `letter_bits` by the raw code
  point, so every `A`/`C`/`Y`-style rule failed on an accented letter. Spanish
  `día` came out as `d` — the word was truncated after its first letter — and
  French `s` between vowels didn't voice when the second vowel was accented
  (`divisé`). The 414-entry table is now transcribed from `dictionary.c`.
- **`-x` printed raw `phonSWITCH` markers.** `-v ru "привет hello"` produced
  `p@-r;iv;'Et _^_Qts` instead of `p@-r;iv;'et (en)h@l'oU(ru)`: the formatter
  still used the old `"SW"`-mnemonic check. `translate_to_codes` now resolves a
  switch into the target language's codes wrapped in markers, and `format_x`
  loads that language's phoneme table to print them. Consecutive switched words
  share one group, as upstream does.
- **Post-passes rewrote switched-language codes with the wrong table.**
  `resolve_virtual_phonemes_in_codes` and friends ran over the whole clause, so
  Arabic `H` inside an English clause printed as `d` (`(ar)mRdb'a:` for
  `(ar)mRHb'a:`). They now skip `phonSWITCH` regions. Arabic, Hindi and Korean
  switched output is byte-identical to upstream again.
- **Conditional dictionary entries were unconditional.** `word_to_phonemes`
  passed `lookup_symbol: true` for ordinary words, which disables the `$atend`
  gate, and never set the capitalisation flags. English "a" therefore read as
  the letter name `ˈeɪ` everywhere instead of reducing to `ɐ` in running text,
  "to" stayed `tuː`, and "I" kept primary stress (its entry is gated on
  `$allcaps`). Clause position and capitalisation are now passed through.
- **A dictionary replacement was cut to its first word in a spaceless script.**
  The textmode expansion split on whitespace; Thai and Chinese replacement text
  has none, so `th 👍` read two syllables of a four-word phrase. It is now
  tokenized like ordinary input — `th 👎` is byte-identical to upstream.
- **A rules line with a post-context but no phoneme column compiled wrong.**
  `c (q` means "c is silent before q"; the compiler took the last column as the
  phonemes and made it "c is pronounced /q/", so a self-compiled English
  dictionary mispronounced "cat".
- **`klattp2` was indexed one slot short** — it continues `klattp` from index 5
  (`Kopen, AVp, Fric, FricBP, Turb`), so frication amplitude is index 2; the
  voiced/noise decision was reading `AVp`.
- **A clause break left stray spaces at the ends of `-x` lines** (`'eI \nb'i:`).

- **Context-dependent phonemes now follow each language's own definition.** `d#`,
  `t#`, `z#` and friends are defined per phoneme table as conditions plus
  `ChangePhoneme` instructions; the port hardcoded English's
  `prevPh(isVoiced)` rule for every language. With the refreshed Portuguese
  rules that made `dente` → `tˈẽŋty` instead of `dˈẽntɨ`. The phoneme's actual
  program is now interpreted (`interpret_phoneme_ctl` in "PhonemeList mode",
  where the first `ChangePhoneme` wins and ends interpretation, as in
  `synthdata.c`), and the resolution is applied on the **codes** path too, so
  `-x` and the synthesizer see the resolved phoneme like upstream: `-x -v pt-br
  noite` → `n'oItSy`, `-x -v de zwanzig` → `tsv'antsIC`.
- **Voicing assimilation across the word boundary.** Those conditions may look at
  the *next word* (`nextPh`, unlike `nextPhW`): Portuguese coda `s#` gives
  `os amigos` → `uz ɐmˈiɡʊʃ` but `três` → `tɾˈeʃ`. The renderer had no
  cross-token context, so the token-final phoneme always took the voiceless
  branch.
- **A voice's `phonemes <name>` directive now outranks its language tag** when
  selecting the phoneme table. The table literally named `pt` is the *Brazilian*
  one, and `pt` (Portuguese (Portugal)) declares `phonemes pt-pt` — so European
  Portuguese was reading with the Brazilian table.
- **`-x` accepts regional locales.** The mnemonic path selected the table by bare
  tag, so `-x -v pt-br` failed with "phoneme table 'pt-br' not found" after the
  same voice worked in `--ipa`.
- **Sinhala ZWJ conjuncts** are one cluster again: `ක්‍ර` → `kɹˈɐ`, not `k ɹˈə`.
  A joiner between letters is part of the word (upstream fixed the same bug).
- **Russian decimal fractions** are read as fractions: `0,25` → "ноль и двадцать
  пять сотых", not "ноль и два пять". Feminine numerators and the
  singular/plural scale word follow the language's own `_0Z<n>` / `_0Z<n>s` and
  `_1f`/`_2f` entries, so Hungarian gets its "tized" too (upstream
  `NUM_DFRACTION_5` / `NUM2_FRACTION_FEMININE`).
- **Composed number entries take end-stress promotion on their last word**, so
  Portuguese `101` reads exactly like `cento e um` (`sˈẽŋtʊ i ˈũŋ`), not with an
  unstressed final word.
- **Chinese tone digits stay in the word** (`你` → `ni3`), instead of splitting
  into a syllable plus the number three.
- **Word boundaries inside `[[…]]`**: `[[h@l'oU w'3:ld]]` ran together as one
  word in `-x` and in the audio.
- **Stress flags now come from upstream's table** for every language the port
  doesn't configure by hand: they were set in only seven arms, so the rest ran
  with none. Basque needs `S_FINAL_VOWEL_UNSTRESSED` to move the stress off a
  word-final vowel — `kaixo` is now `kˈaɪʃo` and `etxe` `ˈetʃe`, both
  byte-identical to upstream. The port's own tuned values still win where it has
  them, since those compensate for differences in its stress fill.
- **An explicit stress mark now wins over the placement rule**, as in C, where
  `GetVowelStress` reports it for every language and the rule is then skipped.
  The port only honoured that for Lojban-style languages, so Crimean Tatar — a
  language with no entry in C's table at all — stressed *both* syllables
  (`s;'el;'Am`); it now matches upstream byte-for-byte on `selâm`, `merhaba` and
  `kitap`. Composed number words are exempt, so `pt "101"` still stresses its
  last word.
- **The default stress rule matches C**: penultimate (`STRESSPOSN_2R`) with
  `unstressed_wd2 = 3`, not first-syllable with 2. That default applies to every
  language the port doesn't configure explicitly.
- **`stressRule` is read from the voice file** (`stressRule <rule> [wd1] [wd2]`),
  which is how ~15 languages are configured upstream — Basque, Cherokee, Latin,
  Quechua, Finnish, Estonian, Arabic and others — instead of in C's language
  table.
- **Every Vietnamese syllable carries a tone.** `CalcPitches_Tone` gives a
  toneless syllable the default tone (`phonDEFAULTTONE`) and turns the
  clause-final one into the falling tone 7, so `xin` is `s'i1n` and `xin` alone
  `s'i7n`. Vietnamese and Mongolian also got their `langopts.word_gap` (`_|`
  between words), which Chinese already had: `vi "xin chào"`, `"tôi"` and
  `"Việt Nam"` are now byte-identical to upstream.
- **Afrikaans, Georgian and Kazakh had the wrong decimal separator** — they read
  "3,14" as two numbers and use the period, as upstream does. `af "3.14"` is now
  byte-identical.
- **A period ends a sentence only before a capital.** `one. two` is one clause
  upstream (the period is silent), `one. Two` is two; `?` and `!` always break.
  The port broke on every period, so `-x` printed an extra line for "etc. and",
  "5. item" and any decimal-looking text. A comma directly between two numbers is
  likewise a separator, not a clause break (`en "3,14"`).
- **Thai tones land on their syllable.** Tones are attached one per vowel, the
  way C keeps them as a field on the vowel: a tone written after the syllable is
  pulled back over the coda (Mandarin `w'u@35n`), a tone written *ahead* of it
  attaches to the following vowel (Thai `kh,a5?…`), and a syllable that already
  has a tone drops the extra. Thai used to render every emitted tone literally —
  `สวัสดี` was `s55ˈa2wms55aɜds`, and is now `sˈa5wmsaɜds`, byte-identical to
  upstream, as are `ขอบคุณ` and `ไทย`; Mandarin and Cantonese are unchanged.
- **A `/` in a phoneme mnemonic is a variant indicator and is not printed**
  (C `WritePhMnemonic`: "discard phoneme variant indicator"). Spanish's
  unstressed `a/` shows as `a`, the dark-l `l/2` as `l`. The port printed the
  whole mnemonic, which is what made `-x` look wrong for Spanish, Italian and
  others — the *phonemes* were right all along. Consonant `ChangePhoneme`s are
  applied again as a result, so the synthesizer gets the dark l.
- **French liaison phonemes that don't surface are dropped from the phoneme
  codes**, not just from the IPA: `-x -v fr "3,14"` printed `trwaz2` where
  upstream has `trwa`, while "deux enfants" correctly keeps its /z/.
- **The default tone is no longer mistaken for a pause.** Code 17 is a
  miscellaneous control code in most tables but `phonDEFAULTTONE` in a tone
  language, so Vietnamese lost its tone in IPA output (`sˈin` for `sˈi1n`).
- **The vowel-initial break used the wrong pause phonemes.** `_|` and `_!` were
  swapped, and the decision read the first vowel's stress *before* the placement
  rule instead of after. German now matches upstream byte-for-byte on `Ich`,
  `achten`, `einen`, `Achtung`, `beachten` and the sentence "Ich habe einen
  Hund", as do Hungarian and Dutch.
- **A `length` instruction inside a phoneme program now overrides the declared
  `std_length`** (and `LengthAdd` adjusts it). German raises /a/ from 110 to 125
  before a rhotic this way, and uses the same pattern on most of its vowels; the
  instruction was parsed and discarded.
- **`PauseBefore` / `PauseAfter` are honoured on every render path.** German
  inserts 15–30 ms of silence before /r/ after a stop ("trotz", "trinken" → 30 ms;
  "Preis" → 15 ms; word-initial "Rose" → none). /r/ is a liquid, so it is rendered
  through the formant path, which now ends its run at the pause and emits the
  silence — the WAV path handles it inline.
- **The phoneme programs' list-editing instructions now run** — `InsertPhoneme`,
  `AppendPhoneme`, `IfNextVowelAppend` and `ChangeNextPhoneme` were parsed and
  discarded. They are what gives English its **linking r**: "sofa area" is
  `sˈəʊfəɹ ˈeəɹiə` (was `sˈəʊfə ˈeəɹiə`) and "father and son" matches upstream
  exactly. The narrower mnemonic-inspecting approximations they replace (a
  `has_rhotic_liaison` check and a special case for the word "or") are gone.
- **Context-dependent vowels resolve through their own program.** Resolution used
  to be limited to the `#`-suffixed "virtual" class; English `@2` — the schwa used
  only for "the" — is `IF nextPh(isVowel) THEN I2 ELSE @`, so "the apple" is now
  `ðɪ ˈapəl` and "the cat" `ðə kˈat`, as upstream has them. `ChangePhoneme(NULL)`
  now **deletes** the phoneme (C: "NULL phoneme, discard") instead of printing its
  mnemonic, which had been putting a stray `_` inside Russian words
  (`pʲˈɑ_tʲ` → `pʲˈɑtʲ`).
- **`isTranslationGiven`** implemented: Turkish, Bashkir and Tatar suppress vowel
  reduction when the pronunciation came from the dictionary or a number key, so
  `tr 100` is `jˈyz` while the typed word `yüz` is `jˈøz` — both matching upstream.
- **Vowel-initial words get their break back** (`langopts.vowel_pause`). German,
  Dutch and Afrikaans (0x30), Hungarian (0x20) and Lojban (0x20c) put a `_|` /
  `_!` in front of a word — or a prefix-stripped stem — that starts with a
  vowel, so the preceding consonant doesn't link into it. The option was parsed
  but never applied; `beachten` is now `b@_!'axt@n`, byte-identical to upstream.
- **Short decimal fractions are read as one number** (upstream `NUM_DFRACTION_2`
  / `NUM_DFRACTION_4`, ~25 languages): Czech `0,25` is "nula čárka **dvacet
  pět**", French `3,14` is "trois virgule **quatorze**", Portuguese "três vírgula
  **catorze**" — not digit by digit. Leading zeros stay individual and a fraction
  longer than the language's limit (two digits, or five for the `_4` set) falls
  back, exactly as C does. `cs 0,25` and `fr 3,14` are now byte-identical to
  upstream.
- **A run of foreign-script words is one switch group.** The port emitted
  `(cmn)w1(ar) (cmn)w2(ar)`; upstream wraps the whole run once, `(cmn)w1 w2(ar)`.
- **German verbs with a standard prefix were unreadable.** The rules scan is
  supposed to stop at a standard prefix/suffix and let the caller re-translate
  the stem; the port kept scanning, so a later suffix overwrote `end_type` while
  `end_phonemes` still held the *prefix's* phonemes. Every `be-`/`ge-` verb with
  an inflectional ending collapsed to the same five phonemes — "beachten",
  "bezahlen", "bewegen" all read `b'@b@`. Now `bezahlen` is `bətsˈɑːlən`,
  byte-identical to upstream.
- **Stress-conditioned allophones reach the phoneme codes.** `ChangeIfDiminished`
  and friends were applied when rendering IPA but not on the codes path, so `-x`
  and the **synthesizer** used the citation vowel: Russian "молоко" was
  `molok'o` where upstream (and our own `--ipa`) say `mVlVk'o`.
- **Event positions are rescaled after tempo/pitch post-processing**, and
  `synth_with_events` now applies that post-pass at all — it previously returned
  audio that differed from `synth()` for the same text (upstream #2470).

- **Every English voice spoke en-GB IPA, including `en-us`.** English voices
  share `en_dict` but not a phoneme table — `en` is en-GB, `en-us` is its own
  table — yet the curated `EN_IPA_OVERRIDES` (derived from
  `espeak-ng -v en -q --ipa`) took precedence over the *active* table's own
  `i_IPA_NAME` phondata. So `en-us` rendered "hello" as `həlˈəʊ` instead of
  `həlˈoʊ`, "over" non-rhotically as `ˈəʊvə` instead of `ˈoʊvɚ`, and "not" with
  RP `ɒ` instead of `ɑː`. Since 0.1.3 correctly applies `dictrules`, the two
  accents diverged on dictionary lookups while still rendering identical IPA,
  which is how the phoneme table — not the rules — was identified as the cause.

  Two changes: the active table's `i_IPA_NAME` is now authoritative, with the
  overrides demoted to a fallback; and the two entries that encode en-GB's own
  GOAT vowel (codes 144/145, `oU`/`oU#` → `əʊ`) moved to a new
  `EN_GB_IPA_OVERRIDES` that applies only when the `en` table is selected, so
  `en-us` falls through to the ipa1 value `oʊ`. The other overrides are
  accent-neutral (syllabic `m-` → `m̩` and friends) and still apply to every
  English table.

  **en-GB output is unchanged**: for every override code, the `en` table's
  phondata is either identical to the override or absent, never different, so
  the reordering is a no-op there. New `PhonemeData::active_table_name()`;
  `phoneme_ipa`/`phoneme_ipa_lang` keep resolving as en-GB, since they have no
  table context. Regression tests in `tests/en_us_phoneme_table.rs`.

## [0.1.3] - 2026-08-01

This release adds new public modules and engine API surface (all
backward-compatible additions), a large expansion of language, script, and
synthesis coverage, and an extensive round of number- and text-normalization
fixes verified against the upstream espeak-ng C binary — including full Slavic
scale-word declension (ru/cs/pl/lt/sk/hr), many per-language cardinal grammars,
and punctuation/symbol handling (dates, times, dashes, dotted acronyms and
versions).

### Added

#### Public API & engine

- **New public modules:** `soundicon`, `voices`, `async_synth`, `mbrola`, and
  (behind the `wav-analysis` feature) `analysis`.
- **Engine builder & voice selection:** `Builder`, `VoiceSpec` /
  `VoiceSpecBuilder`, and `Gender`, alongside the existing `EspeakNg` session
  type.
- **Synthesis events:** `SynthEvent` / `EventKind` (including word-boundary
  events) and `OutputMode` for streaming/callback-style synthesis.
- **Phoneme options:** `TextToPhonemesOptions` for configuring the
  `text_to_phonemes` path (separator, tie characters, IPA vs. mnemonic, …).
- **Sound icons:** `SoundIcon` / `SoundIconTable` and inline `<name>` markup
  support.
- **Voice files:** `voices` parses eSpeak voice definitions, including language
  priorities, aliases, and the `dictrules` directive.
- **Async synthesis:** `async_synth` provides a non-blocking synthesis entry
  point.
- **MBROLA:** `mbrola` module for MBROLA-backed voices.
- **WAV analysis** (`wav-analysis` feature): WAV I/O plus duration,
  energy-envelope, and pitch-contour extraction, with an `analysis::compare`
  helper for validating synthesized audio against a reference WAV without
  requiring bit-exact PCM.

#### Synthesis

- **Klatt formant synthesizer** (`synthesize::klatt`).
- **Tempo / prosody control** (`synthesize::tempo`) for speed- and
  duration-scaled output.
- Expanded harmonic synthesizer, bytecode interpreter, wave generation, and
  sampling paths (formant sequences, phoneme programs, post-effects).

#### Text, numbers & markup

- **SSML** (`translate::ssml`): break, prosody, say-as, phoneme, sub, `mark`,
  and related elements. `<mark name="…"/>` is parsed with its text position
  (`process_markup_with_marks`) and fires an `EventKind::Mark` during
  `synth_with_events`.
- **Inline embedded commands** (`\x01[±]<value><letter>`, C `CTRL_EMBEDDED`):
  parsed and stripped so they no longer garble the spoken text
  (`parse_embedded_commands`), with the pitch/speed/volume (`P`/`S`/`A`) levels
  applied to the utterance.
- **Emoji reading:** base emoji / pictographs are spoken via the dictionary's
  emoji-name entries (with variation-selector, skin-tone, and ZWJ handling).
- **Leading-zero numbers:** `007` → "zero zero seven"; long forms
  (`0800`) are read digit-by-digit; decimals and lone `0` unaffected.
- **CamelCase / mixed-case word splitting:** `CamelCase` → "Camel Case",
  `iPhone` → "i Phone", including the upper→lower boundary
  (`HTMLParser` → "HTML Parser"), with Irish (`ga`) and Dutch-IJ exceptions.
- **Lojban capital-stress notation** (`LOPT_CAPS_IN_WORD`): a mid-word capital
  marks primary stress instead of splitting the word.
- **Hyphen between letters is a word break:** `well-known` → "well known",
  `X-ray` → "eks ray".
- **Per-mille symbols:** `‰` → "per thousand", `‱` → "per ten thousand".
- **Part-of-speech homograph disambiguation:** verb / noun / past forms selected
  from context (`to live` → "lɪv", `have read` → "rɛd", `a convert` → noun).
- Roman-numeral reading and additional number-grammar coverage.

#### Languages & scripts

- **Korean (ko):** Hangul syllable decomposition into conjoining jamo; reads
  `안녕하세요` → "annyeonghaseyo".
- **Sinhala (si):** now reads (abugida letter-groups + virama handling).
- **Urdu (ur) & Sindhi (sd):** now read (Arabic-script rule indexing).
- **Bishnupriya (bpy):** now reads (Bengali-script offset).
- **All ISCII Indic scripts:** Bengali/Assamese, Gurmukhi, Gujarati, Oriya,
  Tamil, Telugu, Kannada, Malayalam, and Devanagari (Hindi/Marathi/Nepali/…)
  read with correct conjuncts, plus Indo-Aryan word-final schwa deletion.
- **Japanese (ja):** katakana support and voiced/semi-voiced kana
  (が/だ/ば/ぱ …) via the general `.replace` mechanism.
- **Regional locale fallback:** locales that ship only a base phoneme table
  (`pt-br`, `fr-ca`, `en-gb`, `es-mx`, …) now resolve their base table instead
  of erroring "phoneme table not found" (upstream #2248).
- **Brazilian Portuguese & Latin-American Spanish** read their regional variants
  via the voice `dictrules` directive (pt-BR ti/di → tʃi/dʒi palatalisation;
  es-419 yeísmo).

#### Dictionary tooling

- **Dictionary-source compiler** (`dictionary::compile`, CLI `--compile` /
  `--compile-phonemes`): compiles real eSpeak `<lang>_rules`, `_list`, and
  `.replace` sources into the binary dictionary format (verified byte-faithful
  on Esperanto and Bosnian), enabling upstream language-data fixes to be applied
  through the port.
- **`.replace` section application:** the rules engine now applies each
  dictionary's `.replace` character-substitution table (59 shipping languages),
  covering typographic ligatures (`ﬁsh` → "fish") and per-language normalisation.

#### CLI & tooling

- New/extended flags: `--markup`, `--voices`, `--compile`, `--compile-phonemes`,
  `--path`, `--phonout`, `--sep`, `--split`, `--punct`, `--stdin`, `--stdout`,
  `-z` (no final pause), and `-k 1` (a brief capital *sound* before capitalised
  words, alongside the existing `-k 2` "capital" announcement).
- New `spectral` binary for spectral inspection of synthesized output.

### Changed

- Regional voices now resolve their phoneme table via the primary BCP-47 subtag
  rather than the full locale tag.
- Japanese katakana and voiced-kana handling now route through the general
  rules-engine `.replace` mechanism instead of language-specific code.

### Fixed

- **Default speaking rate calibrated to eSpeak NG.** Synthesized speech ran
  ~1.2× too long at the default rate because the length pipeline omits eSpeak
  NG's `SetSpeed`/`CalcLengths` `len_speeds` scaling. A calibration factor
  (measured across a diverse corpus against the built C reference) now matches
  the overall pace — geometric-mean duration ratio 1.20 → ~1.05. (Byte-exact
  per-phoneme durations still need the full `CalcLengths` port.)
- **Consonant noise-sample synthesis (fricatives/stops).** The WAV noise samples
  (`s`, `ʃ`, `f`, stop bursts) were *linearly resampled* to change length, which
  stretched and aliased the noise and corrupted its spectral texture. They are
  now played with eSpeak NG's `DoSample2` algorithm — the sample is played once
  and its steady-state middle is **looped** to sustain, then truncated/tailed —
  preserving the natural noise spectrum. Verified against the C oracle: the
  fricative spectral flatness/centroid/roll-off moved substantially closer to
  eSpeak NG's output.

- **Crash safety:** `is_letter` / `is_letter_wc` no longer panics (u8 shift
  overflow) on a malformed or misread rule byte — a bad dictionary now degrades
  gracefully instead of crashing.
- **Chinese (cmn)** no longer returns empty IPA (produces audio output).
- **Large numbers (≥ 10¹²):** the scale split stopped at 10⁹, so
  `1000000000000` read "ten hundred billion"; it now reads "one trillion" and
  covers every scale word a language defines (en/de/ru/it up to 10¹⁸ — e.g.
  German long-scale "eine Billion" at 10¹², "eine Trillion" at 10¹⁸). A language
  that stops earlier (French past "milliard") folds the excess into its highest
  scale instead of dropping it.
- **French clitic schwa:** short French words ending in the reducible "e caduc"
  schwa (`le`, `la`, `du`, `je`, …) rendered a stray trailing dash (`le` →
  "lˈə-"); the dash (a vowel-mnemonic variant marker, not IPA) is now dropped
  (`le` → "lˈə").
- **Romance abbreviated ordinals with a dot:** `1.º` / `2.ª` / `3.º` (the
  RAE-recommended Spanish form; also pt/it/fr) read the cardinal ("tres") because
  the dot broke ordinal detection; they now read the ordinal ("tercero"), the
  same as the un-dotted `3º`. A bare `3.` and German's dot-ordinal (`3.` →
  "dritte") are unaffected.
- **Numero sign in Cyrillic:** `№` was dropped in Russian/Ukrainian/Bulgarian
  (`№5` → "five"); it now reads "номер" (`№5` → "номер пять"). English already
  read "number".
- **Multiply/divide/plus-minus signs in more languages:** `×`, `÷`, and `±` were
  dropped (silent) in most non-English languages; they now read the standard
  operator word in pt, pl, cs, ro, sv, da, nb, tr, ru, uk, bg (e.g. Turkish
  `2×3` → "iki çarpı üç", `10÷2` → "on bölü iki"; `±5` → "plus minus"). Hebrew
  (`he`) gains `+ − × ÷ °` (niqqud-pointed so espeak-Hebrew can pronounce them),
  and Ukrainian gains `°` ("градусів").
- **Degree sign in more languages:** `°` was dropped in sv/da/nb/ru/bg; it now
  reads "degrees" (`sv 20°` → "tjugo grader", `ru -40°` → "minus sorok
  gradusov").
- **Title-abbreviation periods (#2340):** a period after a title abbreviation
  no longer inserts a sentence pause — `Dr. Smith` / `St. Louis` read without a
  break, while a real sentence end (`the end. Next.`) still breaks.
- **German "einundzwanzig":** compound numbers `21`–`91` read "eins und…"; the
  unit 1 apocopates to "ein" before "und", so they now read "einundzwanzig"
  (while standalone `1` and final-position `101` → "hunderteins" keep "eins").
- **English "-ire" words:** `fire`/`tire`/`wire`/`hire`/`choir`/`desire` (the
  "aI3" rhotic diphthong) had no IPA override, so the mnemonic fallback dropped
  the offglide and read "faɪ" (like "fie"); they now read "faɪə". The 🔥 emoji
  (whose name is "fire") is fixed too.
- **Liaison suppression scoped to French:** the word-final "drop a 2/3-suffixed
  phoneme before a non-vowel" rule (French liaison) was applied to every
  language, so Hungarian numbers lost their final consonant (`hu 5` → "ö" instead
  of "öt", `6`/`7`/`15`/`25` likewise). It's now French-only; French liaison
  (`les amis` → "lez ami", `les chats` → "le cha") is unchanged.
- **French liaison inside composed number tokens:** hundreds/thousands leaked an
  erroneous liaison consonant (`200` → "dø**z**sɑ̃", `300` → "tʁwa**z**sɑ̃",
  `2000` → "dø**z** mil"). Word-final liaison only worked across *separate*
  tokens, but a number is one token whose parts are composed with no boundary
  between them. The renderer now resolves a French liaison phoneme by the
  following sound — scanning past internal word separators — keeping it before a
  vowel (`21` → "vɛ̃t‿e‿œ̃", `deux enfants` → "døz ɑ̃fɑ̃") and dropping it
  before a consonant (`200` → "døsɑ̃", `2000` → "dø mil"), matching espeak-ng.
- **French "six"/"dix"/"huit" citation form before a pause:** these carry a
  `3`-liaison (`ph_french` z3/t3) with three realizations — [z]/[t] before a
  vowel ("six ans" → "siz ɑ̃"), silent before a consonant ("six chats" → "si
  ʃa"), and a citation [s]/[t] before a pause. The last was dropped, so isolated
  "six"/`6` read "si" instead of "sis" (and `26` → "vɛ̃tsi", not "vɛ̃tsis"). The
  liaison resolver now distinguishes the `2`/`3` level and whether a pause or a
  word follows, emitting the citation consonant before a pause.
- **Single-letter acronym periods no longer split the sentence:** every `.` was
  treated as a clause boundary, so "U.S.A." became three clauses ("juː" / "ɛs" /
  "eɪ") and "e.g." / "i.e." broke mid-word. A `.` tightly between single letters
  is now a silent acronym separator (matching espeak-ng), while sentence-ending
  periods (`cat. Dog`, `end.`), decimals (`3.14`), and list markers (`A. First`)
  are unchanged.
- **Period between words/digits reads as "dot"/"point"/"punkt":** a `.`
  immediately followed by a letter or digit is the `.` symbol, not a sentence
  end — "cat.Dog" → "cat dot dog", "5.and" → "five dot and", and in comma-decimal
  locales German "3.14" → "drei punkt vierzehn". English "3.14" (where `.` is the
  decimal separator) still reads "three point one four".
- **Times ("10:30") are read as times, not split at the colon:** `H:MM[:SS]`
  (each field exactly two digits) now reads as consecutive numbers with the colon
  silent — "10:30" → "ten thirty" (was two clauses "tˈɛn" / "θˈɜːti"). The hour
  drops a leading zero ("02:30" → "two thirty"), the minute keeps it ("10:05" →
  "ten zero five"), and German/French/Italian insert their connector ("uhr" /
  "heures" / "e") for a valid hour 0–23 ("10:30" → "zehn uhr dreißig").
- **Numero sign `№` matches espeak-ng per language:** English now reads "numero"
  (not "number"); German/Italian read "numero", Portuguese "número", Dutch
  "nummer", Czech "číslo", Russian/Bulgarian "номер"; French, Spanish, Ukrainian,
  Polish and others drop the sign (the bare number is spoken), as in espeak-ng.
- **Hyphen between numbers reads as "dash"/"Strich":** an ASCII `-` directly
  between two numbers ("5-10", "pages 3-7", "3-4-5") is read as the "-" symbol —
  English "dash", German "Strich" — instead of being dropped (which produced an
  ambiguous "five ten"). Only between digits (so "x-5" is unaffected), only the
  ASCII hyphen (en/em dashes stay silent), and only for languages espeak-ng
  voices it in (others drop it, as before).
- **Compound-number joining words for more languages:** Romanian now says
  "douăzeci și unu" (21), Icelandic "þrjátíu og fjórir" (34), and Albanian joins
  with "e" (both "njëzet e një" and "njëqind e …"); Welsh and Armenian no longer
  prefix "one" before "hundred" ("cant" / "harjur", not "un cant" / "mek
  harjur"). Round tens and multiplied hundreds (200, 300) are unchanged.
- **More compound-number fixes:** Kazakh "100" is "жүз" (not "бір жүз", while
  "1000" stays "бір мың"); Azerbaijani "100"/"1000" are "yüz"/"min" (dropping
  "bir"); and Afrikaans is now read units-first like Dutch/German ("een-en-
  twintig", "vier-en-dertig"), while keeping "een honderd"/"een duisend".
- **Amharic & Swahili numbers:** Amharic "100" is "መቶ" (not "አንድ መቶ"; "1000"
  keeps "አንድ ሺ"); Swahili now joins tens and units with "na" ("ishirini na moja",
  "thelethini na nne").
- **Conditional hundreds-conjunction (Afrikaans/Swahili):** the joining word is
  now placed before a single-word remainder ("honderd **en** een/twintig", "mia
  moja **na** ishirini") but not before a compound that already has its own
  ("honderd vier-en-dertig", "mia moja thelethini na nne"). Swahili teens are
  also joined ("kumi **na** tano" = 15).
- **Standalone vs. combining hundred (Tamil/Kannada/Telugu):** a round hundred now
  uses the language's standalone form (Tamil "100" → "நூறு" nūru) while a hundred
  before a remainder keeps the combining form ("101" → "நூற்று ஒன்று" nūṭṭri
  ondru); previously the combining form was used for both.
- **Malayalam "100":** now the bare "നൂറ്" (nūṟṟu) instead of "ഒരു നൂറ്" (oru …) —
  the "one" is dropped and the bare hundred word is used, matching espeak-ng;
  "200"+ keep their dedicated words.
- **Exact-thousand form (Tamil/Malayalam/Sinhala + kn/te/fa/ka):** a round N000
  now uses the language's exact scale word (Tamil "1000" → "ஆயிரம்" āyiram) while
  a thousand before more digits keeps the combining form ("1001" → "ஆயிரத்தி
  ஒன்று" āyiratti onru); Tamil/Malayalam/Sinhala also drop the "one" before
  "thousand" ("ஆயிரம்", not "ஒன்று ஆயிரம்").
- **Russian scale-word declension:** "тысяча"/"миллион" now decline by count and
  agree in gender — "одна тысяча" (1000), "две тысячи" (2000), "пять тысяч"
  (5000), "один миллион", "два миллиона", and compounds keep gender ("двадцать
  одна тысяча"); previously the "one" was dropped and only the genitive-plural
  form was used ("тысяч", "два тысяч").
- **Czech & Polish scale-word declension:** the scale word now takes the 2–4 case
  form — Czech "dva tisíce" / "dva milióny", Polish "dwa tysiące" / "dwa miliony"
  — while 5+ keeps the genitive plural ("pět tisíc", "pięć tysięcy"); previously
  the genitive-plural form was used for all counts.
- **Lithuanian scale-word declension:** "tūkstantis" now inflects by count —
  "tūkstantis" (1, 21), "tūkstančiai" (2–9), "tūkstančių" (teens and multiples of
  ten: 10 000, 20 000, 100 000); previously "tūkstančiai" was used throughout.
- **Slovak scale-word declension:** millions take the 2–4 form ("dva milióny",
  was "dva miliónou"), and "thousand" is feminine for a bare two ("dve tisíc")
  while a compound stays masculine ("dvadsať dva tisíc").
- **Croatian scale-word declension:** "tisuću" (1, 21), "tisuće" (2–4), "tisuća"
  (5+, teens), with a feminine "thousand" ("dvije tisuće", "dvadeset dvije
  tisuće"); previously one form was used for all counts. This completes Slavic
  thousands declension across all five espeak variants (ru, cs/pl, lt, sk, hr).
- **Non-time colon no longer splits the sentence:** a `:` between numbers that
  isn't a valid time (`3:2`, `10:3`) is read as "colon" in English ("three colon
  two") and dropped silently in other languages, instead of breaking the clause.
  Times (`10:30`) and prose colons (`Note: this`) are unchanged.
- **Dotted version numbers:** "1.2.3" now reads "one point two point three"
  instead of "one point two **zero** point three" — a `.` right after a number is
  a continued decimal point, not a fresh "zero point …". Standalone ".5"/"0.5"
  still read "zero point five".
- **Italian compound elision:** `31` read "trenta uno"; the tens word now elides
  its final vowel before a vowel-initial unit, so it reads "trentuno" (and
  `48` → "quarantotto", `88` → "ottantotto"). The 20s already elided.
- **Danish/Faroese/Slovenian units-first cardinals:** `da 21` read the wrong
  order "tyve en" (twenty-one); these languages compose units-first, so it now
  reads "enogtyve" (Slovenian `21` → "enaindvajset"), and their compound
  ordinals work too (`da 21.` → "enogtyvende"). Finnish/Estonian/Norwegian stay
  tens-first.
- **German & Dutch compound ordinals:** `21.` read the wrong two-word "zwanzig
  erste"; these languages form an ordinal by suffixing the whole cardinal, so
  German now reads "einundzwanzigste" (`der 21. Mai` → "der einundzwanzigste
  Mai") and Dutch "eenentwintigste" (`21e`). English's "twenty-first" form is
  unaffected.
- **Vietnamese "ten":** `vi 10` was silent; now reads "mười".
- **Leading minus sign:** `-5` was read "five" (the sign was dropped); it now
  reads "minus five" via the per-language symbol word, while a hyphen after a
  word or number (`COVID-19`, `5-10`, dates) stays a separator. The `+` / `−`
  symbol readings were extended to more languages (pl, pt, ru, uk, bg, cs, ro,
  sv, da, nb, tr — e.g. Turkish `-5` → "eksi beş", `+5` → "artı beş"), so the
  sign is no longer dropped there. A minus before a currency amount is also read
  now (`-$5` → "minus five dollars", `-€10` → "minus ten euros"), while a hyphen
  after a word or number (`US-$5`, `5-$3`) stays a separator.
- **Ordinal-suffix validation:** `2st` no longer renders the mangled "secst";
  only correct suffixes (`1st`, `2nd`, `3rd`, `11th`, …) parse as ordinals.
- **Hex literals:** `0x1F` no longer reads "zero by one F" (dimension guard now
  requires non-zero operands); real dimensions (`8x10` → "eight by ten")
  unchanged.
- **`.replace` table alignment:** corrected an off-by-one field-decode caused by
  rounding the table offset up instead of down.
- **Bosnian (#2269 demo):** the dictionary compiler now resolves the phoneme
  table via the voice's `phonemes` directive (bs → hr), fixing mis-compiled
  mnemonics.
- **Devanagari tokenizer:** no longer splits words at the virama, preserving
  conjuncts.

## [0.1.2] - 2025

- Make phondata optional when only phonemizing (#5).
- Fixes #4.

## [0.1.1] - 2025

- Early release of the pure-Rust eSpeak NG port: text → IPA phoneme pipeline and
  harmonic formant synthesizer, with bundled data crates.
