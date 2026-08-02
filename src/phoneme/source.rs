//! Phoneme-source DSL parser — the text front-end of the phoneme compiler
//! (`compiledata.c`, §1.6), symmetric with the dict `parse_rules_dsl`.
//!
//! Parses the `phonemetable` / `phoneme … endphoneme` source form into
//! [`PhonemeTabList`]s ready for [`compile_phontab`](super::table::compile_phontab).
//! This is the **table-level** slice: it fills each phoneme's identity (mnemonic,
//! code), manner/place/voicing (via the existing [`PhonemeFeature`] machinery),
//! and `length`/`lengthmod`.  The synthesis *program* body (`FMT`/`WAV`/`IF`/
//! `Vowelin`… — compiled into `phonindex`/`phondata` separately) is skipped, so
//! program lines pass through harmlessly.
//!
//! No phoneme-source files ship in this checkout, so the parser is validated by
//! round-tripping synthetic source through `compile_phontab` → `parse_phontab`.

use super::feature::PhonemeFeature;
use super::table::{PhonemeTab, PhonemeTabList};
use super::{PH_LIQUID, PH_PAUSE, PH_STRESS, PH_VOWEL};

/// Parse phoneme-source text into a list of phoneme tables.
///
/// Recognised directives:
/// * `phonemetable <name> [<base>]` — begin a table; `<base>` (a previously
///   declared table) sets the `includes` inheritance index.
/// * `phoneme <mnemonic>` … `endphoneme` — one phoneme; codes are assigned
///   sequentially within the table (starting at 1, mirroring the reader which
///   reserves 0).
/// * inside a block: 3-letter IPA feature tags (`vwl`, `stp`, `blb`, `vcd`, …)
///   applied via [`PhonemeTab::apply_feature`]; the non-feature type words
///   `pause`/`stress`/`vowel`/`liquid`; and `length <n>` / `lengthmod <n>`.
///
/// Unrecognised lines (program body, unknown keywords) are ignored.
pub fn parse_phoneme_dsl(source: &str) -> Vec<PhonemeTabList> {
    let mut tables: Vec<PhonemeTabList> = Vec::new();
    let mut cur: Option<PhonemeTab> = None; // phoneme block in progress
    let mut next_code: u8 = 1;

    for raw in source.lines() {
        // Strip `//` comments and surrounding whitespace.
        let line = match raw.find("//") {
            Some(i) => &raw[..i],
            None => raw,
        };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let mut tokens = line.split_whitespace();
        let Some(head) = tokens.next() else { continue };

        match head {
            "phonemetable" => {
                // Finalize any dangling phoneme (defensive; well-formed source
                // closes each block with `endphoneme`).
                flush_phoneme(&mut cur, tables.last_mut());
                let name = tokens.next().unwrap_or("").to_string();
                let base = tokens.next();
                // `includes` = 1-based index of the base table, or 0 for none.
                let includes = base
                    .and_then(|b| tables.iter().position(|t| t.name == b))
                    .map(|i| (i + 1) as u8)
                    .unwrap_or(0);
                tables.push(PhonemeTabList { name, phonemes: Vec::new(), n_phonemes: 0, includes });
                next_code = 1;
            }
            "phoneme" => {
                flush_phoneme(&mut cur, tables.last_mut());
                if tables.is_empty() {
                    // A phoneme before any table: start an anonymous one.
                    tables.push(PhonemeTabList { name: String::new(), phonemes: Vec::new(), n_phonemes: 0, includes: 0 });
                    next_code = 1;
                }
                let mnem = tokens.next().unwrap_or("");
                let mut ph = PhonemeTab { mnemonic: PhonemeTab::pack_mnemonic(mnem), code: next_code, ..Default::default() };
                next_code = next_code.wrapping_add(1);
                // A `phoneme` head may carry inline feature tags on the same line.
                apply_tokens(&mut ph, tokens);
                cur = Some(ph);
            }
            "endphoneme" => {
                flush_phoneme(&mut cur, tables.last_mut());
            }
            "length" => {
                if let (Some(ph), Some(v)) = (cur.as_mut(), tokens.next().and_then(|t| t.parse::<u32>().ok())) {
                    ph.std_length = v.min(u8::MAX as u32) as u8;
                }
            }
            "lengthmod" => {
                if let (Some(ph), Some(v)) = (cur.as_mut(), tokens.next().and_then(|t| t.parse::<u32>().ok())) {
                    ph.length_mod = v.min(u8::MAX as u32) as u8;
                }
            }
            _ => {
                // Manner/place/voicing tags (possibly several per line), plus the
                // non-feature type words.  Ignored unless inside a phoneme block.
                if let Some(ph) = cur.as_mut() {
                    // Re-include `head` in the token stream for uniform handling.
                    apply_word(ph, head);
                    apply_tokens(ph, tokens);
                }
            }
        }
    }

    // Close a trailing phoneme / fix up counts.
    flush_phoneme(&mut cur, tables.last_mut());
    for t in &mut tables {
        t.n_phonemes = t.phonemes.len();
    }
    tables
}

/// Apply a run of whitespace-separated feature/keyword tokens to `ph`.
fn apply_tokens<'a>(ph: &mut PhonemeTab, tokens: impl Iterator<Item = &'a str>) {
    for tok in tokens {
        apply_word(ph, tok);
    }
}

/// Apply a single source word: a non-feature type keyword, or a 3-letter IPA
/// feature tag.  Unknown words are ignored (program body / unsupported).
fn apply_word(ph: &mut PhonemeTab, word: &str) {
    match word {
        "pause" => ph.typ = PH_PAUSE,
        "stress" => ph.typ = PH_STRESS,
        "vowel" => ph.typ = PH_VOWEL, // alias of the `vwl` feature tag
        "liquid" => ph.typ = PH_LIQUID,
        _ => {
            if let Some(feat) = PhonemeFeature::from_str(word) {
                // Unsupported/unknown features are silently accepted by design.
                let _ = ph.apply_feature(feat);
            }
        }
    }
}

/// Push the in-progress phoneme (if any) into the current table.
fn flush_phoneme(cur: &mut Option<PhonemeTab>, table: Option<&mut PhonemeTabList>) {
    if let (Some(ph), Some(t)) = (cur.take(), table) {
        t.phonemes.push(ph);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::phoneme::table::{compile_phontab, PhonemeTab};
    use crate::phoneme::{PH_STOP, PH_VOWEL, PH_VOICED, PH_VOICELESS, PHFLAG_ARTICULATION, PLACE_BILABIAL};

    #[test]
    fn parse_basic_phoneme_block() {
        let src = "\
phonemetable base
phoneme _
\tpause
\tlength 20
endphoneme
phoneme p
\tvls blb stp
\tlengthmod 2
endphoneme
phoneme a
\tvwl
\tlength 180
endphoneme
";
        let tables = parse_phoneme_dsl(src);
        assert_eq!(tables.len(), 1);
        let t = &tables[0];
        assert_eq!(t.name, "base");
        assert_eq!(t.phonemes.len(), 3);

        // codes assigned sequentially from 1
        assert_eq!(t.phonemes.iter().map(|p| p.code).collect::<Vec<_>>(), vec![1, 2, 3]);

        // `_` pause with length
        assert_eq!(t.phonemes[0].mnemonic_str(), "_");
        assert_eq!(t.phonemes[0].std_length, 20);

        // `p` = voiceless bilabial stop
        let p = &t.phonemes[1];
        assert_eq!(p.mnemonic_str(), "p");
        assert_eq!(p.typ, PH_STOP);
        assert_eq!((p.phflags & PHFLAG_ARTICULATION) >> 16, PLACE_BILABIAL);
        assert!(p.phflags & PH_VOICELESS != 0);
        assert_eq!(p.length_mod, 2);

        // `a` = vowel
        assert_eq!(t.phonemes[2].typ, PH_VOWEL);
        assert_eq!(t.phonemes[2].std_length, 180);
    }

    #[test]
    fn parse_then_compile_round_trips() {
        // Parse synthetic source, compile to the binary phontab, re-parse, and
        // confirm the phoneme identities/types/flags survive the round trip.
        let src = "\
phonemetable base
phoneme t
\tvls alv stp
endphoneme
phoneme d
\tvcd alv stp
endphoneme
phonemetable en base
phoneme a
\tvwl
\tlength 200
endphoneme
";
        let tables = parse_phoneme_dsl(src);
        assert_eq!(tables.len(), 2);
        // Second table inherits the first (`includes` = 1-based index).
        assert_eq!(tables[1].name, "en");
        assert_eq!(tables[1].includes, 1);

        let bytes = compile_phontab(&tables);
        let reparsed = crate::phoneme::load::parse_phontab(&bytes).expect("reparse phontab");
        assert_eq!(reparsed.len(), 2);
        assert_eq!(reparsed[0].name, "base");
        assert_eq!(reparsed[1].name, "en");
        assert_eq!(reparsed[1].includes, 1);

        // `t` voiceless, `d` voiced — both alveolar stops.
        let t = &reparsed[0].phonemes[0];
        let d = &reparsed[0].phonemes[1];
        assert_eq!(t.mnemonic_str(), "t");
        assert_eq!(d.mnemonic_str(), "d");
        assert_eq!(t.typ, PH_STOP);
        assert_eq!(d.typ, PH_STOP);
        assert!(t.phflags & PH_VOICELESS != 0);
        assert!(d.phflags & PH_VOICED != 0);

        // Vowel length preserved through the binary round trip.
        let a = &reparsed[1].phonemes[0];
        assert_eq!(a.typ, PH_VOWEL);
        assert_eq!(a.std_length, 200);
        assert_eq!(a, &PhonemeTab { mnemonic: PhonemeTab::pack_mnemonic("a"), code: a.code, typ: PH_VOWEL, std_length: 200, ..Default::default() });
    }
}
