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

// ---------------------------------------------------------------------------
// Instruction opcode constants (mirrors synthesize.h)
// ---------------------------------------------------------------------------

pub const INSTN_RETURN:   u16 = 0x0001;
pub const INSTN_CONTINUE: u16 = 0x0002;

pub const I_IPA_NAME:       u16 = 0x0d;  // group-0, operand = UTF-8 byte count
pub const I_CHANGE_PHONEME: u16 = 0x01;  // group-0: (opcode<<8)|phoneme_code
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
                // i_FMT implies RETURN (unless followed by i_WAVADD)
                // Check if the next meaningful instruction is WAVADD; if not, stop.
                if pc + 2 < max_words {
                    let next2 = u16::from_le_bytes([
                        phonindex[(pc+2)*2],
                        phonindex[(pc+2)*2 + 1],
                    ]);
                    if next2 >> 12 == 0xf {
                        // i_WAVADD follows: continue to capture wav_addr
                        pc += 2;
                        continue;
                    }
                }
                break; // i_FMT implies RETURN
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
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

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
}
