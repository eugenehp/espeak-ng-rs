//! Phoneme type definitions, flag constants, and binary data loader.
//!
//! Rust port of `phoneme.h`, `phoneme.c`, and the phoneme-loading portions
//! of `synthdata.c`.
//!
//! # Binary data layout
//!
//! The `phontab` file contains a list of phoneme tables:
//! ```text
//! [u8 n_tables, u8 0, u8 0, u8 0]
//! For each table:
//!   [u8 n_phonemes, u8 includes, u8 0, u8 0]
//!   [u8 × 32]   name (null-padded)
//!   [PhonemeTab × n_phonemes]   entries (16 bytes each, little-endian)
//! ```
//!
//! `phondata` starts with `[u32le version, u32le sample_rate, …]`.
//! `phonindex` is a flat array of `u16le` word-indices into `phondata`.
//! `intonations` is a flat array of `Tune` structs (68 bytes each).

pub mod feature;
pub mod load;
pub mod table;

pub use feature::PhonemeFeature;
pub use load::PhonemeData;
pub use table::{PhonemeTab, PhonemeTabList, ReplacePhoneme};

// ---------------------------------------------------------------------------
// Phoneme type constants  (PHONEME_TAB::type)
// ---------------------------------------------------------------------------

/// Phoneme type: pause / silence.  C: `phPAUSE`.
pub const PH_PAUSE:       u8 = 0;
/// Phoneme type: stress marker.  C: `phSTRESS`.
pub const PH_STRESS:      u8 = 1;
/// Phoneme type: vowel.  C: `phVOWEL`.
pub const PH_VOWEL:       u8 = 2;
/// Phoneme type: liquid (l, r).  C: `phLIQUID`.
pub const PH_LIQUID:      u8 = 3;
/// Phoneme type: voiceless stop (p, t, k).  C: `phSTOP`.
pub const PH_STOP:        u8 = 4;
/// Phoneme type: voiced stop (b, d, g).  C: `phVSTOP`.
pub const PH_VSTOP:       u8 = 5;
/// Phoneme type: voiceless fricative (f, s, ʃ).  C: `phFRICATIVE`.
pub const PH_FRICATIVE:   u8 = 6;
/// Phoneme type: voiced fricative (v, z, ʒ).  C: `phVFRICATIVE`.
pub const PH_VFRICATIVE:  u8 = 7;
/// Phoneme type: nasal (m, n, ŋ).  C: `phNASAL`.
pub const PH_NASAL:       u8 = 8;
/// Phoneme type: virtual / placeholder.  C: `phVIRTUAL`.
pub const PH_VIRTUAL:     u8 = 9;
/// Phoneme type: deleted (marked for removal).  C: `phDELETED`.
pub const PH_DELETED:     u8 = 14;
/// Phoneme type: invalid / sentinel.  C: `phINVALID`.
pub const PH_INVALID:     u8 = 15;

// ---------------------------------------------------------------------------
// Place of articulation  (stored in bits 16-19 of phflags)
// ---------------------------------------------------------------------------

/// Place of articulation: bilabial (p, b, m).
pub const PLACE_BILABIAL:        u32 = 1;
/// Place of articulation: labiodental (f, v).
pub const PLACE_LABIODENTAL:     u32 = 2;
/// Place of articulation: dental (θ, ð).
pub const PLACE_DENTAL:          u32 = 3;
/// Place of articulation: alveolar (t, d, s, z, n, l).
pub const PLACE_ALVEOLAR:        u32 = 4;
/// Place of articulation: retroflex.
pub const PLACE_RETROFLEX:       u32 = 5;
/// Place of articulation: palato-alveolar (ʃ, ʒ, tʃ, dʒ).
pub const PLACE_PALATO_ALVEOLAR: u32 = 6;
/// Place of articulation: palatal (j).
pub const PLACE_PALATAL:         u32 = 7;
/// Place of articulation: velar (k, g, ŋ).
pub const PLACE_VELAR:           u32 = 8;
/// Place of articulation: labio-velar (w).
pub const PLACE_LABIO_VELAR:     u32 = 9;
/// Place of articulation: uvular (q, ʁ).
pub const PLACE_UVULAR:          u32 = 10;
/// Place of articulation: pharyngeal.
pub const PLACE_PHARYNGEAL:      u32 = 11;
/// Place of articulation: glottal (h, ʔ).
pub const PLACE_GLOTTAL:         u32 = 12;

// ---------------------------------------------------------------------------
// Phoneme flag bits  (PHONEME_TAB::phflags)
// ---------------------------------------------------------------------------

/// Mask for the articulation place field (bits 16-19).
pub const PHFLAG_ARTICULATION: u32 = 0x000f_0000;

/// Flag: phoneme has reduced / unstressed form.
pub const PH_UNSTRESSED:   u32 = 1 << 1;
/// Flag: phoneme is voiceless.
pub const PH_VOICELESS:    u32 = 1 << 3;
/// Flag: phoneme is voiced.
pub const PH_VOICED:       u32 = 1 << 4;
/// Flag: phoneme is a sibilant fricative.
pub const PH_SIBILANT:     u32 = 1 << 5;
/// Flag: no linking to the previous phoneme.
pub const PH_NOLINK:       u32 = 1 << 6;
/// Flag: phoneme has a trill.
pub const PH_TRILL:        u32 = 1 << 7;
/// Flag: phoneme is palatalised.
pub const PH_PALATAL:      u32 = 1 << 9;
/// Flag: insert a break after this phoneme.
pub const PH_BRKAFTER:     u32 = 1 << 14;
/// Flag: phoneme is non-syllabic (e.g. semivowel in diphthong).
pub const PH_NONSYLLABIC:  u32 = 1 << 20;
/// Flag: phoneme is long.
pub const PH_LONG:         u32 = 1 << 21;
/// Flag: lengthen preceding stop.
pub const PH_LENGTHENSTOP: u32 = 1 << 22;
/// Flag: phoneme is rhotic (r-coloured).
pub const PH_RHOTIC:       u32 = 1 << 23;
/// Flag: do not add a pause before this phoneme.
pub const PH_NOPAUSE:      u32 = 1 << 24;
/// Flag: phoneme has pre-voicing.
pub const PH_PREVOICE:     u32 = 1 << 25;
/// Language-defined flag 1.
pub const PH_FLAG1:        u32 = 1 << 28;
/// Language-defined flag 2.
pub const PH_FLAG2:        u32 = 1 << 29;
/// Flag: local phoneme (language-switching context).
pub const PH_LOCAL:        u32 = 1 << 31;

// ---------------------------------------------------------------------------
// Well-known fixed phoneme codes  (indices into the active phoneme table)
// ---------------------------------------------------------------------------

/// Code 1: control marker.
pub const PHON_CONTROL:       u8 = 1;
/// Code 2: unstressed (`%`).
pub const PHON_STRESS_U:      u8 = 2;
/// Code 3: stress-down (`%%`).
pub const PHON_STRESS_D:      u8 = 3;
/// Code 4: secondary stress `,`.
pub const PHON_STRESS_2:      u8 = 4;
/// Code 5: tertiary stress.
pub const PHON_STRESS_3:      u8 = 5;
/// Code 6: primary stress `ˈ`.
pub const PHON_STRESS_P:      u8 = 6;
/// Code 7: primary stress (tonic syllable) `ˈˈ`.
pub const PHON_STRESS_P2:     u8 = 7;
/// Code 8: revert to previous stress level.
pub const PHON_STRESS_PREV:   u8 = 8;
/// Code 9: pause.
pub const PHON_PAUSE:         u8 = 9;
/// Code 10: short pause.
pub const PHON_PAUSE_SHORT:   u8 = 10;
/// Code 11: no-link pause.
pub const PHON_PAUSE_NOLINK:  u8 = 11;
/// Code 12: lengthening mark `ː`.
pub const PHON_LENGTHEN:      u8 = 12;
/// Code 13: schwa `ə`.
pub const PHON_SCHWA:         u8 = 13;
/// Code 14: short schwa.
pub const PHON_SCHWA_SHORT:   u8 = 14;
/// Code 15: end-of-word boundary `||`.
pub const PHON_END_WORD:      u8 = 15;
/// Code 17: default tone marker.
pub const PHON_DEFAULTTONE:   u8 = 17;
/// Code 18: capital letter marker.
pub const PHON_CAPITAL:       u8 = 18;
/// Code 19: glottal stop `ʔ`.
pub const PHON_GLOTTALSTOP:   u8 = 19;
/// Code 20: syllabic marker.
pub const PHON_SYLLABIC:      u8 = 20;
/// Code 21: language-switch marker.
pub const PHON_SWITCH:        u8 = 21;
/// Code 22: language-defined marker X1.
pub const PHON_X1:            u8 = 22;
/// Code 23: very-short pause.
pub const PHON_PAUSE_VSHORT:  u8 = 23;
/// Code 24: long pause.
pub const PHON_PAUSE_LONG:    u8 = 24;
/// Code 25: reduced /t/ (American English flap).
pub const PHON_T_REDUCED:     u8 = 25;
/// Code 26: tonic stress.
pub const PHON_STRESS_TONIC:  u8 = 26;
/// Code 27: clause-boundary pause.
pub const PHON_PAUSE_CLAUSE:  u8 = 27;
/// First code of the vowel-type range (codes 28-33).
pub const PHON_VOWELTYPES:    u8 = 28;

// ---------------------------------------------------------------------------
// Table dimension limits
// ---------------------------------------------------------------------------

/// Maximum number of phoneme tables that can be loaded simultaneously.
pub const N_PHONEME_TABS:     usize = 150;
/// Number of phoneme slots per table (one per possible code value 0-255).
pub const N_PHONEME_TAB:      usize = 256;
/// Maximum number of bytes in a phoneme table name (null-padded).
pub const N_PHONEME_TAB_NAME: usize = 32;
/// Maximum number of phoneme substitution rules per voice.
pub const N_REPLACE_PHONEMES: usize = 60;

/// Version magic that must match bytes 0-3 of the `phondata` file.
pub const VERSION_PHDATA: u32 = 0x01_48_01;
