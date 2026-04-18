//! Inline parser.
//!
//! Single-pass character walker that recognizes the six substitution groups.
//! Special characters are left as UTF-8 text in the AST; the HTML5 converter
//! handles entity escaping. This keeps the AST readable and language-neutral.

use adoc_core::{AttributeValue, Attributes, Inline};

use crate::subs::Subs;

pub fn parse(text: &str, attrs: &Attributes, subs: Subs) -> Vec<Inline> {
    if text.is_empty() {
        return Vec::new();
    }
    let mut parser = InlineParser {
        src: text,
        attrs,
        subs,
        pos: 0,
    };
    parser.run_until(|_| false)
}

struct InlineParser<'a> {
    src: &'a str,
    attrs: &'a Attributes,
    subs: Subs,
    pos: usize,
}

impl<'a> InlineParser<'a> {
    fn run_until(&mut self, stop: impl Fn(&str) -> bool) -> Vec<Inline> {
        let mut out: Vec<Inline> = Vec::new();
        let mut buf = String::new();
        while self.pos < self.src.len() {
            let remaining = &self.src[self.pos..];
            if stop(remaining) {
                break;
            }

            if self.subs.post_replacements {
                if let Some(consumed) = self.try_linebreak(remaining) {
                    flush(&mut buf, &mut out);
                    out.push(Inline::LineBreak);
                    self.pos += consumed;
                    continue;
                }
            }

            if self.subs.macros {
                if let Some((inline, consumed)) = self.try_macro(remaining) {
                    flush(&mut buf, &mut out);
                    out.push(inline);
                    self.pos += consumed;
                    continue;
                }
                if let Some((inline, consumed)) = self.try_autolink(remaining) {
                    flush(&mut buf, &mut out);
                    out.push(inline);
                    self.pos += consumed;
                    continue;
                }
            }

            if self.subs.attributes {
                if let Some((resolved, consumed)) = self.try_attribute_ref(remaining) {
                    buf.push_str(&resolved);
                    self.pos += consumed;
                    continue;
                }
            }

            if self.subs.quotes {
                if let Some((inline, consumed)) = self.try_quote(remaining, &buf) {
                    flush(&mut buf, &mut out);
                    out.push(inline);
                    self.pos += consumed;
                    continue;
                }
            }

            if self.subs.replacements {
                if let Some((replacement, consumed)) = self.try_replacement(remaining) {
                    buf.push_str(replacement);
                    self.pos += consumed;
                    continue;
                }
            }

            // Default: consume one UTF-8 character.
            let ch = remaining.chars().next().expect("non-empty");
            buf.push(ch);
            self.pos += ch.len_utf8();
        }
        flush(&mut buf, &mut out);
        out
    }

    // --- post-replacements ---

    fn try_linebreak(&self, rem: &str) -> Option<usize> {
        // " +" followed by end of input or newline produces a hard line break.
        // The preprocessor removed newlines, so we match " +" at end only.
        if rem == " +" {
            Some(2)
        } else {
            None
        }
    }

    // --- macros ---

    fn try_macro(&self, rem: &str) -> Option<(Inline, usize)> {
        if let Some(m) = parse_prefix_macro(rem, "link:") {
            return Some(m);
        }
        if let Some(m) = parse_prefix_macro(rem, "mailto:") {
            return Some(m);
        }
        if let Some(m) = parse_image_macro(rem) {
            return Some(m);
        }
        if let Some(m) = parse_xref_macro(rem, self.attrs) {
            return Some(m);
        }
        if let Some(m) = parse_shorthand_xref(rem, self.attrs) {
            return Some(m);
        }
        None
    }

    fn try_autolink(&self, rem: &str) -> Option<(Inline, usize)> {
        for scheme in ["https://", "http://", "ftp://"] {
            if rem.starts_with(scheme) {
                let end = rem
                    .find(|c: char| c.is_whitespace() || matches!(c, '[' | ']' | '<' | '>'))
                    .unwrap_or(rem.len());
                if end > scheme.len() {
                    let url = &rem[..end];
                    // Trim trailing punctuation that's unlikely to be part of the URL.
                    let trimmed = url.trim_end_matches(|c: char| ".,;:!?".contains(c));
                    let text = trimmed.to_string();
                    return Some((
                        Inline::Link {
                            href: text.clone(),
                            text: vec![Inline::Text(text)],
                        },
                        trimmed.len(),
                    ));
                }
            }
        }
        None
    }

    // --- attribute references ---

    fn try_attribute_ref(&self, rem: &str) -> Option<(String, usize)> {
        if !rem.starts_with('{') {
            return None;
        }
        let close = rem.find('}')?;
        let name = &rem[1..close];
        if !is_attribute_name(name) {
            return None;
        }
        let consumed = close + 1;
        let resolved = match self.attrs.get(name) {
            Some(AttributeValue::String(s)) => s.clone(),
            Some(AttributeValue::Bool(true)) => String::new(),
            Some(AttributeValue::Bool(false)) => format!("{{{name}}}"),
            None => format!("{{{name}}}"),
        };
        Some((resolved, consumed))
    }

    // --- quotes ---

    fn try_quote(&self, rem: &str, buf: &str) -> Option<(Inline, usize)> {
        // Try unconstrained first (longer marker wins).
        for (marker, ctor) in UNCONSTRAINED_QUOTES {
            if !rem.starts_with(marker) {
                continue;
            }
            if let Some(inner_len) = find_closing(rem, marker, marker.len(), false) {
                let inner_text = &rem[marker.len()..marker.len() + inner_len];
                let inner = parse(inner_text, self.attrs, self.subs);
                return Some((ctor(inner), marker.len() * 2 + inner_len));
            }
        }
        for (marker, ctor) in CONSTRAINED_QUOTES {
            if !is_constrained_open(rem, marker, buf) {
                continue;
            }
            if let Some(inner_len) = find_closing(rem, marker, marker.len(), true) {
                let inner_text = &rem[marker.len()..marker.len() + inner_len];
                let inner = parse(inner_text, self.attrs, self.subs);
                return Some((ctor(inner), marker.len() * 2 + inner_len));
            }
        }
        None
    }

    // --- replacements ---

    fn try_replacement(&self, rem: &str) -> Option<(&'static str, usize)> {
        for (pattern, replacement) in REPLACEMENTS {
            if rem.starts_with(pattern) {
                return Some((replacement, pattern.len()));
            }
        }
        None
    }
}

fn flush(buf: &mut String, out: &mut Vec<Inline>) {
    if !buf.is_empty() {
        out.push(Inline::Text(std::mem::take(buf)));
    }
}

fn is_attribute_name(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphanumeric() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

// Constrained quotes: single-char markers, require word boundary at edges.
const CONSTRAINED_QUOTES: &[(&str, fn(Vec<Inline>) -> Inline)] = &[
    ("*", Inline::Strong),
    ("_", Inline::Emphasis),
    ("`", Inline::Monospace),
];

// Unconstrained quotes: double-char markers, no boundary requirement.
const UNCONSTRAINED_QUOTES: &[(&str, fn(Vec<Inline>) -> Inline)] = &[
    ("**", Inline::Strong),
    ("__", Inline::Emphasis),
    ("``", Inline::Monospace),
];

fn is_constrained_open(rem: &str, marker: &str, buf: &str) -> bool {
    if !rem.starts_with(marker) {
        return false;
    }
    // Char immediately after the opening marker must not be whitespace.
    let after = &rem[marker.len()..];
    match after.chars().next() {
        None => return false,
        Some(c) if c.is_whitespace() => return false,
        _ => {}
    }
    // Char immediately before the opening marker must be start-of-input
    // or non-word (spec: constrained quote boundary rule).
    match buf.chars().last() {
        None => true,
        Some(c) => !is_word_char(c),
    }
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Finds the closing marker inside `rem` (starting after the opening marker).
/// Returns length of inner text, or `None` if no valid close exists.
fn find_closing(rem: &str, marker: &str, start: usize, constrained: bool) -> Option<usize> {
    let hay = &rem[start..];
    let bytes = hay.as_bytes();
    let mb = marker.as_bytes();
    let mut i = 0;
    while i + mb.len() <= bytes.len() {
        if &bytes[i..i + mb.len()] == mb {
            // The preceding char (inside the inner text) must not be whitespace
            // for constrained quotes, and the span must be non-empty.
            if i == 0 {
                return None;
            }
            let inner = &hay[..i];
            if inner.chars().next_back().map_or(true, |c| c.is_whitespace()) {
                i += 1;
                continue;
            }
            if constrained {
                // Char after the closing marker must not be a word char.
                let after = &hay[i + mb.len()..];
                if after
                    .chars()
                    .next()
                    .map_or(true, |c| !is_word_char(c))
                {
                    return Some(i);
                }
                i += 1;
                continue;
            }
            return Some(i);
        }
        i += 1;
    }
    None
}

const REPLACEMENTS: &[(&str, &str)] = &[
    ("(C)", "\u{00A9}"),
    ("(R)", "\u{00AE}"),
    ("(TM)", "\u{2122}"),
    ("...", "\u{2026}"),
    ("--", "\u{2014}"),
    ("->", "\u{2192}"),
    ("=>", "\u{21D2}"),
    ("<-", "\u{2190}"),
    ("<=", "\u{21D0}"),
];

// --- macro helpers ---

fn parse_prefix_macro(rem: &str, prefix: &str) -> Option<(Inline, usize)> {
    if !rem.starts_with(prefix) {
        return None;
    }
    let after = &rem[prefix.len()..];
    let target_end = after
        .find(|c: char| c == '[' || c.is_whitespace())?;
    if !after[target_end..].starts_with('[') {
        return None;
    }
    let target = &after[..target_end];
    if target.is_empty() {
        return None;
    }
    let attrs_end = find_unescaped(&after[target_end..], ']')?;
    let attrs_str = &after[target_end + 1..target_end + attrs_end];
    let consumed = prefix.len() + target_end + attrs_end + 1;
    let href = if prefix == "mailto:" {
        format!("mailto:{target}")
    } else {
        target.to_string()
    };
    let text_src = attrs_str.split(',').next().unwrap_or("").trim();
    let text = if text_src.is_empty() {
        vec![Inline::Text(target.to_string())]
    } else {
        vec![Inline::Text(text_src.to_string())]
    };
    Some((Inline::Link { href, text }, consumed))
}

fn parse_image_macro(rem: &str) -> Option<(Inline, usize)> {
    let prefix = "image:";
    if !rem.starts_with(prefix) {
        return None;
    }
    let after = &rem[prefix.len()..];
    let target_end = after.find('[')?;
    let target = &after[..target_end];
    if target.is_empty() {
        return None;
    }
    let attrs_end = find_unescaped(&after[target_end..], ']')?;
    let attrs_str = &after[target_end + 1..target_end + attrs_end];
    let consumed = prefix.len() + target_end + attrs_end + 1;
    let parts: Vec<&str> = attrs_str.split(',').map(str::trim).collect();
    let alt = parts.first().copied().unwrap_or("").to_string();
    let width = parts.get(1).filter(|s| !s.is_empty()).map(|s| s.to_string());
    let height = parts.get(2).filter(|s| !s.is_empty()).map(|s| s.to_string());
    Some((
        Inline::Image {
            target: target.to_string(),
            alt,
            width,
            height,
        },
        consumed,
    ))
}

fn parse_xref_macro(rem: &str, attrs: &Attributes) -> Option<(Inline, usize)> {
    let prefix = "xref:";
    if !rem.starts_with(prefix) {
        return None;
    }
    let after = &rem[prefix.len()..];
    let target_end = after.find('[')?;
    let target = &after[..target_end];
    if target.is_empty() {
        return None;
    }
    let attrs_end = find_unescaped(&after[target_end..], ']')?;
    let text_src = &after[target_end + 1..target_end + attrs_end];
    let consumed = prefix.len() + target_end + attrs_end + 1;
    let text = if text_src.is_empty() {
        None
    } else {
        Some(parse(text_src, attrs, Subs::NORMAL))
    };
    Some((
        Inline::Xref {
            target: target.to_string(),
            text,
        },
        consumed,
    ))
}

fn parse_shorthand_xref(rem: &str, attrs: &Attributes) -> Option<(Inline, usize)> {
    if !rem.starts_with("<<") {
        return None;
    }
    let after = &rem[2..];
    let close = after.find(">>")?;
    let inner = &after[..close];
    let consumed = 2 + close + 2;
    let (target, text_src) = match inner.split_once(',') {
        Some((t, rest)) => (t.trim(), Some(rest.trim())),
        None => (inner.trim(), None),
    };
    if target.is_empty() {
        return None;
    }
    let text = text_src.map(|t| parse(t, attrs, Subs::NORMAL));
    Some((
        Inline::Xref {
            target: target.to_string(),
            text,
        },
        consumed,
    ))
}

fn find_unescaped(s: &str, needle: char) -> Option<usize> {
    let mut escape = false;
    for (idx, ch) in s.char_indices() {
        if escape {
            escape = false;
            continue;
        }
        if ch == '\\' {
            escape = true;
            continue;
        }
        if ch == needle {
            return Some(idx);
        }
    }
    None
}
