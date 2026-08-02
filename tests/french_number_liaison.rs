// tests/french_number_liaison.rs
//
// French liaison inside composed number tokens.  A liaison consonant (e.g. the
// /z/ of "deux", "trois", "six") is pronounced before a vowel but silent before
// a consonant.  Word-final liaison already worked across separate tokens ("deux
// enfants" keeps /z/, "deux chats" drops it), but numbers are a *single* token
// whose parts are composed with no word boundary between them, so "200" =
// "deux·cent" used to leak an erroneous /z/ ("døzsɑ̃").  espeak-ng drops it
// ("døsɑ̃") because "cent" is consonant-initial; "2000" = "deux·mille" likewise.
//
// Values below were confirmed against the espeak-ng 1.52.0 C binary (--ipa,
// stress/length marks removed for a segmental comparison).

use espeak_ng::Translator;

fn seg(t: &Translator, text: &str) -> String {
    // Strip stress ("ˈ"/"ˌ") and length ("ː") marks — those are rendering
    // details, not the segmental content this test is about.
    t.text_to_ipa(text)
        .unwrap()
        .chars()
        .filter(|c| !matches!(c, 'ˈ' | 'ˌ' | 'ː'))
        .collect()
}

#[test]
fn hundreds_and_thousands_drop_internal_liaison() {
    let Ok(t) = Translator::new_default("fr") else { eprintln!("[SKIP] no fr dict"); return };
    // hundreds: multiplier ends in a liaison consonant, "cent" (/s/) is a consonant.
    assert_eq!(seg(&t, "200"), "døsɑ̃");     // deux cent  — was "døzsɑ̃"
    assert_eq!(seg(&t, "300"), "tʁwasɑ̃");   // trois cent — was "tʁwazsɑ̃"
    assert_eq!(seg(&t, "600"), "sisɑ̃");     // six cent   — was "sizsɑ̃"
    // thousands: liaison sits before the END_WORD separator, then "mille" (/m/).
    assert_eq!(seg(&t, "2000"), "dø mil");   // deux mille — was "døz mil"
    assert_eq!(seg(&t, "6000"), "si mil");   // six mille  — was "siz mil"
}

#[test]
fn liaison_kept_before_a_vowel() {
    let Ok(t) = Translator::new_default("fr") else { eprintln!("[SKIP] no fr dict"); return };
    // 21 = "vingt et un": the /t/ of "vingt" precedes "et" (vowel) → kept.
    assert_eq!(seg(&t, "21"), "vɛ̃teœ̃");
    // Across a token boundary the /z/ still surfaces before a vowel-initial word.
    assert_eq!(seg(&t, "deux enfants"), "døz ɑ̃fɑ̃");
    // …and "200 euros": internal deux·cent drops /z/, "cent"→"euros" has no liaison.
    assert_eq!(seg(&t, "200 euros"), "døsɑ̃ øʁo");
}

#[test]
fn plain_word_liaison_unchanged() {
    let Ok(t) = Translator::new_default("fr") else { eprintln!("[SKIP] no fr dict"); return };
    // The pre-existing cross-token behaviour must be preserved.
    assert_eq!(seg(&t, "deux chats"), "dø ʃa");   // consonant-initial → dropped
    assert_eq!(seg(&t, "deux cents"), "dø sɑ̃");  // spelled out, two tokens
}

// six / dix / huit carry a `3`-liaison (`ph_french` z3 / t3): [z]/[t] before a
// vowel, silent before a consonant, and a *citation* form ([s] / [t]) before a
// pause — the third case the port used to drop ("six" → "si", not "sis").
#[test]
fn three_liaison_citation_form_before_pause() {
    let Ok(t) = Translator::new_default("fr") else { eprintln!("[SKIP] no fr dict"); return };
    // Isolation / end of utterance → citation consonant surfaces.
    assert_eq!(seg(&t, "six"), "sis");
    assert_eq!(seg(&t, "dix"), "dis");
    assert_eq!(seg(&t, "huit"), "yit");
    assert_eq!(seg(&t, "6"), "sis");
    assert_eq!(seg(&t, "10"), "dis");
    // …including when "six" ends a composed number token.
    assert_eq!(seg(&t, "26"), "vɛ̃tsis");
    assert_eq!(seg(&t, "36"), "tʁɑ̃tsis");
    // A trailing pause (period) is still a pause.
    assert_eq!(seg(&t, "six."), "sis");
}

#[test]
fn three_liaison_vowel_and_consonant_contexts() {
    let Ok(t) = Translator::new_default("fr") else { eprintln!("[SKIP] no fr dict"); return };
    // Before a vowel-initial word → voiced liaison ([z]) / linking [t].
    assert_eq!(seg(&t, "six ans"), "siz ɑ̃");
    assert_eq!(seg(&t, "dix ans"), "diz ɑ̃");
    assert_eq!(seg(&t, "huit ans"), "yit ɑ̃");
    // Before a consonant-initial word → silent (no citation form).
    assert_eq!(seg(&t, "six chats"), "si ʃa");
    assert_eq!(seg(&t, "huit chats"), "yi ʃa");
}
