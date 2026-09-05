//! SSML markup preprocessing.
//!
//! Port of the text-transforming subset of eSpeak NG's `ssml.c` /
//! `readclause.c`.  In eSpeak NG, SSML handling is only active when *markup
//! mode* is enabled (the `-m` command-line flag / `espeakSSML`); otherwise the
//! angle-bracket tags are read out as literal text.
//!
//! [`process_markup`] converts an SSML fragment into a list of
//! [`Segment`]s — runs of plain text, each tagged with the language selected by
//! the enclosing `<voice>` element (or `None` for the document language).
//! [`strip_markup`] is the language-agnostic convenience that concatenates the
//! segment texts.  Both handle:
//!
//! - XML / HTML entities: `&amp; &lt; &gt; &quot; &apos; &nbsp;` and numeric
//!   `&#NN;` / `&#xHH;`.
//! - `<say-as interpret-as="characters">…</say-as>` — spell the content out.
//! - `<sub alias="…">…</sub>` — replace the content with the alias.
//! - `<phoneme ph="…">…</phoneme>` — pronounce the `ph` attribute directly
//!   (bridged to the always-on `[[…]]` inline-phoneme syntax); the element
//!   content is discarded.
//! - `<break/>`, `<p>…</p>`, `<s>…</s>` — insert clause boundaries.
//! - `<voice xml:lang="fr">…</voice>` — translate the content in another
//!   language (also `lang=` / `name=`).
//! - `<emphasis>`, `<prosody>`, `<speak>` and unknown tags — the tag itself is
//!   removed but its text content is kept.
//! - Comments (`<!-- … -->`), PIs (`<? … ?>`) and declarations (`<! … >`) are
//!   discarded.
//!
//! # Not yet handled
//! - Forced letter-name spelling for `<say-as characters>` (the "capitals"
//!   mechanism).
//! - `<phoneme alphabet="ipa">` — the `ph` attribute is passed through as
//!   eSpeak phoneme mnemonics (the default `alphabet="espeak"`); IPA input is
//!   not yet transliterated.

/// Sentinel character emitted at SSML clause boundaries (`<break/>`, `</p>`,
/// `</s>`).  Chosen from the Unicode Private Use Area so it never collides with
/// real input text.  The tokenizer maps it to a
/// [`Token::ClauseBoundary`](crate::translate::Token::ClauseBoundary).
pub(crate) const SSML_BREAK: char = '\u{E000}';

/// How a segment's content should be interpreted.  Text-level `<say-as>` modes
/// (spelling) are applied inline in the walker; this carries the modes that
/// need the *translate* layer (which knows the language), threaded to
/// `text_to_ipa_with_options`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SayAs {
    /// Ordinary reading.
    #[default]
    Normal,
    /// `<say-as interpret-as="ordinal">` — read numbers as ordinals
    /// ("3" → "third").
    Ordinal,
    /// `<say-as interpret-as="date">` — `YYYY-MM-DD` → "Month Dayth Year".
    Date,
    /// `<say-as interpret-as="time">` — `HH:MM` → hour + minute.
    Time,
}

/// Rate / pitch / volume multipliers from a `<prosody>` element (`1.0` = no
/// change).  `None` fields keep the current value.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Prosody {
    pub rate: Option<f32>,
    pub pitch: Option<f32>,
    pub volume: Option<f32>,
}

impl Prosody {
    fn is_empty(&self) -> bool {
        self.rate.is_none() && self.pitch.is_none() && self.volume.is_none()
    }
}

/// Parse a `<prosody>` `rate`/`pitch`/`volume` attribute value into a
/// multiplier: `"50%"` → 0.5, keywords (`slow`/`fast`/`high`/`loud`/…) → fixed
/// factors, `"+20%"`/`"-10%"` → relative.  Returns `None` for unparseable
/// values (e.g. absolute `"200Hz"`, which we don't map).
fn parse_prosody_value(v: &str) -> Option<f32> {
    let v = v.trim();
    match v.to_ascii_lowercase().as_str() {
        "x-slow" | "x-low" | "x-soft" => return Some(0.5),
        "slow" | "low" | "soft" => return Some(0.75),
        "medium" | "default" | "" => return Some(1.0),
        "fast" | "high" | "loud" => return Some(1.4),
        "x-fast" | "x-high" | "x-loud" => return Some(1.8),
        _ => {}
    }
    if let Some(pct) = v.strip_suffix('%') {
        let pct = pct.trim();
        // Relative (`+20%` / `-10%`) or absolute (`80%`).
        if let Some(rest) = pct.strip_prefix(['+', '-']) {
            let mag = rest.trim().parse::<f32>().ok()?;
            let signed = if pct.starts_with('-') { -mag } else { mag };
            return Some(1.0 + signed / 100.0);
        }
        return pct.parse::<f32>().ok().map(|p| p / 100.0);
    }
    None
}

/// If the *entire* speakable content of `input` is wrapped in a single
/// `<prosody>` element (optionally inside `<speak>`), return its rate/pitch/
/// volume multipliers.  Returns `None` for scoped, partial, or multiple
/// `<prosody>` spans, so a caller applies nothing and never changes text that
/// wasn't fully wrapped.  Used to apply document-level prosody to the audio
/// path (the phoneme output is unaffected by prosody).
pub fn document_prosody(input: &str) -> Option<Prosody> {
    let lower = input.to_ascii_lowercase();
    // Exactly one <prosody> element (no nesting / multiple spans).
    if lower.matches("<prosody").count() != 1 || lower.matches("</prosody").count() != 1 {
        return None;
    }
    let open_start = lower.find("<prosody")?;
    let open_end = input[open_start..].find('>')? + open_start; // inclusive of '>'
    let close_start = lower.find("</prosody")?;
    if close_start < open_end {
        return None;
    }
    // Everything outside the <prosody>…</prosody> must be non-speakable
    // (only <speak>/whitespace/comments) — else the span is partial.
    let before = &input[..open_start];
    let after_close = input[close_start..].find('>').map(|p| close_start + p + 1)?;
    let after = &input[after_close..];
    if !strip_markup(before).trim().is_empty() || !strip_markup(after).trim().is_empty() {
        return None;
    }
    let attrs = &input[open_start + "<prosody".len()..open_end];
    let p = Prosody {
        rate: get_attr(attrs, "rate").as_deref().and_then(parse_prosody_value),
        pitch: get_attr(attrs, "pitch").as_deref().and_then(parse_prosody_value),
        volume: get_attr(attrs, "volume").as_deref().and_then(parse_prosody_value),
    };
    (!p.is_empty()).then_some(p)
}

/// A run of plain text produced from SSML, tagged with the language selected by
/// the enclosing `<voice>` element (`None` = the document's own language) and
/// how the run should be interpreted ([`SayAs`]).
#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    pub text: String,
    pub lang: Option<String>,
    pub interpret: SayAs,
}

/// An SSML `<mark name="…"/>` reference point: its `name` and the character
/// offset into the stripped output text where it occurs.  eSpeak NG fires an
/// `espeak_EVENT_MARK` when synthesis reaches this point.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mark {
    pub name: String,
    /// Character offset into `strip_markup(input)` where the mark sits.
    pub position: usize,
}

/// Convert an SSML fragment to plain text for the tokenizer.
///
/// Equivalent to concatenating the text of every [`process_markup`] segment;
/// language information from `<voice>` is discarded.
pub fn strip_markup(input: &str) -> String {
    process_markup(input).into_iter().map(|s| s.text).collect()
}

/// Convert an SSML fragment into language-tagged [`Segment`]s.
pub fn process_markup(input: &str) -> Vec<Segment> {
    process_markup_with_marks(input).0
}

/// Like [`process_markup`], but also returns the `<mark>` reference points found
/// in the fragment (each with its offset into the stripped text).
pub fn process_markup_with_marks(input: &str) -> (Vec<Segment>, Vec<Mark>) {
    let chars: Vec<char> = input.chars().collect();
    let n = chars.len();
    let mut w = Walker::default();
    let mut i = 0;

    while i < n {
        let c = chars[i];

        // ---- markup tag ------------------------------------------------
        if c == '<' {
            if starts_with(&chars, i, "<!--") {
                i = find_seq(&chars, i + 4, "-->").map(|p| p + 3).unwrap_or(n);
                continue;
            }
            if matches!(chars.get(i + 1), Some('!') | Some('?')) {
                i = find_char(&chars, i + 1, '>').map(|p| p + 1).unwrap_or(n);
                continue;
            }
            let close = find_char(&chars, i + 1, '>').unwrap_or(n - 1);
            let inner: String = chars[i + 1..close.min(n)].iter().collect();
            i = (close + 1).min(n);
            w.apply_tag(&inner);
            continue;
        }

        // ---- entity ----------------------------------------------------
        if c == '&' {
            if let Some((decoded, len)) = decode_entity(&chars[i..]) {
                for dc in decoded.chars() {
                    w.emit_char(dc);
                }
                i += len;
                continue;
            }
        }

        // ---- literal character ----------------------------------------
        w.emit_char(c);
        i += 1;
    }

    let marks = std::mem::take(&mut w.marks);
    (w.finish(), marks)
}

/// What an open `<say-as>` element is doing, so the close tag can undo it.
#[derive(Clone, Copy)]
enum SayAsKind {
    /// Spells the content out (incremented `spell_depth`).
    Spell,
    /// Sets a [`SayAs`] interpretation on emitted segments.
    Interpret,
    /// No text/interpret effect (e.g. `cardinal`, unknown).
    Other,
}

/// Map an `interpret-as` value to a translate-layer [`SayAs`] mode (those that
/// need the language), or `None` for text-level / unhandled modes.
fn interpret_mode(ia: &str) -> Option<SayAs> {
    match ia {
        "ordinal" | "vxml:ordinal" => Some(SayAs::Ordinal),
        "date" => Some(SayAs::Date),
        "time" => Some(SayAs::Time),
        _ => None,
    }
}

/// Mutable state for one pass over an SSML fragment.
struct Walker {
    segments: Vec<Segment>,
    cur: String,
    /// `<mark>` reference points collected during the pass.
    marks: Vec<Mark>,
    /// Effective language for each open `<voice>` element (innermost last).
    voice_stack: Vec<Option<String>>,
    /// What each open `<say-as>` element does (to undo on close).
    sayas_stack: Vec<SayAsKind>,
    /// Current interpretation applied to emitted segments.
    interpret: SayAs,
    /// One bool per open `<sub>` — true when its content is suppressed.
    sub_stack: Vec<bool>,
    /// One bool per open `<phoneme>` — true when its content is suppressed
    /// (i.e. a `ph` attribute was present and emitted as `[[…]]`).
    phoneme_stack: Vec<bool>,
    spell_depth: usize,
    suppress_depth: usize,
    /// One entry per open `<prosody>`: the rate / volume / pitch in force when
    /// it opened, so `</prosody>` can restore them.
    ///
    /// Upstream keeps the same stack (`PARAM_STACK`) and writes the resulting
    /// values into the text as embedded commands, which is how a prosody span
    /// affects only the words inside it.
    prosody_stack: Vec<(i32, i32, i32)>,
    /// Current rate (wpm), volume (0-200) and pitch (0-100).
    prosody: (i32, i32, i32),
}

/// The engine's own rate / volume / pitch, which a `<prosody>` percentage is
/// taken against (upstream's `param_stack[0]`).
const PROSODY_BASE: (i32, i32, i32) = (175, 100, 50);

impl Default for Walker {
    fn default() -> Self {
        Walker {
            segments: Vec::new(),
            cur: String::new(),
            marks: Vec::new(),
            voice_stack: Vec::new(),
            sayas_stack: Vec::new(),
            interpret: SayAs::default(),
            sub_stack: Vec::new(),
            phoneme_stack: Vec::new(),
            spell_depth: 0,
            suppress_depth: 0,
            prosody_stack: Vec::new(),
            // Start at the engine's own parameters, so a span that changes
            // nothing emits nothing.
            prosody: PROSODY_BASE,
        }
    }
}

impl Walker {
    fn current_lang(&self) -> Option<String> {
        self.voice_stack.last().cloned().flatten()
    }

    /// Close the current text run, attributing it to the active voice language.
    /// Total characters emitted to the stripped output so far (finished
    /// segments plus the pending buffer) — the offset a `<mark>` sits at.
    fn output_len(&self) -> usize {
        self.segments.iter().map(|s| s.text.chars().count()).sum::<usize>()
            + self.cur.chars().count()
    }

    fn flush(&mut self) {
        if !self.cur.is_empty() {
            let text = std::mem::take(&mut self.cur);
            let lang = self.current_lang();
            self.segments.push(Segment { text, lang, interpret: self.interpret });
        }
    }

    fn finish(mut self) -> Vec<Segment> {
        self.flush();
        self.segments
    }

    fn emit_char(&mut self, c: char) {
        if self.suppress_depth > 0 {
            return;
        }
        if self.spell_depth > 0 && !c.is_whitespace() {
            self.cur.push(' ');
            self.cur.push(c);
            self.cur.push(' ');
        } else {
            self.cur.push(c);
        }
    }

    /// Emit `\x01<value><letter>` for each parameter a `<prosody>` element
    /// changed — the same embedded commands upstream writes, so the change
    /// applies from here to the closing tag rather than to the whole utterance.
    fn emit_prosody(&mut self, next: (i32, i32, i32)) {
        if self.suppress_depth > 0 {
            self.prosody = next;
            return;
        }
        for (cur, new, letter) in [
            (self.prosody.0, next.0, 'S'),
            (self.prosody.1, next.1, 'A'),
            (self.prosody.2, next.2, 'P'),
        ] {
            if cur != new {
                self.cur.push(crate::translate::CTRL_EMBEDDED);
                self.cur.push_str(&new.max(0).to_string());
                self.cur.push(letter);
            }
        }
        self.prosody = next;
    }

    fn emit_break(&mut self) {
        if self.suppress_depth == 0 {
            self.cur.push(SSML_BREAK);
        }
    }

    fn apply_tag(&mut self, inner: &str) {
        let mut t = inner.trim();
        if t.is_empty() {
            return;
        }
        let is_close = t.starts_with('/');
        if is_close {
            t = t[1..].trim_start();
        }
        let self_close = t.ends_with('/');
        if self_close {
            t = t[..t.len() - 1].trim_end();
        }
        let name_end = t.find(char::is_whitespace).unwrap_or(t.len());
        let name = t[..name_end].to_ascii_lowercase();
        let attrs = &t[name_end..];

        match name.as_str() {
            "prosody" => {
                // Percentages and the named steps (`x-slow`, `loud`, …) are
                // taken against the engine's own parameters, as upstream's
                // `SetProsodyParameter` does with `param_stack[0]`.
                if is_close {
                    if let Some(prev) = self.prosody_stack.pop() {
                        self.emit_prosody(prev);
                    }
                } else if !self_close {
                    self.prosody_stack.push(self.prosody);
                    let scale = |attr: &str, base: i32, cur: i32| -> i32 {
                        match get_attr(attrs, attr).as_deref().and_then(parse_prosody_value) {
                            Some(m) => (base as f32 * m).round() as i32,
                            None => cur,
                        }
                    };
                    let next = (
                        scale("rate", PROSODY_BASE.0, self.prosody.0),
                        scale("volume", PROSODY_BASE.1, self.prosody.1),
                        scale("pitch", PROSODY_BASE.2, self.prosody.2),
                    );
                    self.emit_prosody(next);
                }
            }
            "break" => self.emit_break(),
            "p" | "s" => self.emit_break(),
            "mark" => {
                // `<mark name="x"/>`: record the name and its offset into the
                // stripped output (finished segments + the current buffer).
                if let Some(name) = get_attr(attrs, "name") {
                    let position = self.output_len();
                    self.marks.push(Mark { name: name.to_string(), position });
                }
            }

            "voice" => {
                if is_close {
                    self.flush();
                    self.voice_stack.pop();
                } else if !self_close {
                    self.flush();
                    let lang = get_attr(attrs, "xml:lang")
                        .or_else(|| get_attr(attrs, "lang"))
                        .or_else(|| get_attr(attrs, "name"))
                        .map(|l| l.to_ascii_lowercase())
                        .or_else(|| self.current_lang());
                    self.voice_stack.push(lang);
                }
            }

            "say-as" => {
                if is_close {
                    match self.sayas_stack.pop() {
                        Some(SayAsKind::Spell) => {
                            self.spell_depth = self.spell_depth.saturating_sub(1);
                        }
                        Some(SayAsKind::Interpret) => {
                            self.flush();
                            self.interpret = SayAs::Normal;
                        }
                        _ => {}
                    }
                } else if !self_close {
                    let ia = get_attr(attrs, "interpret-as").unwrap_or_default();
                    let ia = ia.to_ascii_lowercase();
                    // `digits` reads each digit separately; spelling the content
                    // out achieves that ("42" → "4 2" → "four two").
                    let kind = if matches!(
                        ia.as_str(),
                        "characters" | "character" | "glyphs" | "spell" | "spell-out"
                            | "digits" | "tts:digits"
                            // Phone numbers: read each digit, `+`→"plus",
                            // separators silent — the spell path already does this.
                            | "telephone" | "tel" | "vxml:phone" | "phone-number"
                    ) {
                        self.spell_depth += 1;
                        SayAsKind::Spell
                    } else if let Some(mode) = interpret_mode(ia.as_str()) {
                        // Date/time/ordinal need the language, so tag the segment
                        // and let the translate layer render it.
                        self.flush();
                        self.interpret = mode;
                        SayAsKind::Interpret
                    } else {
                        SayAsKind::Other
                    };
                    self.sayas_stack.push(kind);
                }
            }

            "sub" => {
                if is_close {
                    if let Some(true) = self.sub_stack.pop() {
                        self.suppress_depth = self.suppress_depth.saturating_sub(1);
                    }
                } else if !self_close {
                    match get_attr(attrs, "alias") {
                        Some(alias) => {
                            self.cur.push(' ');
                            self.cur.push_str(&alias);
                            self.cur.push(' ');
                            self.sub_stack.push(true);
                            self.suppress_depth += 1;
                        }
                        None => self.sub_stack.push(false),
                    }
                }
            }

            "phoneme" => {
                if is_close {
                    // Match the most recent open `<phoneme>`; only lift the
                    // suppression it started.
                    if let Some(true) = self.phoneme_stack.pop() {
                        self.suppress_depth = self.suppress_depth.saturating_sub(1);
                    }
                } else {
                    // `ph` holds eSpeak phoneme mnemonics; emit them as an
                    // inline-phoneme span so the always-on `[[…]]` tokenizer
                    // renders them, and drop the element's readable content.
                    match get_attr(attrs, "ph").filter(|p| !p.is_empty()) {
                        Some(ph) => {
                            if self.suppress_depth == 0 {
                                self.cur.push_str("[[");
                                self.cur.push_str(&ph);
                                self.cur.push_str("]]");
                            }
                            if !self_close {
                                self.phoneme_stack.push(true);
                                self.suppress_depth += 1;
                            }
                        }
                        // No usable `ph`: behave like an unknown tag (strip the
                        // tag, keep content). Track the open so the close pops.
                        None => {
                            if !self_close {
                                self.phoneme_stack.push(false);
                            }
                        }
                    }
                }
            }

            // Formatting / prosody / root: strip the tag, keep content.
            _ => {}
        }
    }
}

/// Look up an attribute value in the tag's attribute string.
///
/// Handles `name="value"` and `name='value'`; whitespace around `=` is
/// tolerated.  Namespaced names (`xml:lang`) match on the local part too.
fn get_attr(attrs: &str, key: &str) -> Option<String> {
    let bytes: Vec<char> = attrs.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        if !is_name_start(bytes[i]) {
            i += 1;
            continue;
        }
        let start = i;
        while i < bytes.len() && is_name_char(bytes[i]) {
            i += 1;
        }
        let raw: String = bytes[start..i].iter().collect();
        let local = raw.rsplit(':').next().unwrap_or(&raw);
        let matches = raw.eq_ignore_ascii_case(key) || local.eq_ignore_ascii_case(key);

        while i < bytes.len() && bytes[i].is_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] != '=' {
            continue;
        }
        i += 1;
        while i < bytes.len() && bytes[i].is_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            return None;
        }
        let quote = bytes[i];
        if quote == '"' || quote == '\'' {
            i += 1;
            let vstart = i;
            while i < bytes.len() && bytes[i] != quote {
                i += 1;
            }
            let value: String = bytes[vstart..i.min(bytes.len())].iter().collect();
            if matches {
                return Some(value);
            }
            i += 1;
        } else {
            let vstart = i;
            while i < bytes.len() && !bytes[i].is_whitespace() {
                i += 1;
            }
            if matches {
                return Some(bytes[vstart..i].iter().collect());
            }
        }
    }
    None
}

/// Decode an XML/HTML entity beginning at `s[0] == '&'`.
///
/// Returns `(decoded, chars_consumed)` including the trailing `;`, or `None`
/// if the text is not a recognised entity.
fn decode_entity(s: &[char]) -> Option<(String, usize)> {
    debug_assert_eq!(s[0], '&');
    let semi = s.iter().take(12).position(|&c| c == ';')?;
    if semi < 2 {
        return None;
    }
    let body: String = s[1..semi].iter().collect();
    let decoded = if let Some(num) = body.strip_prefix('#') {
        let cp = if let Some(hex) = num.strip_prefix(['x', 'X']) {
            u32::from_str_radix(hex, 16).ok()?
        } else {
            num.parse::<u32>().ok()?
        };
        char::from_u32(cp)?.to_string()
    } else {
        match body.as_str() {
            "amp" => "&",
            "lt" => "<",
            "gt" => ">",
            "quot" => "\"",
            "apos" => "'",
            "nbsp" => " ",
            _ => return None,
        }
        .to_string()
    };
    Some((decoded, semi + 1))
}

// ---------------------------------------------------------------------------
// Small char-slice helpers
// ---------------------------------------------------------------------------

fn is_name_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_' || c == ':'
}
fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == ':' || c == '-' || c == '.'
}

fn starts_with(chars: &[char], at: usize, pat: &str) -> bool {
    pat.chars().enumerate().all(|(k, pc)| chars.get(at + k) == Some(&pc))
}

fn find_char(chars: &[char], from: usize, target: char) -> Option<usize> {
    (from..chars.len()).find(|&k| chars[k] == target)
}

fn find_seq(chars: &[char], from: usize, pat: &str) -> Option<usize> {
    let pc: Vec<char> = pat.chars().collect();
    if pc.is_empty() || from >= chars.len() {
        return None;
    }
    (from..=chars.len().saturating_sub(pc.len()))
        .find(|&k| pc.iter().enumerate().all(|(j, &p)| chars[k + j] == p))
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn show(s: &str) -> String {
        s.replace(SSML_BREAK, "|")
    }

    #[test]
    fn plain_text_unchanged() {
        assert_eq!(strip_markup("hello world"), "hello world");
    }

    #[test]
    fn mark_captures_name_and_position() {
        let (_segs, marks) = process_markup_with_marks("one <mark name=\"a\"/> two <mark name=\"b\"/>");
        assert_eq!(marks.len(), 2);
        assert_eq!(marks[0].name, "a");
        assert_eq!(marks[1].name, "b");
        // Positions are offsets into the stripped text; "a" comes before "b".
        assert!(marks[0].position < marks[1].position, "{marks:?}");
        // A mark before any text sits at position 0.
        let (_s, m) = process_markup_with_marks("<mark name=\"start\"/>hi");
        assert_eq!(m[0].position, 0, "{m:?}");
        // A mark carries no text and doesn't disturb the stripped output.
        assert_eq!(strip_markup("a<mark name=\"x\"/>b"), "ab");
    }

    #[test]
    fn strips_unknown_and_formatting_tags() {
        assert_eq!(strip_markup("<p>hello</p>"), format!("{b}hello{b}", b = SSML_BREAK));
        assert_eq!(strip_markup("a <emphasis>b</emphasis> c"), "a b c");
        assert_eq!(strip_markup("<speak>hi</speak>"), "hi");
        assert_eq!(strip_markup("x <foo bar='1'>y</foo> z"), "x y z");
    }

    #[test]
    fn decodes_named_entities() {
        assert_eq!(strip_markup("Tom &amp; Jerry"), "Tom & Jerry");
        assert_eq!(strip_markup("3 &lt; 5 &gt; 1"), "3 < 5 > 1");
        assert_eq!(strip_markup("q&quot;s &apos;t&apos;"), "q\"s 't'");
        assert_eq!(strip_markup("a &bogus; b"), "a &bogus; b");
    }

    #[test]
    fn decodes_numeric_entities() {
        assert_eq!(strip_markup("x &#65; y"), "x A y");
        assert_eq!(strip_markup("x &#x41; y"), "x A y");
        assert_eq!(strip_markup("&#233;"), "é");
    }

    #[test]
    fn say_as_characters_spells_out() {
        assert_eq!(
            strip_markup(r#"<say-as interpret-as="characters">cat</say-as>"#)
                .split_whitespace()
                .collect::<Vec<_>>(),
            ["c", "a", "t"]
        );
        assert_eq!(
            strip_markup(r#"<say-as interpret-as="cardinal">42</say-as>"#),
            "42"
        );
    }

    #[test]
    fn say_as_digits_spells_each_digit() {
        // `digits` spells the content out (each digit read separately).
        assert_eq!(
            strip_markup(r#"<say-as interpret-as="digits">42</say-as>"#)
                .split_whitespace()
                .collect::<Vec<_>>(),
            ["4", "2"]
        );
        // Leading zeros are preserved as separate digits.
        assert_eq!(
            strip_markup(r#"<say-as interpret-as="digits">007</say-as>"#)
                .split_whitespace()
                .collect::<Vec<_>>(),
            ["0", "0", "7"]
        );
        // `cardinal`/`number` are read as a whole number (content untouched).
        assert_eq!(strip_markup(r#"<say-as interpret-as="cardinal">42</say-as>"#), "42");
    }

    #[test]
    fn say_as_telephone_spells_out() {
        // Phone numbers spell each digit; `+`/`-` are kept as separate tokens
        // (spoken/silent downstream).
        assert_eq!(
            strip_markup(r#"<say-as interpret-as="telephone">555</say-as>"#)
                .split_whitespace()
                .collect::<Vec<_>>(),
            ["5", "5", "5"]
        );
        assert_eq!(
            strip_markup(r#"<say-as interpret-as="tel">+1</say-as>"#)
                .split_whitespace()
                .collect::<Vec<_>>(),
            ["+", "1"]
        );
    }

    #[test]
    fn say_as_ordinal_tags_segment() {
        let segs = process_markup(r#"the <say-as interpret-as="ordinal">3</say-as> item"#);
        // The number is its own segment, tagged Ordinal.
        let ordinal: Vec<_> = segs.iter().filter(|s| s.interpret == SayAs::Ordinal).collect();
        assert_eq!(ordinal.len(), 1, "{segs:?}");
        assert_eq!(ordinal[0].text.trim(), "3");
        // Surrounding text stays Normal.
        assert!(segs.iter().any(|s| s.interpret == SayAs::Normal && s.text.contains("the")));
        assert!(segs.iter().any(|s| s.interpret == SayAs::Normal && s.text.contains("item")));
    }

    #[test]
    fn document_prosody_whole_wrap() {
        let p = document_prosody(r#"<prosody rate="50%">hello world</prosody>"#).unwrap();
        assert_eq!(p.rate, Some(0.5));
        // Inside <speak>, keyword value.
        let p = document_prosody(r#"<speak><prosody rate="fast">x</prosody></speak>"#).unwrap();
        assert_eq!(p.rate, Some(1.4));
        // Pitch (relative) + volume (keyword).
        let p = document_prosody(r#"<prosody pitch="+20%" volume="soft">x</prosody>"#).unwrap();
        assert_eq!(p.pitch, Some(1.2));
        assert_eq!(p.volume, Some(0.75));
    }

    #[test]
    fn document_prosody_partial_or_multiple_is_none() {
        // Not wrapping all speakable content → None (never applied globally).
        assert!(document_prosody(r#"plain <prosody rate="50%">x</prosody>"#).is_none());
        assert!(document_prosody(r#"<prosody rate="50%">x</prosody> plain"#).is_none());
        // Multiple prosody spans → None.
        assert!(
            document_prosody(r#"<prosody rate="50%">a</prosody><prosody rate="200%">b</prosody>"#)
                .is_none()
        );
        // No prosody, or no mappable attribute → None.
        assert!(document_prosody("hello").is_none());
        assert!(document_prosody(r#"<prosody pitch="200Hz">x</prosody>"#).is_none());
    }

    #[test]
    fn say_as_date_time_tag_segments() {
        let segs = process_markup(r#"<say-as interpret-as="date">2024-01-15</say-as>"#);
        assert!(segs.iter().any(|s| s.interpret == SayAs::Date && s.text.contains("2024")));
        let segs = process_markup(r#"<say-as interpret-as="time">14:30</say-as>"#);
        assert!(segs.iter().any(|s| s.interpret == SayAs::Time && s.text.contains("14:30")));
    }

    #[test]
    fn sub_replaces_with_alias() {
        assert_eq!(
            strip_markup(r#"<sub alias="World Health Organization">WHO</sub>"#).trim(),
            "World Health Organization"
        );
        assert_eq!(strip_markup("<sub>WHO</sub>"), "WHO");
    }

    #[test]
    fn phoneme_ph_becomes_inline_brackets() {
        // `ph` is emitted as inline-phoneme brackets; content is discarded.
        assert_eq!(
            strip_markup(r#"<phoneme ph="h@l'oU">hello</phoneme>"#),
            "[[h@l'oU]]"
        );
        assert_eq!(
            strip_markup(r#"x <phoneme ph="k">see</phoneme> y"#),
            "x [[k]] y"
        );
        // Self-closing form: no content to discard.
        assert_eq!(strip_markup(r#"<phoneme ph="t"/>"#), "[[t]]");
        // alphabet attribute is tolerated; ph still passes through.
        assert_eq!(
            strip_markup(r#"<phoneme alphabet="espeak" ph="s">ess</phoneme>"#),
            "[[s]]"
        );
        // No `ph`: strip the tag, keep the content (unknown-tag behaviour).
        assert_eq!(strip_markup("<phoneme>hello</phoneme>"), "hello");
        // Empty `ph`: treated as no `ph`.
        assert_eq!(strip_markup(r#"<phoneme ph="">hi</phoneme>"#), "hi");
    }

    #[test]
    fn phoneme_suppression_is_balanced_with_siblings() {
        // Text after a </phoneme> must not stay suppressed.
        assert_eq!(
            strip_markup(r#"a <phoneme ph="k">x</phoneme> b <sub alias="cee">c</sub> d"#)
                .split_whitespace()
                .collect::<Vec<_>>(),
            ["a", "[[k]]", "b", "cee", "d"]
        );
    }

    #[test]
    fn break_inserts_boundary() {
        assert_eq!(show(&strip_markup("one<break time=\"500ms\"/>two")), "one|two");
        assert_eq!(show(&strip_markup("a<break/>b")), "a|b");
    }

    #[test]
    fn comments_and_pis_discarded() {
        assert_eq!(strip_markup("a<!-- hidden -->b"), "ab");
        assert_eq!(strip_markup("a<?xml version='1.0'?>b"), "ab");
        assert_eq!(strip_markup("<!DOCTYPE speak>hi"), "hi");
    }

    #[test]
    fn nested_say_as_and_sub_are_balanced() {
        let out = strip_markup(r#"say <sub alias="one">1</sub> then <sub alias="two">2</sub>"#);
        assert_eq!(out.split_whitespace().collect::<Vec<_>>(), ["say", "one", "then", "two"]);
    }

    #[test]
    fn unterminated_tag_is_ignored() {
        assert_eq!(strip_markup("hello <broken"), "hello ");
    }

    #[test]
    fn voice_produces_language_tagged_segments() {
        let segs = process_markup(r#"hello <voice xml:lang="fr">bonjour</voice> bye"#);
        let non_empty: Vec<_> = segs.into_iter().filter(|s| !s.text.trim().is_empty()).collect();
        assert_eq!(non_empty.len(), 3);
        assert_eq!(non_empty[0].lang, None);
        assert_eq!(non_empty[1].lang.as_deref(), Some("fr"));
        assert_eq!(non_empty[1].text.trim(), "bonjour");
        assert_eq!(non_empty[2].lang, None);
    }

    #[test]
    fn voice_name_and_lang_attrs() {
        assert_eq!(
            process_markup(r#"<voice name="de">x</voice>"#)[0].lang.as_deref(),
            Some("de")
        );
        assert_eq!(
            process_markup(r#"<voice lang="es">x</voice>"#)[0].lang.as_deref(),
            Some("es")
        );
    }

    #[test]
    fn no_voice_means_no_language_tags() {
        let segs = process_markup("just plain <emphasis>text</emphasis> here");
        assert!(segs.iter().all(|s| s.lang.is_none()));
    }
}
