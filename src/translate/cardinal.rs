//! Cardinal numbers: port of `LookupNum2`, `LookupNum3`, `LookupThousands` and
//! the cardinal part of `TranslateNumber_1` in `numbers.c`.

#![deny(clippy::cast_possible_truncation)]

use super::{
    lookup_num_phonemes, primary_bcp47_subtag, trim_lookup, PHON_END_WORD, PHON_STRESS_3,
    PHON_STRESS_P,
};
use crate::dictionary::file::Dictionary;

// `langopts.numbers`.
const NUM_SWAP_TENS: u32 = 0x10;
const NUM_AND_UNITS: u32 = 0x20;
const NUM_HUNDRED_AND: u32 = 0x40;
const NUM_SINGLE_AND: u32 = 0x80;
const NUM_SINGLE_STRESS: u32 = 0x100;
const NUM_SINGLE_VOWEL: u32 = 0x200;
const NUM_OMIT_1_HUNDRED: u32 = 0x400;
const NUM_1900: u32 = 0x800;
const NUM_AND_HUNDRED: u32 = 0x4_0000;
const NUM_THOUSAND_AND: u32 = 0x8_0000;
const NUM_VIGESIMAL: u32 = 0x10_0000;
const NUM_OMIT_1_THOUSAND: u32 = 0x20_0000;
const NUM_ZERO_HUNDRED: u32 = 0x40_0000;
const NUM_HUNDRED_AND_DIGIT: u32 = 0x80_0000;
const NUM_SINGLE_STRESS_L: u32 = 0x1000_0000;

// `langopts.numbers2`; bits 1-3 are the scales whose count is feminine.
const NUM2_SWAP_THOUSANDS: u32 = 0x200;
const NUM2_ORDINAL_NO_AND: u32 = 0x800;
const NUM2_MULTIPLE_ORDINAL: u32 = 0x1000;
const NUM2_NO_TEEN_ORDINALS: u32 = 0x2000;
const NUM2_MYRIADS: u32 = 0x4000;
const NUM2_OMIT_1_HUNDRED_ONLY: u32 = 0x2_0000;
const NUM2_ORDINAL_AND_THOUSANDS: u32 = 0x4_0000;
const NUM2_ZERO_TENS: u32 = 0x10_0000;
const NUM2_FRACTION_FEMININE: u32 = 0x20_0000;

// `NUM_DFRACTION_*`: how the digits after a decimal point are read.
const NUM_DFRACTION_BITS: u32 = 0xe000;
const NUM_DFRACTION_1: u32 = 0x2000;
const NUM_DFRACTION_2: u32 = 0x4000;
const NUM_DFRACTION_3: u32 = 0x6000;
const NUM_DFRACTION_4: u32 = 0x8000;
const NUM_DFRACTION_5: u32 = 0xa000;
const NUM_DFRACTION_6: u32 = 0xc000;
const NUM_DFRACTION_7: u32 = 0xe000;

/// The number options of a language (`langopts` in `tr_languages.c`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct NumberFlags {
    numbers: u32,
    numbers2: u32,
    /// Bit `n` starts a new group where `n` digits remain.
    break_numbers: u32,
    malayalam: bool,
}

/// How an ordinal number is read: C's `ordinal` and `ph_ordinal2`.
#[derive(Debug, Default, Clone)]
pub(super) struct Ordinal {
    /// The variant form (`_1q`, lang=hu).
    pub variant: bool,
    /// The spoken suffix (`_#st`), or empty.
    pub suffix: Vec<u8>,
    /// Its form for a number without higher digits (`_x#…`).
    pub suffix_x: Vec<u8>,
}

/// One spoken group of a number.
#[derive(Debug)]
pub(super) struct Group {
    pub phonemes: Vec<u8>,
    /// More of the number follows, so a pause does too.
    pub pause: bool,
}

impl NumberFlags {
    /// `None` for a language that reads numbers digit by digit.
    pub(super) fn for_lang(lang: &str) -> Option<Self> {
        let lang = lang.to_ascii_lowercase();
        let (numbers, numbers2, break_numbers) =
            table(&lang).or_else(|| table(primary_bcp47_subtag(&lang)))?;
        Some(NumberFlags {
            numbers,
            numbers2,
            break_numbers,
            malayalam: primary_bcp47_subtag(&lang) == "ml",
        })
    }

    fn has(&self, flag: u32) -> bool {
        self.numbers & flag != 0
    }

    /// C's `langopts.numbers`.
    pub(super) fn numbers(&self) -> u32 {
        self.numbers
    }

    fn has2(&self, flag: u32) -> bool {
        self.numbers2 & flag != 0
    }

    /// The count of this scale is feminine (`NUM2_THOUSANDPLEX_VAR_*`).
    fn feminine(&self, scale: u32) -> bool {
        scale <= 3 && self.has2(1 << scale)
    }
}

/// `(numbers, numbers2, break_numbers)`, as C sets them.
fn table(lang: &str) -> Option<(u32, u32, u32)> {
    Some(match lang {
        "ab" | "ba" | "cmn" | "crh" | "cv" | "gn" | "hak" | "haw" | "hyw" | "ja" | "kaa"
        | "kok" | "ky" | "lb" | "lij" | "mt" | "mto" | "nci" | "nog" | "ps" | "qya" | "sjn"
        | "ti" | "tk" | "ug" | "zh" => (0x1, 0x0, 0x49249248),
        "af" => (0x10008d0, 0x0, 0x49249248),
        "am" | "cy" => (0x400, 0x0, 0x49249248),
        "an" => (0x9200528, 0x800, 0x49249248),
        "ar" => (0x2c0470, 0x0, 0x49249248),
        "as" | "bn" => (0x10, 0x0, 0x24924aa8),
        "az" => (0x205508, 0x0, 0x49249248),
        "be" => (0x408, 0x42, 0x49249248),
        "bg" => (0xb0014e8, 0x0, 0x49249248),
        "bpy" => (0x1, 0x200, 0x24924aa8),
        "bs" | "hr" | "sr" => (0x200454c, 0x14a, 0x49249248),
        "ca" | "es" | "pap" => (0x5208528, 0x1800, 0x49249248),
        "cs" => (0x1004400, 0x88, 0x49249248),
        "da" | "fo" | "kl" => (0xb010c58, 0x0, 0x49249248),
        "de" => (0x1011018, 0x0, 0x49249248),
        "el" | "grc" => (0x108, 0x1802, 0x49249248),
        "en" => (0x1000840, 0x0, 0x49249248),
        "eo" => (0x1001408, 0x0, 0x49249248),
        "et" | "fi" => (0x15008, 0x0, 0x49249248),
        "eu" => (0x300548, 0x0, 0x49249248),
        "fa" => (0x60, 0x0, 0x49249248),
        "fr" => (0x7129508, 0x0, 0x49249248),
        "ga" | "gd" => (0x200400, 0x0, 0x49249248),
        "gu" | "hi" | "mr" | "or" | "pa" => (0x10, 0x0, 0x14aa8),
        "he" => (0x40e0, 0x0, 0x49249248),
        "ht" => (0x1128500, 0x0, 0x49249248),
        "hu" => (0xb21b400, 0x0, 0x49249248),
        "hy" => (0x1408, 0x0, 0x49249248),
        "ia" => (0x5200408, 0x1800, 0x49249248),
        "id" | "ms" => (0x1001008, 0x0, 0x49249248),
        "io" => (0x1001468, 0x0, 0x49249248),
        "is" => (0x8e8, 0x2, 0x49249248),
        "it" => (0xb002608, 0x2000, 0x49249248),
        "ka" => (0x130a420, 0x0, 0x49249248),
        "kk" | "mn" => (0xc400, 0x0, 0x49249248),
        "kn" => (0x1, 0x0, 0x14a8),
        "ko" => (0x400, 0x4000, 0x11111110),
        "ku" => (0x40468, 0x0, 0x49249248),
        "la" => (0x1000000, 0x0, 0x49249248),
        "lt" => (0x18408, 0x100, 0x49249248),
        "ltg" | "lv" => (0x18408, 0x0, 0x49249248),
        "mk" => (0x204428, 0x8a, 0x49249248),
        "ml" => (0x200400, 0x20000, 0x14a8),
        "nb" => (0x11848, 0x0, 0x49249248),
        "ne" => (0x10, 0x0, 0xaaaaaaa8),
        "nl" => (0x211c18, 0x0, 0x49249248),
        "om" => (0x440, 0x200, 0x49249248),
        "pl" => (0x5008, 0xc0, 0x49249248),
        "pt" => (0x2004068, 0x3800, 0x49249248),
        "qdb" => (0x19, 0x0, 0x49249248),
        "ro" => (0x1007028, 0x1e, 0x49249248),
        "ru" | "uk" => (0xa408, 0x200042, 0x49249248),
        "rup" => (0x7008, 0x0, 0x49249248),
        "sd" | "ur" => (0x10, 0x0, 0x52a8),
        "shn" => (0x1, 0x0, 0x18),
        "si" => (0x1020e000, 0x10000, 0x14aa8),
        "sk" => (0x1004400, 0x80, 0x49249248),
        "sl" => (0x1015418, 0x100, 0x49249248),
        "smj" => (0x15418, 0x0, 0x49249248),
        "sq" => (0x8068, 0x0, 0x49249248),
        "sv" => (0x1908, 0x0, 0x49249248),
        "sw" | "tn" => (0x4e0, 0x0, 0x49249248),
        "ta" => (0x200000, 0x40000, 0x14a8),
        "te" => (0x1, 0x80000, 0x14a8),
        "tr" => (0x204508, 0x0, 0x49249248),
        "tt" => (0x208508, 0x0, 0x49249248),
        "vi" => (0xc08008, 0x0, 0x49249248),
        "yue" => (0x1, 0x100000, 0x18),
        _ => return None,
    })
}

/// `M_Variant`: the key of a scale word, by the count before it.
fn scale_key(flags: &NumberFlags, value: u32) -> &'static str {
    const VAR_BITS: u32 = 0x1c0;
    let teen = (11..20).contains(&(value % 100));
    let units = value % 10;
    let few = (2..=4).contains(&units);
    match flags.numbers2 & VAR_BITS {
        0x40 if !teen && units == 1 => "1MA",
        0x40 if !teen && few => "0MA",
        0x80 if (2..=4).contains(&value) => "0MA",
        0xc0 if !teen && few => "0MA",
        0x100 if teen || units == 0 => "0MB",
        0x100 if units == 1 => "0MA",
        0x140 if !teen && units == 1 => "1M",
        0x140 if !teen && few => "0MA",
        _ => "0M",
    }
}

struct Speller<'a> {
    dict: &'a Dictionary,
    flags: NumberFlags,
    /// `speak_missing_thousands`.
    missing: i32,
    /// `ph_ordinal2` and `ph_ordinal2x`.
    suffix: Vec<u8>,
    suffix_x: Vec<u8>,
    /// `LookupNum3` control `0x400`: the feminine form of the last digits.
    feminine_units: bool,
}

impl Speller<'_> {
    fn word(&self, key: &str) -> Vec<u8> {
        trim_lookup(&lookup_num_phonemes(self.dict, key)).to_vec()
    }

    /// The first of `keys` the dictionary has.
    fn first(&self, keys: &[String]) -> Vec<u8> {
        keys.iter()
            .map(|key| self.word(key))
            .find(|ph| !ph.is_empty())
            .unwrap_or_default()
    }

    /// `LookupThousands`: the scale word, and whether it includes the count.
    fn thousands(&mut self, value: u32, scale: u32, exact: bool, ordinal: bool) -> (Vec<u8>, bool) {
        let mut keys = Vec::new();
        if value > 0 {
            if exact {
                if ordinal {
                    keys.push(format!("_{value}M{scale}o"));
                }
                keys.push(format!("_{value}M{scale}x"));
            }
            keys.push(format!("_{value}M{scale}"));
        }
        let mut word = self.first(&keys);
        let mut found = !word.is_empty();
        let mut of = Vec::new();
        if !found {
            if value % 100 >= 20 {
                of = self.word("_0of");
            }
            let key = scale_key(&self.flags, value);
            let mut keys = Vec::new();
            if exact {
                if ordinal {
                    keys.push(format!("_{key}{scale}o"));
                }
                keys.push(format!("_{key}{scale}x"));
            }
            keys.push(format!("_{key}{scale}"));
            word = self.first(&keys);
            if word.is_empty() {
                if scale > 3 && self.word(&format!("_0M{}", scale - 1)).is_empty() {
                    // Neither this scale nor the one below has a name.
                    word = self.word("_0M2");
                    self.missing = 3;
                }
                if word.is_empty() {
                    // Repeat "thousand" for a scale without a name.
                    word = self.word(&format!("_{value}M1"));
                    found = !word.is_empty();
                    if !found {
                        word = self.word("_0M1");
                    }
                    self.missing = 2;
                }
            }
        }
        of.extend(word);
        let omit = value == 1 && scale == 1 && self.flags.has(NUM_OMIT_1_THOUSAND);
        (of, found || omit)
    }

    /// `LookupNum2`: tens and units, and whether they were joined by "and".
    fn num2(&self, value: u32, scale: u32, control: u32) -> (Vec<u8>, bool) {
        let flags = &self.flags;
        let last = control & 2 != 0;
        let feminine = control & 8 != 0;
        let is_ordinal = control & 1 != 0;
        let ord_type = if control & 0x20 != 0 { 'q' } else { 'o' };
        let mut ph_ordinal = Vec::new();
        let mut found_ordinal = false;
        // The form used before a hundred or a scale word.
        let combining = if flags.has2(NUM2_ORDINAL_AND_THOUSANDS) && scale <= 1 {
            'o'
        } else {
            'a'
        };

        let mut whole = Vec::new();
        if feminine {
            whole = self.first(&[format!("_{value}fx"), format!("_{value}f")]);
        } else if is_ordinal {
            ph_ordinal.clone_from(&self.suffix);
            if control & 4 != 0 {
                whole = self.word(&format!("_{value}{ord_type}x"));
                if !whole.is_empty() && !self.suffix_x.is_empty() {
                    ph_ordinal.clone_from(&self.suffix_x);
                }
            }
            if whole.is_empty() {
                whole = self.word(&format!("_{value}{ord_type}"));
            }
            found_ordinal = !whole.is_empty();
        }
        if whole.is_empty() {
            let mut keys = Vec::new();
            if !last {
                keys.push(format!("_{value}{combining}"));
            }
            // (Portuguese makes 10-99 from tens and units.)
            if !(is_ordinal && flags.has2(NUM2_NO_TEEN_ORDINALS)) {
                keys.push(format!("_{value}"));
            }
            whole = self.first(&keys);
        }

        let mut tens = Vec::new();
        let mut digits = Vec::new();
        if value < 10 && control & 0x10 != 0 {
            tens = self.word("_0");
            digits = whole;
        } else if !whole.is_empty() {
            digits = whole;
        } else {
            let mut units = value % 10;
            let suffix = if control & 0x200 != 0 { "f" } else { "" };
            if is_ordinal {
                tens = self.word(&format!("_{}X{ord_type}", value / 10));
                if !tens.is_empty() {
                    found_ordinal = true;
                    if units != 0 && flags.has2(NUM2_MULTIPLE_ORDINAL) {
                        tens.extend(&self.suffix);
                    }
                }
            }
            if !found_ordinal {
                tens = self.word(&format!("_{}X{suffix}", value / 10));
            }
            if tens.is_empty() && flags.has(NUM_VIGESIMAL) {
                units = value % 20;
                tens = self.word(&format!("_{}X", (value / 10) & 0xfe));
            }
            if units > 0 {
                if feminine {
                    digits = self.word(&format!("_{units}f"));
                }
                if is_ordinal && !flags.has(NUM_SWAP_TENS) {
                    digits = self.word(&format!("_{units}{ord_type}"));
                    found_ordinal |= !digits.is_empty();
                }
                let mut keys = Vec::new();
                if !last || flags.has(NUM_SWAP_TENS) {
                    keys.push(format!("_{units}{combining}"));
                }
                keys.push(format!("_{units}"));
                if digits.is_empty() {
                    digits = self.first(&keys);
                }
                // Italian "trenta" + "uno" is "trentuno".
                if flags.has(NUM_SINGLE_VOWEL) && matches!(units, 1 | 8) {
                    tens.pop();
                }
            }
        }

        if is_ordinal && !found_ordinal && ph_ordinal.is_empty() {
            if value >= 20 && (value % 10 == 0 || flags.has(NUM_SWAP_TENS)) {
                ph_ordinal = self.word("_ord20");
            }
            if ph_ordinal.is_empty() {
                ph_ordinal = self.word("_ord");
            }
        }

        let joined =
            flags.has(NUM_SWAP_TENS | NUM_AND_UNITS) && !tens.is_empty() && !digits.is_empty();
        let mut out = Vec::new();
        if joined {
            let and = if is_ordinal && flags.has2(NUM2_ORDINAL_NO_AND) {
                Vec::new()
            } else {
                self.word("_0and")
            };
            let (first, second) = if flags.has(NUM_SWAP_TENS) {
                (digits, tens)
            } else {
                (tens, digits)
            };
            out.extend(first);
            out.extend(and);
            out.extend(second);
        } else {
            out.extend(tens);
            out.extend(digits);
        }
        out.extend(ph_ordinal);
        if flags.has(NUM_SINGLE_STRESS_L) {
            reduce_stresses(out.iter_mut());
        } else if flags.has(NUM_SINGLE_STRESS) {
            reduce_stresses(out.iter_mut().rev());
        }
        (out, joined)
    }

    /// `LookupNum3`: up to four digits.
    fn num3(
        &mut self,
        value: u32,
        suppress_null: bool,
        scale: u32,
        previous: bool,
        ordinal: Option<bool>,
    ) -> Vec<u8> {
        let flags = self.flags;
        let mut previous = previous;
        let mut suppress_null = suppress_null;
        let mut hundreds = value / 100;
        let tensunits = value % 100;
        let mut out = Vec::new();
        let mut has_thousands = false;
        let zero_hundred = flags.has(NUM_ZERO_HUNDRED) && (previous || hundreds >= 10);

        if hundreds > 0 || zero_hundred {
            let mut hundred = if ordinal.is_some() && tensunits == 0 {
                self.word("_0Co")
            } else {
                Vec::new()
            };
            if hundred.is_empty() && tensunits == 0 {
                hundred = self.word("_0C0");
            }
            if hundred.is_empty() {
                hundred = self.word("_0C");
            }
            if hundreds >= 10 && !(flags.has(NUM_1900) && hundreds == 19) {
                let scale_above = if flags.has2(NUM2_MYRIADS) {
                    0
                } else {
                    scale + 1
                };
                let count = hundreds / 10;
                let (word, includes_count) =
                    self.thousands(count, scale_above, value % 1000 == 0, ordinal.is_some());
                let mut digits = Vec::new();
                if !includes_count {
                    let mut control = if flags.feminine(scale_above) { 8 } else { 0 };
                    if flags.malayalam {
                        control = 0x208;
                    }
                    digits = self.num2(count, scale, control).0;
                }
                let (first, second) = if flags.has2(NUM2_SWAP_THOUSANDS) {
                    (word, digits)
                } else {
                    (digits, word)
                };
                out.extend(first);
                out.push(PHON_END_WORD);
                out.extend(second);
                out.push(PHON_END_WORD);
                hundreds %= 10;
                if hundreds == 0 && !zero_hundred {
                    hundred.clear();
                }
                suppress_null = true;
                previous = true;
                has_thousands = true;
            }
            if hundreds > 0 || zero_hundred {
                if flags.has(NUM_AND_HUNDRED) && (previous || has_thousands) {
                    out.extend(self.word("_0and"));
                }
                suppress_null = true;
                let mut digits = Vec::new();
                if ordinal.is_some() && (tensunits == 0 || flags.has2(NUM2_MULTIPLE_ORDINAL)) {
                    digits = self.word(&format!("_{hundreds}Co"));
                    if flags.has2(NUM2_MULTIPLE_ORDINAL) && tensunits > 0 {
                        digits.extend(&self.suffix);
                    }
                }
                if hundreds == 0 {
                    digits = self.word("_0");
                } else {
                    if digits.is_empty()
                        && !(hundreds == 1 && flags.has2(NUM2_OMIT_1_HUNDRED_ONLY) && !previous)
                    {
                        let mut keys = Vec::new();
                        if tensunits == 0 {
                            keys.push(format!("_{hundreds}C0"));
                        }
                        keys.push(format!("_{hundreds}C"));
                        digits = self.first(&keys);
                    }
                    if !digits.is_empty() {
                        hundred.clear();
                    } else if !(hundreds == 1 && flags.has(NUM_OMIT_1_HUNDRED)) {
                        digits = self.num2(hundreds, scale, 0).0;
                    }
                }
                out.extend(digits);
            }
            out.extend(hundred);
        }

        let mut and = Vec::new();
        if tensunits > 0 && !(ordinal.is_some() && flags.has2(NUM2_MULTIPLE_ORDINAL)) {
            let wanted =
                flags.has(NUM_HUNDRED_AND) || (flags.has(NUM_HUNDRED_AND_DIGIT) && tensunits < 10);
            if wanted && (value > 100 || (previous && scale == 0)) {
                and = self.word("_0and");
            }
            if flags.has(NUM_THOUSAND_AND) && hundreds == 0 && (previous || has_thousands) {
                and = self.word("_0and");
            }
        }

        let mut last = Vec::new();
        if tensunits != 0 || !suppress_null {
            let mut control = if scale == 0 {
                match ordinal {
                    // Tens and units only, no higher digits.
                    Some(variant) => {
                        3 | if value < 100 && !previous { 4 } else { 0 }
                            | if variant { 0x20 } else { 0 }
                    }
                    None if self.feminine_units => 2 | 8,
                    None => 2,
                }
            } else if flags.feminine(scale) {
                8
            } else {
                0
            };
            if flags.malayalam && scale == 1 {
                control |= 0x208;
            }
            if flags.has2(NUM2_ZERO_TENS) && (previous || hundreds > 0) {
                control |= 0x10;
            }
            let (phonemes, joined) = self.num2(tensunits, scale, control);
            last = phonemes;
            if joined && flags.has(NUM_SINGLE_AND) {
                and.clear();
            }
        } else if ordinal.is_some() && !self.suffix.is_empty() {
            last.clone_from(&self.suffix);
        }
        out.extend(and);
        out.push(PHON_END_WORD);
        out.extend(last);
        out
    }
}

/// Keep only the first primary stress.
fn reduce_stresses<'a>(phonemes: impl Iterator<Item = &'a mut u8>) {
    let mut found = false;
    for ph in phonemes.filter(|ph| **ph == PHON_STRESS_P) {
        if found {
            *ph = PHON_STRESS_3;
        }
        found = true;
    }
}

/// Without empty words, but for a first one: C's number starts a word after it.
fn tidy(phonemes: &[u8]) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::with_capacity(phonemes.len());
    for &ph in phonemes {
        if ph != PHON_END_WORD || out.last() != Some(&PHON_END_WORD) {
            out.push(ph);
        }
    }
    if out.last() == Some(&PHON_END_WORD) {
        out.pop();
    }
    out
}

/// The spoken groups of a cardinal without leading zeros.
pub(super) fn groups(digits: &str, dict: &Dictionary, flags: NumberFlags) -> Option<Vec<Group>> {
    groups_with(digits, dict, flags, None)
}

/// [`groups`], split into groups even when there are four digits or fewer
/// (a number written with a group separator).
pub(super) fn groups_split(
    digits: &str,
    dict: &Dictionary,
    flags: NumberFlags,
) -> Option<Vec<Group>> {
    groups_inner(digits, dict, flags, None, true)
}

/// [`groups`], read as an ordinal when `ordinal` is given.
pub(super) fn groups_with(
    digits: &str,
    dict: &Dictionary,
    flags: NumberFlags,
    ordinal: Option<&Ordinal>,
) -> Option<Vec<Group>> {
    groups_inner(digits, dict, flags, ordinal, false)
}

fn groups_inner(
    digits: &str,
    dict: &Dictionary,
    flags: NumberFlags,
    ordinal: Option<&Ordinal>,
    split: bool,
) -> Option<Vec<Group>> {
    // A number of up to four digits isn't split.
    let mut parts: Vec<u32> = Vec::new();
    let mut group = 0u32;
    for (i, digit) in digits.chars().enumerate() {
        group = group.checked_mul(10)?.checked_add(digit.to_digit(10)?)?;
        let left = digits.len() - i - 1;
        let breaks = left > 0 && left < 32 && flags.break_numbers & (1 << left) != 0;
        if left == 0 || ((digits.len() > 4 || split) && breaks) {
            parts.push(group);
            group = 0;
        }
    }
    let mut speller = Speller {
        dict,
        flags,
        missing: 0,
        suffix: ordinal.map(|o| o.suffix.clone()).unwrap_or_default(),
        suffix_x: ordinal.map(|o| o.suffix_x.clone()).unwrap_or_default(),
        feminine_units: false,
    };
    let mut out = Vec::new();
    let n = parts.len();
    for (i, &part) in parts.iter().enumerate() {
        let scale = u32::try_from(n - 1 - i).ok()?;
        let previous = i > 0;
        let exact = parts.iter().skip(i + 1).all(|&p| p == 0);
        let mut value = part;
        let mut suppress_null = part == 0 && previous;
        let mut append = Vec::new();
        if suppress_null {
            if speller.missing == 1 && speller.word(&format!("_0M{}", scale + 1)).is_empty() {
                append = speller.word(&format!("_0M{scale}"));
            }
        } else if scale > 0 {
            let (word, includes_count) = speller.thousands(part, scale, exact, false);
            append = word;
            if includes_count {
                value = 0;
                suppress_null = true;
            }
        }
        // (Only the last group is the word with the ordinal mark.)
        let ord = ordinal.filter(|_| scale == 0).map(|o| o.variant);
        let number = speller.num3(value, suppress_null, scale, previous, ord);
        let mut phonemes = Vec::new();
        let (first, second) = if scale > 0 && flags.has2(NUM2_SWAP_THOUSANDS) {
            (append, number)
        } else {
            (number, append)
        };
        // A group that ends in a word break (Vietnamese "không trăm", with no
        // tens) keeps it before the pause that follows.
        let ends_word = second.is_empty() && first.last() == Some(&PHON_END_WORD);
        phonemes.extend(first);
        phonemes.push(PHON_END_WORD);
        phonemes.extend(second);
        let mut phonemes = tidy(&phonemes);
        if ends_word && !exact && !phonemes.is_empty() {
            phonemes.push(PHON_END_WORD);
        }
        speller.missing -= 1;
        // (A silent group in the middle still has its pause.)
        if !phonemes.is_empty() || !exact {
            out.push(Group {
                phonemes,
                pause: !exact,
            });
        }
    }
    Some(out)
}

/// C's `ph_buf` in `TranslateNumber_1`, which a fraction must fit.
const N_WORD_PHONEMES: usize = 200;

/// `LookupDecimal`: a long fraction read in groups of three, each with its
/// scale word, without the integer overflow of reading it whole (Russian
/// `0,123456789012`).  `None` past the scale names C has, or when it outgrows
/// C's buffer: the digits are then spoken one by one.
fn lookup_decimal(speller: &mut Speller<'_>, digits: &[u8], feminine: bool) -> Option<Vec<u8>> {
    let count = digits.len();
    let mut group_len = (count - 1) % 3 + 1;
    let mut group = u32::try_from((count - 1) / 3).ok()?;
    if group > 10 {
        return None;
    }
    let swap = speller.flags.has2(NUM2_SWAP_THOUSANDS);
    let mut out: Vec<u8> = Vec::new();
    let mut previous = false;
    let mut rest = digits;
    while !rest.is_empty() {
        let (part, tail) = rest.split_at(group_len.min(rest.len()));
        rest = tail;
        let value = part.iter().fold(0u32, |v, &d| v * 10 + u32::from(d.wrapping_sub(b'0')));
        if value != 0 || (group == 0 && out.is_empty()) {
            let (scale, exact) = if group > 0 {
                speller.thousands(value, group, false, false)
            } else {
                (Vec::new(), false)
            };
            let number = if exact {
                Vec::new()
            } else {
                speller.feminine_units = feminine;
                let n = speller.num3(value, false, group, previous, None);
                speller.feminine_units = false;
                n
            };
            let mut piece = Vec::new();
            if group > 0 {
                if exact {
                    piece.extend(scale);
                } else if swap {
                    piece.extend(scale);
                    piece.push(PHON_END_WORD);
                    piece.extend(number);
                } else {
                    piece.extend(number);
                    piece.push(PHON_END_WORD);
                    piece.extend(scale);
                }
                piece.push(PHON_END_WORD);
            } else {
                piece = number;
            }
            if out.len() + piece.len() + 1 > N_WORD_PHONEMES {
                return None;
            }
            out.extend(piece);
            previous = true;
        }
        group = group.saturating_sub(1);
        group_len = 3;
    }
    Some(out)
}

/// The digits after a decimal point, as C's `TranslateNumber_1` reads them:
/// the phonemes that follow `_dpt` (a word break where C puts one).
pub(super) fn fraction(digits: &str, dict: &Dictionary, flags: NumberFlags) -> Vec<u8> {
    let mut speller = Speller {
        dict,
        flags,
        missing: 0,
        suffix: Vec::new(),
        suffix_x: Vec::new(),
        feminine_units: false,
    };
    let bytes = digits.as_bytes();
    let mut out = Vec::new();
    let mut n = 0usize;
    let mut decimal_count = bytes.len();
    let mut fraction_suffix = false;
    let num3 = |speller: &mut Speller<'_>, s: &[u8], feminine: bool| -> Vec<u8> {
        let value: u32 = std::str::from_utf8(s)
            .ok()
            .and_then(|t| t.parse().ok())
            .unwrap_or(0);
        speller.feminine_units = feminine;
        let ph = speller.num3(value, false, 0, false, None);
        speller.feminine_units = false;
        ph
    };
    let mode = flags.numbers & NUM_DFRACTION_BITS;
    match mode {
        NUM_DFRACTION_2 | NUM_DFRACTION_4 => {
            let max = if mode == NUM_DFRACTION_4 { 5 } else { 2 };
            while bytes.get(n) == Some(&b'0') {
                out.extend(speller.word("_0"));
                decimal_count -= 1;
                n += 1;
            }
            if decimal_count <= max && bytes.get(n).is_some_and(u8::is_ascii_digit) {
                out.extend(num3(
                    &mut speller,
                    bytes.get(n..).unwrap_or_default(),
                    false,
                ));
                n += decimal_count;
            }
        }
        NUM_DFRACTION_1 | NUM_DFRACTION_5 | NUM_DFRACTION_6 => 'mode: {
            let feminine = flags.has2(NUM2_FRACTION_FEMININE);
            // Only the last two digits choose the suffix, and more than five
            // are read in groups with their scale words (`LookupDecimal`).
            let digit = |k: usize| bytes.get(k).map_or(0, |&b| u32::from(b.wrapping_sub(b'0')));
            let mut value = digit(decimal_count.wrapping_sub(1));
            if decimal_count > 1 {
                value += 10 * digit(decimal_count - 2);
            }
            let mut ph = if decimal_count <= 5 {
                num3(&mut speller, bytes, feminine)
            } else {
                match lookup_decimal(&mut speller, bytes, feminine) {
                    Some(ph) => ph,
                    None => break 'mode,
                }
            };
            if bytes.first() == Some(&b'0') || mode != NUM_DFRACTION_1 {
                let mut suffix = Vec::new();
                if feminine && value % 10 == 1 && value % 100 != 11 {
                    suffix = speller.word(&format!("_0Z{decimal_count}s"));
                }
                if suffix.is_empty() {
                    suffix = speller.word(&format!("_0Z{decimal_count}"));
                }
                if suffix.is_empty() {
                    // Revert to speaking single digits.
                    break 'mode;
                }
                if mode == NUM_DFRACTION_6 {
                    out.extend(suffix);
                } else {
                    if ph.len() + suffix.len() + 1 > N_WORD_PHONEMES {
                        break 'mode;
                    }
                    ph.extend(suffix);
                }
                fraction_suffix = true;
            }
            out.extend(ph);
            n += decimal_count;
        }
        NUM_DFRACTION_3 => {
            if decimal_count <= 4 && bytes.first() != Some(&b'0') {
                out.extend(num3(&mut speller, bytes, false));
                n += decimal_count;
            }
        }
        NUM_DFRACTION_7 => {
            while decimal_count > 1 {
                decimal_count -= 1;
                let Some(&d) = bytes.get(n) else { break };
                let alt = speller.word(&format!("_{}d", d as char));
                if alt.is_empty() {
                    break;
                }
                n += 1;
                out.extend(alt);
            }
        }
        _ => {}
    }
    // Any digits left are spoken one by one.
    while let Some(&d) = bytes.get(n) {
        n += 1;
        out.push(PHON_END_WORD);
        out.extend(speller.num2(u32::from(d - b'0'), 0, 2).0);
    }
    if !fraction_suffix {
        out.extend(speller.word("_dpt2"));
    }
    out
}
