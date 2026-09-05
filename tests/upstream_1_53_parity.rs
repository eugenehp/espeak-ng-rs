// tests/upstream_1_53_parity.rs
//
// Parity with upstream eSpeak NG master (1.53.0-dev, `f135499`), covering the
// behaviours this port gained when the bundled data was refreshed from 1.52.0
// to master and the accompanying engine fixes landed.  Every expectation here
// was checked against a locally built upstream binary of the same revision.
//
// Skips (rather than fails) when the language's data isn't present.

use espeak_ng::translate::ClauseTerminator;
use espeak_ng::Translator;

fn tr(lang: &str) -> Option<Translator> {
    Translator::new(lang, None).ok()
}

macro_rules! skip_unless {
    ($lang:expr) => {
        match tr($lang) {
            Some(t) => t,
            None => {
                eprintln!("[SKIP] no {} data", $lang);
                return;
            }
        }
    };
}

// ── Foreign-script text is spoken, not dropped ──────────────────────────────

#[test]
fn latin_words_under_a_non_latin_voice_are_spoken() {
    // A Latin word inside a Cyrillic clause used to vanish entirely: `-v ru
    // "привет hello"` returned only the Russian.  Upstream translates the run
    // with the Latin voice; so do we (`(en)…(ru)` switch markers, as in C).
    let t = skip_unless!("ru");
    let got = t.text_to_ipa("привет hello").unwrap();
    assert!(got.contains("(en)"), "ru + Latin should switch to en: {got}");
    assert!(got.contains("həl") || got.contains("hɛl"), "the Latin word is spoken: {got}");
}

#[test]
fn arabic_under_english_is_spoken() {
    let t = skip_unless!("en");
    let got = t.text_to_ipa("hello مرحبا").unwrap();
    assert!(got.contains("(ar)"), "en + Arabic should switch to ar: {got}");
}

#[test]
fn serbian_reads_both_of_its_scripts() {
    // Serbian is digraphic — Latin must NOT switch away to English.
    let t = skip_unless!("sr");
    let latin = t.text_to_ipa("zdravo").unwrap();
    let cyrillic = t.text_to_ipa("здраво").unwrap();
    assert!(!latin.contains("(en)"), "sr Latin stays in sr: {latin}");
    assert_eq!(latin.trim(), cyrillic.trim(), "sr reads both scripts alike");
}

// ── Emoji sequences ─────────────────────────────────────────────────────────

#[test]
fn skin_tone_sequences_are_spoken_as_base_plus_modifier() {
    // `👍🏽` is one token so it can match a multi-codepoint entry, but CLDR has
    // no name for every base+tone pair — the sequence used to produce *nothing*.
    let t = skip_unless!("en");
    let got = t.text_to_ipa("👍🏽").unwrap();
    assert!(!got.trim().is_empty(), "skin-tone emoji must not be silent");
    assert!(got.contains("skˈɪn"), "reads the modifier name too: {got}");
    assert!(got.contains("θˈʌmz"), "reads the base name: {got}");
}

#[test]
fn subdivision_flags_are_named() {
    // 🏴 + tag letters: England / Scotland, not "black flag".
    let t = skip_unless!("en");
    let england = t.text_to_ipa("\u{1F3F4}\u{E0067}\u{E0062}\u{E0065}\u{E006E}\u{E0067}\u{E007F}").unwrap();
    assert!(england.contains("ɪŋɡlənd"), "England flag names the country: {england}");
    assert!(!england.contains("flˈaɡ"), "not 'black flag': {england}");
}

// ── Script details ──────────────────────────────────────────────────────────

#[test]
fn sinhala_zwj_conjuncts_stay_one_cluster() {
    // consonant + virama + ZWJ + consonant is one conjunct (`ක්‍ර` = kra); the
    // ZWJ used to break the word, giving two letters ("k ɹˈə").
    let t = skip_unless!("si");
    let got = t.text_to_ipa("ක්‍ර").unwrap();
    assert!(!got.trim().contains(' '), "conjunct is one word: {got}");
    assert_eq!(got.trim(), "kɹˈɐ");
}

// ── Context-dependent ("virtual") phonemes ──────────────────────────────────

#[test]
fn virtual_phonemes_follow_their_own_language_definition() {
    // `d#` is `IF prevPh(isVoiced) THEN d ELSE t` in English, an unconditional
    // `ChangePhoneme(d)` in European Portuguese, and branches to the affricate
    // in Brazilian Portuguese.  Hardcoding English's rule gave pt `tˈẽŋty`.
    if let Some(t) = tr("pt") {
        let got = t.text_to_ipa("dente").unwrap();
        assert!(got.starts_with('d'), "pt 'dente' starts with [d]: {got}");
        assert!(!got.contains("tʃ"), "European pt does not affricate: {got}");
    }
    if let Some(t) = tr("pt-br") {
        let got = t.text_to_ipa("dente").unwrap();
        assert!(got.contains("tʃ"), "pt-br affricates before /i/: {got}");
    }
    if let Some(t) = tr("de") {
        // German word-final `g` is the ich-laut, which the phoneme's program says.
        let got = t.text_to_ipa("zwanzig").unwrap();
        assert!(got.ends_with('ç'), "de 'zwanzig' ends in [ç]: {got}");
    }
}

#[test]
fn voicing_assimilation_crosses_the_word_boundary() {
    // Portuguese coda `s#` is voiced by the *next word's* first sound.
    let t = skip_unless!("pt");
    assert!(t.text_to_ipa("três").unwrap().ends_with('ʃ'), "before a pause: [ʃ]");
    let phrase = t.text_to_ipa("os amigos").unwrap();
    assert!(phrase.starts_with("uz"), "before a vowel: [z] — {phrase}");
}

// ── Numbers ─────────────────────────────────────────────────────────────────

#[test]
fn russian_decimals_are_read_as_fractions() {
    // "0,25" is "ноль и двадцать пять сотых", not "ноль и два пять".
    let t = skip_unless!("ru");
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();
    // Numerator read as a number + the scale word, not digit by digit.
    let quarter = ipa("0,25");
    assert!(quarter.contains("dvˈɑtsʌtʲ"), "0,25 says 'двадцать': {quarter}");
    assert!(quarter.contains("sˈot"), "0,25 says 'сотых': {quarter}");
    assert!(!quarter.contains("dvˈɑ "), "not read as separate digits: {quarter}");
    // Numerator 1 takes the feminine singular ("одна десятая"), 2 the feminine
    // plural ("две десятых").
    assert!(ipa("0,1").contains("ʌdnˈɑ"), "0,1 uses feminine 'одна': {}", ipa("0,1"));
    assert!(ipa("0,2").contains("dvʲ"), "0,2 uses feminine 'две': {}", ipa("0,2"));
    // 11 is not "одиннадцать одна" — no feminine, plural suffix.
    assert!(ipa("0,11").contains("sˈot"), "0,11 says hundredths: {}", ipa("0,11"));
}

#[test]
fn portuguese_number_words_keep_final_stress() {
    // A composed number entry ("cento‖e‖um") must take end-stress promotion on
    // its LAST word, exactly as the same words typed out do.
    let t = skip_unless!("pt");
    // The final word must carry the stress; upstream likewise renders the
    // digit and the spelled-out form with slightly different vowels
    // (`sˈẽntu` vs `sˈẽntw`), so compare the stress, not the whole string.
    let last_word_stressed = |s: String| {
        s.split_whitespace().last().is_some_and(|w| w.starts_with('ˈ'))
    };
    assert!(last_word_stressed(t.text_to_ipa("101").unwrap()), "101 stresses 'um'");
    assert!(last_word_stressed(t.text_to_ipa("cento e um").unwrap()), "words stress 'um'");
    assert_eq!(t.text_to_ipa("21").unwrap(), t.text_to_ipa("vinte e um").unwrap());
}

// ── Suffix chains ───────────────────────────────────────────────────────────

#[test]
fn stacked_suffixes_keep_the_stem() {
    // English `'s` is SUFX_M ("more suffixes allowed"), so `s's's` is the stem
    // "s" plus two possessives; the stem used to be lost.
    let t = skip_unless!("en");
    let got = t.text_to_ipa("s's's").unwrap();
    assert!(got.starts_with('ˈ') || got.starts_with('ɛ'), "stem 'ess' survives: {got}");
    assert!(got.contains("ɛs"), "stem 'ess' survives: {got}");
}

// ── New API ─────────────────────────────────────────────────────────────────

#[test]
fn text_to_ipa_reports_the_clause_terminator() {
    let t = skip_unless!("en");
    let (ipa, end) = t.text_to_ipa_with_terminator("Are you there?").unwrap();
    assert!(!ipa.is_empty());
    assert_eq!(end, ClauseTerminator::Question);
    assert!(end.is_sentence());
    assert_eq!(end.as_char(), Some('?'));

    assert_eq!(t.text_to_ipa_with_terminator("Hello.").unwrap().1, ClauseTerminator::Period);
    assert_eq!(t.text_to_ipa_with_terminator("Hello, there").unwrap().1, ClauseTerminator::None);
    assert_eq!(t.text_to_ipa_with_terminator("Hello,").unwrap().1, ClauseTerminator::Comma);
    assert_eq!(t.text_to_ipa_with_terminator("Wow!").unwrap().1, ClauseTerminator::Exclamation);
}

// ── Data coverage ───────────────────────────────────────────────────────────

#[test]
fn every_advertised_voice_can_speak() {
    // The bundled data used to advertise voices (crh, mn, ps, rup, fo, kaa …)
    // with no dictionary behind them: `--voices` listed them and `-v crh`
    // failed with "dict not found".
    let Ok(data_dir) = std::env::var("ESPEAK_DATA_PATH") else {
        eprintln!("[SKIP] ESPEAK_DATA_PATH not set");
        return;
    };
    let voices = espeak_ng::voices::list_voices(std::path::Path::new(&data_dir));
    assert!(voices.len() > 100, "expected the full voice list, got {}", voices.len());
    let mut broken = Vec::new();
    for v in &voices {
        match Translator::new(&v.language, None) {
            Ok(t) => {
                if t.text_to_ipa("test").is_err() {
                    broken.push(v.language.clone());
                }
            }
            Err(_) => broken.push(v.language.clone()),
        }
    }
    assert!(broken.is_empty(), "voices advertised but unusable: {broken:?}");
}

// ── Tone languages ──────────────────────────────────────────────────────────

#[test]
fn chinese_readings_come_from_the_dictionary_with_tones() {
    // Chinese dictionaries are compiled as replacement *text* (`你` → `ni3`), so
    // FLAG_TEXTMODE's meaning is reversed for them.  Reading the entry as
    // phoneme codes produced gibberish (`你` → `qr`), and the tone phonemes —
    // phSTRESS-type entries carrying a program — were dropped entirely.
    let t = skip_unless!("cmn");
    assert_eq!(t.text_to_ipa("你").unwrap(), "nˈi2");
    assert_eq!(t.text_to_ipa("中文").unwrap(), "ts.ˈonɡ5 wˈuəɜn");
}

#[test]
fn cantonese_reads_with_tones() {
    let t = skip_unless!("yue");
    assert_eq!(t.text_to_ipa("你好").unwrap(), "nˈei5 hˈou2");
}

#[test]
fn mandarin_third_tone_sandhi() {
    // 214 + 214 → 35 + 214 ("nǐ hǎo" is said "ní hǎo"); a lone 214 before another
    // tone becomes 21.
    let t = skip_unless!("cmn");
    let nihao = t.text_to_ipa("你好").unwrap();
    assert!(nihao.starts_with("niɜ"), "3+3 sandhi raises the first tone: {nihao}");
    // The sandhi pair is one word: no space, and no stress on the first half.
    assert_eq!(nihao, "niɜχˈɑu2", "the pair reads as one word: {nihao}");
    let beijing = t.text_to_ipa("北京").unwrap();
    assert!(beijing.contains("pˈei2"), "214 before another tone → 21: {beijing}");
}

// ── Rules-engine fixes found while comparing against the oracle ─────────────

#[test]
fn german_prefixed_verbs_translate_their_stem() {
    // The rules scan stops at a standard prefix and the caller re-translates the
    // stem.  The port used to keep scanning, so a later suffix overwrote
    // `end_type` while `end_phonemes` still held the prefix's phonemes: every
    // `be-`/`ge-` verb with an inflectional ending collapsed to `b'@b@`.
    let t = skip_unless!("de");
    let beachten = t.text_to_ipa("beachten").unwrap();
    assert!(beachten.contains("axt"), "the stem is translated: {beachten}");
    assert!(beachten.ends_with("ən"), "the ending is kept: {beachten}");
    assert_eq!(t.text_to_ipa("bezahlen").unwrap(), "bətsˈɑːlən");
}

#[test]
fn unstressed_allophones_reach_the_phoneme_codes() {
    // Stress-conditioned allophones (`ChangeIfDiminished`) were applied only when
    // rendering IPA, so `-x` and the synthesizer used the citation vowel:
    // Russian "молоко" was `molok'o` instead of `mVlVk'o`.
    let Some(t) = tr("ru") else {
        eprintln!("[SKIP] no ru data");
        return;
    };
    let ipa = t.text_to_ipa("молоко").unwrap();
    assert!(ipa.starts_with("mʌɭʌ"), "unstressed о reduces to ʌ: {ipa}");

    // The same reduction must be visible in the phoneme codes, which is what the
    // synthesizer and `-x` read: the two unstressed vowels must not be the same
    // phoneme as the stressed one.
    let codes = t.translate_to_codes("молоко").unwrap();
    let vowels: Vec<u8> = codes
        .iter()
        .filter(|c| !c.is_boundary && c.code > 8)
        .map(|c| c.code)
        .collect();
    assert!(
        vowels.windows(2).any(|w| w[0] != w[1]),
        "the codes carry distinct vowels: {vowels:?}"
    );
}

// ── Second round: pauses, fractions, switch grouping ────────────────────────

#[test]
fn vowel_initial_words_get_a_break() {
    // `langopts.vowel_pause`: German/Dutch/Afrikaans (0x30) and Hungarian (0x20)
    // put a break in front of a vowel-initial word — and in front of a
    // vowel-initial stem left by a prefix — so the previous consonant doesn't
    // link into it.  The port had the option field but never applied it.
    let t = skip_unless!("de");
    let codes = t.translate_to_codes("achten").unwrap();
    assert!(
        codes.first().is_some_and(|c| c.code == 9 || c.code == 11 || c.code == 12),
        "a pause phoneme opens the word: {:?}",
        codes.iter().take(3).map(|c| c.code).collect::<Vec<_>>()
    );
    // The IPA surface is unchanged by the pause phoneme.
    assert_eq!(t.text_to_ipa("beachten").unwrap(), "bəˈaxtən");
}

#[test]
fn short_fractions_are_read_as_one_number() {
    // Upstream `NUM_DFRACTION_2` / `_4`: "3,14" is "… fourteen", not "… one four".
    // Leading zeros stay individual and a long fraction falls back.
    if let Some(t) = tr("cs") {
        let ipa = t.text_to_ipa("0,25").unwrap();
        assert!(ipa.contains("dvˈatset"), "cs 0,25 says 'dvacet pět': {ipa}");
        assert!(!ipa.contains("dvˈa pj"), "not read as two separate digits: {ipa}");
    }
    if let Some(t) = tr("fr") {
        let ipa = t.text_to_ipa("3,14").unwrap();
        assert!(ipa.contains("katˈɔʁz"), "fr 3,14 says 'quatorze': {ipa}");
    }
    if let Some(t) = tr("pt") {
        let ipa = t.text_to_ipa("3,14").unwrap();
        assert!(ipa.contains("kˌɐtˈɔɾz"), "pt 3,14 says 'catorze': {ipa}");
        // More digits than the language's limit → digit by digit again.
        let long = t.text_to_ipa("1,234").unwrap();
        assert!(long.split_whitespace().count() >= 4, "long fraction stays digits: {long}");
    }
    // A language without the mode still reads digits individually.
    if let Some(t) = tr("en") {
        assert_eq!(t.text_to_ipa("3.14").unwrap(), "θɹˈiː pɔɪnt wˈɒn fˈɔː");
    }
}

#[test]
fn a_run_of_foreign_script_words_is_one_switch_group() {
    // Upstream emits `(cmn)w1 w2(ar)` for a run, not a switch per word.
    let t = skip_unless!("ar");
    let ipa = t.text_to_ipa("إذا 如果您 النص").unwrap();
    assert_eq!(ipa.matches("(cmn)").count(), 1, "one opening marker: {ipa}");
    assert_eq!(ipa.matches("(ar)").count(), 1, "one closing marker: {ipa}");
}

// ── Fourth round: phoneme-program list edits ────────────────────────────────

#[test]
fn english_linking_r_and_glides_come_from_the_phoneme_program() {
    // `IfNextVowelAppend(r-)` on the final vowels gives English its linking r,
    // and `AppendPhoneme(;)` gives the glide after `aI`.  Both instructions were
    // no-ops, so "sofa area" lost the /r/ and the port faked a narrower version
    // of the rule by inspecting mnemonics.
    let t = skip_unless!("en");
    assert_eq!(t.text_to_ipa("sofa area").unwrap(), "sˈəʊfəɹ ˈeəɹiə");
    assert_eq!(t.text_to_ipa("father and son").unwrap(), "fˈɑːðəɹ and sˈʌn");
    // No linking r before a consonant.
    assert_eq!(t.text_to_ipa("sofa bed").unwrap(), "sˈəʊfə bˈɛd");
    // The `;` glide is synthesis-only: it shows up in `-x`, not in IPA.
    assert_eq!(t.text_to_ipa("my apple").unwrap(), "maɪ ˈapəl");
    let codes = t.translate_to_codes("my apple").unwrap();
    assert!(codes.len() > 6, "the glide is present in the phoneme codes");
}

#[test]
fn context_dependent_vowels_resolve_through_their_program() {
    // English `@2` (the schwa used only for "the") is `IF nextPh(isVowel) THEN
    // I2 ELSE @`, so "the" changes before a vowel.
    let t = skip_unless!("en");
    assert_eq!(t.text_to_ipa("the apple").unwrap(), "ðɪ ˈapəl");
    assert_eq!(t.text_to_ipa("the cat").unwrap(), "ðə kˈat");
}

#[test]
fn dictionary_pronunciations_skip_vowel_reduction() {
    // Turkish tests `isTranslationGiven` to keep a dictionary/number reading
    // intact: "100" (a number key) is `jˈyz`, while the typed word "yüz" goes
    // through the rules and reduces to `jˈøz`.
    let t = skip_unless!("tr");
    assert_eq!(t.text_to_ipa("100").unwrap(), "jˈyz");
    assert_eq!(t.text_to_ipa("yüz").unwrap(), "jˈøz");
}

// ── Sixth round: one tone per syllable ──────────────────────────────────────

#[test]
fn thai_tones_attach_to_their_syllable() {
    // C keeps the tone as a field on the vowel, so a syllable carries exactly
    // one no matter where the rules emitted it.  Thai writes the tone *ahead* of
    // the syllable and often emits it twice, which the port used to render
    // literally (`s55ˈa2wms55aɜds`).
    let t = skip_unless!("th");
    assert_eq!(t.text_to_ipa("สวัสดี").unwrap(), "sˈa5wmsaɜds");
    let codes = t.translate_to_codes("ขอบคุณ").unwrap();
    // One tone phoneme per vowel, none stacked.
    assert!(!codes.is_empty());
}

#[test]
fn tone_placement_does_not_disturb_chinese() {
    // The same pass handles Mandarin, where the tone is written after the
    // syllable and must be pulled back over the coda (`w'u@35n`).
    if let Some(t) = tr("cmn") {
        assert_eq!(t.text_to_ipa("中文").unwrap(), "ts.ˈonɡ5 wˈuəɜn");
    }
    if let Some(t) = tr("yue") {
        assert_eq!(t.text_to_ipa("你好").unwrap(), "nˈei5 hˈou2");
    }
}

// ── Seventh round: default tones, decimal separators, clause rules ──────────

#[test]
fn vietnamese_syllables_all_carry_a_tone() {
    // `CalcPitches_Tone` gives a toneless syllable `phonDEFAULTTONE`, and
    // Vietnamese turns the clause-final one into the falling tone 7.
    let t = skip_unless!("vi");
    let codes = |s: &str| t.translate_to_codes(s).unwrap().len();
    assert!(codes("xin") > 3, "the default tone is present");
    // The word gap (`langopts.word_gap`) closes each Vietnamese word.
    let ipa = t.text_to_ipa("xin chào").unwrap();
    assert!(!ipa.is_empty());
}

#[test]
fn decimal_separators_match_upstream() {
    // Afrikaans, Georgian and Kazakh read "3,14" as *two numbers* — their
    // decimal separator is the period, which the port had backwards.
    for lang in ["af", "ka", "kk"] {
        let Some(t) = tr(lang) else { continue };
        let comma = t.text_to_ipa("3,14").unwrap();
        let period = t.text_to_ipa("3.14").unwrap();
        assert_ne!(comma, period, "{lang}: the two forms must differ");
    }
    // …while the comma-decimal languages are unchanged.
    if let Some(t) = tr("sv") {
        assert!(t.text_to_ipa("3,14").unwrap().contains("pˈɵŋkt"), "sv keeps its comma decimal");
    }
}

#[test]
fn a_period_ends_a_sentence_only_before_a_capital() {
    // Upstream: "one. two" is one clause, "one. Two" is two.
    let t = skip_unless!("en");
    assert!(!t.text_to_ipa("one. two").unwrap().contains('\n'));
    assert!(t.text_to_ipa("one. Two").unwrap().contains('\n'));
    // `?` and `!` always break.
    assert!(t.text_to_ipa("one? two").unwrap().contains('\n'));
    // A comma between digits is a separator, not a clause break.
    assert!(!t.text_to_ipa("3,14").unwrap().contains('\n'));
}

// ── Eighth round: PauseBefore on every render path ──────────────────────────

#[test]
fn german_r_takes_its_pause_after_a_stop() {
    // `ph_german`'s /r/ asks for `PauseBefore 30` after /t/ and 15 after the
    // other stops.  The instruction was parsed and dropped, then applied only to
    // WAV-rendered phonemes — but /r/ is a liquid, so it goes through the
    // formant path, which now breaks its run and inserts the silence.
    let Ok(data_dir) = std::env::var("ESPEAK_DATA_PATH") else {
        eprintln!("[SKIP] ESPEAK_DATA_PATH not set");
        return;
    };
    let mut phdata = match espeak_ng::phoneme::PhonemeData::load(std::path::Path::new(&data_dir)) {
        Ok(p) => p,
        Err(_) => return,
    };
    if phdata.select_table_by_name("de").is_err() {
        eprintln!("[SKIP] no de phoneme table");
        return;
    }
    let r = phdata.lookup_phoneme("r");
    let t = phdata.lookup_phoneme("t");
    let a = phdata.lookup_phoneme("a");
    let program = phdata.get(r).expect("r phoneme").program;

    let pause_with = |prev: u8| {
        let nb = espeak_ng::synthesize::bytecode::Neighbours {
            prev,
            this: r,
            next: a,
            ..Default::default()
        };
        espeak_ng::synthesize::bytecode::interpret_phoneme_ctl(
            program,
            &phdata.phonindex,
            &nb,
            |c| phdata.get(c).cloned(),
            false,
        )
        .pause_before_ms
    };

    assert_eq!(pause_with(t), 30, "/t/ + /r/ asks for a 30 ms pause");
    assert_eq!(pause_with(a), 0, "no pause after a vowel");
}

#[test]
fn an_explicit_stress_mark_wins_over_the_placement_rule() {
    // C's `GetVowelStress` returns the position of an explicit primary-stress
    // marker for every language, and the placement rule is then skipped.  The
    // port only honoured that for Lojban-style languages, so Crimean Tatar —
    // which has no `tr_languages.c` entry at all — stressed both syllables.
    let t = skip_unless!("crh");
    for word in ["selâm", "merhaba", "kitap"] {
        let ipa = t.text_to_ipa(word).unwrap();
        assert_eq!(ipa.matches('ˈ').count(), 1, "one primary stress in {word:?}: {ipa}");
    }
}

#[test]
fn stress_rule_comes_from_the_voice_file_when_it_is_there() {
    // ~15 languages are configured through `stressRule` in their voice file
    // rather than in C's language table.
    let Ok(data_dir) = std::env::var("ESPEAK_DATA_PATH") else {
        eprintln!("[SKIP] ESPEAK_DATA_PATH not set");
        return;
    };
    let dir = std::path::Path::new(&data_dir);
    assert_eq!(espeak_ng::voices::voice_stress_rule(dir, "eu"), Some(vec![15]));
    assert_eq!(espeak_ng::voices::voice_stress_rule(dir, "la"), Some(vec![2, 0, 2]));
    assert_eq!(espeak_ng::voices::voice_stress_rule(dir, "en"), None);
}

#[test]
fn languages_without_a_hardcoded_arm_get_upstream_stress_flags() {
    // `stress_flags` was set in only seven language arms, so everything else ran
    // with none — Basque needs `S_FINAL_VOWEL_UNSTRESSED` to keep the stress off
    // a word-final vowel.
    let t = skip_unless!("eu");
    assert_eq!(t.text_to_ipa("kaixo").unwrap(), "kˈaɪʃo");
    assert_eq!(t.text_to_ipa("etxe").unwrap(), "ˈetʃe");
}

#[test]
fn a_conditional_length_overrides_the_declared_one() {
    // German's /a/ is `length 110`, raised to 125 before a rhotic:
    //   phoneme a … length 110  IF nextPh(isRhotic) THEN length 125 ENDIF
    // C seeds `pd_param[i_SET_LENGTH]` with `std_length` and lets the program
    // change it; the port ignored the instruction and always used the declared
    // value.  Lengths are in mS/2 units, so 110 → 55 and 125 → 62.
    let Ok(data_dir) = std::env::var("ESPEAK_DATA_PATH") else {
        eprintln!("[SKIP] ESPEAK_DATA_PATH not set");
        return;
    };
    let mut phdata = match espeak_ng::phoneme::PhonemeData::load(std::path::Path::new(&data_dir)) {
        Ok(p) => p,
        Err(_) => return,
    };
    if phdata.select_table_by_name("de").is_err() {
        eprintln!("[SKIP] no de phoneme table");
        return;
    }
    let a = phdata.lookup_phoneme("a");
    let r = phdata.lookup_phoneme("r");
    let t = phdata.lookup_phoneme("t");
    let program = phdata.get(a).expect("a phoneme").program;

    let length_before = |next: u8| {
        let nb = espeak_ng::synthesize::bytecode::Neighbours {
            prev: t,
            this: a,
            next,
            ..Default::default()
        };
        espeak_ng::synthesize::bytecode::interpret_phoneme_ctl(
            program, &phdata.phonindex, &nb, |c| phdata.get(c).cloned(), false,
        )
        .set_length
    };

    assert_eq!(length_before(r), Some(62), "/a/ lengthens before a rhotic");
    assert_ne!(length_before(t), Some(62), "…but not before /t/");
}

// ── Twelfth round: variant indicators, liaison, default tone in IPA ─────────

#[test]
fn phoneme_variant_indicators_are_not_printed() {
    // A `/` in a mnemonic starts a "phoneme variant indicator" that espeak drops
    // when printing (C `WritePhMnemonic`), so Spanish's unstressed `a/` shows as
    // `a` and the dark-l `l/2` as `l`.  The port printed the whole mnemonic.
    let Ok(data_dir) = std::env::var("ESPEAK_DATA_PATH") else {
        eprintln!("[SKIP] ESPEAK_DATA_PATH not set");
        return;
    };
    let mut phdata = match espeak_ng::phoneme::PhonemeData::load(std::path::Path::new(&data_dir)) {
        Ok(p) => p,
        Err(_) => return,
    };
    if phdata.select_table_by_name("es").is_err() {
        return;
    }
    let a_var = phdata.lookup_phoneme("a/");
    if a_var != 0 {
        let ph = phdata.get(a_var).unwrap();
        assert_eq!(ph.mnemonic_str(), "a/", "the mnemonic itself keeps the slash");
        assert_eq!(ph.mnemonic_display(), "a", "…but it is not printed");
    }
}

#[test]
fn french_liaison_that_does_not_surface_is_dropped_from_the_codes() {
    // `z2` is only pronounced before a vowel; the IPA path already resolved it,
    // the codes path kept it (`trwaz2`).
    let t = skip_unless!("fr");
    let codes = |s: &str| t.translate_to_codes(s).unwrap().len();
    assert!(codes("deux enfants") > codes("deux chats"), "the liaison surfaces before a vowel");
}

#[test]
fn the_default_tone_is_not_mistaken_for_a_pause() {
    // Code 17 is a control code in most tables but `phonDEFAULTTONE` in a tone
    // language; skipping it as a pause dropped Vietnamese's tone from the IPA.
    let t = skip_unless!("vi");
    assert_eq!(t.text_to_ipa("xin chào").unwrap(), "sˈi1n tʃˈaː2w");
}
