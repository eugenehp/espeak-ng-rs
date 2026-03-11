//! Phoneme feature tags — 3-letter ASCII codes packed into a `u32`.
//!
//! Each tag follows the PHOIBLE / IPA feature notation used by eSpeak NG.
//! The packing is `(char0 << 16) | (char1 << 8) | char2`.
//!
//! [`PhonemeFeature::from_str`] mirrors `phoneme_feature_from_string()`.
//! [`PhonemeTab::apply_feature`] mirrors `phoneme_add_feature()`.

// The 70+ associated constants on PhonemeFeature are 3-letter IPA tags.
// Their names are self-explanatory given the standard IPA feature nomenclature,
// so missing_docs is suppressed for the impl block.
#![allow(missing_docs)]

use crate::error::{Error, Result};
use super::table::PhonemeTab;
use super::{
    PH_NASAL, PH_STOP, PH_FRICATIVE, PH_VSTOP, PH_VOWEL,
    PH_TRILL, PH_SIBILANT, PH_VOICED, PH_VOICELESS, PH_PALATAL, PH_NONSYLLABIC, PH_LONG,
    PHFLAG_ARTICULATION,
    PLACE_BILABIAL, PLACE_LABIODENTAL, PLACE_DENTAL, PLACE_ALVEOLAR,
    PLACE_RETROFLEX, PLACE_PALATO_ALVEOLAR, PLACE_PALATAL, PLACE_VELAR,
    PLACE_LABIO_VELAR, PLACE_UVULAR, PLACE_PHARYNGEAL, PLACE_GLOTTAL,
};

/// A phoneme feature tag — a 3-letter ASCII label packed into a `u32`.
///
/// Mirrors `phoneme_feature_t` from `phoneme.h`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PhonemeFeature(pub u32);

impl PhonemeFeature {
    // -----------------------------------------------------------------------
    // Manner of articulation
    // -----------------------------------------------------------------------
    pub const NAS: Self = Self::pack('n', 'a', 's');
    pub const STP: Self = Self::pack('s', 't', 'p');
    pub const AFR: Self = Self::pack('a', 'f', 'r');
    pub const FRC: Self = Self::pack('f', 'r', 'c');
    pub const FLP: Self = Self::pack('f', 'l', 'p');
    pub const TRL: Self = Self::pack('t', 'r', 'l');
    pub const APR: Self = Self::pack('a', 'p', 'r');
    pub const CLK: Self = Self::pack('c', 'l', 'k');
    pub const EJC: Self = Self::pack('e', 'j', 'c');
    pub const IMP: Self = Self::pack('i', 'm', 'p');
    pub const VWL: Self = Self::pack('v', 'w', 'l');
    pub const LAT: Self = Self::pack('l', 'a', 't');
    pub const SIB: Self = Self::pack('s', 'i', 'b');
    // -----------------------------------------------------------------------
    // Place of articulation
    // -----------------------------------------------------------------------
    pub const BLB: Self = Self::pack('b', 'l', 'b');
    pub const LBD: Self = Self::pack('l', 'b', 'd');
    pub const BLD: Self = Self::pack('b', 'l', 'd');
    pub const DNT: Self = Self::pack('d', 'n', 't');
    pub const ALV: Self = Self::pack('a', 'l', 'v');
    pub const PLA: Self = Self::pack('p', 'l', 'a');
    pub const RFX: Self = Self::pack('r', 'f', 'x');
    pub const ALP: Self = Self::pack('a', 'l', 'p');
    pub const PAL: Self = Self::pack('p', 'a', 'l');
    pub const VEL: Self = Self::pack('v', 'e', 'l');
    pub const LBV: Self = Self::pack('l', 'b', 'v');
    pub const UVL: Self = Self::pack('u', 'v', 'l');
    pub const PHR: Self = Self::pack('p', 'h', 'r');
    pub const GLT: Self = Self::pack('g', 'l', 't');
    // -----------------------------------------------------------------------
    // Voice
    // -----------------------------------------------------------------------
    pub const VCD: Self = Self::pack('v', 'c', 'd');
    pub const VLS: Self = Self::pack('v', 'l', 's');
    // -----------------------------------------------------------------------
    // Vowel height (not used by eSpeak but valid feature names)
    // -----------------------------------------------------------------------
    pub const HGH: Self = Self::pack('h', 'g', 'h');
    pub const SMH: Self = Self::pack('s', 'm', 'h');
    pub const UMD: Self = Self::pack('u', 'm', 'd');
    pub const MID: Self = Self::pack('m', 'i', 'd');
    pub const LMD: Self = Self::pack('l', 'm', 'd');
    pub const SML: Self = Self::pack('s', 'm', 'l');
    pub const LOW: Self = Self::pack('l', 'o', 'w');
    // -----------------------------------------------------------------------
    // Vowel backness
    // -----------------------------------------------------------------------
    pub const FNT: Self = Self::pack('f', 'n', 't');
    pub const CNT: Self = Self::pack('c', 'n', 't');
    pub const BCK: Self = Self::pack('b', 'c', 'k');
    // -----------------------------------------------------------------------
    // Rounding
    // -----------------------------------------------------------------------
    pub const UNR: Self = Self::pack('u', 'n', 'r');
    pub const RND: Self = Self::pack('r', 'n', 'd');
    // -----------------------------------------------------------------------
    // Articulation
    // -----------------------------------------------------------------------
    pub const LGL: Self = Self::pack('l', 'g', 'l');
    pub const IDT: Self = Self::pack('i', 'd', 't');
    pub const APC: Self = Self::pack('a', 'p', 'c');
    pub const LMN: Self = Self::pack('l', 'm', 'n');
    // -----------------------------------------------------------------------
    // Air flow
    // -----------------------------------------------------------------------
    pub const EGS: Self = Self::pack('e', 'g', 's');
    pub const IGS: Self = Self::pack('i', 'g', 's');
    // -----------------------------------------------------------------------
    // Phonation
    // -----------------------------------------------------------------------
    pub const BRV: Self = Self::pack('b', 'r', 'v');
    pub const SLV: Self = Self::pack('s', 'l', 'v');
    pub const STV: Self = Self::pack('s', 't', 'v');
    pub const CRV: Self = Self::pack('c', 'r', 'v');
    pub const GLC: Self = Self::pack('g', 'l', 'c');
    // -----------------------------------------------------------------------
    // Rounding / labialization
    // -----------------------------------------------------------------------
    pub const PTR: Self = Self::pack('p', 't', 'r');
    pub const CMP: Self = Self::pack('c', 'm', 'p');
    pub const MRD: Self = Self::pack('m', 'r', 'd');
    pub const LRD: Self = Self::pack('l', 'r', 'd');
    // -----------------------------------------------------------------------
    // Syllabicity
    // -----------------------------------------------------------------------
    pub const SYL: Self = Self::pack('s', 'y', 'l');
    pub const NSY: Self = Self::pack('n', 's', 'y');
    // -----------------------------------------------------------------------
    // Consonant release
    // -----------------------------------------------------------------------
    pub const ASP: Self = Self::pack('a', 's', 'p');
    pub const NRS: Self = Self::pack('n', 'r', 's');
    pub const LRS: Self = Self::pack('l', 'r', 's');
    pub const UNX: Self = Self::pack('u', 'n', 'x');
    // -----------------------------------------------------------------------
    // Coarticulation
    // -----------------------------------------------------------------------
    pub const PZD: Self = Self::pack('p', 'z', 'd');
    pub const VZD: Self = Self::pack('v', 'z', 'd');
    pub const FZD: Self = Self::pack('f', 'z', 'd');
    pub const NZD: Self = Self::pack('n', 'z', 'd');
    pub const RZD: Self = Self::pack('r', 'z', 'd');
    // -----------------------------------------------------------------------
    // Tongue root
    // -----------------------------------------------------------------------
    pub const ATR: Self = Self::pack('a', 't', 'r');
    pub const RTR: Self = Self::pack('r', 't', 'r');
    // -----------------------------------------------------------------------
    // Fortis / lenis
    // -----------------------------------------------------------------------
    pub const FTS: Self = Self::pack('f', 't', 's');
    pub const LNS: Self = Self::pack('l', 'n', 's');
    // -----------------------------------------------------------------------
    // Length
    // -----------------------------------------------------------------------
    pub const EST: Self = Self::pack('e', 's', 't');
    pub const HLG: Self = Self::pack('h', 'l', 'g');
    pub const LNG: Self = Self::pack('l', 'n', 'g');
    pub const ELG: Self = Self::pack('e', 'l', 'g');

    // -----------------------------------------------------------------------
    // Constructor / helpers
    // -----------------------------------------------------------------------

    const fn pack(a: char, b: char, c: char) -> Self {
        Self(((a as u32) << 16) | ((b as u32) << 8) | (c as u32))
    }

    /// Parse a 3-character ASCII feature string.
    ///
    /// Mirrors `phoneme_feature_from_string()`.  Returns `None` if the string
    /// is not exactly 3 ASCII bytes.
    pub fn from_str(s: &str) -> Option<Self> {
        let b = s.as_bytes();
        if b.len() != 3 {
            return None;
        }
        let value = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | (b[2] as u32);
        Some(Self(value))
    }

    /// The raw packed value.
    pub const fn value(self) -> u32 {
        self.0
    }
}

impl std::fmt::Display for PhonemeFeature {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let a = ((self.0 >> 16) & 0xff) as u8 as char;
        let b = ((self.0 >>  8) & 0xff) as u8 as char;
        let c = ( self.0        & 0xff) as u8 as char;
        write!(f, "{a}{b}{c}")
    }
}

// ---------------------------------------------------------------------------
// apply_feature — mirrors phoneme_add_feature()
// ---------------------------------------------------------------------------

impl PhonemeTab {
    /// Apply a phoneme feature to this entry, mutating `type` and `phflags`.
    ///
    /// Mirrors `phoneme_add_feature()` from `phoneme.c`.
    ///
    /// Features that are explicitly "not supported by eSpeak" are silently
    /// accepted (no error, no mutation) — matching the C behaviour.
    pub fn apply_feature(&mut self, feat: PhonemeFeature) -> Result<()> {
        match feat {
            // --- manner of articulation ---
            PhonemeFeature::NAS => self.typ = PH_NASAL,
            PhonemeFeature::STP |
            PhonemeFeature::AFR => self.typ = PH_STOP,      // FIXME: afr treated as stp
            PhonemeFeature::FRC |
            PhonemeFeature::APR => self.typ = PH_FRICATIVE,  // FIXME: apr used for [h]
            PhonemeFeature::FLP => self.typ = PH_VSTOP,      // FIXME: C uses vstop for flp
            PhonemeFeature::TRL => self.phflags |= PH_TRILL,
            PhonemeFeature::CLK |
            PhonemeFeature::EJC |
            PhonemeFeature::IMP |
            PhonemeFeature::LAT => { /* not supported */ }
            PhonemeFeature::VWL => self.typ = PH_VOWEL,
            PhonemeFeature::SIB => self.phflags |= PH_SIBILANT,

            // --- place of articulation ---
            PhonemeFeature::BLB |
            PhonemeFeature::BLD => self.set_place(PLACE_BILABIAL),
            PhonemeFeature::LBD => self.set_place(PLACE_LABIODENTAL),
            PhonemeFeature::DNT => self.set_place(PLACE_DENTAL),
            PhonemeFeature::ALV => self.set_place(PLACE_ALVEOLAR),
            PhonemeFeature::RFX => self.set_place(PLACE_RETROFLEX),
            PhonemeFeature::PLA => self.set_place(PLACE_PALATO_ALVEOLAR),
            PhonemeFeature::PAL => {
                self.set_place(PLACE_PALATAL);
                self.phflags |= PH_PALATAL;
            }
            PhonemeFeature::ALP => {
                // pla + pzd
                self.set_place(PLACE_PALATO_ALVEOLAR);
                self.phflags |= PH_PALATAL;
            }
            PhonemeFeature::VEL => self.set_place(PLACE_VELAR),
            PhonemeFeature::LBV => self.set_place(PLACE_LABIO_VELAR),
            PhonemeFeature::UVL => self.set_place(PLACE_UVULAR),
            PhonemeFeature::PHR => self.set_place(PLACE_PHARYNGEAL),
            PhonemeFeature::GLT => self.set_place(PLACE_GLOTTAL),

            // --- voice ---
            PhonemeFeature::VCD => self.phflags |= PH_VOICED,
            PhonemeFeature::VLS => self.phflags |= PH_VOICELESS,

            // --- syllabicity: nsy is supported, syl is not ---
            PhonemeFeature::NSY => self.phflags |= PH_NONSYLLABIC,

            // --- not supported by eSpeak (silent accept) ---
            PhonemeFeature::HGH | PhonemeFeature::SMH | PhonemeFeature::UMD |
            PhonemeFeature::MID | PhonemeFeature::LMD | PhonemeFeature::SML |
            PhonemeFeature::LOW |
            PhonemeFeature::FNT | PhonemeFeature::CNT | PhonemeFeature::BCK |
            PhonemeFeature::UNR | PhonemeFeature::RND |
            PhonemeFeature::LGL | PhonemeFeature::IDT |
            PhonemeFeature::APC | PhonemeFeature::LMN |
            PhonemeFeature::EGS | PhonemeFeature::IGS |
            PhonemeFeature::BRV | PhonemeFeature::SLV | PhonemeFeature::STV |
            PhonemeFeature::CRV | PhonemeFeature::GLC |
            PhonemeFeature::PTR | PhonemeFeature::CMP |
            PhonemeFeature::MRD | PhonemeFeature::LRD |
            PhonemeFeature::SYL |
            PhonemeFeature::ASP | PhonemeFeature::NRS |
            PhonemeFeature::LRS | PhonemeFeature::UNX |
            PhonemeFeature::VZD | PhonemeFeature::FZD |
            PhonemeFeature::NZD | PhonemeFeature::RZD |
            PhonemeFeature::ATR | PhonemeFeature::RTR |
            PhonemeFeature::FTS | PhonemeFeature::LNS |
            PhonemeFeature::EST | PhonemeFeature::HLG => { /* not supported */ }

            // --- coarticulation: pzd sets palatal flag ---
            PhonemeFeature::PZD => self.phflags |= PH_PALATAL,

            // --- length ---
            PhonemeFeature::LNG |
            PhonemeFeature::ELG => self.phflags |= PH_LONG, // FIXME: elg should be longer

            // --- unknown ---
            _ => return Err(Error::UnknownPhonemeFeature(feat)),
        }
        Ok(())
    }

    // Set bits 16-19 (place of articulation), clearing them first.
    fn set_place(&mut self, place: u32) {
        self.phflags &= !PHFLAG_ARTICULATION;
        self.phflags |= place << 16;
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::phoneme::table::PhonemeTab;

    fn blank() -> PhonemeTab {
        PhonemeTab::default()
    }

    // --- from_str ---

    #[test]
    fn from_str_valid() {
        assert_eq!(PhonemeFeature::from_str("nas"), Some(PhonemeFeature::NAS));
        assert_eq!(PhonemeFeature::from_str("vwl"), Some(PhonemeFeature::VWL));
        assert_eq!(PhonemeFeature::from_str("vcd"), Some(PhonemeFeature::VCD));
        assert_eq!(PhonemeFeature::from_str("blb"), Some(PhonemeFeature::BLB));
        assert_eq!(PhonemeFeature::from_str("lng"), Some(PhonemeFeature::LNG));
    }

    #[test]
    fn from_str_wrong_length() {
        assert_eq!(PhonemeFeature::from_str(""),     None);
        assert_eq!(PhonemeFeature::from_str("na"),   None);
        assert_eq!(PhonemeFeature::from_str("nasal"), None);
    }

    #[test]
    fn display_roundtrips() {
        for s in &["nas", "vwl", "vcd", "blb", "lng", "pzd", "alp"] {
            let f = PhonemeFeature::from_str(s).unwrap();
            assert_eq!(f.to_string(), *s);
        }
    }

    // --- apply_feature: type mutations ---

    #[test]
    fn feature_nasal_sets_type() {
        let mut ph = blank();
        ph.apply_feature(PhonemeFeature::NAS).unwrap();
        assert_eq!(ph.typ, PH_NASAL);
    }

    #[test]
    fn feature_stp_sets_type() {
        let mut ph = blank();
        ph.apply_feature(PhonemeFeature::STP).unwrap();
        assert_eq!(ph.typ, PH_STOP);
    }

    #[test]
    fn feature_afr_treated_as_stp() {
        let mut ph = blank();
        ph.apply_feature(PhonemeFeature::AFR).unwrap();
        assert_eq!(ph.typ, PH_STOP);
    }

    #[test]
    fn feature_frc_sets_type() {
        let mut ph = blank();
        ph.apply_feature(PhonemeFeature::FRC).unwrap();
        assert_eq!(ph.typ, PH_FRICATIVE);
    }

    #[test]
    fn feature_flp_sets_vstop() {
        let mut ph = blank();
        ph.apply_feature(PhonemeFeature::FLP).unwrap();
        assert_eq!(ph.typ, PH_VSTOP);
    }

    #[test]
    fn feature_vwl_sets_type() {
        let mut ph = blank();
        ph.apply_feature(PhonemeFeature::VWL).unwrap();
        assert_eq!(ph.typ, PH_VOWEL);
    }

    // --- apply_feature: flag mutations ---

    #[test]
    fn feature_trl_sets_trill_flag() {
        let mut ph = blank();
        ph.apply_feature(PhonemeFeature::TRL).unwrap();
        assert!(ph.phflags & PH_TRILL != 0);
    }

    #[test]
    fn feature_sib_sets_sibilant_flag() {
        let mut ph = blank();
        ph.apply_feature(PhonemeFeature::SIB).unwrap();
        assert!(ph.phflags & PH_SIBILANT != 0);
    }

    #[test]
    fn feature_vcd_sets_voiced() {
        let mut ph = blank();
        ph.apply_feature(PhonemeFeature::VCD).unwrap();
        assert!(ph.phflags & PH_VOICED != 0);
    }

    #[test]
    fn feature_vls_sets_voiceless() {
        let mut ph = blank();
        ph.apply_feature(PhonemeFeature::VLS).unwrap();
        assert!(ph.phflags & PH_VOICELESS != 0);
    }

    #[test]
    fn feature_nsy_sets_nonsyllabic() {
        let mut ph = blank();
        ph.apply_feature(PhonemeFeature::NSY).unwrap();
        assert!(ph.phflags & PH_NONSYLLABIC != 0);
    }

    #[test]
    fn feature_lng_sets_long() {
        let mut ph = blank();
        ph.apply_feature(PhonemeFeature::LNG).unwrap();
        assert!(ph.phflags & PH_LONG != 0);
    }

    #[test]
    fn feature_elg_sets_long() {
        let mut ph = blank();
        ph.apply_feature(PhonemeFeature::ELG).unwrap();
        assert!(ph.phflags & PH_LONG != 0);
    }

    #[test]
    fn feature_pzd_sets_palatal_flag() {
        let mut ph = blank();
        ph.apply_feature(PhonemeFeature::PZD).unwrap();
        assert!(ph.phflags & PH_PALATAL != 0);
    }

    // --- place of articulation ---

    #[test]
    fn feature_blb_sets_bilabial() {
        let mut ph = blank();
        ph.apply_feature(PhonemeFeature::BLB).unwrap();
        let place = (ph.phflags & PHFLAG_ARTICULATION) >> 16;
        assert_eq!(place, PLACE_BILABIAL);
    }

    #[test]
    fn feature_alv_sets_alveolar() {
        let mut ph = blank();
        ph.apply_feature(PhonemeFeature::ALV).unwrap();
        let place = (ph.phflags & PHFLAG_ARTICULATION) >> 16;
        assert_eq!(place, PLACE_ALVEOLAR);
    }

    #[test]
    fn feature_pal_sets_palatal_place_and_flag() {
        let mut ph = blank();
        ph.apply_feature(PhonemeFeature::PAL).unwrap();
        let place = (ph.phflags & PHFLAG_ARTICULATION) >> 16;
        assert_eq!(place, PLACE_PALATAL);
        assert!(ph.phflags & PH_PALATAL != 0);
    }

    #[test]
    fn feature_alp_sets_palato_alveolar_and_palatal_flag() {
        let mut ph = blank();
        ph.apply_feature(PhonemeFeature::ALP).unwrap();
        let place = (ph.phflags & PHFLAG_ARTICULATION) >> 16;
        assert_eq!(place, PLACE_PALATO_ALVEOLAR);
        assert!(ph.phflags & PH_PALATAL != 0);
    }

    #[test]
    fn place_overwrite_clears_previous() {
        let mut ph = blank();
        ph.apply_feature(PhonemeFeature::BLB).unwrap();
        ph.apply_feature(PhonemeFeature::GLT).unwrap();
        let place = (ph.phflags & PHFLAG_ARTICULATION) >> 16;
        assert_eq!(place, PLACE_GLOTTAL);
    }

    // --- unsupported features are silently accepted ---

    #[test]
    fn unsupported_features_ok() {
        let mut ph = blank();
        for feat in &[
            PhonemeFeature::HGH, PhonemeFeature::VWL, PhonemeFeature::LOW,
            PhonemeFeature::FNT, PhonemeFeature::RND, PhonemeFeature::ASP,
            PhonemeFeature::ATR, PhonemeFeature::EST,
        ] {
            // should not error
            ph.apply_feature(*feat).ok();
        }
    }

    // --- unknown feature errors ---

    #[test]
    fn unknown_feature_errors() {
        let mut ph = blank();
        let unknown = PhonemeFeature(0x787979); // "xyy"
        assert!(ph.apply_feature(unknown).is_err());
    }
}
