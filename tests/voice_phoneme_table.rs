// tests/voice_phoneme_table.rs
//
// A voice may declare `phonemes <name>` to share another language's phoneme
// table (Bosnian → hr, Norwegian Bokmål → no).  `resolve_phoneme_table` reads
// that directive; the dictionary compiler (`--compile`) relies on it so a
// shared-table language resolves its mnemonics against the *right* table
// instead of silently falling back to en (which turned bs's `U:` into "ts").

use espeak_ng::translate::resolve_phoneme_table;
use std::path::Path;

#[test]
fn voice_phonemes_directive_is_resolved() {
    let dir = Path::new("espeak-ng-data");
    if !dir.join("lang/zls/bs").exists() {
        eprintln!("[SKIP] no bs voice");
        return;
    }
    // Declared `phonemes <name>` overrides.
    assert_eq!(resolve_phoneme_table(dir, "bs"), "hr");
    assert_eq!(resolve_phoneme_table(dir, "nb"), "no");
    // A language with no directive resolves to its own code.
    assert_eq!(resolve_phoneme_table(dir, "en"), "en");
}
