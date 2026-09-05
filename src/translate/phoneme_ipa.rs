//! IPA rendering straight from the clause's phoneme-code list.
//!
//! This is a port of C's `GetTranslatedPhonemeString()` (dictionary.c) together
//! with the `use_ipa` half of `WritePhMnemonic()`.  Both walk the *finished*
//! phoneme list — the same list `-x` prints — so `--ipa` and `-x` can never
//! disagree about which phonemes a clause contains, only about how each one is
//! spelled.
//!
//! The important part is where the spelling comes from.  C does not have a
//! table of IPA names: it **runs each phoneme's bytecode program** with that
//! phoneme's real neighbours and takes whatever `ipa <string>` the executed
//! path reached (`phdata->ipa_string`).  The name is therefore contextual —
//! Italian `*` is `r` between vowels and `ɾ` elsewhere, and Spanish `i;` is
//! plain `i` rather than the `iʲ` its mnemonic would suggest.  Only when the
//! program reaches no `ipa` instruction does C fall back to mapping the
//! mnemonic's characters through `ipa1[]`.

use std::path::Path;

use crate::phoneme::{
    PhonemeData, PHON_END_WORD, PHON_STRESS_2, PHON_STRESS_3, PHON_STRESS_D, PHON_STRESS_P,
    PHON_STRESS_P2, PHON_STRESS_PREV, PHON_STRESS_TONIC, PHON_STRESS_U,
};
use crate::synthesize::bytecode::{interpret_phoneme, Neighbours};

use super::ipa_table::{encode_utf8, ipa1_char};
use super::{CodeMarker, PhonemeCode};

/// C `WritePhMnemonic(..., use_ipa = 1)` for one phoneme of the list.
///
/// `nb` supplies the neighbours the phoneme's program conditions on; pass
/// [`Neighbours::default`] for a phoneme with no context (C's
/// `InterpretPhoneme2`, which surrounds it with pauses).
pub fn write_ph_mnemonic_ipa(phdata: &PhonemeData, code: u8, nb: &Neighbours) -> String {
    // phonEND_WORD is a list marker, not a sound.
    if code == PHON_END_WORD {
        return String::new();
    }
    let Some(ph) = phdata.get(code) else {
        return String::new();
    };

    if ph.program != 0 {
        let fx = interpret_phoneme(ph.program, &phdata.phonindex, nb, |c| phdata.get(c).cloned());
        if let Some(s) = fx.ipa_string.as_deref() {
            let b = s.as_bytes();
            // A leading space is C's "this phoneme has no IPA name" marker.
            if b.first() == Some(&0x20) {
                return String::new();
            }
            // A leading byte below 0x20 is a flags byte, not text.
            let s = match b.first() {
                Some(&c) if c < 0x20 => &s[1..],
                _ => s,
            };
            if !s.is_empty() {
                return s.to_string();
            }
        }
    }

    mnemonic_to_ipa_c(ph.mnemonic, ph.typ == 2 /* phVOWEL */)
}

/// The mnemonic fallback of `WritePhMnemonic`, character for character.
///
/// Unlike [`super::ipa_table::mnemonic_to_ipa`] this keeps a `-` in a vowel
/// mnemonic, because C does: French's reducible schwa `@-` prints `ə-`.
fn mnemonic_to_ipa_c(mnemonic: u32, is_vowel: bool) -> String {
    let mut out = Vec::new();
    let mut first = true;
    let mut mnem = mnemonic;
    while mnem != 0 {
        let c = (mnem & 0xff) as u8;
        mnem >>= 8;
        if c == 0 || c == b'/' {
            break; // end, or the phoneme-variant indicator
        }
        if first && c == b'_' {
            break; // pause phonemes are not shown
        }
        if c == b'#' && is_vowel {
            break; // `#` is subscript-h, and only for consonants
        }
        if !first && c.is_ascii_digit() {
            continue; // digits after the first character are variant numbers
        }
        encode_utf8(ipa1_char(c), &mut out);
        first = false;
    }
    String::from_utf8(out).unwrap_or_default()
}

/// Render a whole clause's code list as IPA — C's `GetTranslatedPhonemeString`
/// with `espeakPHONEMES_IPA` set.
///
/// The structure deliberately mirrors the CLI's `-x` writer: the same code list,
/// the same word/clause boundaries, the same `(lang)` switch markers.  Only the
/// per-phoneme spelling and the stress characters differ.
///
/// `preserve_punctuation` is the port's phonemizer mode: the punctuation that
/// upstream simply drops is echoed into the output, and a clause's terminator is
/// printed before its line break.
pub fn codes_to_ipa(
    codes: &[PhonemeCode],
    phdata: &PhonemeData,
    data_dir: &Path,
    lang: &str,
    preserve_punctuation: bool,
) -> String {
    // Indices of the codes that are actual sounds, for neighbour lookups.
    let is_sound =
        |c: &PhonemeCode| !c.is_boundary && c.code > 8 && c.code != PHON_END_WORD;
    let real: Vec<usize> = codes
        .iter()
        .enumerate()
        .filter(|(_, c)| is_sound(c))
        .map(|(i, _)| i)
        .collect();
    // Position within `real` for each code index, so the loop below can find a
    // phoneme's neighbours without rescanning.
    let mut rank = vec![usize::MAX; codes.len()];
    for (n, &i) in real.iter().enumerate() {
        rank[i] = n;
    }

    let mut out = String::new();
    // The table the codes that follow belong to; `None` = the clause's own.
    let mut switched: Option<PhonemeData> = None;
    let mut straight_quote_open = false;

    for (i, pc) in codes.iter().enumerate() {
        if let Some(CodeMarker::Punctuation(c)) = &pc.marker {
            if preserve_punctuation {
                // An opening bracket or quote binds to the word that follows.
                let opening = match c {
                    '(' | '[' | '{' | '\u{201c}' | '\u{2018}' => true,
                    '"' => !straight_quote_open,
                    _ => false,
                };
                if *c == '"' {
                    straight_quote_open = !straight_quote_open;
                }
                if opening && !out.is_empty() && !out.ends_with([' ', '\n']) {
                    out.push(' ');
                }
                out.push(*c);
                if opening {
                    // Swallow the separator the tokenizer emits after it.
                    continue;
                }
            }
            continue;
        }
        if let Some(CodeMarker::LangSwitch(target)) = &pc.marker {
            // A *closing* marker binds to the word it follows, so the word-space
            // moves after it — as in `-x`.
            let trailing_space = target == lang && out.ends_with(' ');
            while target == lang && out.ends_with(' ') {
                out.pop();
            }
            out.push_str(&format!("({target})"));
            if trailing_space {
                out.push(' ');
            }
            switched = if target == lang {
                None
            } else {
                PhonemeData::load(data_dir).ok().and_then(|mut d| {
                    super::select_phoneme_table(&mut d, data_dir, target).ok().map(|_| d)
                })
            };
            continue;
        }
        let phdata: &PhonemeData = switched.as_ref().unwrap_or(phdata);

        if pc.is_boundary || pc.code == PHON_END_WORD {
            if pc.is_boundary && pc.code == 0 && pc.clause_char.is_some() {
                if preserve_punctuation {
                    if let Some(c) = pc.clause_char.filter(|c| ".,!?;:".contains(*c)) {
                        while out.ends_with(' ') {
                            out.pop();
                        }
                        out.push(c);
                    }
                }
                out.push('\n');
            } else {
                out.push(' ');
            }
            continue;
        }

        match pc.code {
            PHON_STRESS_P | PHON_STRESS_P2 | PHON_STRESS_TONIC => out.push('\u{2c8}'),
            PHON_STRESS_2 | PHON_STRESS_3 => out.push('\u{2cc}'),
            PHON_STRESS_U | PHON_STRESS_PREV | PHON_STRESS_D => {}
            0 => {}
            code => {
                // A stress *marker* phoneme (type 1 with no program) encodes its
                // level in `std_length`, exactly as the `-x` writer reads it.
                if let Some(ph) = phdata.get(code) {
                    if ph.typ == 1 && ph.program == 0 {
                        match ph.std_length {
                            4 => out.push('\u{2c8}'),
                            2 | 3 => out.push('\u{2cc}'),
                            _ => {}
                        }
                        continue;
                    }
                }
                let nb = neighbours_at(codes, &real, rank[i], phdata);
                out.push_str(&write_ph_mnemonic_ipa(phdata, code, &nb));
            }
        }
    }

    // As in `-x`: a suppressed clause boundary leaves doubled separators behind,
    // and word boundaries either side of a clause break leave stray edge spaces.
    while out.contains("  ") {
        out = out.replace("  ", " ");
    }
    let _ = straight_quote_open;
    out.split('\n').map(str::trim).collect::<Vec<_>>().join("\n").trim_end().to_string()
}

/// The neighbour context for the phoneme that is `real[n]`, built the same way
/// the code-list phoneme-program pass builds it.
fn neighbours_at(
    codes: &[PhonemeCode],
    real: &[usize],
    n: usize,
    phdata: &PhonemeData,
) -> Neighbours {
    if n == usize::MAX {
        return Neighbours::default();
    }
    let i = real[n];
    let at = |k: Option<&usize>| k.map(|&k| codes[k].code).unwrap_or(0);
    let next_wordstart = real.get(n + 1).is_some_and(|&k| {
        codes[i + 1..k]
            .iter()
            .any(|c| c.is_boundary || c.code == PHON_END_WORD)
    });
    // Stress level of the syllable this phoneme belongs to, for programs that
    // condition on it.
    let stress = codes[..i]
        .iter()
        .rev()
        .take_while(|c| !c.is_boundary)
        .find_map(|c| match c.code {
            PHON_STRESS_P | PHON_STRESS_P2 | PHON_STRESS_TONIC => Some(4u8),
            PHON_STRESS_2 | PHON_STRESS_3 => Some(3),
            c2 if matches!(phdata.get(c2), Some(p) if p.typ == 2) => Some(0),
            _ => None,
        })
        .unwrap_or(1);
    let starts = super::word_starts(codes, real);
    Neighbours {
        prev: at(n.checked_sub(1).and_then(|k| real.get(k))),
        this: codes[i].code,
        next: at(real.get(n + 1)),
        next2: at(real.get(n + 2)),
        stress,
        this_wordstart: starts[n],
        prev_wordstart: n.checked_sub(1).is_some_and(|k| starts[k]),
        next_wordstart,
        next2_wordstart: next_wordstart,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data() -> Option<PhonemeData> {
        let dir = std::path::Path::new("espeak-ng-data");
        dir.join("phontab").exists().then(|| PhonemeData::load(dir).ok())?
    }

    /// The mnemonic fallback keeps a vowel's `-` (French `@-` → `ə-`) and drops
    /// a `/` variant, a leading `_`, a consonant-only `#`, and later digits.
    #[test]
    fn mnemonic_fallback_matches_c() {
        let p = |s: &str, v: bool| {
            mnemonic_to_ipa_c(crate::phoneme::PhonemeTab::pack_mnemonic(s), v)
        };
        assert_eq!(p("@-", true), "ə-");
        assert_eq!(p("a/2", true), "a");
        assert_eq!(p("_:", false), "");
        assert_eq!(p("a#", true), "a");
        assert_eq!(p("t#", false), "tʰ");
        assert_eq!(p("i2", true), "i");
        assert_eq!(p("3:", true), "ɜː");
    }

    /// Spanish `i;` carries an explicit `ipa i` in its program, so it must not
    /// come out as the mnemonic's `iʲ`.
    #[test]
    fn program_ipa_beats_the_mnemonic() {
        let Some(mut phdata) = data() else { return };
        if phdata.select_table_by_name("es").is_err() {
            return;
        }
        let code = (0..255u8)
            .find(|&c| phdata.get(c).is_some_and(|p| p.mnemonic_str() == "i;"));
        let Some(code) = code else { return };
        assert_eq!(
            write_ph_mnemonic_ipa(&phdata, code, &Neighbours::default()),
            "i"
        );
    }
}
