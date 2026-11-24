//! French [ɥ] in IPA output: an opt-in refinement of [`text_to_ipa`].
//!
//! eSpeak NG has no French phoneme for the semivowel [ɥ] (*nuit*, *huit*,
//! *lui*) and writes it with the vowel `y`: `nyˈi`. Upstream PR #2280 adds a
//! dedicated phoneme through the French rules and dictionary; it is not
//! merged, and output here stays byte-identical to the C library unless this
//! refinement is asked for. It changes only how the IPA is written — the
//! sound is the same `y`, as in the PR, whose `ɥ` changes back to `y` for
//! synthesis.
//!
//! The rule is the phonological one: within a word, [y] before a vowel is
//! the glide [ɥ] — before [i] always (*nuit, pluie, bruit, linguiste*), and
//! before another vowel unless an obstruent + liquid cluster precedes it,
//! where French keeps the vowel (*cruel* [kʁyɛl], *gluant* [ɡlyɑ̃]).
//! A stress mark between them moves before the glide, which begins the
//! stressed syllable: `nyˈi` → `nˈɥi`.
//!
//! [`text_to_ipa`]: crate::text_to_ipa

const VOWELS: &[char] = &['i', 'e', 'ɛ', 'a', 'ɑ', 'o', 'ɔ', 'u', 'y', 'ø', 'œ', 'ə'];
const STRESS: &[char] = &['ˈ', 'ˌ'];
const OBSTRUENTS: &[char] = &['p', 'b', 't', 'd', 'k', 'ɡ', 'g', 'f', 'v'];
const LIQUIDS: &[char] = &['ʁ', 'r', 'l'];

/// Writes French [y] before a vowel as [ɥ] (see the module docs).
///
/// ```
/// use espeak_ng::french_semivowels;
/// assert_eq!(french_semivowels("nyˈi"), "nˈɥi");
/// assert_eq!(french_semivowels("kʁyˈɛl"), "kʁyˈɛl");
/// assert_eq!(french_semivowels("ty ˈas"), "ty ˈas");
/// ```
pub fn french_semivowels(ipa: &str) -> String {
    let chars: Vec<char> = ipa.chars().collect();
    let mut out = String::with_capacity(ipa.len() + 4);
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == 'y' {
            // The next sound in the same word, past a stress mark.
            let mut j = i + 1;
            let stress = chars.get(j).copied().filter(|c| STRESS.contains(c));
            if stress.is_some() {
                j += 1;
            }
            let next = chars.get(j).copied();
            // Two sounds before: an obstruent then a liquid.
            let cluster = i >= 2 && LIQUIDS.contains(&chars[i - 1]) && OBSTRUENTS.contains(&chars[i - 2]);
            let glide = match next {
                Some('i') => true,
                Some(v) if VOWELS.contains(&v) => !cluster,
                _ => false,
            };
            if glide {
                if let Some(s) = stress {
                    out.push(s);
                }
                out.push('ɥ');
                i = j;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::french_semivowels as g;

    #[test]
    fn glide_before_i_always() {
        assert_eq!(g("nyˈi"), "nˈɥi");
        assert_eq!(g("yˈit"), "ˈɥit");
        assert_eq!(g("plyˈi"), "plˈɥi");
        assert_eq!(g("bʁyˈi"), "bʁˈɥi");
        assert_eq!(g("lɛ̃ɡyˈist"), "lɛ̃ɡˈɥist");
        assert_eq!(g("tyijˈo"), "tɥijˈo");
    }

    #[test]
    fn glide_before_other_vowels_but_not_after_a_cluster() {
        assert_eq!(g("nyˈaʒ"), "nˈɥaʒ");
        assert_eq!(g("ʒyˈɛ̃"), "ʒˈɥɛ̃");
        assert_eq!(g("tyˈe"), "tˈɥe");
        assert_eq!(g("lyˈœʁ"), "lˈɥœʁ");
        assert_eq!(g("kʁyˈɛl"), "kʁyˈɛl");
        assert_eq!(g("ɡlyˈɑ̃"), "ɡlyˈɑ̃");
    }

    #[test]
    fn leaves_everything_else() {
        assert_eq!(g("ty ˈas"), "ty ˈas", "not across words");
        assert_eq!(g("ʒə- lə- vwˈa"), "ʒə- lə- vwˈa");
        assert_eq!(g("ʁˈy"), "ʁˈy");
        assert_eq!(g("bɔ̃ʒˈuʁ"), "bɔ̃ʒˈuʁ");
        assert_eq!(g(""), "");
    }
}
