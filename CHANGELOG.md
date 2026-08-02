# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
