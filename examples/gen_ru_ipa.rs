/// Generate Russian IPA override table by querying espeak-ng.
/// 
/// This generates entries for `RU_IPA_OVERRIDES` (similar to EN_IPA_OVERRIDES)
/// to correct the IPA rendering for Russian phoneme codes that have complex
/// conditional bytecode (stress-dependent IPA, etc.).
use espeak_ng::phoneme::PhonemeData;
use espeak_ng::translate::default_data_dir;
use std::path::PathBuf;

fn main() {
    let data_dir = PathBuf::from(default_data_dir());
    let mut phdata = PhonemeData::load(&data_dir).unwrap();
    phdata.select_table_by_name("ru").unwrap();

    let mut overrides = Vec::new();

    for code in 0u8..=255 {
        let Some(ph) = phdata.get(code) else { continue };
        
        // Skip control codes (stress, pause, etc.)
        if ph.typ == 0 || ph.typ == 1 { continue; }
        
        let mnem_bytes = ph.mnemonic.to_le_bytes();
        let mnem_str: String = mnem_bytes.iter()
            .take_while(|&&b| b != 0)
            .map(|&b| b as char)
            .collect();
        
        if mnem_str.is_empty() { continue; }
        
        // Get IPA from espeak-ng using phoneme input syntax
        let input = format!("[[{}]]", mnem_str);
        let out = std::process::Command::new("espeak-ng")
            .args(["-v", "ru", "-q", "--ipa", &input])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default();
        
        // espeak-ng --ipa adds stress mark before the phoneme when it's the only one
        // Strip leading stress marks for comparison
        let c_ipa = out.trim_start_matches(['ˈ', 'ˌ']).to_string();
        
        // What our mnemonic fallback produces
        let rust_ipa = mnemonic_fallback(&mnem_str, ph.typ == 2);
        
        if !c_ipa.is_empty() && c_ipa != rust_ipa {
            println!("// code={code} mnemonic={mnem_str:?} type={}  rust={rust_ipa:?} -> espeak={c_ipa:?}",
                ph.typ);
            // Escape Unicode for Rust string literal
            let escaped: String = c_ipa.chars()
                .map(|c| if c.is_ascii() {
                    c.to_string()
                } else {
                    format!("\\u{{{:04X}}}", c as u32)
                })
                .collect();
            overrides.push(format!("    ({code}, \"{escaped}\"),  // {mnem_str}"));
        }
    }

    println!();
    println!("pub static RU_IPA_OVERRIDES: &[(u8, &str)] = &[");
    for line in &overrides {
        println!("{line}");
    }
    println!("];");
}

fn mnemonic_fallback(mnem: &str, is_vowel: bool) -> String {
    // Replicate mnemonic_to_ipa logic
    let mut out = String::new();
    let mut first = true;
    for c in mnem.chars() {
        if c == '/' { break; }
        if c == '#' && is_vowel { break; }
        if !first && c.is_ascii_digit() { continue; }
        out.push_str(ipa1_char(c as u8));
        first = false;
    }
    out
}

fn ipa1_char(c: u8) -> &'static str {
    // Simplified ipa1 table (key entries)
    match c {
        b'a' => "a", b'b' => "b", b'c' => "c", b'd' => "d",
        b'e' => "e", b'f' => "f", b'g' => "\u{0261}", b'h' => "h",
        b'i' => "i", b'j' => "j", b'k' => "k", b'l' => "l",
        b'm' => "m", b'n' => "n", b'o' => "o", b'p' => "p",
        b'q' => "\u{03C7}", b'r' => "r", b's' => "s", b't' => "t",
        b'u' => "u", b'v' => "v", b'w' => "w", b'x' => "x",
        b'y' => "y", b'z' => "z",
        b'A' => "\u{00E6}", b'B' => "\u{03B2}", b'C' => "\u{0283}",
        b'D' => "\u{00F0}", b'E' => "\u{025B}", b'F' => "\u{028B}",
        b'G' => "\u{0281}", b'H' => "\u{0265}", b'I' => "\u{026A}",
        b'J' => "\u{02B2}", b'K' => "\u{026C}", b'L' => "\u{026B}",
        b'M' => "\u{026F}", b'N' => "\u{014B}", b'O' => "\u{0254}",
        b'P' => "\u{028B}", b'Q' => "\u{0264}", b'R' => "\u{0280}",
        b'S' => "\u{0283}", b'T' => "\u{03B8}", b'U' => "\u{028A}",
        b'V' => "\u{028C}", b'W' => "\u{026F}", b'X' => "\u{03C7}",
        b'Y' => "\u{028F}", b'Z' => "\u{0292}",
        b'0' => "\u{0252}", b'1' => "\u{0251}", b'2' => "\u{025C}",
        b'3' => "\u{025C}", b'4' => "\u{025B}", b'5' => "\u{0259}",
        b'6' => "\u{00E6}", b'7' => "\u{0269}", b'8' => "\u{0275}",
        b'9' => "\u{0264}",
        _ => "?",
    }
}
