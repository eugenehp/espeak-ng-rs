//! Simplified phoneme bytecode scanner.
//!
//! eSpeak NG stores per-phoneme programs as sequences of 16-bit instructions
//! in the `phonindex` file.  For synthesis we need to extract:
//! - `fmt_addr` — address in `phondata` of the formant frame sequence
//!   (encoded by the `I_FMT` = 0xb000 instruction).
//! - `wav_addr` — address of a WAV sample for stop consonants
//!   (encoded by `I_WAV` = 0xc000).
//! - `ipa_string` — the IPA string for this phoneme
//!   (encoded by `I_IPA_NAME` = 0x0d).
//!
//! The full interpreter is ~400 C lines in `synthdata.c`.  We implement a
//! forward-scanner that respects multi-word instruction sizes.

// Instruction opcode constants mirror synthesize.h and are used internally.
#![allow(missing_docs)]

use crate::phoneme::PhonemeTab;

// ---------------------------------------------------------------------------
// Instruction opcode constants (mirrors synthesize.h)
// ---------------------------------------------------------------------------

pub const INSTN_RETURN:   u16 = 0x0001;
pub const INSTN_CONTINUE: u16 = 0x0002;

pub const I_IPA_NAME:       u16 = 0x0d;  // group-0, operand = UTF-8 byte count
pub const I_CHANGE_PHONEME: u16 = 0x01;  // group-0: (opcode<<8)|phoneme_code
pub const I_REPLACE_NEXT_PHONEME: u16 = 0x02;
pub const I_INSERT_PHONEME:       u16 = 0x03;
pub const I_APPEND_PHONEME:       u16 = 0x04;
pub const I_APPEND_IFNEXTVOWEL:   u16 = 0x05;
pub const I_SET_LENGTH:           u16 = 0x0a;
pub const I_ADD_LENGTH:           u16 = 0x0c;
pub const I_PAUSE_BEFORE:         u16 = 0x07;
pub const I_PAUSE_AFTER:          u16 = 0x08;
pub const I_CALLPH:    u16 = 0x9100;
pub const I_PITCHENV:  u16 = 0x9200;
pub const I_AMPENV:    u16 = 0x9300;
pub const I_VOWELIN:   u16 = 0xa100;
pub const I_VOWELOUT:  u16 = 0xa200;
pub const I_FMT:       u16 = 0xb000;
pub const I_WAV:       u16 = 0xc000;
pub const I_VWLSTART:  u16 = 0xd000;
pub const I_VWLENDING: u16 = 0xe000;
pub const I_WAVADD:    u16 = 0xf000;

// ---------------------------------------------------------------------------
// num_instn_words — how many u16 words does this instruction consume?
// ---------------------------------------------------------------------------

/// Return the number of 16-bit words consumed by an instruction.
///
/// Mirrors `NumInstnWords()` from synthdata.c.
pub fn num_instn_words(instn: u16) -> usize {
    // Mirrors NumInstnWords() from synthdata.c
    // static const char n_words[16] = { 0,1,0,0,1,1,0,1,1,2,4,0,0,0,0,0 };
    const N_WORDS: [u8; 16] = [0, 1, 0, 0, 1, 1, 0, 1, 1, 2, 4, 0, 0, 0, 0, 0];

    let hi4 = (instn >> 12) as usize;
    let n = N_WORDS[hi4];
    if n > 0 {
        return n as usize;
    }

    match hi4 {
        0 => {
            // Group 0: most are 1 word; i_IPA_NAME has trailing data words.
            // Encoding: word = (opcode << 8) | operand; opcode is in HIGH byte.
            let opcode = (instn >> 8) as u8;
            if opcode == I_IPA_NAME as u8 {
                let data = (instn & 0xff) as usize; // UTF-8 byte count
                1 + (data + 1) / 2                  // header + ceil(data/2) words
            } else {
                1
            }
        }
        2 | 3 => {
            // Condition instruction: check for 2-word form
            // C: if ((n=instn&0x0f00)==0x600)||(n==0xd00)) return 2; return 1;
            let n = instn & 0x0f00;
            if n == 0x0600 || n == 0x0d00 { 2 } else { 1 }
        }
        6 => {
            // JUMP: check for 12-word switch form (SwitchOnVowelType)
            let type2 = (instn & 0x0f00) >> 9;
            if type2 == 5 || type2 == 6 { 12 } else { 1 }
        }
        // 0xb (i_FMT), 0xc (i_WAV), 0xd (i_VWLSTART), 0xe (i_VWLENDING),
        // 0xf (i_WAVADD): 2 words (instruction + address word)
        // Check if followed by i_WAVADD (4 words total).
        0xb | 0xc | 0xd | 0xe | 0xf => 2,
        _ => 1,
    }
}

// ---------------------------------------------------------------------------
// Phoneme data extracted by the scanner
// ---------------------------------------------------------------------------

/// Result of scanning a phoneme program.
#[derive(Debug, Clone, Default)]
pub struct PhonemeExtract {
    /// Address in phondata of the FMT (formant) frame sequence.
    /// `None` if no i_FMT instruction was found.
    pub fmt_addr: Option<u32>,
    /// Amplitude parameter for the FMT sequence (from the instruction's param field).
    pub fmt_param: i8,

    /// Address in phondata of a WAV (sampled waveform) to mix in.
    /// `None` if no i_WAV instruction was found.
    pub wav_addr: Option<u32>,
    /// Amplitude parameter for the WAV mix.
    pub wav_param: i8,

    /// VowelStart address (for vowel onset transitions).
    pub vwlstart_addr: Option<u32>,
    /// VowelEnding address (for vowel coda transitions).
    pub vwlending_addr: Option<u32>,

    /// If the phoneme does an unconditional ChangePhoneme(code), the target code.
    /// The caller should look up this code's synthesis data as a fallback.
    pub change_phoneme_code: Option<u8>,

    /// `InsertPhoneme(x)` — insert `x` *before* this phoneme.
    pub insert_phoneme: Option<u8>,
    /// `AppendPhoneme(x)` — insert `x` *after* this phoneme.
    pub append_phoneme: Option<u8>,
    /// `IfNextVowelAppend(x)` — insert `x` after this phoneme when the next one
    /// is a vowel (English linking `r-`, and the `;` glide after `aI`).
    pub append_if_next_vowel: Option<u8>,
    /// `ChangeNextPhoneme(x)` — replace the *following* phoneme with `x`.
    pub replace_next_phoneme: Option<u8>,
    /// `PauseBefore <ms>` — silence to insert ahead of this phoneme (German
    /// puts 15-30 ms before /r/ after a stop).
    pub pause_before_ms: u8,
    /// `PauseAfter <ms>` — silence to insert after this phoneme.
    pub pause_after_ms: u8,
    /// `length <n>` *inside* a program — overrides the phoneme's declared
    /// `std_length` for this context (German shortens/lengthens several vowels
    /// this way).  In mS/2 units, as `std_length` is.
    pub set_length: Option<u8>,
    /// `LengthAdd <n>` — a signed adjustment to the length.
    pub add_length: i8,

    /// Vowel-transition data for coarticulation (GAPS §36, Stage 2).
    /// `[0..2]` come from `i_VOWELIN`, `[2..4]` from `i_VOWELOUT`.  Consumed
    /// by `FormantTransition2` (Stage 4).  Zero = no transition specified.
    #[allow(dead_code)]
    pub vowel_transition: [u32; 4],

    /// `pd_FORNEXTPH`: the `vwlstart_addr` was produced by a `NextVowelStarts`
    /// switch (`SwitchOnVowelType` instn_type 2), i.e. it is the onset glide the
    /// consonant specifies *for the following vowel* — not the phoneme's own
    /// onset.  Mirrors `phdata->pd_control & pd_FORNEXTPH` in synthdata.c and
    /// gates onset selection in the vowel branch of synthesize.c.
    pub pd_fornextph: bool,

    /// The `ipa <string>` the program reached on the path its conditions
    /// actually took — C's `phdata->ipa_string`, which `WritePhMnemonic` uses in
    /// preference to the phoneme's mnemonic when writing IPA.  `None` when the
    /// program has no `ipa` instruction on that path (fall back to the
    /// mnemonic); `Some("")` cannot occur, but a string whose first byte is
    /// `0x20` means "this phoneme has no IPA name" and prints nothing.
    ///
    /// Because it is read off the *executed* path, it is context-sensitive in
    /// the same way C's is: Italian `*` is `r` between vowels and `ɾ` elsewhere.
    pub ipa_string: Option<String>,
}

// ---------------------------------------------------------------------------
// scan_phoneme — main entry point
// ---------------------------------------------------------------------------

/// Scan the bytecode program for a phoneme, extracting synthesis addresses.
///
/// `program` — the index into `phonindex` (stored in `PhonemeTab::program`).
/// `phonindex` — the raw bytes of the phonindex file.
///
/// This is a simplified forward scanner that does not evaluate conditions.
/// It returns the FIRST occurrence of each instruction type encountered while
/// walking the linear bytecode.  For phonemes with conditional branches this
/// gives the "condition-true" path, which corresponds to the primary synthesis
/// route (voiced for voiced consonants, etc.).
///
/// Mirrors the core of `InterpretPhoneme()` in synthdata.c.
/// Evaluate a consonant's `SwitchNextVowelType` table to pick the VowelStart
/// (onset-transition) sequence for a following vowel of the given `start_type`.
///
/// Mirrors `SwitchOnVowelType(..., instn_type=2)` in synthdata.c: a JUMP with
/// `type2 == 5` is followed by a 12-word table of six `VWLSTART` entries (one
/// per vowel category `phonVOWELTYPES..+6`, i.e. `start_type` 28–33).  Returns
/// the phondata address of the selected onset sequence, or `None`.
pub fn select_vowel_start(program: u16, phonindex: &[u8], next_start_type: u8) -> Option<u32> {
    const PHON_VOWEL_TYPES: i32 = 28;
    if program == 0 {
        return None;
    }
    let voweltype = next_start_type as i32 - PHON_VOWEL_TYPES;
    if !(0..6).contains(&voweltype) {
        return None;
    }
    let voweltype = voweltype as usize;
    let max_words = phonindex.len() / 2;
    let word = |i: usize| -> u16 {
        u16::from_le_bytes([phonindex[i * 2], phonindex[i * 2 + 1]])
    };
    let mut pc = program as usize;
    let scan_limit = pc + 128;
    while pc < max_words && pc < scan_limit {
        let instn = word(pc);
        if instn == INSTN_RETURN {
            break;
        }
        if instn >> 12 == 6 && ((instn & 0x0f00) >> 9) == 5 {
            // SwitchNextVowelType: prog = pc + voweltype*2;
            // addr = ((prog[1] & 0xf) << 16 + prog[2]) * 4
            let base = pc + voweltype * 2;
            if base + 2 < max_words {
                let p1 = word(base + 1) as u32;
                let p2 = word(base + 2) as u32;
                return Some((((p1 & 0xf) << 16) + p2) * 4);
            }
            return None;
        }
        pc += num_instn_words(instn);
    }
    None
}

pub fn scan_phoneme(program: u16, phonindex: &[u8]) -> PhonemeExtract {
    let mut result = PhonemeExtract::default();

    if program == 0 {
        return result;
    }

    let mut pc = program as usize; // word index (each word = 2 bytes)
    let max_words = phonindex.len() / 2;

    // Safety limit: most phoneme programs are < 64 instructions.
    let scan_limit = pc + 128;

    loop {
        if pc >= max_words || pc >= scan_limit {
            break;
        }

        let byte_off = pc * 2;
        let instn = u16::from_le_bytes([phonindex[byte_off], phonindex[byte_off + 1]]);

        // ── RETURN ───────────────────────────────────────────────────────────
        if instn == INSTN_RETURN {
            break;
        }

        let hi4 = instn >> 12;

        match hi4 {
            0xb => {
                // i_FMT: followed by one address word.
                // Address = ((instn & 0xf) << 18) | (next_word << 2)
                if result.fmt_addr.is_none() && pc + 1 < max_words {
                    let next = u16::from_le_bytes([
                        phonindex[(pc+1)*2],
                        phonindex[(pc+1)*2 + 1],
                    ]);
                    let addr = ((instn & 0xf) as u32) << 18 | ((next as u32) << 2);
                    result.fmt_addr = Some(addr);
                    // The param is stored in bits 11-4 of the instruction, sign-extended.
                    result.fmt_param = ((instn >> 4) & 0xff) as i8;
                }
                // i_FMT implies RETURN unless immediately followed by i_WAVADD.
                // (VWLSTART selection is handled separately by
                // `select_vowel_start`, which does its own bounded scan.)
                if pc + 2 < max_words {
                    let next2 = u16::from_le_bytes([
                        phonindex[(pc + 2) * 2],
                        phonindex[(pc + 2) * 2 + 1],
                    ]);
                    if next2 >> 12 == 0xf {
                        pc += 2;
                        continue;
                    }
                }
                break;
            }
            0xc => {
                // i_WAV: sampled waveform
                if result.wav_addr.is_none() && pc + 1 < max_words {
                    let next = u16::from_le_bytes([
                        phonindex[(pc+1)*2],
                        phonindex[(pc+1)*2 + 1],
                    ]);
                    let addr = ((instn & 0xf) as u32) << 18 | ((next as u32) << 2);
                    result.wav_addr = Some(addr);
                    result.wav_param = ((instn >> 4) & 0xff) as i8;
                }
                break; // i_WAV also implies RETURN
            }
            0xd => {
                // i_VWLSTART
                if result.vwlstart_addr.is_none() && pc + 1 < max_words {
                    let next = u16::from_le_bytes([
                        phonindex[(pc+1)*2],
                        phonindex[(pc+1)*2 + 1],
                    ]);
                    result.vwlstart_addr = Some(
                        ((instn & 0xf) as u32) << 18 | ((next as u32) << 2)
                    );
                }
                pc += 2;
                continue;
            }
            0xe => {
                // i_VWLENDING
                if result.vwlending_addr.is_none() && pc + 1 < max_words {
                    let next = u16::from_le_bytes([
                        phonindex[(pc+1)*2],
                        phonindex[(pc+1)*2 + 1],
                    ]);
                    result.vwlending_addr = Some(
                        ((instn & 0xf) as u32) << 18 | ((next as u32) << 2)
                    );
                }
                pc += 2;
                continue;
            }
            0xf => {
                // i_WAVADD
                if result.wav_addr.is_none() && pc + 1 < max_words {
                    let next = u16::from_le_bytes([
                        phonindex[(pc+1)*2],
                        phonindex[(pc+1)*2 + 1],
                    ]);
                    result.wav_addr = Some(
                        ((instn & 0xf) as u32) << 18 | ((next as u32) << 2)
                    );
                }
                // WAVADD after FMT → this was the last instruction
                break;
            }
            9 => {
                // CALLPH (0x9100), PITCHENV (0x9200), AMPENV (0x9300)
                let instn2 = ((instn >> 8) & 0xf) as u8;
                if instn2 == 1 && pc + 1 < max_words {
                    // i_CALLPH: the next word is the program index to call.
                    // data = ((instn & 0xf) << 16) | next_word
                    let next = u16::from_le_bytes([phonindex[(pc+1)*2], phonindex[(pc+1)*2+1]]);
                    let called_prog = ((((instn & 0xf) as u32) << 16) | next as u32) as usize;
                    // Recursively scan the called program and merge results.
                    if called_prog > 0 && called_prog < max_words {
                        let sub = scan_phoneme(called_prog as u16, phonindex);
                        if result.fmt_addr.is_none() { result.fmt_addr = sub.fmt_addr; result.fmt_param = sub.fmt_param; }
                        if result.wav_addr.is_none() { result.wav_addr = sub.wav_addr; result.wav_param = sub.wav_param; }
                        if result.vwlstart_addr.is_none() { result.vwlstart_addr = sub.vwlstart_addr; }
                        if result.vwlending_addr.is_none() { result.vwlending_addr = sub.vwlending_addr; }
                        for k in 0..4 {
                            if result.vowel_transition[k] == 0 {
                                result.vowel_transition[k] = sub.vowel_transition[k];
                            }
                        }
                    }
                    // After CALLPH, a RETURN follows in the caller — stop scanning.
                }
                pc += 2;
                continue;
            }
            0 => {
                // Group 0: check for i_CHANGE_PHONEME (opcode in high byte = 0x01)
                let opcode = (instn >> 8) as u8;
                if opcode == I_CHANGE_PHONEME as u8 {
                    // Record the target phoneme code (low byte); prefer last unconditional
                    result.change_phoneme_code = Some((instn & 0xff) as u8);
                }
                pc += num_instn_words(instn);
                continue;
            }
            0xa => {
                // i_VOWELIN (0xa1) / i_VOWELOUT (0xa2): 4-word vowel-transition
                // data. C (synthdata.c): vt[k]=((prog0&0xff)<<16)+prog1,
                //                        vt[k+1]=(prog2<<16)+prog3.
                if pc + 3 < max_words {
                    let w = |o: usize| {
                        u16::from_le_bytes([phonindex[(pc + o) * 2], phonindex[(pc + o) * 2 + 1]])
                            as u32
                    };
                    let t0 = ((instn as u32 & 0xff) << 16) | w(1);
                    let t1 = (w(2) << 16) | w(3);
                    match (instn >> 8) & 0xf {
                        1 => { // i_VOWELIN
                            result.vowel_transition[0] = t0;
                            result.vowel_transition[1] = t1;
                        }
                        2 => { // i_VOWELOUT
                            result.vowel_transition[2] = t0;
                            result.vowel_transition[3] = t1;
                        }
                        _ => {}
                    }
                }
                pc += 4;
                continue;
            }
            _ => {
                // All other instructions: advance by the correct number of words.
                pc += num_instn_words(instn);
                continue;
            }
        }
    }

    result
}

// ---------------------------------------------------------------------------
// interpret_phoneme — context-aware interpreter (ports InterpretPhoneme)
// ---------------------------------------------------------------------------
//
// `scan_phoneme` is a linear forward scanner that takes the FIRST occurrence of
// each instruction — it ignores conditions.  For consonants whose program
// selects the formant sequence with `IF prevPh(..)/nextPh(..)` branches (e.g.
// `l`, which hides `FMT(l/l_)` inside an `IF nextPh(isNotVowel)` block before
// the default `FMT(l/l)`), that picks the wrong FMT and the phoneme is
// mis-rendered.  `interpret_phoneme` evaluates those conditions against the
// surrounding phonemes, faithfully porting `InterpretPhoneme` +
// `InterpretCondition` from synthdata.c (the synthesis-stage call:
// `InterpretPhoneme(NULL, 0, ...)`, i.e. tr == NULL, control == 0).

// Condition / instruction constants (mirror synthesize.h / phoneme.h).
const COND_MARK:        u16 = 0x2000; // (instn & 0xe000) == 0x2000 → condition
const COND_OR_FLAG:     u16 = 0x1000;
const INSTN_NOT:        u16 = 0x0003;
const JUMP_FALSE_MASK:  u16 = 0xf800;
const JUMP_FALSE:       u16 = 0x6800;

// CONDITION_IS_* categories (bits 5-7 in attribute mode).
const COND_IS_TYPE:  u16 = 0x00;
const COND_IS_PLACE: u16 = 0x20;
const COND_IS_FLAG:  u16 = 0x40;
const COND_IS_OTHER: u16 = 0x80;

// "other" condition codes (attribute-mode data value, bits 0-4).
const OTHER_IS_AFTER_STRESS: u16 = 9;
const OTHER_IS_NOT_VOWEL:    u16 = 10;
const OTHER_IS_FINAL_VOWEL:  u16 = 11;
const OTHER_IS_VOICED:       u16 = 12;
const OTHER_IS_FIRST_VOWEL:  u16 = 13;
const OTHER_IS_SECOND_VOWEL: u16 = 14;
const OTHER_IS_TRANSLATION:  u16 = 16;
const OTHER_IS_BREAK:        u16 = 17;
const OTHER_IS_WORD_START:   u16 = 18;
const OTHER_IS_WORD_END:     u16 = 19;

// Phoneme type / flag constants (mirror phoneme.h).
const PH_PAUSE:  u8 = 0;
const PH_VOWEL:  u8 = 2;
const PH_LIQUID: u8 = 3;
const PH_VOICED_FLAG: u32 = 1 << 4; // phFLAGBIT_VOICED

const PHON_VOWEL_TYPES: i32 = 28; // start_type/end_type base for vowel categories

/// Immediate neighbour window used to evaluate a phoneme's conditional program.
///
/// Codes are phoneme indices in the active table; `0` denotes a pause/boundary
/// (rendered as a synthetic `phPAUSE` entry so `prevPh(isPause)` etc. behave as
/// in C, where clause boundaries are real pause phonemes).  The `*_wordstart`
/// flags mark `sourceix != 0` positions for the `*PhW` condition variants.
#[derive(Debug, Clone)]
pub struct Neighbours {
    pub prev: u8,
    pub this: u8,
    pub next: u8,
    pub next2: u8,
    /// Stress level (0-6) of this phoneme's syllable.
    pub stress: u8,
    /// Stress level of the *next* phoneme's syllable, or [`STRESS_UNKNOWN`].
    ///
    /// C's `StressCondition` reads the stress from the phoneme itself when it is
    /// a vowel and otherwise from the phoneme that follows it, so a consonant
    /// needs its neighbour's level.  When this is `STRESS_UNKNOWN` the caller
    /// did not track per-syllable stress (the synthesis stage doesn't need to)
    /// and `stress` is used for both.
    pub next_stress: u8,
    /// Stress level of the *previous* and *next-but-one* phonemes' syllables,
    /// or [`STRESS_UNKNOWN`].  A stress condition can name any of the window's
    /// phonemes (`nextVowel(isMaxStress)`, `prevPh(isStressed)`), and evaluating
    /// them all against *this* phoneme's level made Russian `мама` read
    /// `m'ama#` where upstream has `m'Ama#`.
    pub prev_stress: u8,
    pub next2_stress: u8,
    /// The highest stress level in this phoneme's word (C's `plist->wordstress`),
    /// or [`STRESS_UNKNOWN`].  `ChangeIfStressed` compares against it.
    pub wordstress: u8,
    /// This phoneme's position among the *vowels of its word*, 1-based
    /// (C's `CountVowelPosition`).  `0` when the caller doesn't track it, which
    /// makes `isFirstVowel`/`isSecondVowel` false — Malayalam's `a` keeps its
    /// full quality in the first syllable because of it.
    pub vowel_position: u8,
    /// The language's `LOPT_REDUCE`: bit 0 lets the stress-conditioned changes
    /// apply to dictionary-given words, bit 1 promotes the most stressed
    /// syllable of an unstressed word to primary.
    pub reduce: u32,
    /// The word's pronunciation came from the dictionary rather than the
    /// letter-to-sound rules (C's `SFLAG_DICTIONARY`), which Turkish, Bashkir
    /// and Tatar test with `isTranslationGiven` to suppress vowel reduction.
    pub translation_given: bool,
    pub this_wordstart: bool,
    pub prev_wordstart: bool,
    pub next_wordstart: bool,
    pub next2_wordstart: bool,
    /// The next vowel **in this word**, and its syllable's stress level — C's
    /// `nextVowel` condition, which scans forward to the next vowel and stops
    /// at a word boundary rather than looking a fixed distance ahead.  `0`
    /// means the caller doesn't track it (the condition then falls back to the
    /// two-phoneme window).  Russian's pre-tonic rule needs it:
    /// `V` is `IF nextVowel(isMaxStress) THEN ChangePhoneme(a)`, and in
    /// `двадцатьпять` the stressed vowel is three phonemes away.
    pub next_vowel: u8,
    pub next_vowel_stress: u8,
    /// The previous vowel in this word (C's `worddata->prev_vowel`) and its
    /// stress level; `0` = none.
    pub prev_vowel: u8,
    pub prev_vowel_stress: u8,
    /// Whether the caller filled `next_vowel`/`prev_vowel` by scanning the
    /// word.  When it did, `next_vowel == 0` *means* "no further vowel in this
    /// word" and `isFinalVowel` is exact; when it didn't, the conditions fall
    /// back to looking within the two-phoneme window.
    pub vowels_tracked: bool,
}

/// `next_stress`/`wordstress` value meaning "the caller doesn't track this".
pub const STRESS_UNKNOWN: u8 = u8::MAX;

impl Default for Neighbours {
    fn default() -> Self {
        Neighbours {
            prev: 0,
            this: 0,
            next: 0,
            next2: 0,
            stress: 0,
            next_stress: STRESS_UNKNOWN,
            prev_stress: STRESS_UNKNOWN,
            next2_stress: STRESS_UNKNOWN,
            vowel_position: 0,
            wordstress: STRESS_UNKNOWN,
            reduce: 0,
            translation_given: false,
            next_vowel: 0,
            next_vowel_stress: STRESS_UNKNOWN,
            prev_vowel: 0,
            prev_vowel_stress: STRESS_UNKNOWN,
            vowels_tracked: false,
            this_wordstart: false,
            prev_wordstart: false,
            next_wordstart: false,
            next2_wordstart: false,
        }
    }
}

/// Read a little-endian u16 word at word-index `i` (0 if out of bounds).
#[inline]
fn word_at(phonindex: &[u8], i: usize) -> u16 {
    let b = i * 2;
    if b + 1 < phonindex.len() {
        u16::from_le_bytes([phonindex[b], phonindex[b + 1]])
    } else {
        0
    }
}

/// Interpret a phoneme's bytecode program with neighbour context, selecting the
/// formant/wave sequence that its conditions actually reach.  Mirrors
/// `InterpretPhoneme()` (synthesis stage) from synthdata.c.
pub fn interpret_phoneme<F>(
    program: u16,
    phonindex: &[u8],
    nb: &Neighbours,
    lookup: F,
) -> PhonemeExtract
where
    F: Fn(u8) -> Option<PhonemeTab>,
{
    interpret_phoneme_ctl(program, phonindex, nb, lookup, false)
}

/// As [`interpret_phoneme`], with upstream's `control & 0x100` ("PhonemeList
/// mode") selectable.
///
/// In that mode — the *translate* stage, where the phoneme list is still being
/// built — the first `ChangePhoneme(x)` the program reaches **wins and ends
/// interpretation** (`synthdata.c`: "found ChangePhoneme() in PhonemeList mode,
/// exit").  At synthesis time (`false`) the instruction is only recorded, and
/// scanning continues to find the FMT/WAV data.
///
/// The distinction matters for phonemes like Brazilian Portuguese `t#`:
///
/// ```text
/// IF nextPhW(#i) THEN ChangePhoneme(tS)
/// ELIF thisPh(isWordEnd) THEN ChangePhoneme(tS)
/// ENDIF
/// ChangePhoneme(t)
/// ```
///
/// Without the early exit the trailing unconditional `ChangePhoneme(t)`
/// overwrites the affricate the taken branch just selected, so `noite` came out
/// `nˈoɪty` instead of `nˈoɪtʃɪ`.
pub fn interpret_phoneme_ctl<F>(
    program: u16,
    phonindex: &[u8],
    nb: &Neighbours,
    lookup: F,
    phoneme_list_mode: bool,
) -> PhonemeExtract
where
    F: Fn(u8) -> Option<PhonemeTab>,
{
    let mut result = PhonemeExtract::default();
    if program == 0 {
        return result;
    }
    let max_words = phonindex.len() / 2;

    // sound_addr / sound_param indices: 0=FMT 1=WAV 2=VWLSTART 3=VWLEND 4=ADDWAV
    let mut sound_addr: [u32; 5] = [0; 5];
    let mut sound_param: [i8; 5] = [0; 5];
    let mut change_phoneme: Option<u8> = None;
    let mut vowel_transition: [u32; 4] = [0; 4];

    const N_RETURN: usize = 10;
    let mut return_stack: [usize; N_RETURN] = [0; N_RETURN];
    let mut n_return = 0usize;

    let mut pc = program as usize;
    let mut end_flag: i32 = 0;
    let mut guard = 0usize;

    while end_flag != 1 {
        if pc >= max_words {
            break;
        }
        guard += 1;
        if guard > 8192 {
            break; // runaway guard
        }

        let instn = word_at(phonindex, pc);
        let hi4 = instn >> 12;
        let instn2 = ((instn >> 8) & 0xf) as usize;

        match hi4 {
            0 => {
                // Group 0: parameters / RETURN / IPA name.
                let data = (instn & 0xff) as usize;
                if instn2 == 0 {
                    match data {
                        0x0001 /* INSTN_RETURN */ => end_flag = 1,
                        _ => {} // INSTN_CONTINUE and others: no-op
                    }
                    pc += 1;
                } else if instn2 == I_IPA_NAME as usize {
                    // Followed by UTF-8 bytes, two per instruction word.  C caps
                    // the copy at 16 bytes (`ix < 16`), and a later `ipa` on the
                    // same path overwrites an earlier one.
                    let n = data.min(16);
                    let mut bytes = Vec::with_capacity(n);
                    for j in 0..n.div_ceil(2) {
                        let w = word_at(phonindex, pc + 1 + j);
                        bytes.push((w >> 8) as u8);
                        bytes.push((w & 0xff) as u8);
                    }
                    bytes.truncate(n);
                    result.ipa_string = String::from_utf8(bytes).ok();
                    pc += 1 + (data + 1) / 2; // header + ceil(data/2) UTF-8 words
                } else {
                    let param = (instn & 0xff) as u8;
                    match instn2 as u16 {
                        I_CHANGE_PHONEME => {
                            change_phoneme = Some(param);
                            if phoneme_list_mode {
                                // PhonemeList stage: the first ChangePhoneme wins.
                                end_flag = 1;
                            }
                        }
                        // The list-editing instructions are only *recorded* here;
                        // the caller applies them to the phoneme list (C keeps
                        // them in `phdata->pd_param[]` and `MakePhonemeList`
                        // acts on them).
                        I_REPLACE_NEXT_PHONEME => result.replace_next_phoneme = Some(param),
                        I_INSERT_PHONEME => result.insert_phoneme = Some(param),
                        I_APPEND_PHONEME => result.append_phoneme = Some(param),
                        I_APPEND_IFNEXTVOWEL => result.append_if_next_vowel = Some(param),
                        I_SET_LENGTH => result.set_length = Some(param),
                        I_ADD_LENGTH => result.add_length = param as i8,
                        I_PAUSE_BEFORE => result.pause_before_ms = param,
                        I_PAUSE_AFTER => result.pause_after_ms = param,
                        _ => {}
                    }
                    pc += 1;
                }
            }
            1 => {
                // `ChangeIf<condition>(ph)`.  C only evaluates these when it has
                // a Translator, which is exactly the translate stage
                // (`InterpretPhoneme(tr, 0x100, …)` in phonemelist.c); at
                // synthesis time `tr` is NULL and the instruction is skipped.
                if phoneme_list_mode && instn2 < 8 {
                    if stress_condition(nb, (instn2 & 7) as u16, 1, true, &lookup) {
                        change_phoneme = Some((instn & 0xff) as u8);
                        end_flag = 1; // change phoneme, exit
                    }
                }
                pc += 1;
            }
            2 | 3 => {
                // Condition sequence with a boolean accumulator.
                let mut or_flag = false;
                let mut truth = true;
                let mut cinstn = instn;
                while (cinstn & 0xe000) == COND_MARK {
                    let mut t2 = interpret_condition(pc, phonindex, nb, &lookup);
                    pc += num_instn_words(cinstn);
                    if word_at(phonindex, pc) == INSTN_NOT {
                        t2 = !t2;
                        pc += 1;
                    }
                    truth = if or_flag { truth || t2 } else { truth && t2 };
                    or_flag = (cinstn & COND_OR_FLAG) != 0;
                    cinstn = word_at(phonindex, pc);
                }
                // `pc` now points at the instruction after the condition sequence.
                if !truth {
                    if (cinstn & JUMP_FALSE_MASK) == JUMP_FALSE {
                        pc += (cinstn & 0xff) as usize;
                    } else {
                        pc += num_instn_words(cinstn);
                        if (word_at(phonindex, pc) & 0xfe00) == 0x6000 {
                            pc += 1; // skip trailing ELSE jump
                        }
                    }
                }
                // (No common increment: pc already at the next instruction.)
            }
            6 => {
                // JUMP family.
                match instn2 >> 1 {
                    0 => {
                        // Unconditional forward JUMP.
                        pc = pc.wrapping_add((instn & 0xff) as usize);
                    }
                    5 => {
                        // NextVowelStarts: pick VWLSTART by next vowel's start_type.
                        // This is the onset glide specified "for the next
                        // phoneme" → set pd_FORNEXTPH (C SwitchOnVowelType).
                        result.pd_fornextph = true;
                        if let Some((addr, param)) = switch_on_vowel_type(
                            phonindex, pc, lookup(nb.next).map(|p| p.start_type),
                        ) {
                            sound_addr[2] = addr;
                            sound_param[2] = param;
                        }
                        pc += 13; // JUMP word + 12 table words
                    }
                    6 => {
                        // PrevVowelEndings: pick VWLEND by prev vowel's end_type.
                        if let Some((addr, param)) = switch_on_vowel_type(
                            phonindex, pc, lookup(nb.prev).map(|p| p.end_type),
                        ) {
                            sound_addr[3] = addr;
                            sound_param[3] = param;
                        }
                        pc += 13;
                    }
                    _ => {
                        // Conditional jumps: already handled in the condition case.
                        pc += 1;
                    }
                }
            }
            9 => {
                // CALLPH / PITCHENV / AMPENV: instruction + 1 data word.
                let data = (((instn & 0xf) as usize) << 16) | word_at(phonindex, pc + 1) as usize;
                match instn2 {
                    1 => {
                        // Call another phoneme/procedure.
                        if n_return < N_RETURN && data > 0 && data < max_words {
                            return_stack[n_return] = pc + 2;
                            n_return += 1;
                            pc = data;
                        } else {
                            pc += 2;
                        }
                    }
                    _ => {
                        // PITCHENV (2) / AMPENV (3): not needed for FMT selection.
                        pc += 2;
                    }
                }
            }
            0xa => {
                // VOWELIN (0xa1) / VOWELOUT (0xa2): 4-word transition data.
                let w = |o: usize| word_at(phonindex, pc + o) as u32;
                let t0 = ((instn as u32 & 0xff) << 16) | w(1);
                let t1 = (w(2) << 16) | w(3);
                match instn2 {
                    1 => {
                        vowel_transition[0] = t0;
                        vowel_transition[1] = t1;
                    }
                    2 => {
                        vowel_transition[2] = t0;
                        vowel_transition[3] = t1;
                    }
                    _ => {}
                }
                pc += 4;
            }
            0xb | 0xc | 0xd | 0xe | 0xf => {
                // FMT / WAV / VWLSTART / VWLEND / ADDWAV.
                let idx = (hi4 - 0xb) as usize; // 0..4
                let addr = ((instn & 0xf) as u32) << 18 | ((word_at(phonindex, pc + 1) as u32) << 2);
                sound_addr[idx] = addr;
                sound_param[idx] = ((instn >> 4) & 0xff) as i8; // signed char
                let after = word_at(phonindex, pc + 2);
                if after != INSTN_CONTINUE {
                    if idx < 2 {
                        // FMT() and WAV() imply Return.
                        end_flag = 1;
                        if (after >> 12) == 0xf {
                            end_flag = 2; // Return after the following addWav()
                        }
                    } else if idx == 4 {
                        // addWav(): return if the previous instruction was FMT/WAV.
                        end_flag -= 1;
                    }
                }
                pc += 2;
            }
            _ => {
                pc += num_instn_words(instn).max(1);
            }
        }

        // Return from a called procedure/phoneme (mirrors the loop-bottom pop).
        if end_flag == 1 && n_return > 0 {
            end_flag = 0;
            n_return -= 1;
            pc = return_stack[n_return];
        }
    }

    if sound_addr[0] != 0 {
        result.fmt_addr = Some(sound_addr[0]);
        result.fmt_param = sound_param[0];
    }
    if sound_addr[1] != 0 {
        result.wav_addr = Some(sound_addr[1]);
        result.wav_param = sound_param[1];
    } else if sound_addr[4] != 0 {
        // addWav with no separate WAV: treat as the mixed-in sample (parity with
        // scan_phoneme's WAVADD handling).
        result.wav_addr = Some(sound_addr[4]);
        result.wav_param = sound_param[4];
    }
    if sound_addr[2] != 0 {
        result.vwlstart_addr = Some(sound_addr[2]);
    }
    if sound_addr[3] != 0 {
        result.vwlending_addr = Some(sound_addr[3]);
    }
    result.change_phoneme_code = change_phoneme;
    result.vowel_transition = vowel_transition;
    result
}

/// Evaluate the `SwitchOnVowelType` table at `pc` (a type-5/6 JUMP) for a given
/// neighbour vowel category (`start_type` for NextVowelStarts, `end_type` for
/// PrevVowelEndings).  Returns `(byte_addr, param)` or `None` if the neighbour
/// is not a vowel category or is from another table.  Mirrors
/// `SwitchOnVowelType()` in synthdata.c.
fn switch_on_vowel_type(
    phonindex: &[u8],
    pc: usize,
    voweltype_raw: Option<u8>,
) -> Option<(u32, i8)> {
    let vt = voweltype_raw? as i32 - PHON_VOWEL_TYPES;
    if !(0..6).contains(&vt) {
        return None;
    }
    let base = pc + (vt as usize) * 2;
    let p1 = word_at(phonindex, base + 1) as u32;
    let p2 = word_at(phonindex, base + 2) as u32;
    let addr = (((p1 & 0xf) << 16) + p2) * 4;
    let param = ((p1 >> 4) & 0xff) as i8;
    Some((addr, param))
}

/// Evaluate a single condition instruction at word-index `pc`.  Mirrors
/// `InterpretCondition()` (synthesis stage) from synthdata.c.
fn interpret_condition<F>(
    pc: usize,
    phonindex: &[u8],
    nb: &Neighbours,
    lookup: &F,
) -> bool
where
    F: Fn(u8) -> Option<PhonemeTab>,
{
    // Resolve a phoneme code to its table entry; code 0 → synthetic pause.
    let resolve = |code: u8| -> PhonemeTab {
        if code == 0 {
            PhonemeTab::default() // type 0 == phPAUSE, mnemonic 0
        } else {
            lookup(code).unwrap_or_default()
        }
    };

    let instn = word_at(phonindex, pc) & 0xfff;
    let mut data = instn & 0xff;
    let instn2 = (instn >> 8) as usize; // 0..15

    if instn2 >= 14 {
        // Other conditions (PreVoicing / Klatt / Mbrola): false at synthesis.
        return false;
    }

    let mut which = instn2 % 7;
    if which == 6 {
        // Extended 'which' in the following word (nextVowel/prevVowel/…).
        which = word_at(phonindex, pc + 1) as usize;
    }

    // Word-boundary guards for the *PhW variants (C `sourceix` checks).
    let mut check_endtype = false;
    let code: u8 = match which {
        0 => {
            check_endtype = true;
            nb.prev
        }
        5 => {
            if nb.this_wordstart {
                return false; // prevPhW across a word boundary
            }
            check_endtype = true;
            nb.prev
        }
        1 => nb.this,
        2 => nb.next,
        4 => {
            if nb.next_wordstart {
                return false; // nextPhW across a word boundary
            }
            nb.next
        }
        3 => nb.next2,
        6 => {
            if nb.next_wordstart || nb.next2_wordstart {
                return false; // next2PhW across a word boundary
            }
            nb.next2
        }
        7 => {
            // nextVowel: the next vowel in this word, scanned by the caller.
            if nb.vowels_tracked {
                if nb.next_vowel == 0 {
                    return false; // no further vowel in this word
                }
                nb.next_vowel
            } else {
                // Not tracked: look within the two-phoneme window instead.
                if nb.next_wordstart {
                    return false;
                }
                if resolve(nb.next).typ == PH_VOWEL {
                    nb.next
                } else {
                    if nb.next2_wordstart {
                        return false;
                    }
                    if resolve(nb.next2).typ == PH_VOWEL {
                        nb.next2
                    } else {
                        return false; // no vowel within window
                    }
                }
            }
        }
        8 => {
            // prevVowel in this word (C `worddata->prev_vowel`), or — when the
            // caller doesn't track it — the previous phoneme if it is a vowel.
            check_endtype = true;
            if nb.vowels_tracked {
                if nb.prev_vowel == 0 {
                    return false;
                }
                nb.prev_vowel
            } else if resolve(nb.prev).typ == PH_VOWEL {
                nb.prev
            } else {
                return false;
            }
        }
        // next3PhW (9) / prev2PhW (10): outside our window → conservative false.
        _ => return false,
    };

    let ph = resolve(code);

    if instn2 < 7 {
        // 'data' is a phoneme number (or a vowel-type value).
        if let Some(target) = lookup(data as u8) {
            if target.mnemonic != 0 && target.mnemonic == ph.mnemonic {
                return true;
            }
        }
        if check_endtype && ph.typ == PH_VOWEL {
            return data as u8 == ph.end_type; // prevPh() match on end_type
        }
        return data as u8 == ph.start_type; // thisPh()/nextPh() match on start_type
    }

    // Attribute conditions.
    data = instn & 0x1f;
    match instn & 0xe0 {
        COND_IS_TYPE => ph.typ as u16 == data,
        COND_IS_PLACE => ((ph.phflags >> 16) & 0xf) as u16 == data,
        COND_IS_FLAG => (ph.phflags & (1u32 << data)) != 0,
        COND_IS_OTHER => match data {
            0..=4 => stress_condition(nb, data, which, false, &lookup),
            OTHER_IS_AFTER_STRESS => false, // needs history outside our window
            OTHER_IS_NOT_VOWEL => ph.typ != PH_VOWEL,
            OTHER_IS_FINAL_VOWEL => {
                // C scans to the end of the *word*: the last vowel in it is
                // final, whatever follows in the next word.  Turkish turns
                // every word-final `i`/`e`/`o` into `I`/`E`/`O` this way, so
                // the two-phoneme window read `eksi işareti` as `Eks'i` —
                // seeing the next word's vowel and deciding it was not final.
                if nb.vowels_tracked {
                    nb.next_vowel == 0
                } else {
                    let n = resolve(nb.next);
                    let n2 = resolve(nb.next2);
                    !(n.typ == PH_VOWEL || n2.typ == PH_VOWEL)
                }
            }
            OTHER_IS_VOICED => {
                ph.typ == PH_VOWEL || ph.typ == PH_LIQUID || (ph.phflags & PH_VOICED_FLAG) != 0
            }
            OTHER_IS_BREAK => ph.typ == PH_PAUSE,
            OTHER_IS_WORD_START => match which {
                0 | 5 => nb.prev_wordstart,
                1 => nb.this_wordstart,
                2 | 4 => nb.next_wordstart,
                3 | 6 => nb.next2_wordstart,
                _ => false,
            },
            OTHER_IS_WORD_END => {
                // The phoneme after the moved position starts a new word / pause.
                match which {
                    1 => nb.next == 0 || nb.next_wordstart || resolve(nb.next).typ == PH_PAUSE,
                    0 | 5 => nb.this_wordstart,
                    _ => false,
                }
            }
            OTHER_IS_TRANSLATION => which == 1 && nb.translation_given,
            OTHER_IS_FIRST_VOWEL => which == 1 && nb.vowel_position == 1,
            OTHER_IS_SECOND_VOWEL => which == 1 && nb.vowel_position == 2,
            _ => false,
        },
        _ => false,
    }
}

/// `StressCondition()` from synthdata.c.
///
/// The level is read from this phoneme when it is a vowel and otherwise from the
/// phoneme after it; a consonant with no following vowel has no stress level and
/// the condition is false.  `is_change_if` marks the `ChangeIf…` caller, which C
/// additionally refuses for a word whose phonemes came from the dictionary list
/// unless the language sets `LOPT_REDUCE & 1`.
fn stress_condition<F>(
    nb: &Neighbours,
    condition: u16,
    which: usize,
    is_change_if: bool,
    lookup: &F,
) -> bool
where
    F: Fn(u8) -> Option<PhonemeTab>,
{
    const CONDITION_LEVEL: [u16; 4] = [1, 2, 4, 15];
    let is_vowel = |c: u8| lookup(c).is_some_and(|p| p.typ == PH_VOWEL);
    let known = |v: u8| (v != STRESS_UNKNOWN).then_some(v);

    // C moves `plist` to the phoneme the condition names *before* reading its
    // stress level, so `nextVowel(isMaxStress)` asks about the next vowel.
    let selected = match which {
        0 | 5 => known(nb.prev_stress),
        8 => known(nb.prev_vowel_stress).or_else(|| known(nb.prev_stress)),
        // (7 handled below, where the tracked vowel wins.)
        2 | 4 => known(nb.next_stress),
        3 | 6 => known(nb.next2_stress),
        7 => {
            if nb.vowels_tracked {
                known(nb.next_vowel_stress)
            } else if is_vowel(nb.next) {
                known(nb.next_stress)
            } else {
                known(nb.next2_stress)
            }
        }
        _ => None, // `thisPh`, handled below
    };

    let mut level = if let Some(v) = selected {
        v
    } else if is_vowel(nb.this) {
        nb.stress
    } else if nb.next_stress == STRESS_UNKNOWN {
        // The caller tracks no per-syllable stress (synthesis stage): keep the
        // phoneme's own level, which is what the FMT-selecting conditions want.
        nb.stress
    } else if is_vowel(nb.next) {
        nb.next_stress
    } else {
        return false; // no stress level for this consonant
    };

    if is_change_if && nb.translation_given && nb.reduce & 1 == 0 {
        // "Don't change phonemes which are given for the word in the dictionary."
        return false;
    }
    let wordstress = if nb.wordstress == STRESS_UNKNOWN { 4 } else { nb.wordstress };
    if nb.reduce & 2 != 0 && level >= wordstress {
        // The most stressed syllable of an unstressed word counts as stressed.
        level = 4; // STRESS_IS_PRIMARY
    }

    let level = (level as u16) & 0xf;
    match condition {
        4 /* STRESS_IS_PRIMARY */ => level >= wordstress as u16,
        3 /* STRESS_IS_SECONDARY */ => level > 3,
        c => level < CONDITION_LEVEL[(c & 3) as usize],
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// A phoneme-program instruction for the program compiler — the phoneme-bytecode
/// half of `compiledata.c`, and the inverse of `scan_phoneme`/`InterpretPhoneme`.
#[derive(Debug, Clone, Copy)]
pub enum ProgInstn {
    /// `i_FMT`: a formant-frame-sequence address in `phondata` + a length param.
    Fmt { addr: u32, param: i8 },
    /// `i_WAV`: a WAV-sample address in `phondata`.
    Wav { addr: u32 },
    /// `i_VWLSTART`: the vowel-onset transition frame-sequence address.
    VwlStart { addr: u32 },
    /// `i_VWLENDING`: the vowel-exit transition frame-sequence address.
    VwlEnding { addr: u32 },
    /// `i_CALLPH`: call another phoneme program by index (its FMT/WAV/… are
    /// merged into the caller by `scan_phoneme`).
    CallPh { program: u16 },
    /// `INSTN_CONTINUE`: fall through to the next instruction.
    Continue,
    /// A raw instruction word (for opcodes not modelled here).
    Raw(u16),
    /// `RETURN`.
    Return,
}

/// Compile a phoneme program to `phonindex` bytecode (little-endian `u16`
/// words) — the inverse of the `InterpretPhoneme`/`scan_phoneme` reader.
///
/// Address encoding matches the reader: `i_FMT`/`i_WAV` store `param` in bits
/// 11-4, the address high bits (`>>18`) in bits 3-0, and `(addr>>2)&0xffff` in
/// the following word.  Minimal opcode set; conditions/jumps/CALLPH/IPA-name are
/// not yet modelled (emit them via [`ProgInstn::Raw`]).
pub fn compile_program(instns: &[ProgInstn]) -> Vec<u8> {
    let mut out = Vec::new();
    for instn in instns {
        match *instn {
            ProgInstn::Fmt { addr, param } => {
                let word = I_FMT
                    | (((param as u8) as u16) << 4)
                    | (((addr >> 18) & 0xf) as u16);
                out.extend_from_slice(&word.to_le_bytes());
                out.extend_from_slice(&(((addr >> 2) & 0xffff) as u16).to_le_bytes());
            }
            ProgInstn::Wav { addr } => {
                out.extend_from_slice(&(I_WAV | (((addr >> 18) & 0xf) as u16)).to_le_bytes());
                out.extend_from_slice(&(((addr >> 2) & 0xffff) as u16).to_le_bytes());
            }
            ProgInstn::VwlStart { addr } => {
                out.extend_from_slice(&(I_VWLSTART | (((addr >> 18) & 0xf) as u16)).to_le_bytes());
                out.extend_from_slice(&(((addr >> 2) & 0xffff) as u16).to_le_bytes());
            }
            ProgInstn::VwlEnding { addr } => {
                out.extend_from_slice(&(I_VWLENDING | (((addr >> 18) & 0xf) as u16)).to_le_bytes());
                out.extend_from_slice(&(((addr >> 2) & 0xffff) as u16).to_le_bytes());
            }
            ProgInstn::CallPh { program } => {
                // `i_CALLPH` (0x9100) then the callee's program index; the reader
                // recovers it as `((instn & 0xf) << 16) | next`, so a `u16` index
                // fits entirely in the following word.
                out.extend_from_slice(&I_CALLPH.to_le_bytes());
                out.extend_from_slice(&program.to_le_bytes());
            }
            ProgInstn::Continue => out.extend_from_slice(&INSTN_CONTINUE.to_le_bytes()),
            ProgInstn::Raw(w) => out.extend_from_slice(&w.to_le_bytes()),
            ProgInstn::Return => out.extend_from_slice(&INSTN_RETURN.to_le_bytes()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compile_program_round_trip() {
        // Compile [i_FMT(addr, param), RETURN], place at program=1, scan it back.
        let addr = 0x1234u32 * 4; // word-aligned phondata address
        let param = 42i8;
        let bytecode = compile_program(&[ProgInstn::Fmt { addr, param }, ProgInstn::Return]);
        let phonindex = pad(&bytecode);
        let r = scan_phoneme(PROG, &phonindex);
        assert_eq!(r.fmt_addr, Some(addr), "FMT address did not round-trip");
        assert_eq!(r.fmt_param, param, "FMT param did not round-trip");
        assert!(r.wav_addr.is_none());
    }

    #[test]
    fn compile_program_vwl_and_continue_round_trip() {
        // [VWLSTART, VWLENDING, CONTINUE, FMT, RETURN] — the VWL addresses read
        // back, and CONTINUE must not halt the scan (FMT after it is still found).
        let vs = 0x1000u32;
        let ve = 0x2000u32;
        let fmt = 0x3000u32;
        let bytecode = compile_program(&[
            ProgInstn::VwlStart { addr: vs },
            ProgInstn::VwlEnding { addr: ve },
            ProgInstn::Continue,
            ProgInstn::Fmt { addr: fmt, param: 7 },
            ProgInstn::Return,
        ]);
        let phonindex = pad(&bytecode);
        let r = scan_phoneme(PROG, &phonindex);
        assert_eq!(r.vwlstart_addr, Some(vs), "VWLSTART did not round-trip");
        assert_eq!(r.vwlending_addr, Some(ve), "VWLENDING did not round-trip");
        assert_eq!(r.fmt_addr, Some(fmt), "FMT after CONTINUE was not reached");
        assert_eq!(r.fmt_param, 7);
    }

    #[test]
    fn compile_program_callph_round_trip() {
        // main = [CALLPH sub, RETURN]; sub = [FMT, RETURN].  scan_phoneme(main)
        // must follow the call and surface the called program's FMT.
        // Layout: pad(word 0) + main(words 1–3) + sub(words 4–6) → sub at word 4.
        let sub_off = 4u16;
        let addr = 0x2000u32;
        let main = compile_program(&[ProgInstn::CallPh { program: sub_off }, ProgInstn::Return]);
        assert_eq!(main.len(), 6, "CALLPH+RETURN should be 3 words"); // sub_off assumption
        let sub = compile_program(&[ProgInstn::Fmt { addr, param: 9 }, ProgInstn::Return]);

        let mut bytes = main;
        bytes.extend_from_slice(&sub);
        let phonindex = pad(&bytes);

        let r = scan_phoneme(PROG, &phonindex);
        assert_eq!(r.fmt_addr, Some(addr), "CALLPH did not resolve the callee's FMT");
        assert_eq!(r.fmt_param, 9);
    }

    // All test phonindices start with one padding word (2 bytes) so that
    // program=1 points to the first instruction.  (Program 0 is reserved as
    // "no program" in espeak-ng, so scan_phoneme(0, ...) returns empty.)

    const PROG: u16 = 1; // program index used in all tests

    fn pad(insns: &[u8]) -> Vec<u8> {
        let mut v = vec![0u8, 0u8]; // word 0 = padding
        v.extend_from_slice(insns);
        v
    }

    /// Build a minimal phonindex with a single i_FMT instruction at program=1.
    fn make_fmt_phonindex(fmt_addr: u32) -> Vec<u8> {
        let instn: u16 = 0xb000 | (((fmt_addr >> 18) & 0xf) as u16);
        let next:  u16 = ((fmt_addr >> 2) & 0xffff) as u16;
        let mut insns = vec![0u8; 4];
        insns[0..2].copy_from_slice(&instn.to_le_bytes());
        insns[2..4].copy_from_slice(&next.to_le_bytes());
        pad(&insns)
    }

    /// Build phonindex with: [i_IPA_NAME(2 bytes) | i_FMT]
    fn make_ipa_then_fmt(fmt_addr: u32) -> Vec<u8> {
        let ipa_instn: u16 = ((I_IPA_NAME as u16) << 8) | 2; // 0x0d02
        let ipa_data:  u16 = u16::from_be_bytes([b'e', b':']);
        let fmt_instn: u16 = 0xb000 | (((fmt_addr >> 18) & 0xf) as u16);
        let fmt_next:  u16 = ((fmt_addr >> 2) & 0xffff) as u16;
        let mut insns = vec![0u8; 8];
        insns[0..2].copy_from_slice(&ipa_instn.to_le_bytes());
        insns[2..4].copy_from_slice(&ipa_data.to_le_bytes());
        insns[4..6].copy_from_slice(&fmt_instn.to_le_bytes());
        insns[6..8].copy_from_slice(&fmt_next.to_le_bytes());
        pad(&insns)
    }

    #[test]
    fn scan_simple_fmt() {
        let addr = 0x1234u32 * 4;
        let phonindex = make_fmt_phonindex(addr);
        let result = scan_phoneme(PROG, &phonindex);
        assert_eq!(result.fmt_addr, Some(addr));
        assert!(result.wav_addr.is_none());
    }

    #[test]
    fn scan_extracts_vowel_transition() {
        // i_VOWELIN (0xa1) with high byte 0x0a, then 3 data words, then RETURN.
        // C: vt[0] = ((prog0 & 0xff) << 16) + prog1 = 0x0a1234
        //    vt[1] = (prog2 << 16) + prog3          = 0x12345678
        let words: [u16; 5] = [0xa10a, 0x1234, 0x1234, 0x5678, INSTN_RETURN];
        let mut insns = Vec::new();
        for w in &words {
            insns.extend_from_slice(&w.to_le_bytes());
        }
        let r = scan_phoneme(PROG, &pad(&insns));
        assert_eq!(r.vowel_transition[0], 0x0a_1234);
        assert_eq!(r.vowel_transition[1], 0x1234_5678);
        assert_eq!(r.vowel_transition[2], 0); // no VOWELOUT
        assert_eq!(r.vowel_transition[3], 0);
    }

    #[test]
    fn scan_ipa_then_fmt() {
        let addr = 0x5678u32 * 4;
        let phonindex = make_ipa_then_fmt(addr);
        let result = scan_phoneme(PROG, &phonindex);
        assert_eq!(result.fmt_addr, Some(addr));
    }

    #[test]
    fn scan_zero_program_returns_empty() {
        let phonindex = vec![0u8; 4];
        let result = scan_phoneme(0, &phonindex);   // 0 = "no program"
        assert!(result.fmt_addr.is_none());
        assert!(result.wav_addr.is_none());
    }

    #[test]
    fn scan_return_stops_early() {
        // RETURN then FMT — should not find FMT
        let addr = 0x1000u32 * 4;
        let fmt_instn: u16 = 0xb000 | (((addr >> 18) & 0xf) as u16);
        let fmt_next:  u16 = ((addr >> 2) & 0xffff) as u16;
        let mut insns = vec![0u8; 8];
        insns[0..2].copy_from_slice(&INSTN_RETURN.to_le_bytes());
        insns[2..4].copy_from_slice(&[0, 0]);
        insns[4..6].copy_from_slice(&fmt_instn.to_le_bytes());
        insns[6..8].copy_from_slice(&fmt_next.to_le_bytes());
        let result = scan_phoneme(PROG, &pad(&insns));
        assert!(result.fmt_addr.is_none(), "RETURN should stop scanner");
    }

    #[test]
    fn num_instn_words_fmt() {
        // i_FMT = 0xb000 → 2 words
        assert_eq!(num_instn_words(0xb000), 2);
        assert_eq!(num_instn_words(0xb123), 2);
    }

    #[test]
    fn num_instn_words_vowelin() {
        // 0xa100 → 4 words
        assert_eq!(num_instn_words(0xa100), 4);
        assert_eq!(num_instn_words(0xa200), 4);
    }

    #[test]
    fn num_instn_words_ipa_name_4bytes() {
        // IPA_NAME with 4 bytes of data: 1 header + ceil(4/2) = 1 + 2 = 3 words
        let instn: u16 = ((I_IPA_NAME as u16) << 8) | 4;
        assert_eq!(num_instn_words(instn), 3);
    }

    #[test]
    fn num_instn_words_callph() {
        assert_eq!(num_instn_words(0x9100), 2);
        assert_eq!(num_instn_words(0x9200), 2);
        assert_eq!(num_instn_words(0x9300), 2);
    }

    #[test]
    fn scan_wav_only() {
        let addr = 0x2000u32 * 4;
        let wav_instn: u16 = 0xc000 | (((addr >> 18) & 0xf) as u16);
        let wav_next:  u16 = ((addr >> 2) & 0xffff) as u16;
        let mut insns = vec![0u8; 4];
        insns[0..2].copy_from_slice(&wav_instn.to_le_bytes());
        insns[2..4].copy_from_slice(&wav_next.to_le_bytes());
        let result = scan_phoneme(PROG, &pad(&insns));
        assert!(result.fmt_addr.is_none());
        assert_eq!(result.wav_addr, Some(addr));
    }

    #[test]
    fn scan_wavadd_after_fmt() {
        let fmt_a = 0x1000u32 * 4;
        let wav_a = 0x2000u32 * 4;
        let fmt_instn: u16 = 0xb000 | (((fmt_a >> 18) & 0xf) as u16);
        let fmt_next:  u16 = ((fmt_a >> 2) & 0xffff) as u16;
        let add_instn: u16 = 0xf000 | (((wav_a >> 18) & 0xf) as u16);
        let add_next:  u16 = ((wav_a >> 2) & 0xffff) as u16;
        let mut insns = vec![0u8; 8];
        insns[0..2].copy_from_slice(&fmt_instn.to_le_bytes());
        insns[2..4].copy_from_slice(&fmt_next.to_le_bytes());
        insns[4..6].copy_from_slice(&add_instn.to_le_bytes());
        insns[6..8].copy_from_slice(&add_next.to_le_bytes());
        let result = scan_phoneme(PROG, &pad(&insns));
        assert_eq!(result.fmt_addr, Some(fmt_a));
        assert_eq!(result.wav_addr, Some(wav_a));
    }

    // ── interpret_phoneme (context-aware) ─────────────────────────────────

    fn mk_ph(typ: u8, mnem: &str) -> PhonemeTab {
        PhonemeTab {
            typ,
            mnemonic: PhonemeTab::pack_mnemonic(mnem),
            ..Default::default()
        }
    }

    fn words_bytes(words: &[u16]) -> Vec<u8> {
        let mut v = vec![0u8, 0u8]; // pad word 0
        for w in words {
            v.extend_from_slice(&w.to_le_bytes());
        }
        v
    }

    #[test]
    fn interpret_matches_scan_for_plain_fmt() {
        let addr = 0x1234u32 * 4;
        let phonindex = make_fmt_phonindex(addr);
        let nb = Neighbours::default();
        let r = interpret_phoneme(PROG, &phonindex, &nb, |_| None);
        assert_eq!(r.fmt_addr, Some(addr));
    }

    #[test]
    fn interpret_picks_branch_by_next_vowel() {
        // Mirrors the `l` pattern:
        //   IF nextPh(isNotVowel) THEN FMT(A) RETURN ENDIF
        //   FMT(B)
        // A linear scanner grabs FMT(A); the interpreter must pick FMT(B) when
        // the next phoneme is a vowel.
        let a = 0x1000u32 * 4;
        let b = 0x2000u32 * 4;
        let cond = 0x298a; // condition: nextPh(isNotVowel)
        let jf = JUMP_FALSE | 3; // if false, skip 3 words → FMT(B)
        let fmt_a_i = 0xb000 | (((a >> 18) & 0xf) as u16);
        let fmt_a_n = ((a >> 2) & 0xffff) as u16;
        let fmt_b_i = 0xb000 | (((b >> 18) & 0xf) as u16);
        let fmt_b_n = ((b >> 2) & 0xffff) as u16;
        let phonindex = words_bytes(&[cond, jf, fmt_a_i, fmt_a_n, fmt_b_i, fmt_b_n]);

        let lookup = |c: u8| -> Option<PhonemeTab> {
            match c {
                20 => Some(mk_ph(PH_VOWEL, "a")),
                30 => Some(mk_ph(4, "t")), // voiceless stop
                _ => None,
            }
        };

        // next = vowel → condition false → default FMT(B)
        let nb_v = Neighbours { this: 10, next: 20, ..Default::default() };
        let r = interpret_phoneme(PROG, &phonindex, &nb_v, &lookup);
        assert_eq!(r.fmt_addr, Some(b), "next=vowel should reach default FMT(B)");

        // next = consonant → condition true → FMT(A)
        let nb_c = Neighbours { this: 10, next: 30, ..Default::default() };
        let r = interpret_phoneme(PROG, &phonindex, &nb_c, &lookup);
        assert_eq!(r.fmt_addr, Some(a), "next=consonant should reach FMT(A)");
    }

    #[test]
    fn interpret_prevph_code_match() {
        // IF prevPh(t) THEN FMT(A) RETURN ENDIF ; FMT(B)
        // prevPh phoneme-code match: which=0 (prevPh), phoneme-code mode.
        // instn2 = 0, code byte = phoneme code of 't' (= 30 here).
        let a = 0x1000u32 * 4;
        let b = 0x2000u32 * 4;
        let cond = 0x2000 | 30; // prevPh(code 30)
        let jf = JUMP_FALSE | 3;
        let fmt_a_i = 0xb000 | (((a >> 18) & 0xf) as u16);
        let fmt_a_n = ((a >> 2) & 0xffff) as u16;
        let fmt_b_i = 0xb000 | (((b >> 18) & 0xf) as u16);
        let fmt_b_n = ((b >> 2) & 0xffff) as u16;
        let phonindex = words_bytes(&[cond, jf, fmt_a_i, fmt_a_n, fmt_b_i, fmt_b_n]);

        let lookup = |c: u8| -> Option<PhonemeTab> {
            match c {
                30 => Some(mk_ph(4, "t")),
                31 => Some(mk_ph(4, "p")),
                _ => None,
            }
        };

        // prev = t → FMT(A)
        let nb_t = Neighbours { this: 10, prev: 30, ..Default::default() };
        let r = interpret_phoneme(PROG, &phonindex, &nb_t, &lookup);
        assert_eq!(r.fmt_addr, Some(a), "prev=t should reach FMT(A)");

        // prev = p → FMT(B)
        let nb_p = Neighbours { this: 10, prev: 31, ..Default::default() };
        let r = interpret_phoneme(PROG, &phonindex, &nb_p, &lookup);
        assert_eq!(r.fmt_addr, Some(b), "prev=p should reach default FMT(B)");
    }

    #[test]
    fn interpret_callph_returns_and_continues() {
        // CALLPH(sub) then FMT(main). The called sub does FMT(SUB); RETURN.
        // After the call returns, the main FMT must win (last FMT set).
        let main_addr = 0x3000u32 * 4;
        let sub_addr = 0x4000u32 * 4;
        // Layout (word indices, pad at 0):
        //  1: CALLPH hi          2: CALLPH data(=6, the sub program index)
        //  3: FMT(main) instn    4: FMT(main) addr
        //  5: RETURN
        //  6: FMT(sub) instn     7: FMT(sub) addr   (the called procedure)
        let callph_i = 0x9100u16; // CALLPH, data hi nibble = 0
        let callph_d = 6u16; // sub program word index
        let fmt_m_i = 0xb000 | (((main_addr >> 18) & 0xf) as u16);
        let fmt_m_n = ((main_addr >> 2) & 0xffff) as u16;
        let fmt_s_i = 0xb000 | (((sub_addr >> 18) & 0xf) as u16);
        let fmt_s_n = ((sub_addr >> 2) & 0xffff) as u16;
        let phonindex = words_bytes(&[
            callph_i, callph_d, fmt_m_i, fmt_m_n, INSTN_RETURN, fmt_s_i, fmt_s_n,
        ]);
        let nb = Neighbours { this: 10, ..Default::default() };
        let r = interpret_phoneme(PROG, &phonindex, &nb, |_| None);
        // The sub sets FMT(sub); after RETURN the main FMT overwrites it.
        assert_eq!(r.fmt_addr, Some(main_addr), "main FMT should win after CALLPH");
    }
}
