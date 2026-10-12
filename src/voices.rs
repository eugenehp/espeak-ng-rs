//! Voice enumeration for the CLI `--voices` listing.
//!
//! Reads the voice-definition files under `<data_dir>/lang/` (one file per
//! voice, grouped into language-family subdirectories) and extracts the fields
//! eSpeak NG's `--voices` prints: priority ("Pty"), the primary language code,
//! any additional language codes, the human-readable voice name, and the file
//! path relative to `lang/`.  Mirrors the listing half of `voices.c` /
//! `espeak_ListVoices` — the identification lines only, not the full acoustic
//! voice spec.

use std::fs;
use std::path::Path;

/// One voice, parsed from a `lang/**` definition file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceEntry {
    /// Priority ("Pty") from the primary `language` line (eSpeak default 5;
    /// lower = preferred).
    pub priority: u8,
    /// Primary language code (the first `language` line).
    pub language: String,
    /// Additional language codes (subsequent `language` lines).
    pub other_languages: Vec<String>,
    /// Every `language` line as `(code, priority)` — priority is per-line, which
    /// matters for scoring (`en` is priority 2 in the en-GB voice but 3 in the
    /// en-US voice).
    pub languages: Vec<(String, u8)>,
    /// Human-readable name from the `name` line (may contain spaces).
    pub name: String,
    /// Voice-file path relative to `lang/`, e.g. `gmw/en`.
    pub file: String,
    /// Gender: `'M'`, `'F'`, or `'-'` when unspecified.
    pub gender: char,
    /// Age in years from the `age` line; `0` when unspecified.
    pub age: u8,
    /// `intonation <n>` — selects a row of `punct_to_tone`, i.e. which tunes a
    /// clause's punctuation maps to.  Twenty-two shipped languages set it.
    pub intonation: Option<u8>,
    /// `stressLength <l0…l7>` — per-stress-level durations in mS/2, replacing
    /// the built-in `stress_lengths` table for this language.
    pub stress_length: Option<Vec<i32>>,
    /// `stressOpt <bit…>` — each number sets that bit of `langopts.stress_flags`
    /// (C's `ReadNumbers`).  Finnish and Estonian configure their whole stress
    /// behaviour this way: `stressOpt 1 2 4 6` is
    /// `S_FINAL_DIM_ONLY | S_FINAL_NO_2 | S_2_TO_HEAVY`.
    pub stress_opt: Option<u32>,
    /// `pitch <base> <range>` from the voice file — the two numbers C turns into
    /// `pitch_base = (base - 9) << 12` and `pitch_range = (range - base) * 108`,
    /// and from which it derives the voice's `formant_factor`.
    pub pitch: Option<(i32, i32)>,
    /// `dictionary <name>` directive — the dict stem to load when it differs from
    /// the language code (e.g. Norwegian Bokmål `nb` → `no`).  `None` = use the code.
    pub dictionary: Option<String>,
    /// `stressRule <rule> [unstressed_wd1] [unstressed_wd2]` — the language's
    /// stress placement, for the ~15 languages configured through their voice
    /// file rather than C's `tr_languages.c` (Basque, Quechua, Cherokee, Latin,
    /// Finnish, Estonian, Arabic, …).
    pub stress_rule: Option<Vec<i32>>,
    /// `words <word_gap> [vowel_pause]` — the inter-word pause the language
    /// asks for and, optionally, its vowel-pause bits.  Latin's `words 2` is
    /// what puts a short pause after every word (`s'alwE_ m'UndE_`).
    pub words: Option<Vec<i32>>,
    /// `replace <flags> <old> <new>` — phoneme substitutions the voice applies to
    /// the finished list.  Bit 0 = only at a word end, bit 1 = not in a stressed
    /// syllable, bit 2 = only at a word start; `NULL` as the new phoneme deletes
    /// it.  Russian's `replace 03 a a#` is what makes a word-final unstressed
    /// `a` into `a#`.
    pub replace: Vec<(u8, String, String)>,
    /// `phonemes <name>` directive — the phoneme table to select when it differs
    /// from the language code (e.g. `nb` → `no`).  `None` = use the code.
    pub phonemes: Option<String>,
    /// `dictrules <n…>` directive — condition numbers that select variant
    /// dict/rule groups (`?n`).  E.g. European `pt` has `dictrules 1`, Brazilian
    /// `pt-BR` has `dictrules 2`.  Each sets bit `n` of the dict condition.
    pub dict_rules: Vec<u8>,
    /// `brackets <n>` / `bracketsAnnounced <n>` — how long a bracket or quote
    /// pauses (`LOPT_BRACKET_PAUSE`, default 4; the announced form defaults to
    /// 2).  Hungarian sets both to 0, which is why `hu "a ( b"` has no pause.
    pub brackets: Option<u8>,
    /// See [`Self::brackets`] — the `--punct` value.
    pub brackets_announced: Option<u8>,
}

/// Enumerate every voice under `<data_dir>/lang/`, sorted by language code then
/// name.
///
/// Returns an empty vector if `lang/` is absent.  Files without a `language`
/// line (not voice definitions) are skipped.
pub fn list_voices(data_dir: &Path) -> Vec<VoiceEntry> {
    let lang_root = data_dir.join("lang");
    let mut out = Vec::new();
    collect_dir(&lang_root, &lang_root, &mut out);
    out.sort_by(|a, b| a.language.cmp(&b.language).then_with(|| a.name.cmp(&b.name)));
    out
}

/// A request for a voice, matched against the available [`VoiceEntry`] list.
/// Any field left `None` is not constrained.  Mirrors the fields eSpeak NG's
/// `SetVoiceScores` scores against.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VoiceQuery {
    /// BCP-47 language tag (e.g. `"en"`, `"en-us"`, `"pt-br"`).
    pub language: Option<String>,
    /// Voice name (e.g. `"English (Great Britain)"`) or a language code.
    pub name: Option<String>,
    /// Preferred gender: `'M'` / `'F'`.
    pub gender: Option<char>,
}

/// First subtag of a language code (`en-gb` → `en`).
fn primary_subtag(code: &str) -> &str {
    code.split('-').next().unwrap_or(code)
}

/// Score how well `code` (one of a voice's language codes) matches the requested
/// `want` tag.  Exact > locale-prefix > same-language; `0` for no relation.
fn language_component_score(code: &str, want: &str) -> i32 {
    if code == want {
        500
    } else if want.starts_with(&format!("{code}-")) || code.starts_with(&format!("{want}-")) {
        // "en-us" requested, voice "en"; or vice-versa.
        300
    } else if primary_subtag(code) == primary_subtag(want) {
        150
    } else {
        0
    }
}

/// Score a single voice against a query (higher = better; `0` = no match).
fn voice_score(v: &VoiceEntry, q: &VoiceQuery) -> i32 {
    let mut score = 0;

    if let Some(name) = &q.name {
        let name_l = name.to_ascii_lowercase();
        if v.name.eq_ignore_ascii_case(name) {
            score += 1000;
        } else if v.name.to_ascii_lowercase().contains(&name_l) {
            score += 200;
        }
        // eSpeak treats a "name" as either a voice name or a language code.
        if std::iter::once(&v.language)
            .chain(&v.other_languages)
            .any(|l| l.eq_ignore_ascii_case(name))
        {
            score += 800;
        }
    }

    if let Some(want) = &q.language {
        let want = want.to_ascii_lowercase();
        let best = v
            .languages
            .iter()
            .map(|(c, prio)| {
                let m = language_component_score(&c.to_ascii_lowercase(), &want);
                if m > 0 { m - *prio as i32 } else { 0 } // prefer higher priority (lower number)
            })
            .max()
            .unwrap_or(0);
        score += best;
    }

    if let Some(g) = q.gender {
        if v.gender == g {
            score += 50;
        }
    }

    score
}

/// Pick the best-matching voice for a query, or `None` if nothing matches.
///
/// Mirrors `SetVoiceScores` / `espeak_ng_SetVoiceByProperties`: scores every
/// voice by language (exact/locale-prefix/same-language, weighted by priority),
/// name, and gender, returning the highest.
pub fn find_voice<'a>(voices: &'a [VoiceEntry], query: &VoiceQuery) -> Option<&'a VoiceEntry> {
    voices
        .iter()
        .map(|v| (voice_score(v, query), v))
        .filter(|(s, _)| *s > 0)
        .max_by_key(|(s, _)| *s)
        .map(|(_, v)| v)
}

fn collect_dir(root: &Path, dir: &Path, out: &mut Vec<VoiceEntry>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for entry in rd.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_dir(root, &path, out);
        } else if let Some(v) = parse_voice_file(root, &path) {
            out.push(v);
        }
    }
}

/// Read a voice definition from an explicit file path — `--load` /
/// `espeak_ng_SetVoiceByFile`, for a voice that is not installed under
/// `<data>/lang/`.
///
/// Returns `None` if the file cannot be read or has no `language` line.  Only
/// the directives the port reads from a voice file are honoured; the rest of
/// the engine still looks the language's settings up in the data directory.
pub fn load_voice_file(path: &Path) -> Option<VoiceEntry> {
    let root = path.parent().unwrap_or(Path::new(""));
    parse_voice_file(root, path)
}

fn parse_voice_file(root: &Path, path: &Path) -> Option<VoiceEntry> {
    let bytes = fs::read(path).ok()?;
    let text = String::from_utf8_lossy(&bytes);

    let mut name = String::new();
    let mut priority = 5u8;
    let mut language = String::new();
    let mut other_languages = Vec::new();
    let mut languages: Vec<(String, u8)> = Vec::new();
    let mut gender = '-';
    let mut age = 0u8;
    let mut dictionary = None;
    let mut pitch: Option<(i32, i32)> = None;
    let mut intonation: Option<u8> = None;
    let mut stress_length: Option<Vec<i32>> = None;
    let mut stress_opt: Option<u32> = None;
    let mut stress_rule: Option<Vec<i32>> = None;
    let mut words: Option<Vec<i32>> = None;
    let mut replace: Vec<(u8, String, String)> = Vec::new();
    let mut phonemes = None;
    let mut dict_rules: Vec<u8> = Vec::new();
    let mut brackets: Option<u8> = None;
    let mut brackets_announced: Option<u8> = None;

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("//") {
            continue;
        }
        let mut it = line.split_whitespace();
        let Some(key) = it.next() else { continue };
        match key {
            "name" => name = it.collect::<Vec<_>>().join(" "),
            "language" => {
                let Some(code) = it.next() else { continue };
                // Optional per-line priority (default 5, eSpeak's default).
                let prio = it.next().and_then(|p| p.parse::<u8>().ok()).unwrap_or(5);
                languages.push((code.to_string(), prio));
                if language.is_empty() {
                    language = code.to_string();
                    priority = prio;
                } else {
                    other_languages.push(code.to_string());
                }
            }
            "gender" => {
                gender = match it.next().map(|g| g.to_ascii_lowercase()).as_deref() {
                    Some("male") => 'M',
                    Some("female") => 'F',
                    _ => '-',
                };
            }
            "age" => {
                if let Some(a) = it.next().and_then(|a| a.parse::<u8>().ok()) {
                    age = a;
                }
            }
            // A voice may borrow another language's dictionary / phoneme table.
            "dictionary" => dictionary = it.next().map(str::to_string),
            "pitch" => {
                let n: Vec<i32> = it.by_ref().filter_map(|x| x.parse().ok()).collect();
                if n.len() >= 2 {
                    pitch = Some((n[0], n[1]));
                }
            }
            "intonation" => intonation = it.next().and_then(|x| x.parse().ok()),
            "stressopt" | "stressOpt" => {
                let bits = it
                    .by_ref()
                    .take_while(|t| !t.starts_with("//"))
                    .filter_map(|x| x.parse::<u32>().ok())
                    .filter(|&b| b < 32)
                    .fold(0u32, |acc, b| acc | (1 << b));
                if bits != 0 {
                    stress_opt = Some(bits);
                }
            }
            "stresslength" | "stressLength" => {
                let n: Vec<i32> = it.by_ref().filter_map(|x| x.parse().ok()).collect();
                if !n.is_empty() {
                    stress_length = Some(n);
                }
            }
            "stressrule" | "stressRule" => {
                let nums: Vec<i32> = it.filter_map(|n| n.parse::<i32>().ok()).collect();
                if !nums.is_empty() {
                    stress_rule = Some(nums);
                }
            }
            "replace" => {
                // `%d %s %s` in C, so the flags are decimal ("03" is 3).
                let flags = it.next().and_then(|n| n.parse::<u8>().ok());
                let old = it.next().map(str::to_string);
                let new = it.next().unwrap_or("NULL").to_string();
                if let (Some(f), Some(o)) = (flags, old) {
                    replace.push((f, o, new));
                }
            }
            "words" => {
                let nums: Vec<i32> = it.filter_map(|n| n.parse::<i32>().ok()).collect();
                if !nums.is_empty() {
                    words = Some(nums);
                }
            }
            "phonemes" => phonemes = it.next().map(str::to_string),
            // Condition numbers selecting variant dict/rule groups (`dictrules 2`).
            "dictrules" => dict_rules.extend(it.filter_map(|n| n.parse::<u8>().ok())),
            "brackets" => brackets = it.next().and_then(|n| n.parse().ok()),
            "bracketsannounced" | "bracketsAnnounced" => {
                brackets_announced = it.next().and_then(|n| n.parse().ok())
            }
            _ => {}
        }
    }

    if language.is_empty() {
        return None; // not a voice definition
    }
    if name.is_empty() {
        name = language.clone();
    }
    let file = path.strip_prefix(root).ok()?.to_string_lossy().replace('\\', "/");
    Some(VoiceEntry {
        priority, language, other_languages, languages, name, file, gender, age,
        intonation, stress_length, stress_opt, pitch, dictionary, stress_rule, words, replace,
        phonemes,
        dict_rules,
        brackets,
        brackets_announced,
    })
}

/// The `stressRule` numbers from a language's voice file, if it sets them.
///
/// About fifteen languages are configured this way instead of in C's
/// `tr_languages.c` — Basque (`stressRule 15`), Cherokee, Latin, Quechua,
/// Finnish, Estonian, Arabic and friends — so the placement rule has to come
/// from the data, not from a table in the port.
pub fn voice_stress_rule(data_dir: &Path, lang: &str) -> Option<Vec<i32>> {
    list_voices(data_dir)
        .into_iter()
        .find(|v| v.language.eq_ignore_ascii_case(lang))
        .and_then(|v| v.stress_rule)
}

/// The `replace <flags> <old> <new>` lines a language's voice file declares.
pub fn voice_replacements(data_dir: &Path, lang: &str) -> Vec<(u8, String, String)> {
    list_voices(data_dir)
        .into_iter()
        .find(|v| v.language.eq_ignore_ascii_case(lang))
        .map(|v| v.replace)
        .unwrap_or_default()
}

/// The `words <word_gap> [vowel_pause]` a language's voice file declares.
///
/// `word_gap & 7` indexes C's `pause_phonemes[]` for the pause inserted at every
/// word boundary; the second number, when present, replaces
/// `langopts.vowel_pause`.
pub fn voice_words(data_dir: &Path, lang: &str) -> Option<Vec<i32>> {
    list_voices(data_dir)
        .into_iter()
        .find(|v| v.language.eq_ignore_ascii_case(lang))
        .and_then(|v| v.words)
}

/// The `pitch <base> <range>` a language's voice file declares, if any.
///
/// C turns it into `pitch_base`/`pitch_range` *and* a `formant_factor` — a
/// voice pitched away from the default gets its formants shifted with it.
pub fn voice_pitch(data_dir: &Path, lang: &str) -> Option<(i32, i32)> {
    list_voices(data_dir)
        .into_iter()
        .find(|v| v.language.eq_ignore_ascii_case(lang))
        .and_then(|v| v.pitch)
}

/// The `intonation <n>` group a language's voice file selects, if any.
///
/// Part of reading the language configuration from data rather than a
/// transcribed table (upstream #1466): twenty-two shipped languages set it, and
/// it decides which tune each clause terminator maps to.
pub fn voice_intonation(data_dir: &Path, lang: &str) -> Option<u8> {
    list_voices(data_dir)
        .into_iter()
        .find(|v| v.language.eq_ignore_ascii_case(lang))
        .and_then(|v| v.intonation)
}

/// The `stressOpt` bits a language's voice file sets, if any — these *replace*
/// the built-in `stress_flags` for that language, as they do in C.
pub fn voice_stress_opt(data_dir: &Path, lang: &str) -> Option<u32> {
    list_voices(data_dir)
        .into_iter()
        .find(|v| v.language.eq_ignore_ascii_case(lang))
        .and_then(|v| v.stress_opt)
}

/// The `stressLength <l0…l7>` table a language's voice file declares, if any.
pub fn voice_stress_length(data_dir: &Path, lang: &str) -> Option<Vec<i32>> {
    list_voices(data_dir)
        .into_iter()
        .find(|v| v.language.eq_ignore_ascii_case(lang))
        .and_then(|v| v.stress_length)
}

/// Look up a voice by its primary language code and return `(dictionary,
/// phonemes)` directive names (each `None` when the voice uses its own code or
/// no voice file matches).  Used to resolve aliased voices like Norwegian
/// Bokmål (`nb` → `dictionary no` / `phonemes no`).
pub fn voice_data_overrides(data_dir: &Path, lang: &str) -> (Option<String>, Option<String>) {
    let voices = list_voices(data_dir);
    match voices.iter().find(|v| v.language.eq_ignore_ascii_case(lang)) {
        Some(v) => (v.dictionary.clone(), v.phonemes.clone()),
        None => (None, None),
    }
}

/// The dict-condition bitmask for a voice: `dictrules <n…>` sets bit `n`, which
/// activates the `?n` conditional dict/rule groups (`pt`=`dictrules 1`→0x2 for
/// European, `pt-BR`=`dictrules 2`→0x4 for Brazilian).  `0` when the voice has
/// no `dictrules` directive.
pub fn voice_dict_condition(data_dir: &Path, lang: &str) -> u32 {
    let voices = list_voices(data_dir);
    voices
        .iter()
        .find(|v| v.language.eq_ignore_ascii_case(lang))
        .map(|v| v.dict_rules.iter().fold(0u32, |acc, &n| acc | (1u32 << n)))
        .unwrap_or(0)
}

/// How long a bracket or quote pauses for this voice: `(brackets,
/// bracketsAnnounced)` from the voice file, defaulting to C's `(4, 2)`.  The
/// pause loop spends 2 per `phonPAUSE`, so 4 is two pauses and 0 is none.
pub fn voice_bracket_pause(data_dir: &Path, lang: &str) -> (u8, u8) {
    let voices = list_voices(data_dir);
    voices
        .iter()
        .find(|v| v.language.eq_ignore_ascii_case(lang))
        .map_or((4, 2), |v| (v.brackets.unwrap_or(4), v.brackets_announced.unwrap_or(2)))
}

/// An MBROLA voice: the `voices/mb/mb-<db>` file's `language` and its
/// `mbrola <database> <phtrans>` directive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MbrolaVoice {
    /// The eSpeak language this voice speaks — `mb-en1` → `en`.
    pub language: String,
    /// The voice file's own `pitch <base> <range>`, if it has one — the MBROLA
    /// voices set their own (`mb-la1` is `82 117`, the female ones `140 220`),
    /// and the `.pho` pitch targets have to follow it.
    pub pitch: Option<(i32, i32)>,
    /// Diphone database name, e.g. `en1`.
    pub database: String,
    /// Phoneme-translation table file name, e.g. `en1_phtrans`.
    pub phtrans: String,
}

/// Load an MBROLA voice definition by name (`mb-en1`, `mb/en1`, or `en1`).
///
/// Reads `<data>/voices/mb/mb-<name>`: `mbrola <db> <phtrans>` names the diphone
/// database and the phoneme-name translation table, and the `language <code>
/// [priority]` lines give the language to translate with.  A voice usually
/// lists several — `mb-en1` has `en-uk 3`, `en-gb 3`, `en 2` — and the *lowest*
/// priority number is the best match, so `en` wins.
pub fn load_mbrola_voice(data_dir: &Path, name: &str) -> Option<MbrolaVoice> {
    let db = name.strip_prefix("mb-").or_else(|| name.strip_prefix("mb/")).unwrap_or(name);
    let text = std::fs::read_to_string(data_dir.join("voices").join("mb").join(format!("mb-{db}")))
        .ok()?;

    let mut best: Option<(i32, String)> = None;
    let mut voice = None;
    let mut pitch = None;
    for line in text.lines() {
        let line = line.split("//").next().unwrap_or("").trim();
        let mut f = line.split_whitespace();
        match f.next() {
            Some("language") => {
                if let Some(code) = f.next() {
                    let pty = f.next().and_then(|p| p.parse().ok()).unwrap_or(5);
                    if best.as_ref().is_none_or(|(b, _)| pty < *b) {
                        best = Some((pty, code.to_string()));
                    }
                }
            }
            Some("pitch") => {
                let n: Vec<i32> = f.by_ref().filter_map(|x| x.parse().ok()).collect();
                if n.len() >= 2 {
                    pitch = Some((n[0], n[1]));
                }
            }
            Some("mbrola") => {
                let database = f.next()?.to_string();
                let phtrans = f.next().unwrap_or("").to_string();
                let phtrans =
                    if phtrans.is_empty() { format!("{database}_phtrans") } else { phtrans };
                voice = Some((database, phtrans));
            }
            _ => {}
        }
    }
    let (database, phtrans) = voice?;
    let language = best.map(|(_, c)| c).unwrap_or_else(|| "en".into());
    Some(MbrolaVoice { language, pitch, database, phtrans })
}

/// One `formant <index> <freq%> <height%> [width%]` line from a variant file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VariantFormant {
    pub index: u8,
    pub freq: i32,
    pub height: i32,
    pub width: i32,
}

/// Acoustic parameters parsed from a variant voice file (`voices/!v/<name>`),
/// e.g. `f3`, `m3`, `whisper`.  A variant modifies the *base* voice's synthesis
/// (pitch, formants, breathiness, …).
///
/// These are parsed and exposed but **not yet applied** to the synthesizer —
/// that step is frame/spectrum-verifiable and tracked in `GAPS.md` §11.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VariantParams {
    /// `name` line (e.g. `female3`).
    pub name: String,
    /// Gender: `'M'`, `'F'`, or `'-'`.
    pub gender: char,
    /// Age in years (0 = unspecified).
    pub age: u8,
    /// `pitch <base> <range>` in Hz.
    pub pitch: Option<(i32, i32)>,
    /// `flutter <n>` — pitch flutter (breathier voices raise it).
    pub flutter: Option<i32>,
    /// `voicing <n>`.
    pub voicing: Option<i32>,
    /// `consonants <n>`.
    pub consonants: Option<i32>,
    /// `roughness <n>`.
    pub roughness: Option<i32>,
    /// `echo <delay_ms> <amplitude>`.
    pub echo: Option<(i32, i32)>,
    /// `formant <i> <freq%> <height%> [width%]` lines.
    pub formants: Vec<VariantFormant>,
    /// `tone <freq> <height> …` — up to six `(frequency, amplitude)` points
    /// shaping the harmonic spectrum (`ReadTonePoints`).  `128` is "no change";
    /// the default curve is `600 170 1200 135 2000 110 3000 110`.
    pub tone: Option<[i32; 12]>,
    /// `stressAmp …` (per stress level).
    pub stress_amp: Vec<i32>,
    /// `stressAdd …`.
    pub stress_add: Vec<i32>,
    /// `breath …`.
    pub breath: Vec<i32>,
    /// `breathw …`.
    pub breathw: Vec<i32>,
}

/// Load and parse a variant voice file at `<data_dir>/voices/!v/<name>`.
///
/// Variant names are case-sensitive (`f3`, `Alex`, `whisper`).  Returns `None`
/// if the file is absent or is not a `language variant` definition.
pub fn load_variant(data_dir: &Path, name: &str) -> Option<VariantParams> {
    let path = data_dir.join("voices").join("!v").join(name);
    let bytes = fs::read(&path).ok()?;
    parse_variant(&String::from_utf8_lossy(&bytes))
}

fn parse_variant(text: &str) -> Option<VariantParams> {
    let mut v = VariantParams { gender: '-', ..Default::default() };
    let mut is_variant = false;

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("//") {
            continue;
        }
        let mut it = line.split_whitespace();
        let Some(key) = it.next() else { continue };
        // Remaining tokens parsed as signed integers, in order.
        let ints = |rest: std::str::SplitWhitespace| -> Vec<i32> {
            rest.filter_map(|x| x.parse::<i32>().ok()).collect()
        };
        match key {
            "language" => {
                if it.next() == Some("variant") {
                    is_variant = true;
                }
            }
            "name" => v.name = it.collect::<Vec<_>>().join(" "),
            "gender" => {
                v.gender = match it.next().map(|g| g.to_ascii_lowercase()).as_deref() {
                    Some("male") => 'M',
                    Some("female") => 'F',
                    _ => '-',
                };
                if let Some(a) = it.next().and_then(|a| a.parse::<u8>().ok()) {
                    v.age = a;
                }
            }
            "pitch" => {
                let n = ints(it);
                if n.len() >= 2 {
                    v.pitch = Some((n[0], n[1]));
                }
            }
            "flutter" => v.flutter = it.next().and_then(|x| x.parse().ok()),
            "voicing" => v.voicing = it.next().and_then(|x| x.parse().ok()),
            "consonants" => v.consonants = it.next().and_then(|x| x.parse().ok()),
            "roughness" => v.roughness = it.next().and_then(|x| x.parse().ok()),
            "echo" => {
                let n = ints(it);
                if n.len() >= 2 {
                    v.echo = Some((n[0], n[1]));
                }
            }
            "formant" => {
                let n = ints(it);
                if n.len() >= 3 {
                    v.formants.push(VariantFormant {
                        index: n[0] as u8,
                        freq: n[1],
                        height: n[2],
                        width: n.get(3).copied().unwrap_or(100),
                    });
                }
            }
            "tone" => {
                // `ReadTonePoints`: up to six freq/height pairs, `-1` ending
                // the list so the last height extends to the top of the range.
                let n = ints(it);
                let mut pts = [-1i32; 12];
                for (i, &x) in n.iter().take(12).enumerate() {
                    pts[i] = x;
                }
                if n.len() < 12 {
                    pts[n.len().min(11)] = -1;
                }
                v.tone = Some(pts);
            }
            "stressamp" | "stressAmp" => v.stress_amp = ints(it),
            "stressadd" | "stressAdd" => v.stress_add = ints(it),
            "breath" => v.breath = ints(it),
            "breathw" => v.breathw = ints(it),
            _ => {}
        }
    }

    (is_variant || !v.name.is_empty()).then_some(v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn data() -> Option<PathBuf> {
        let d = PathBuf::from("espeak-ng-data");
        d.join("lang").exists().then_some(d)
    }

    #[test]
    fn lists_known_voices() {
        let Some(d) = data() else {
            eprintln!("[SKIP] no local espeak-ng-data");
            return;
        };
        let vs = list_voices(&d);
        assert!(vs.len() > 50, "expected many voices, got {}", vs.len());

        let en = vs.iter().find(|v| v.language == "en-gb").expect("en-gb voice");
        assert_eq!(en.name, "English (Great Britain)");
        assert_eq!(en.file, "gmw/en");
        assert_eq!(en.priority, 2);
        // `language en 2` is listed as an additional language of the same file.
        assert!(en.other_languages.iter().any(|l| l == "en"));

        assert!(vs.iter().any(|v| v.language == "af" && v.name == "Afrikaans"));

        // Sorted by language code ascending.
        assert!(vs.windows(2).all(|w| w[0].language <= w[1].language));
    }

    #[test]
    fn find_voice_scores_language_name_gender() {
        let Some(d) = data() else {
            eprintln!("[SKIP] no local espeak-ng-data");
            return;
        };
        let voices = list_voices(&d);
        let lang_of = |q: VoiceQuery| find_voice(&voices, &q).map(|v| v.language.clone());

        // "fr" resolves to a French voice (its primary code may be "fr" or "fr-fr").
        assert!(
            lang_of(VoiceQuery { language: Some("fr".into()), ..Default::default() })
                .is_some_and(|l| l.starts_with("fr")),
            "fr should resolve to a French voice"
        );
        // Locale falls back to the highest-priority match: "en" → en-gb (priority 2).
        assert_eq!(lang_of(VoiceQuery { language: Some("en".into()), ..Default::default() }).as_deref(), Some("en-gb"));
        // A more specific request prefers the exact locale when present.
        assert_eq!(lang_of(VoiceQuery { language: Some("en-us".into()), ..Default::default() }).as_deref(), Some("en-us"));

        // By human-readable voice name.
        assert_eq!(
            find_voice(&voices, &VoiceQuery { name: Some("Afrikaans".into()), ..Default::default() })
                .map(|v| v.language.as_str()),
            Some("af")
        );
        // A "name" that is actually a language code also resolves.
        assert_eq!(
            find_voice(&voices, &VoiceQuery { name: Some("de".into()), ..Default::default() })
                .map(|v| v.language.as_str()),
            Some("de")
        );
        // No match → None.
        assert!(find_voice(&voices, &VoiceQuery { language: Some("zzq".into()), ..Default::default() }).is_none());
        assert!(find_voice(&voices, &VoiceQuery::default()).is_none());
    }

    #[test]
    fn every_entry_has_language_and_name() {
        let Some(d) = data() else { return };
        for v in list_voices(&d) {
            assert!(!v.language.is_empty(), "empty language in {}", v.file);
            assert!(!v.name.is_empty(), "empty name in {}", v.file);
        }
    }

    #[test]
    fn parse_variant_f3() {
        let src = "\
language variant
name female3
gender female

pitch 140 240
formant 0 105  80 150
formant 1 120  75 150 -50
stressAmp 18 18 20 20 20 20 20 20
breath 0 2 3 3 3 3 3 2
echo 120 10
roughness 4
";
        let v = parse_variant(src).expect("valid variant");
        assert_eq!(v.name, "female3");
        assert_eq!(v.gender, 'F');
        assert_eq!(v.pitch, Some((140, 240)));
        assert_eq!(v.echo, Some((120, 10)));
        assert_eq!(v.roughness, Some(4));
        assert_eq!(v.stress_amp, vec![18, 18, 20, 20, 20, 20, 20, 20]);
        assert_eq!(v.breath, vec![0, 2, 3, 3, 3, 3, 3, 2]);
        assert_eq!(v.formants.len(), 2);
        assert_eq!(v.formants[1], VariantFormant { index: 1, freq: 120, height: 75, width: 150 });
    }

    #[test]
    fn parse_variant_not_a_variant_file() {
        // A file without `language variant` and no `name` is rejected.
        assert!(parse_variant("pitch 80 120\nflutter 3\n").is_none());
    }

    #[test]
    fn load_variant_real_files() {
        let Some(d) = data() else {
            eprintln!("[SKIP] no local espeak-ng-data");
            return;
        };
        let f3 = load_variant(&d, "f3").expect("f3 variant");
        assert_eq!(f3.gender, 'F');
        assert_eq!(f3.pitch, Some((140, 240)));

        let m3 = load_variant(&d, "m3").expect("m3 variant");
        assert_eq!(m3.gender, 'M');
        // A female variant sits higher than a male one.
        assert!(f3.pitch.unwrap().0 > m3.pitch.unwrap().0);

        // Nonexistent variant → None.
        assert!(load_variant(&d, "definitely-not-a-variant").is_none());
    }
}
