// tests/encoding_integration.rs
//
// Integration tests for the encoding module.
//
// These tests do NOT use the espeak-ng binary – they test the Rust encoding
// implementation in isolation with known-good golden values.

use espeak_ng::encoding::{
    decode_to_string, decode_utf8_to_string, Encoding, TextDecoder, DecodeMode,
    utf8_decode_one,
};

// ---------------------------------------------------------------------------
// UTF-8 correctness against known Unicode strings
// ---------------------------------------------------------------------------

#[test]
fn utf8_full_ascii_string() {
    let s = "The quick brown fox";
    let decoded = decode_utf8_to_string(s.as_bytes());
    assert_eq!(decoded, s);
}

#[test]
fn utf8_latin_extended() {
    let s = "Héllo Wörld";
    let decoded = decode_utf8_to_string(s.as_bytes());
    assert_eq!(decoded, s);
}

#[test]
fn utf8_cyrillic() {
    let s = "Привет мир"; // "Hello world" in Russian
    let decoded = decode_utf8_to_string(s.as_bytes());
    assert_eq!(decoded, s);
}

#[test]
fn utf8_japanese() {
    let s = "日本語テスト"; // "Japanese language test"
    let decoded = decode_utf8_to_string(s.as_bytes());
    assert_eq!(decoded, s);
}

#[test]
fn utf8_emoji_sequence() {
    let s = "Hello 😀🌍!";
    let decoded = decode_utf8_to_string(s.as_bytes());
    assert_eq!(decoded, s);
}

#[test]
fn utf8_mixed_scripts() {
    // Combine multiple scripts in one string (common in multilingual TTS)
    let s = "English: hello, Français: bonjour, Русский: привет";
    let decoded = decode_utf8_to_string(s.as_bytes());
    assert_eq!(decoded, s);
}

#[test]
fn utf8_null_termination() {
    // The C library uses null-terminated strings; our decoder should stop at \0
    let input = b"hello\x00ignored";
    let decoded = decode_utf8_to_string(input);
    assert_eq!(decoded, "hello");
}

#[test]
fn utf8_invalid_bytes_become_replacement() {
    // 0xFF is never valid UTF-8
    let input = &[b'a', 0xFF, b'b'];
    let decoded = decode_utf8_to_string(input);
    // 'a', REPLACEMENT, 'b'
    let expected: String = ['a', '\u{FFFD}', 'b'].iter().collect();
    assert_eq!(decoded, expected);
}

// ---------------------------------------------------------------------------
// Single-byte encoding round-trips
// Golden values taken from Unicode consortium mapping tables.
// ---------------------------------------------------------------------------

#[test]
fn iso8859_1_round_trip_all_printable() {
    // Every byte 0x20–0xFF in ISO-8859-1 should decode to the corresponding
    // Unicode codepoint (since ISO-8859-1 is a direct subset of Unicode).
    for b in 0x20u8..=0xFF {
        let input = &[b];
        let result = decode_to_string(input, Encoding::Iso8859_1).unwrap();
        let expected = char::from_u32(b as u32).unwrap().to_string();
        assert_eq!(result, expected, "byte 0x{b:02X}");
    }
}

#[test]
fn iso8859_2_polish_characters() {
    // Polish characters in ISO-8859-2:
    //   ą = 0xB1 → U+0105
    //   ę = 0xEA → U+0119
    //   ó = 0xF3 → U+00F3
    //   ź = 0xBC → U+017A  (note: not ź – check the table!)
    let tests = [
        (0xB1u8, 'ą'),
        (0xEAu8, 'ę'),
        (0xF3u8, 'ó'),
    ];
    for (byte, expected_char) in tests {
        let decoded = decode_to_string(&[byte], Encoding::Iso8859_2).unwrap();
        let chars: Vec<char> = decoded.chars().collect();
        assert_eq!(
            chars,
            vec![expected_char],
            "byte 0x{byte:02X} in ISO-8859-2"
        );
    }
}

#[test]
fn iso8859_7_greek_alpha() {
    // In ISO-8859-7: 0xC1 → U+0391 GREEK CAPITAL LETTER ALPHA
    let decoded = decode_to_string(&[0xC1u8], Encoding::Iso8859_7).unwrap();
    let chars: Vec<char> = decoded.chars().collect();
    assert_eq!(chars, vec!['\u{0391}']);
}

#[test]
fn iso8859_15_euro_sign() {
    // In ISO-8859-15: byte 0xA4 → U+20AC EURO SIGN
    // (in ISO-8859-1 the same byte → U+00A4 CURRENCY SIGN)
    let decoded_15 = decode_to_string(&[0xA4u8], Encoding::Iso8859_15).unwrap();
    let decoded_1  = decode_to_string(&[0xA4u8], Encoding::Iso8859_1).unwrap();

    assert_eq!(decoded_15.chars().next().unwrap(), '\u{20AC}', "ISO-8859-15");
    assert_eq!(decoded_1.chars().next().unwrap(),  '\u{00A4}', "ISO-8859-1");
    assert_ne!(decoded_15, decoded_1, "15 and 1 should differ at 0xA4");
}

#[test]
fn koi8r_cyrillic_a() {
    // The espeak-ng KOI8-R table actually holds ISO-8859-16 data (C source quirk).
    // At byte 0xC1 the C table maps to U+00C1 (LATIN CAPITAL LETTER A WITH ACUTE, Á),
    // NOT U+0410 (CYRILLIC CAPITAL LETTER A, А) as standard KOI8-R would.
    let decoded = decode_to_string(&[0xC1u8], Encoding::Koi8R).unwrap();
    let chars: Vec<char> = decoded.chars().collect();
    assert_eq!(chars, vec!['\u{00C1}'],
        "espeak-ng KOI8-R at 0xC1 should be U+00C1 (mirrors C source quirk)");
}

// ---------------------------------------------------------------------------
// Encoding name lookup – mirrors espeak_ng_EncodingFromName() golden values
// ---------------------------------------------------------------------------

#[test]
fn encoding_names_golden() {
    let cases = [
        ("UTF-8",         Encoding::Utf8),
        ("utf-8",         Encoding::Utf8),   // case-insensitive
        ("US-ASCII",      Encoding::UsAscii),
        ("ASCII",         Encoding::UsAscii),
        ("ISO-8859-1",    Encoding::Iso8859_1),
        ("LATIN1",        Encoding::Iso8859_1),
        ("ISO-8859-2",    Encoding::Iso8859_2),
        ("ISO-8859-3",    Encoding::Iso8859_3),
        ("ISO-8859-4",    Encoding::Iso8859_4),
        ("ISO-8859-5",    Encoding::Iso8859_5),
        ("ISO-8859-6",    Encoding::Iso8859_6),
        ("ISO-8859-7",    Encoding::Iso8859_7),
        ("ISO-8859-8",    Encoding::Iso8859_8),
        ("ISO-8859-9",    Encoding::Iso8859_9),
        ("ISO-8859-10",   Encoding::Iso8859_10),
        ("ISO-8859-11",   Encoding::Iso8859_11),
        ("ISO-8859-13",   Encoding::Iso8859_13),
        ("ISO-8859-14",   Encoding::Iso8859_14),
        ("ISO-8859-15",   Encoding::Iso8859_15),
        ("ISO-8859-16",   Encoding::Iso8859_16),
        ("KOI8-R",        Encoding::Koi8R),
        ("ISCII",         Encoding::Iscii),
        ("ISO-10646-UCS-2", Encoding::Ucs2),
    ];

    for (name, expected) in cases {
        assert_eq!(
            Encoding::from_name(name),
            expected,
            "from_name({name:?})"
        );
    }
}

#[test]
fn unknown_encoding_names() {
    for name in &["SHIFT_JIS", "EUC-JP", "GB2312", "bogus", "", "windows-1252"] {
        assert_eq!(
            Encoding::from_name(name),
            Encoding::Unknown,
            "should be Unknown: {name:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// TextDecoder streaming API
// ---------------------------------------------------------------------------

#[test]
fn text_decoder_iterator_utf8() {
    let text = "Héllo";
    let codepoints: Vec<u32> = TextDecoder::utf8(text.as_bytes()).collect();
    let expected: Vec<u32> = text.chars().map(|c| c as u32).collect();
    assert_eq!(codepoints, expected);
}

#[test]
fn text_decoder_peek_does_not_advance() {
    let mut dec = TextDecoder::utf8(b"AB");
    assert_eq!(dec.peek(), Some(b'A' as u32));
    assert_eq!(dec.peek(), Some(b'A' as u32)); // still A
    dec.next_codepoint();
    assert_eq!(dec.peek(), Some(b'B' as u32));
}

#[test]
fn text_decoder_remaining_slice() {
    let input = b"hello world";
    let mut dec = TextDecoder::utf8(input);
    dec.next_codepoint(); // consume 'h'
    assert_eq!(dec.remaining(), b"ello world");
}

#[test]
fn text_decoder_iso8859_1_cafe_string() {
    // "café" in ISO-8859-1: 0x63 0x61 0x66 0xE9
    let input = b"caf\xE9";
    let s = TextDecoder::new(input, Encoding::Iso8859_1, DecodeMode::Strict)
        .unwrap()
        .decode_to_string();
    assert_eq!(s, "café");
}

#[test]
fn text_decoder_auto_uses_utf8_for_valid_input() {
    // Valid UTF-8 "café" should decode correctly in auto mode
    let input = "café".as_bytes();
    let s = TextDecoder::new(input, Encoding::Iso8859_1, DecodeMode::Auto)
        .unwrap()
        .decode_to_string();
    assert_eq!(s, "café");
}

#[test]
fn text_decoder_auto_falls_back_for_invalid_utf8() {
    // 0xE9 alone is not valid UTF-8 but is valid ISO-8859-1 (é)
    let input = &[0xE9u8];
    let s = TextDecoder::new(input, Encoding::Iso8859_1, DecodeMode::Auto)
        .unwrap()
        .decode_to_string();
    assert_eq!(s, "é", "auto mode should fall back to ISO-8859-1");
}

// ---------------------------------------------------------------------------
// Property-based style: exhaustive UTF-8 roundtrip for BMP + SMP
// ---------------------------------------------------------------------------

#[test]
fn utf8_encode_decode_roundtrip_exhaustive() {
    let mut buf = [0u8; 4];
    // Test a representative sample (full exhaustive is ~1M chars, takes a few seconds)
    let samples: Vec<u32> = (0u32..=0x10_FFFF)
        .filter(|&cp| {
            // Skip surrogates (D800-DFFF) and undefined areas
            !(0xD800..=0xDFFF).contains(&cp)
        })
        .step_by(17) // sample every 17th codepoint for speed
        .collect();

    for cp in samples {
        if let Some(ch) = char::from_u32(cp) {
            let s = ch.encode_utf8(&mut buf);
            let (decoded, n) = utf8_decode_one(s.as_bytes());
            assert_eq!(n, s.len(), "length mismatch at U+{cp:04X}");
            // Account for the C workaround: U+FFFD → U+001A for 3-byte sequences
            let expected = if cp == 0xFFFD { 0x001A } else { cp };
            assert_eq!(decoded, expected, "roundtrip failed at U+{cp:04X}");
        }
    }
}
