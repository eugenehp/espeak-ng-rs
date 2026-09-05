//! Unicode compatibility (NFKC) expansions — upstream #2523.
//!
//! Applied as a *fallback*: a token that produces no phonemes is retried with
//! its expansion, so the characters the port already reads correctly (full-width
//! letters, circled digits, superscripts, Roman numerals) are untouched.

/// Compatibility (NFKC) expansions for the characters this port would otherwise
/// speak as nothing — squared units (`㎏` → `kg`), letterlike symbols
/// (`℃` → `°C`, `№`), enclosed forms, ligatures, and the full/half-width forms.
///
/// Upstream #2523 normalizes input with NFKC before synthesis; doing the whole
/// of NFKC would mean vendoring the Unicode decomposition tables, so this covers
/// exactly the characters this port would otherwise **drop**. Anything it
/// already reads — `™` as "trademark", `№` as "numero", circled digits,
/// superscripts, full-width letters — keeps its own, better, reading and is not
/// in this table.
static COMPATIBILITY_EXPANSIONS: &[(u32, &str)] = &[
    (0x2100, "a/c"),
    (0x2101, "a/s"),
    (0x2105, "c/o"),
    (0x2106, "c/u"),
    (0x2107, "Ɛ"),
    (0x210F, "ħ"),
    (0x2113, "l"),
    (0x2120, "SM"),
    (0x2121, "TEL"),
    (0x212B, "Å"),
    (0x213B, "FAX"),
    (0x2145, "D"),
    (0x2146, "d"),
    (0x2147, "e"),
    (0x2148, "i"),
    (0x2149, "j"),
    (0x3250, "PTE"),
    (0x3251, "21"),
    (0x3252, "22"),
    (0x3253, "23"),
    (0x3254, "24"),
    (0x3255, "25"),
    (0x3256, "26"),
    (0x3257, "27"),
    (0x3258, "28"),
    (0x3259, "29"),
    (0x325A, "30"),
    (0x325B, "31"),
    (0x325C, "32"),
    (0x325D, "33"),
    (0x325E, "34"),
    (0x325F, "35"),
    (0x32B1, "36"),
    (0x32B2, "37"),
    (0x32B3, "38"),
    (0x32B4, "39"),
    (0x32B5, "40"),
    (0x32B6, "41"),
    (0x32B7, "42"),
    (0x32B8, "43"),
    (0x32B9, "44"),
    (0x32BA, "45"),
    (0x32BB, "46"),
    (0x32BC, "47"),
    (0x32BD, "48"),
    (0x32BE, "49"),
    (0x32BF, "50"),
    (0x32CC, "Hg"),
    (0x32CD, "erg"),
    (0x32CE, "eV"),
    (0x32CF, "LTD"),
    (0x3371, "hPa"),
    (0x3372, "da"),
    (0x3373, "AU"),
    (0x3374, "bar"),
    (0x3375, "oV"),
    (0x3376, "pc"),
    (0x3377, "dm"),
    (0x3378, "dm2"),
    (0x3379, "dm3"),
    (0x337A, "IU"),
    (0x3380, "pA"),
    (0x3381, "nA"),
    (0x3383, "mA"),
    (0x3384, "kA"),
    (0x3385, "KB"),
    (0x3386, "MB"),
    (0x3387, "GB"),
    (0x3388, "cal"),
    (0x3389, "kcal"),
    (0x338A, "pF"),
    (0x338B, "nF"),
    (0x338E, "mg"),
    (0x338F, "kg"),
    (0x3390, "Hz"),
    (0x3391, "kHz"),
    (0x3392, "MHz"),
    (0x3393, "GHz"),
    (0x3394, "THz"),
    (0x3396, "ml"),
    (0x3397, "dl"),
    (0x3398, "kl"),
    (0x3399, "fm"),
    (0x339A, "nm"),
    (0x339C, "mm"),
    (0x339D, "cm"),
    (0x339E, "km"),
    (0x339F, "mm2"),
    (0x33A0, "cm2"),
    (0x33A1, "m2"),
    (0x33A2, "km2"),
    (0x33A3, "mm3"),
    (0x33A4, "cm3"),
    (0x33A5, "m3"),
    (0x33A6, "km3"),
    (0x33A9, "Pa"),
    (0x33AA, "kPa"),
    (0x33AB, "MPa"),
    (0x33AC, "GPa"),
    (0x33AD, "rad"),
    (0x33B0, "ps"),
    (0x33B1, "ns"),
    (0x33B3, "ms"),
    (0x33B4, "pV"),
    (0x33B5, "nV"),
    (0x33B7, "mV"),
    (0x33B8, "kV"),
    (0x33B9, "MV"),
    (0x33BA, "pW"),
    (0x33BB, "nW"),
    (0x33BD, "mW"),
    (0x33BE, "kW"),
    (0x33BF, "MW"),
    (0x33C2, "a.m."),
    (0x33C3, "Bq"),
    (0x33C4, "cc"),
    (0x33C5, "cd"),
    (0x33C7, "Co."),
    (0x33C8, "dB"),
    (0x33C9, "Gy"),
    (0x33CA, "ha"),
    (0x33CB, "HP"),
    (0x33CC, "in"),
    (0x33CD, "KK"),
    (0x33CE, "KM"),
    (0x33CF, "kt"),
    (0x33D0, "lm"),
    (0x33D1, "ln"),
    (0x33D2, "log"),
    (0x33D3, "lx"),
    (0x33D4, "mb"),
    (0x33D5, "mil"),
    (0x33D6, "mol"),
    (0x33D7, "PH"),
    (0x33D8, "p.m."),
    (0x33D9, "PPM"),
    (0x33DA, "PR"),
    (0x33DB, "sr"),
    (0x33DC, "Sv"),
    (0x33DD, "Wb"),
    (0x33FF, "gal"),
    (0xFB03, "ffi"),
    (0xFB04, "ffl"),
    (0xFB29, "+"),
    (0xFE30, ".."),
    (0xFE33, "_"),
    (0xFE34, "_"),
    (0xFE35, "("),
    (0xFE36, ")"),
    (0xFE37, "{"),
    (0xFE38, "}"),
    (0xFE47, "["),
    (0xFE48, "]"),
    (0xFE4D, "_"),
    (0xFE4E, "_"),
    (0xFE4F, "_"),
    (0xFF01, "!"),
    (0xFF02, "\""),
    (0xFF07, "'"),
    (0xFF08, "("),
    (0xFF09, ")"),
    (0xFF0C, ","),
    (0xFF0D, "-"),
    (0xFF0E, "."),
    (0xFF1A, ":"),
    (0xFF1B, ";"),
    (0xFF1C, "<"),
    (0xFF1E, ">"),
    (0xFF1F, "?"),
    (0xFF3B, "["),
    (0xFF3C, "\\"),
    (0xFF3D, "]"),
    (0xFF3E, "^"),
    (0xFF3F, "_"),
    (0xFF40, "`"),
    (0xFF5B, "{"),
    (0xFF5C, "|"),
    (0xFF5D, "}"),
    (0xFFE0, "¢"),
    (0xFFE1, "£"),
    (0xFFE2, "¬"),
    (0xFFE4, "¦"),
    (0xFFE5, "¥"),
];

/// The NFKC expansion of `c`, when it has one worth speaking.
///
/// Binary search: the table is generated in code-point order.
pub fn compatibility_expansion(c: char) -> Option<&'static str> {
    let cp = c as u32;
    COMPATIBILITY_EXPANSIONS
        .binary_search_by_key(&cp, |&(k, _)| k)
        .ok()
        .map(|i| COMPATIBILITY_EXPANSIONS[i].1)
}

/// Expand every character of `word` that has a compatibility form, returning
/// `None` when none of them do (so the caller can skip the retry).
pub fn expand_compatibility(word: &str) -> Option<String> {
    if !word.chars().any(|c| compatibility_expansion(c).is_some()) {
        return None;
    }
    let mut out = String::with_capacity(word.len());
    for c in word.chars() {
        match compatibility_expansion(c) {
            Some(e) => out.push_str(e),
            None => out.push(c),
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn squared_units_and_letterlike_expand() {
        assert_eq!(compatibility_expansion('㎏'), Some("kg"));
        assert_eq!(compatibility_expansion('㎐'), Some("Hz"));
        // ℃ is *not* in the table: the port already reads it as
        // "degrees celsius", which is better than its NFKC form.
        assert_eq!(compatibility_expansion('℃'), None);
        assert_eq!(compatibility_expansion('ﬄ'), Some("ffl"));
        // Characters the port already reads well keep their own name.
        assert_eq!(compatibility_expansion('™'), None);
        assert_eq!(compatibility_expansion('№'), None);
        assert_eq!(compatibility_expansion('ａ'), None);
        // Not a compatibility character.
        assert_eq!(compatibility_expansion('a'), None);
        assert_eq!(compatibility_expansion('你'), None);
    }

    #[test]
    fn expansion_is_all_or_nothing_per_word() {
        assert_eq!(expand_compatibility("abc"), None);
        assert_eq!(expand_compatibility("5㎏").as_deref(), Some("5kg"));
    }

    /// The table must stay sorted, or the binary search silently misses entries.
    #[test]
    fn table_is_sorted_and_nonempty() {
        assert!(COMPATIBILITY_EXPANSIONS.len() > 150);
        assert!(COMPATIBILITY_EXPANSIONS.windows(2).all(|w| w[0].0 < w[1].0));
    }
}
