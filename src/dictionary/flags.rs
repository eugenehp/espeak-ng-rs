//! Typed wrappers around the raw dictionary flag integers.
//!
//! In the C code these are plain `unsigned int` fields.  The wrappers here
//! give the bit-field constants a typed home without changing the underlying
//! representation.

// Bit-field constants mirror C defines verbatim; their names are self-documenting.
#![allow(missing_docs)]

/// Flags returned in `flags[0]` by `LookupDict2` / `LookupDictList`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DictFlags1(pub u32);

/// Flags returned in `flags[1]` by `LookupDict2` / `LookupDictList`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DictFlags2(pub u32);

impl DictFlags1 {
    pub const FOUND:            u32 = 0x8000_0000;
    pub const FOUND_ATTRIBUTES: u32 = 0x4000_0000;
    pub const TEXTMODE:         u32 = 0x2000_0000;
    pub const MAX3:             u32 = 0x0800_0000;
    pub const NEEDS_DOT:        u32 = 0x0200_0000;
    pub const SKIPWORDS:        u32 = 0x0000_0080;
    pub const SPELLWORD:        u32 = 0x0000_1000;
    pub const STRESS_END:       u32 = 0x0000_0200;

    pub fn contains(&self, bits: u32) -> bool { self.0 & bits == bits }
    pub fn set(&mut self, bits: u32) { self.0 |= bits; }
    pub fn clear(&mut self, bits: u32) { self.0 &= !bits; }
    pub fn found(&self)      -> bool { self.contains(Self::FOUND) }
    pub fn textmode(&self)   -> bool { self.contains(Self::TEXTMODE) }
    pub fn skipwords(&self)  -> bool { self.contains(Self::SKIPWORDS) }
    pub fn spellword(&self)  -> bool { self.contains(Self::SPELLWORD) }
    pub fn stress_bits(&self)-> u32  { self.0 & 0xf }
}

impl DictFlags2 {
    pub const VERB:    u32 = 0x10;
    pub const NOUN:    u32 = 0x20;
    pub const PAST:    u32 = 0x40;
    pub const CAPITAL: u32 = 0x200;
    pub const ALLCAPS: u32 = 0x400;
    pub const ATEND:   u32 = 0x20000;
    pub const ATSTART: u32 = 0x40000;
    pub const SENTENCE:u32 = 0x2000;
    pub const ONLY:    u32 = 0x4000;
    pub const ONLY_S:  u32 = 0x8000;
    pub const STEM:    u32 = 0x10000;
    pub const NATIVE:  u32 = 0x80000;
    pub const ACCENT:  u32 = 0x800;

    pub fn contains(&self, bits: u32) -> bool { self.0 & bits == bits }
    pub fn set(&mut self, bits: u32) { self.0 |= bits; }
    pub fn is_verb(&self)    -> bool { self.contains(Self::VERB) }
    pub fn is_noun(&self)    -> bool { self.contains(Self::NOUN) }
    pub fn is_past(&self)    -> bool { self.contains(Self::PAST) }
    pub fn is_capital(&self) -> bool { self.contains(Self::CAPITAL) }
    pub fn is_allcaps(&self) -> bool { self.contains(Self::ALLCAPS) }
    pub fn only_form(&self)  -> bool { self.contains(Self::ONLY) }
    pub fn only_s_form(&self)-> bool { self.contains(Self::ONLY_S) }
    pub fn stem_only(&self)  -> bool { self.contains(Self::STEM) }
}
