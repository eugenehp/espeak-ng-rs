# Migrating from 0.2 to 0.3

The next release breaks the public API, so it has to be 0.3.0, not 0.2.1.
`cargo semver-checks` against the 0.2.0 release lists the breaks given below.

**If you only use the high-level API, nothing changes.** That means
`EspeakNg`, its builder, `Translator::new` / `text_to_ipa` /
`text_to_ipa_ssml` / `text_to_ipa_words`, `VoiceSpec` and the CLI. What
does change is the output: speech sounds different and some text reads
differently (see *Behaviour* below).

The breaks are all in the lower-level modules (`synthesize`, `dictionary`,
`translate`, `voices`). Their structs have public fields, and adding a
field breaks any code that builds one with a struct literal.

## Removed

| 0.2 | 0.3 |
|---|---|
| `VoiceParams::formant_freq_pct`, `formant_height_pct`, `formant_width_pct` (percent, F0-F6) | `VoiceParams::formants: FormantScale`, in C's units (256 = as in the data) for F0-F8, with a per-formant `freq_add` in Hz |
| `VoiceParams::formant_scaling_active()` | none; compare `formants` with `FormantScale::default()` |
| `synthesize::wavegen::SynthSegment`, `wavegen_segment()` | `synthesize_frames_seq()`, which renders a whole run of frames as C does |
| `synthesize::setlengths::length_mod_to_samples()` | frames carry their length in samples (`SpectFrame::samples`, `SpectFrame::duration()`) |

`FormantScale::set(ix, freq, height, width, freq_add)` takes the numbers as
they appear on a voice file's `formant` line, in percent:

```rust
// 0.2
let voice = VoiceParams { formant_freq_pct: [100, 110, 100, 100, 100, 100, 100], ..Default::default() };

// 0.3
let mut formants = FormantScale::default();
formants.set(1, 110, 100, 100, 0);
let voice = VoiceParams { formants, ..Default::default() };
```

A height or width in 0.3 replaces the voice's default, as it does in C. In
0.2 it scaled that default, so the same numbers now sound different.

## Changed signatures

| Function | Change |
|---|---|
| `synthesize::klatt::synthesize_frames_klatt` | two new parameters: `source: u8` (the glottal source, 1-5) and `scale: &FormantScale` |
| `synthesize::setlengths::calc_vowel_length_mod` | `more_syllables` is a count (`u8`), not a `bool`; it has an extra `speeds: [u32; 3]` parameter (`len_speeds(wpm)`) |
| `dictionary::stress::get_vowel_stress` | two new parameters: `stress_flags: u32`, `stressed_syllable: i32` |

`setlengths::vowel_length_mod` is the new form of `calc_vowel_length_mod`. It
takes a `VowelContext` and a language's `LangProsody`.

## New fields

These structs gained fields. Code that builds one with a struct literal has to
add them. Most have a `Default`, so ending the literal with
`..Default::default()` keeps working:

- `synthesize::VoiceParams`: `prosody`, `klatt_source`, `voicing`, `formants`,
  `roughness`, `flutter_amp`, `breath`, `breathw`, `consonant_amp`
- `synthesize::phondata::SpectFrame`: `modulation`, `samples`
- `synthesize::intonation::Syllable`: `tone_env`
- `synthesize::bytecode::Neighbours`: `after_stress`, `prev2`, `next3`,
  `next3_wordstart`, `prevoicing`, `next_vowel`, `next_vowel_stress`,
  `prev_vowel`, `prev_vowel_stress`, `vowels_tracked`
- `synthesize::bytecode::PhonemeExtract`: `vwlstart_param`, `vwlending_param`
- `dictionary::stress::StressOpts`: `alt_attributes`, `final_consonant`,
  `prefix_stress`
- `dictionary::lookup::LookupCtx`: `word2`; `dictionary::rules::RuleContext`:
  `word_start`
- `voices::VariantParams`: `klatt`, `speed`, `stress_length`;
  `voices::VariantFormant`: `freq_add`; `voices::VoiceEntry`: `brackets`,
  `brackets_announced`
- `translate::LangOptions`: `bracket_pause`, `ipa_separator`, `ipa_tie`
- `translate::PosExpect`: `following`, `hyphen_after`, `hyphen_before`,
  `prev_letters`; `translate::WordResult`: `skipwords`

`translate::NumberGrammar` also has a **private** field now: the C number
options it was built for. Code outside the crate can't build one with a
literal any more. Use `LangOptions::for_lang(lang).number_grammar` and change
fields on that.

`translate::Token` has a new variant, `Operator(char)`. A `match` on it needs
an arm for it or a wildcard.

## Behaviour

These need no code changes, but tests that pin the output will notice them:

- **Audio.** Vowel lengths, consonant lengths and the pauses before them,
  clause pauses, loudness by stress, and voiced-stop murmurs are now ported from
  C. Mandarin, Cantonese and Vietnamese tones are now sung, not just written in
  `-x` output. Words join without a gap, as in C.
- **Voices.** A voice or variant's `speed`, `voicing`, `breath`, `consonants`,
  `roughness`, `flutter`, `stressLength`, `stressAdd`, `klatt` and `echo` lines
  now take effect. So do the same lines in a language's own file: Swedish's
  `speed 155` is one.
- **Text.** The bundled data is espeak-ng master `f0fb11a8`. Numbers use C's
  own routines for each language, and many words and stress patterns match C
  where they didn't. `CHANGELOG.md` has the list.
- **Input.** Invalid UTF-8 is replaced instead of rejected.
- **Caching.** A `Translator` reads its dictionary and phoneme data once. It
  reads them again if you change `options.lang` or `options.dict_condition`, but
  not if the files on disk change.
