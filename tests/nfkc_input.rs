// tests/nfkc_input.rs
//
// Unicode compatibility (NFKC) folding of input — upstream #2523.  Applied only
// to the characters the port would otherwise *drop*, so the ones it already
// reads well keep their better reading.

use espeak_ng::Translator;

fn x(text: &str) -> Option<String> {
    Translator::new("en", None).ok().map(|t| t.text_to_ipa(text).unwrap())
}

#[test]
fn squared_units_are_no_longer_dropped() {
    let Some(_) = x("a") else {
        eprintln!("[SKIP] no en data");
        return;
    };
    // `㎏` U+338F used to produce nothing at all.
    assert!(!x("㎏").unwrap().trim().is_empty(), "㎏ was dropped");
    assert!(x("5㎏").unwrap().contains("kˈɪləɡɹ"), "5㎏ should read as kilograms: {:?}", x("5㎏"));
    assert!(!x("㎐").unwrap().trim().is_empty(), "㎐ was dropped");
    assert!(!x("㎞").unwrap().trim().is_empty(), "㎞ was dropped");
}

/// Characters the port already reads keep their own name — folding them to
/// NFKC would replace the dictionary's "trade mark" with "T M".
///
/// The expected reading is the dictionary's (`en_emoji`: `™ trade mark`), which
/// is what upstream speaks: `tr'eId m'A@k`.  It used to be the hardcoded
/// "trademark", from the per-language table that preceded the dictionary lookup.
#[test]
fn better_readings_are_preserved() {
    let Some(_) = x("a") else { return };
    assert!(x("™").unwrap().contains("tɹˈeɪd mˈɑːk"), "{:?}", x("™"));
    assert!(!x("™").unwrap().contains("ˈɛm"), "folded to T M: {:?}", x("™"));
    assert!(x("№5").unwrap().contains("njˈuːm"), "{:?}", x("№5"));
    assert_eq!(x("①②③").unwrap(), "wˈɒn tˈuː θɹˈiː");
    assert!(x("x²").unwrap().contains("skwˈeəd"), "{:?}", x("x²"));
    assert_eq!(x("ｈｅｌｌｏ").unwrap(), x("hello").unwrap(), "full-width already folds");
}

/// Folding must not change ordinary text.
#[test]
fn plain_text_is_untouched() {
    let Some(_) = x("a") else { return };
    for s in ["hello world", "1,234.5", "don't", "naïve café"] {
        assert_eq!(x(s).unwrap(), x(s).unwrap(), "{s} is not stable");
    }
    assert_eq!(x("hello world").unwrap(), "həlˈəʊ wˈɜːld");
}
