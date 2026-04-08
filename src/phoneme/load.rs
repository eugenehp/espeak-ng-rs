//! Binary phoneme data loader.
//!
//! Port of `LoadPhData()`, `ReadPhFile()`, and `SelectPhonemeTable()` from
//! `synthdata.c`.  Reads `phontab`, `phonindex`, `phondata`, and
//! `intonations` from the espeak-ng data directory.
//
// Reads four binary files from the espeak-ng-data directory:
//
//   phontab      — phoneme table list (parsed here)
//   phonindex    — u16 offsets into phondata (kept as raw bytes)
//   phondata     — synthesis waveform/formant data (kept as raw bytes)
//   intonations  — array of TUNE structs (kept as raw bytes)

use std::path::Path;

use crate::error::{Error, Result};
use super::{
    N_PHONEME_TAB, N_PHONEME_TAB_NAME, VERSION_PHDATA,
    table::{PhonemeTab, PhonemeTabList},
};

const PHONEME_TAB_ENTRY_SIZE: usize = 16;

// ---------------------------------------------------------------------------
// PhonemeData — the fully loaded dataset
// ---------------------------------------------------------------------------

/// All phoneme data loaded from an espeak-ng-data directory.
///
/// Mirrors the combination of `phoneme_tab_list`, `phoneme_tab`,
/// `phoneme_index`, `phondata_ptr`, `tunes`, `n_tunes`, and `samplerate`
/// from `synthdata.c`.
pub struct PhonemeData {
    /// All phoneme table definitions parsed from `phontab`.
    pub tables: Vec<PhonemeTabList>,
    /// Sample rate read from `phondata` header (typically 22050).
    pub sample_rate: u32,

    /// Raw bytes of `phondata` (waveform / formant data).
    pub phondata: Vec<u8>,
    /// Raw bytes of `phonindex` (u16 offsets into phondata).
    pub phonindex: Vec<u8>,
    /// Raw bytes of `intonations` (array of TUNE structs, 68 bytes each).
    pub intonations: Vec<u8>,

    /// Currently selected table index (–1 = none selected yet).
    current_table: i32,
    /// For each phoneme code (0-255): which (table_idx, entry_idx) is active.
    /// `None` means "no phoneme with that code in the active set".
    active: Box<[Option<(usize, usize)>; N_PHONEME_TAB]>,
}

impl PhonemeData {
    // -----------------------------------------------------------------------
    // Loading
    // -----------------------------------------------------------------------

    /// Load all phoneme data from the given data directory.
    ///
    /// Mirrors `LoadPhData()` from `synthdata.c`.
    pub fn load(data_dir: &Path) -> Result<Self> {
        let phoneme_tab_data = read_file(data_dir, "phontab")?;
        let phonindex        = read_file(data_dir, "phonindex")?;
        let phondata         = read_file(data_dir, "phondata")?;
        let intonations      = read_file(data_dir, "intonations")?;

        // Verify version magic from phondata bytes 0-3
        if phondata.len() < 8 {
            return Err(Error::InvalidData("phondata too short".into()));
        }
        let version = u32::from_le_bytes(phondata[0..4].try_into().unwrap());
        if version != VERSION_PHDATA {
            return Err(Error::VersionMismatch { got: version, expected: VERSION_PHDATA });
        }
        let sample_rate = u32::from_le_bytes(phondata[4..8].try_into().unwrap());

        // Parse phontab
        let tables = parse_phontab(&phoneme_tab_data)?;

        let active = Box::new([None; N_PHONEME_TAB]);

        Ok(Self {
            tables,
            sample_rate,
            phondata,
            phonindex,
            intonations,
            current_table: -1,
            active,
        })
    }

    // -----------------------------------------------------------------------
    // Table selection — mirrors SelectPhonemeTable / SetUpPhonemeTable
    // -----------------------------------------------------------------------

    /// Select the active phoneme table by index.
    ///
    /// Mirrors `SelectPhonemeTable(number)` from `synthdata.c`.
    /// Idempotent: calling with the same index twice is a no-op.
    pub fn select_table(&mut self, number: usize) -> Result<()> {
        if self.current_table == number as i32 {
            return Ok(());
        }
        if number >= self.tables.len() {
            return Err(Error::InvalidData(
                format!("phoneme table index {number} out of range (have {})", self.tables.len())
            ));
        }
        *self.active = [None; N_PHONEME_TAB];
        self.setup_table_recursive(number);
        self.current_table = number as i32;
        Ok(())
    }

    /// Select the active phoneme table by name.
    ///
    /// Mirrors `SelectPhonemeTableName(name)`.
    pub fn select_table_by_name(&mut self, name: &str) -> Result<usize> {
        let idx = self.find_table(name)?;
        self.select_table(idx)?;
        Ok(idx)
    }

    /// Find a table index by name without selecting it.
    pub fn find_table(&self, name: &str) -> Result<usize> {
        self.tables
            .iter()
            .position(|t| t.name == name)
            .ok_or_else(|| Error::InvalidData(format!("phoneme table '{name}' not found")))
    }

    // Recursive: first install any base table, then overlay this table.
    fn setup_table_recursive(&mut self, idx: usize) {
        let includes = self.tables[idx].includes;
        if includes > 0 {
            self.setup_table_recursive((includes - 1) as usize);
        }
        // Install all phonemes from this table, indexed by code.
        let n = self.tables[idx].phonemes.len();
        for entry_idx in 0..n {
            let code = self.tables[idx].phonemes[entry_idx].code as usize;
            if code < N_PHONEME_TAB {
                self.active[code] = Some((idx, entry_idx));
            }
        }
    }

    // -----------------------------------------------------------------------
    // Phoneme lookup — mirrors PhonemeCode / LookupPhonemeString
    // -----------------------------------------------------------------------

    /// Look up a phoneme by its packed mnemonic `u32`.
    ///
    /// Mirrors `PhonemeCode(mnem)`.  Returns 0 if not found.
    pub fn phoneme_code(&self, mnem: u32) -> u8 {
        for slot in self.active.iter().flatten() {
            let (table_idx, entry_idx) = *slot;
            let ph = &self.tables[table_idx].phonemes[entry_idx];
            if ph.mnemonic == mnem {
                return ph.code;
            }
        }
        0
    }

    /// Look up a phoneme code by mnemonic string (up to 4 ASCII chars).
    ///
    /// Mirrors `LookupPhonemeString(string)`.
    pub fn lookup_phoneme(&self, name: &str) -> u8 {
        self.phoneme_code(PhonemeTab::pack_mnemonic(name))
    }

    /// Get the `PhonemeTab` for a given phoneme code in the active table.
    ///
    /// Returns `None` if no phoneme with that code is active.
    pub fn get(&self, code: u8) -> Option<&PhonemeTab> {
        // Walk the active array — entries are (table_idx, entry_idx) tuples
        // stored per-code.  We track the "last write wins" semantics of C.
        self.get_from_active(code as usize)
    }

    fn get_from_active(&self, code: usize) -> Option<&PhonemeTab> {
        if code >= N_PHONEME_TAB {
            return None;
        }
        let (table_idx, entry_idx) = self.active[code]?;
        Some(&self.tables[table_idx].phonemes[entry_idx])
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    /// Number of loaded phoneme tables.
    pub fn n_tables(&self) -> usize { self.tables.len() }

    /// Number of intonation tunes (each is 68 bytes).
    pub fn n_tunes(&self) -> usize { self.intonations.len() / 68 }

    /// Raw slice of `phondata` at a byte offset.
    pub fn phondata_at(&self, offset: usize) -> &[u8] {
        &self.phondata[offset..]
    }

    /// Look up the IPA string for a phoneme by reading its i_IPA_NAME bytecode.
    ///
    /// `program` is the index into phonindex (stored in PhonemeTab.program).
    /// Returns `None` if the FIRST instruction is not i_IPA_NAME.
    ///
    /// We only check the FIRST instruction to avoid reading IPA from
    /// conditional branches in the phoneme program (which require context).
    /// This correctly handles phonemes like 'r' (program=71, first instn = i_IPA_NAME)
    /// while ignoring phonemes like 'l' (program=3486, IPA deep in conditionals).
    /// Resolve a phoneme code through synthesis-stage `ChangeIf` instructions.
    ///
    /// Some phonemes (e.g. Russian `o`, `a`) use stress-conditional
    /// `ChangeIfNotStressed(X)` or `ChangeIfStressed(X)` bytecode instructions
    /// (type-1 in the phonindex) to select between two acoustic forms.
    ///
    /// `is_stressed` is true when the phoneme is under primary stress.
    ///
    /// This implements a simplified version of C's `InterpretPhoneme2`/
    /// `InterpretPhoneme` for the stress-ChangeIf path.
    pub fn resolve_stressed_phoneme(&self, code: u8, is_stressed: bool) -> u8 {
        let Some(ph) = self.get(code) else { return code };
        if ph.program == 0 { return code; }

        // STRESS constants (from synthesize.h)
        const STRESS_IS_DIMINISHED:   u8 = 0;
        const STRESS_IS_PRIMARY:      u8 = 4;

        // condition_level[condition] table from StressCondition()
        // condition 0→1, 1→2, 2→4(PRIMARY), 3→15
        const CONDITION_LEVEL: [u8; 4] = [1, 2, 4, 15];

        let stress_level: u8 = if is_stressed { STRESS_IS_PRIMARY } else { STRESS_IS_DIMINISHED };
        let prog = ph.program as usize;
        let pi = &self.phonindex;

        // Only apply complex bytecode tracing for vowels.
        // Consonant phonemes may also have ChangePhoneme instructions (palatalization etc.)
        // but those are context-dependent and we handle them via letter_bits in the rules engine.
        if ph.typ != 2 { return code; } // not a vowel

        // i_JUMP_FALSE: type=6, instn2>>1 = 4 (0x6800..0x68ff)
        // i_CONDITION type-2 with `thisPh(isMaxStress)` = 0x2884
        // `data = instn & 0x1f` is the condition data (STRESS_IS_PRIMARY=4 for isMaxStress)
        // `instn & 0xe0 = 0x80` = CONDITION_IS_OTHER
        const THIS_PH_IS_MAX_STRESS: u16 = 0x2884; // thisPh(isMaxStress)

        let mut i = 0usize;
        while i < 16 {
            let off = (prog + i) * 2;
            if off + 2 > pi.len() { break; }
            let w = u16::from_le_bytes([pi[off], pi[off + 1]]);
            let instn_type = w >> 12;
            let instn2 = ((w >> 8) & 0xf) as u8;
            let data_u8 = (w & 0xff) as u8;   // lower 8 bits for phoneme code / jump offset

            if instn_type == 1 && instn2 < 8 {
                // ChangeIf(condition, phoneme) — type 1
                // StressCondition(condition):
                //   condition == STRESS_IS_PRIMARY(4) → fires when stressed
                //   others (0,1,2) → fires when NOT stressed
                let fires = if instn2 == STRESS_IS_PRIMARY {
                    is_stressed
                } else if (instn2 as usize) < CONDITION_LEVEL.len() {
                    stress_level < CONDITION_LEVEL[instn2 as usize]
                } else {
                    false
                };

                if fires && data_u8 != 0 {
                    if self.get(data_u8).is_some() {
                        return data_u8; // changed phoneme code
                    }
                    return code;
                }
                // Condition didn't fire; continue scanning
                i += 1;
            } else if w == THIS_PH_IS_MAX_STRESS {
                // thisPh(isMaxStress) condition block.
                // Next word should be JUMP_FALSE (0x68xx) — skip by `data` if NOT stressed.
                if !is_stressed {
                    // Condition is false when not stressed; check for JUMP_FALSE to skip
                    let next_off = (prog + i + 1) * 2;
                    if next_off + 2 <= pi.len() {
                        let jw = u16::from_le_bytes([pi[next_off], pi[next_off + 1]]);
                        if (jw & 0xf800) == 0x6800 {
                            // JUMP_FALSE: jump by data when condition is false
                            let jump = (jw & 0xff) as usize;
                            i += 2 + jump; // skip past the condition + jump + offset
                            continue;
                        }
                    }
                    // No JUMP_FALSE found; stop scan
                    break;
                } else {
                    // Condition is TRUE (stressed); skip the JUMP_FALSE instruction
                    let next_off = (prog + i + 1) * 2;
                    if next_off + 2 <= pi.len() {
                        let jw = u16::from_le_bytes([pi[next_off], pi[next_off + 1]]);
                        if (jw & 0xf800) == 0x6800 {
                            // Skip the JUMP_FALSE, continue after it
                            i += 2;
                            continue;
                        }
                    }
                    i += 1;
                }
            } else if instn_type == 2 || instn_type == 3 {
                // Other type-2/3 conditionals (context-dependent): skip both the
                // condition and any following JUMP_FALSE
                let next_off = (prog + i + 1) * 2;
                if next_off + 2 <= pi.len() {
                    let jw = u16::from_le_bytes([pi[next_off], pi[next_off + 1]]);
                    if (jw & 0xf800) == 0x6800 {
                        // Skip condition + JUMP_FALSE + skip the FALSE branch
                        let jump = (jw & 0xff) as usize;
                        // If not stressed, take the false branch (jump)
                        if !is_stressed {
                            i += 2 + jump;
                        } else {
                            i += 2; // take the true branch
                        }
                        continue;
                    }
                }
                // No JUMP_FALSE; stop scan
                break;
            } else if instn_type == 6 {
                // Unconditional jump (instn2>>1 == 0 means case 0: prog += data-1)
                if (instn2 >> 1) == 0 {
                    // Actually prog += (instn & 0xff) - 1, so jump = data_u8 - 1
                    let jump_by = (data_u8 as usize).saturating_sub(1);
                    i += 1 + jump_by;
                } else {
                    break;
                }
            } else if instn_type == 0 {
                if instn2 == 1 {
                    // ChangePhoneme(data) — type 0, instn2=1
                    if self.get(data_u8).is_some() {
                        return data_u8;
                    }
                    return code;
                }
                // Other type-0 instructions: skip
                i += 1;
            } else if instn_type >= 0xb {
                // FMT/WAV — stop
                break;
            } else {
                i += 1;
            }
        }
        code // no change
    }

    ///
    /// Mirrors the `InterpretPhoneme2(ph->code, &phdata)` path in WritePhMnemonic()
    /// in dictionary.c, which uses NULL context (unconditional path only).
    pub fn phoneme_ipa_string(&self, program: u16) -> Option<String> {
        if program == 0 { return None; }

        const I_IPA_NAME: u16 = 0x0d; // i_IPA_NAME from synthesize.h
        const MAX_SCAN: usize = 8;    // scan up to this many instructions for logic phonemes

        let phonindex = &self.phonindex;
        let prog = program as usize;

        let first_offset = prog * 2;
        if first_offset + 2 > phonindex.len() { return None; }
        let first_instn = u16::from_le_bytes([phonindex[first_offset], phonindex[first_offset + 1]]);

        // Synthesis-only phonemes (i_FMT=0xb000+, i_WAV=0xc000+, etc.) have no
        // conditional IPA logic — scanning forward would bleed into adjacent phoneme
        // programs. Only check the first instruction for these.
        // Logic phonemes (i_VOWELIN=0xa100) may have a linear preamble before
        // the IPA_NAME opcode — scan forward, but stop at any conditional or jump.
        // Conditionals (types 2/3) and jumps (type 6) must NOT be traversed because
        // their branches are context-dependent and will yield wrong IPA strings
        // (e.g. Danish 'l' → ɐ̯ only before a vowel, not unconditionally).
        let max_scan = if first_instn >= 0xb000 { 1 } else { MAX_SCAN };

        // Scan through up to max_scan instructions looking for i_IPA_NAME.
        for i in 0..max_scan {
            let offset = (prog + i) * 2;
            if offset + 2 > phonindex.len() { break; }

            let instn = u16::from_le_bytes([phonindex[offset], phonindex[offset + 1]]);
            let instn_type = instn >> 12;
            let instn2     = (instn >> 8) & 0xf;
            let data       = (instn & 0xff) as usize;

            // Stop scanning if we hit a conditional (types 2/3), jump (type 6),
            // or CALLPH (0x9100, type 9).  These are either context-dependent or
            // require a full synthesizer context to execute.  Continuing past them
            // yields wrong IPA (e.g. Danish ʔo → ɒ, Danish l → ɐ̯ instead of l).
            if instn_type == 2 || instn_type == 3 || instn_type == 6 {
                return None;
            }
            if instn == 0x9100 {
                // i_CALLPH: context-dependent; stop here and use mnemonic fallback.
                return None;
            }

            // Stop scanning when we encounter a synthesis instruction mid-program
            // (i_FMT=0xb000+, i_WAV=0xc000+, etc.) after the first instruction.
            // An i_IPA_NAME that follows synthesis data belongs to a sub-phoneme
            // or branch, not the main unconditional IPA for this phoneme.
            // Exception: if this is the very first instruction (i==0), we already
            // handled the pure-synthesis-phoneme case via max_scan=1 above.
            // Here we guard against the mixed case (logic preamble + i_FMT + IPA).
            if i > 0 && instn >= 0xb000 {
                return None;
            }

            if instn_type == 0 && instn2 == I_IPA_NAME {
                if data == 0 {
                    // explicit "no name"
                    return None;
                }
                // data = number of UTF-8 bytes; packed two per instruction word
                let mut ipa_bytes = Vec::with_capacity(data);
                let n_words = (data + 1) / 2;
                for j in 0..n_words {
                    let word_off = (prog + i + 1 + j) * 2;
                    if word_off + 2 > phonindex.len() { break; }
                    let word = u16::from_le_bytes([
                        phonindex[word_off],
                        phonindex[word_off + 1],
                    ]);
                    ipa_bytes.push(((word >> 8) & 0xff) as u8);
                    ipa_bytes.push((word & 0xff) as u8);
                }
                ipa_bytes.truncate(data);
                return String::from_utf8(ipa_bytes).ok()
                    .filter(|s| !s.is_empty() && !s.starts_with('\u{0001}'));
            }
        }
        None
    }
}

// ---------------------------------------------------------------------------
// phontab parser
// ---------------------------------------------------------------------------

fn parse_phontab(data: &[u8]) -> Result<Vec<PhonemeTabList>> {
    if data.is_empty() {
        return Err(Error::InvalidData("phontab is empty".into()));
    }

    let n_tables = data[0] as usize;
    let mut pos = 4usize; // skip [n_tables, 0, 0, 0]
    let mut tables = Vec::with_capacity(n_tables);

    for _i in 0..n_tables {
        // 4-byte table header
        if pos + 4 > data.len() {
            return Err(Error::InvalidData("phontab truncated in table header".into()));
        }
        let n_phonemes = data[pos] as usize;
        let includes   = data[pos + 1];
        pos += 4;

        // 32-byte name
        if pos + N_PHONEME_TAB_NAME > data.len() {
            return Err(Error::InvalidData("phontab truncated in table name".into()));
        }
        let name_buf: &[u8; N_PHONEME_TAB_NAME] = data[pos..pos + N_PHONEME_TAB_NAME]
            .try_into()
            .unwrap();
        let name = PhonemeTabList::parse_name(name_buf);
        pos += N_PHONEME_TAB_NAME;

        // n_phonemes × 16-byte entries
        let entries_size = n_phonemes * PHONEME_TAB_ENTRY_SIZE;
        if pos + entries_size > data.len() {
            return Err(Error::InvalidData(
                format!("phontab truncated in phoneme entries for table '{name}'")
            ));
        }
        let mut phonemes = Vec::with_capacity(n_phonemes);
        for j in 0..n_phonemes {
            let off = pos + j * PHONEME_TAB_ENTRY_SIZE;
            let entry: &[u8; 16] = data[off..off + 16].try_into().unwrap();
            phonemes.push(PhonemeTab::from_bytes(entry));
        }
        pos += entries_size;

        tables.push(PhonemeTabList { name, phonemes, n_phonemes, includes });
    }

    Ok(tables)
}

// ---------------------------------------------------------------------------
// File reading helper — mirrors ReadPhFile()
// ---------------------------------------------------------------------------

fn read_file(dir: &Path, name: &str) -> Result<Vec<u8>> {
    let path = dir.join(name);
    std::fs::read(&path).map_err(|e| Error::Io(e))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // The installed data directory.
    const DATA_DIR: &str = "/usr/share/espeak-ng-data";

    fn data_available() -> bool {
        Path::new(DATA_DIR).join("phontab").exists()
    }

    #[test]
    fn load_phdata_basic() {
        if !data_available() { return; }
        let d = PhonemeData::load(Path::new(DATA_DIR)).expect("load_phdata");

        // Should have loaded 134 tables (value from installed 1.52.0 data)
        assert_eq!(d.n_tables(), 134, "unexpected table count");

        // Correct sample rate
        assert_eq!(d.sample_rate, 22050);

        // Correct tune count (2312 bytes / 68 = 34)
        assert_eq!(d.n_tunes(), 34);

        // phondata version bytes check (already validated internally)
        assert!(d.phondata.len() > 8);
    }

    #[test]
    fn table_names_include_base() {
        if !data_available() { return; }
        let d = PhonemeData::load(Path::new(DATA_DIR)).unwrap();
        assert_eq!(d.tables[0].name, "base");
        assert_eq!(d.tables[1].name, "base1");
        // 'en' table should exist somewhere
        assert!(d.tables.iter().any(|t| t.name == "en"), "no 'en' table found");
    }

    #[test]
    fn base_table_phoneme_count() {
        if !data_available() { return; }
        let d = PhonemeData::load(Path::new(DATA_DIR)).unwrap();
        // base table has 35 phonemes per the hexdump analysis
        assert_eq!(d.tables[0].n_phonemes, 35, "base table phoneme count");
        assert_eq!(d.tables[0].includes,   0,  "base table has no parent");
    }

    #[test]
    fn base1_includes_base() {
        if !data_available() { return; }
        let d = PhonemeData::load(Path::new(DATA_DIR)).unwrap();
        // base1 includes table index 0 (includes=1 means 1-based)
        assert_eq!(d.tables[1].includes, 1);
    }

    #[test]
    fn phoneme_tab_roundtrip_bytes() {
        if !data_available() { return; }
        let raw = std::fs::read(Path::new(DATA_DIR).join("phontab")).unwrap();
        let tables = parse_phontab(&raw).unwrap();

        // Re-serialise every entry and compare byte-for-byte.
        // Skip entry 0 of every table (it's a sentinel/null entry).
        let mut offset = 4usize;
        for tbl in &tables {
            offset += 4 + N_PHONEME_TAB_NAME; // table header + name
            for (j, ph) in tbl.phonemes.iter().enumerate() {
                let original = &raw[offset..offset + 16];
                let serialised = ph.to_bytes();
                assert_eq!(
                    serialised, original,
                    "round-trip mismatch in table '{}' entry {j}",
                    tbl.name
                );
                offset += 16;
            }
        }
    }

    #[test]
    fn select_table_en() {
        if !data_available() { return; }
        let mut d = PhonemeData::load(Path::new(DATA_DIR)).unwrap();
        let idx = d.select_table_by_name("en").expect("'en' table");
        assert!(idx < d.n_tables());
        // Should be idempotent
        d.select_table(idx).unwrap();
        assert_eq!(d.current_table, idx as i32);
    }

    #[test]
    fn lookup_pause_phoneme() {
        if !data_available() { return; }
        let mut d = PhonemeData::load(Path::new(DATA_DIR)).unwrap();
        d.select_table_by_name("en").unwrap();
        // "_" is the short pause phoneme, code = PHON_PAUSE_SHORT = 10
        let code = d.lookup_phoneme("_");
        assert_eq!(code, 10, "pause phoneme code");
    }

    #[test]
    fn lookup_unknown_returns_zero() {
        if !data_available() { return; }
        let mut d = PhonemeData::load(Path::new(DATA_DIR)).unwrap();
        d.select_table_by_name("en").unwrap();
        assert_eq!(d.lookup_phoneme("???"), 0);
    }

    #[test]
    fn select_nonexistent_table_errors() {
        if !data_available() { return; }
        let mut d = PhonemeData::load(Path::new(DATA_DIR)).unwrap();
        assert!(d.select_table_by_name("no_such_language").is_err());
    }

    #[test]
    fn find_table_returns_index() {
        if !data_available() { return; }
        let d = PhonemeData::load(Path::new(DATA_DIR)).unwrap();
        let idx = d.find_table("base").unwrap();
        assert_eq!(idx, 0);
    }
}
