//! How a language times and weights its syllables: `LANGUAGE_OPTIONS` values from
//! `tr_languages.c`, as C sets them.

/// The prosody options of a language.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LangProsody {
    /// Added to the length modifier of the tonic syllable.
    pub lengthen_tonic: u32,
    /// The largest vowel length modifier.
    pub max_length_mod: u32,
    /// Vowels in a word's last syllable aren't lengthened.
    pub same_length_mods: bool,
    /// What a doubled stop adds to the pause before it, in ms.
    pub long_stop: u32,
    /// Syllables have lexical tones instead of a tune.
    pub tone_language: bool,
    /// The last syllable of a clause isn't lengthened (`S_NO_EOC_LENGTHEN`)…
    pub no_eoc_lengthen: bool,
    /// …or is, but short vowels no more than long ones (`S_EO_CLAUSE1`).
    pub eoc_lengthen_evenly: bool,
    pub word_gap: u32,
    /// The least that a sonorant before a short vowel is shortened to.
    pub sonorant_min: u32,
    /// Vowel length and amplitude by stress level.
    pub stress_lengths: [u32; 8],
    pub stress_amps: [u8; 8],
}

impl Default for LangProsody {
    fn default() -> Self {
        LangProsody {
            lengthen_tonic: 20,
            max_length_mod: 500,
            same_length_mods: false,
            long_stop: 100,
            tone_language: false,
            no_eoc_lengthen: false,
            eoc_lengthen_evenly: false,
            word_gap: 0,
            sonorant_min: 95,
            stress_lengths: [182, 140, 220, 220, 220, 240, 260, 280],
            stress_amps: [18, 18, 20, 20, 20, 22, 22, 20],
        }
    }
}

impl LangProsody {
    /// The options of `lang`, or of its primary subtag.
    pub fn for_lang(lang: &str) -> Self {
        let lang = lang.to_ascii_lowercase();
        let primary = lang.split('-').next().unwrap_or(&lang);
        table(&lang).or_else(|| table(primary)).unwrap_or_default()
    }
}

fn table(lang: &str) -> Option<LangProsody> {
    let default = LangProsody::default();
    Some(match lang {
        "af" => LangProsody {
            stress_lengths: [170, 140, 220, 220, 0, 0, 250, 270],
            ..default
        },
        "am" | "fr" => LangProsody {
            same_length_mods: true,
            stress_lengths: [190, 170, 190, 200, 0, 0, 190, 240],
            stress_amps: [18, 16, 18, 18, 18, 18, 18, 18],
            ..default
        },
        "an" | "ca" | "es" | "ia" | "pap" => LangProsody {
            same_length_mods: true,
            sonorant_min: 120,
            stress_lengths: [160, 145, 155, 150, 0, 0, 200, 245],
            stress_amps: [16, 14, 15, 16, 20, 20, 22, 22],
            ..default
        },
        "as" | "bn" | "bpy" => LangProsody {
            same_length_mods: true,
            stress_lengths: [180, 180, 210, 210, 0, 0, 230, 240],
            stress_amps: [18, 18, 18, 18, 20, 20, 22, 22],
            ..default
        },
        "az" | "tr" => LangProsody {
            stress_lengths: [190, 180, 200, 230, 0, 0, 240, 250],
            stress_amps: [18, 16, 20, 21, 20, 21, 21, 20],
            ..default
        },
        "be" => LangProsody {
            stress_lengths: [160, 140, 200, 140, 0, 0, 240, 160],
            stress_amps: [12, 10, 8, 8, 0, 0, 16, 17],
            ..default
        },
        "bs" | "hr" | "mk" => LangProsody {
            stress_lengths: [180, 160, 200, 200, 0, 0, 220, 230],
            stress_amps: [17, 17, 20, 20, 20, 22, 22, 21],
            ..default
        },
        "ca-ba" | "ca-nw" | "ca-va" | "chr-US-Qaaa-x-west" | "cmn-latn-pinyin" | "en-029" | "en-gb" | "en-gb-scotland" | "en-gb-x-gbclan" | "en-gb-x-gbcwmd" | "en-gb-x-rp" | "en-shaw" | "en-us" | "en-us-nyc" | "es-419" | "es-mx" | "fr-be" | "fr-ch" | "fr-fr" | "hy-arevela" | "hy-arevmda" | "pt-br" | "pt-pt" | "ru-cl" | "ru-lv" | "vi-vn-x-central" | "vi-vn-x-south" | "zh-cmn" | "zh-yue" => LangProsody {
            ..default
        },
        "cmn" | "yue" | "zh" => LangProsody {
            same_length_mods: true,
            tone_language: true,
            word_gap: 33,
            stress_lengths: [230, 150, 230, 230, 230, 0, 240, 250],
            stress_amps: [22, 16, 22, 22, 22, 22, 22, 22],
            ..default
        },
        "cs" | "sk" => LangProsody {
            stress_lengths: [190, 190, 210, 210, 0, 0, 210, 210],
            stress_amps: [17, 16, 20, 20, 20, 22, 22, 21],
            ..default
        },
        "cy" => LangProsody {
            sonorant_min: 120,
            stress_lengths: [170, 220, 180, 180, 0, 0, 250, 270],
            stress_amps: [17, 15, 18, 18, 0, 0, 22, 20],
            ..default
        },
        "da" | "nb" => LangProsody {
            stress_lengths: [160, 140, 200, 200, 0, 0, 220, 230],
            ..default
        },
        "de" => LangProsody {
            word_gap: 8,
            stress_lengths: [150, 130, 200, 200, 0, 0, 270, 270],
            stress_amps: [20, 20, 20, 20, 20, 22, 22, 20],
            ..default
        },
        "el" | "grc" => LangProsody {
            same_length_mods: true,
            sonorant_min: 130,
            stress_lengths: [155, 180, 210, 210, 0, 0, 270, 300],
            stress_amps: [15, 12, 20, 20, 20, 22, 22, 21],
            ..default
        },
        "en" => LangProsody {
            stress_lengths: [182, 140, 220, 220, 0, 0, 248, 275],
            ..default
        },
        "eo" | "io" => LangProsody {
            stress_lengths: [150, 140, 180, 180, 0, 0, 200, 200],
            stress_amps: [16, 14, 20, 20, 20, 22, 22, 21],
            ..default
        },
        "et" | "fi" => LangProsody {
            long_stop: 130,
            ..default
        },
        "eu" => LangProsody {
            stress_lengths: [200, 200, 200, 200, 0, 0, 210, 230],
            stress_amps: [16, 16, 18, 18, 18, 18, 18, 18],
            ..default
        },
        "gn" => LangProsody {
            same_length_mods: true,
            ..default
        },
        "gu" | "ne" => LangProsody {
            same_length_mods: true,
            stress_lengths: [230, 230, 230, 230, 0, 0, 230, 230],
            stress_amps: [19, 19, 19, 19, 19, 19, 19, 19],
            ..default
        },
        "hak" => LangProsody {
            tone_language: true,
            ..default
        },
        "hi" | "mr" | "or" | "pa" => LangProsody {
            same_length_mods: true,
            stress_lengths: [190, 190, 210, 210, 0, 0, 230, 250],
            stress_amps: [17, 14, 20, 19, 20, 22, 22, 21],
            ..default
        },
        "hu" => LangProsody {
            same_length_mods: true,
            stress_lengths: [185, 195, 195, 190, 0, 0, 210, 220],
            stress_amps: [17, 17, 19, 19, 20, 22, 22, 21],
            ..default
        },
        "hy" => LangProsody {
            stress_lengths: [250, 200, 250, 250, 0, 0, 250, 250],
            ..default
        },
        "id" | "ms" => LangProsody {
            stress_lengths: [160, 200, 180, 180, 0, 0, 220, 240],
            stress_amps: [16, 18, 18, 18, 20, 22, 22, 21],
            ..default
        },
        "is" => LangProsody {
            stress_lengths: [180, 160, 200, 200, 0, 0, 240, 250],
            ..default
        },
        "it" => LangProsody {
            same_length_mods: true,
            sonorant_min: 130,
            stress_lengths: [160, 140, 150, 165, 0, 0, 218, 305],
            stress_amps: [17, 15, 18, 16, 20, 22, 22, 22],
            ..default
        },
        "jbo" => LangProsody {
            max_length_mod: 368,
            stress_lengths: [145, 145, 170, 160, 0, 0, 330, 350],
            ..default
        },
        "ka" => LangProsody {
            stress_lengths: [200, 200, 210, 210, 0, 0, 230, 230],
            stress_amps: [18, 18, 18, 18, 20, 20, 22, 22],
            ..default
        },
        "kk" => LangProsody {
            lengthen_tonic: 0,
            same_length_mods: true,
            no_eoc_lengthen: true,
            stress_lengths: [190, 180, 230, 230, 0, 0, 250, 250],
            stress_amps: [18, 16, 20, 21, 20, 21, 21, 20],
            ..default
        },
        "kl" => LangProsody {
            stress_lengths: [230, 230, 230, 230, 0, 0, 230, 230],
            stress_amps: [19, 19, 19, 19, 19, 19, 19, 19],
            ..default
        },
        "kn" | "te" => LangProsody {
            same_length_mods: true,
            stress_lengths: [230, 230, 240, 240, 0, 0, 260, 260],
            stress_amps: [18, 18, 18, 18, 20, 20, 22, 22],
            ..default
        },
        "ku" => LangProsody {
            stress_lengths: [180, 180, 190, 180, 0, 0, 230, 240],
            stress_amps: [18, 18, 20, 20, 20, 22, 22, 21],
            ..default
        },
        "ltg" | "lv" => LangProsody {
            eoc_lengthen_evenly: true,
            stress_lengths: [180, 180, 180, 160, 0, 0, 230, 180],
            stress_amps: [14, 10, 10, 8, 0, 0, 20, 15],
            ..default
        },
        "ml" => LangProsody {
            same_length_mods: true,
            stress_lengths: [180, 160, 240, 240, 0, 0, 260, 260],
            stress_amps: [19, 19, 19, 19, 19, 19, 19, 19],
            ..default
        },
        "mn" => LangProsody {
            lengthen_tonic: 0,
            same_length_mods: true,
            no_eoc_lengthen: true,
            word_gap: 1,
            stress_lengths: [190, 180, 220, 220, 0, 0, 260, 240],
            stress_amps: [16, 16, 17, 17, 20, 20, 22, 18],
            ..default
        },
        "nl" => LangProsody {
            stress_lengths: [160, 135, 210, 210, 0, 0, 260, 280],
            ..default
        },
        "om" => LangProsody {
            stress_lengths: [200, 200, 200, 200, 0, 0, 200, 200],
            stress_amps: [18, 15, 20, 20, 20, 22, 22, 22],
            ..default
        },
        "pl" => LangProsody {
            stress_lengths: [160, 190, 175, 175, 0, 0, 200, 210],
            stress_amps: [17, 13, 19, 19, 20, 22, 22, 21],
            ..default
        },
        "pt" => LangProsody {
            same_length_mods: true,
            stress_lengths: [170, 115, 210, 240, 0, 0, 260, 280],
            stress_amps: [16, 11, 19, 21, 20, 22, 22, 21],
            ..default
        },
        "ro" | "rup" => LangProsody {
            stress_lengths: [170, 170, 180, 180, 0, 0, 240, 260],
            stress_amps: [15, 13, 18, 18, 20, 22, 22, 21],
            ..default
        },
        "ru" | "uk" => LangProsody {
            stress_lengths: [150, 140, 220, 220, 0, 0, 260, 280],
            stress_amps: [16, 16, 18, 18, 20, 24, 24, 22],
            ..default
        },
        "shn" => LangProsody {
            same_length_mods: true,
            tone_language: true,
            ..default
        },
        "si" | "ta" => LangProsody {
            same_length_mods: true,
            stress_lengths: [200, 200, 210, 210, 0, 0, 230, 230],
            stress_amps: [18, 18, 18, 18, 20, 20, 22, 22],
            ..default
        },
        "smj" => LangProsody {
            long_stop: 130,
            stress_lengths: [150, 180, 200, 200, 0, 0, 210, 250],
            stress_amps: [18, 16, 22, 22, 20, 22, 22, 22],
            ..default
        },
        "sq" => LangProsody {
            stress_lengths: [150, 150, 180, 180, 0, 0, 300, 300],
            stress_amps: [16, 12, 16, 16, 20, 20, 21, 19],
            ..default
        },
        "sr" => LangProsody {
            stress_lengths: [160, 150, 200, 200, 0, 0, 250, 260],
            stress_amps: [17, 17, 20, 20, 20, 22, 22, 21],
            ..default
        },
        "sv" => LangProsody {
            stress_lengths: [160, 135, 220, 220, 0, 0, 250, 280],
            stress_amps: [16, 16, 20, 20, 20, 22, 22, 21],
            ..default
        },
        "sw" | "tn" => LangProsody {
            same_length_mods: true,
            stress_lengths: [160, 170, 200, 200, 0, 0, 320, 340],
            stress_amps: [16, 12, 19, 19, 20, 22, 22, 21],
            ..default
        },
        "tt" => LangProsody {
            stress_lengths: [190, 170, 190, 200, 0, 0, 190, 240],
            stress_amps: [18, 16, 18, 18, 18, 18, 18, 18],
            ..default
        },
        "vi" => LangProsody {
            same_length_mods: true,
            tone_language: true,
            word_gap: 33,
            stress_lengths: [150, 150, 180, 180, 210, 230, 230, 240],
            stress_amps: [16, 16, 16, 16, 22, 22, 22, 22],
            ..default
        },
        _ => return None,
    })
}
