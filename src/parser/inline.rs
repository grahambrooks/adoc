//! Inline parser.
//!
//! Single-pass character walker that recognizes the six substitution groups.
//! Special characters are left as UTF-8 text in the AST; the HTML5 converter
//! handles entity escaping. This keeps the AST readable and language-neutral.

use std::fmt::Write;

use crate::ast::{AttributeValue, Attributes, Inline};

use super::subs::Subs;

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
                if let Some((inline, consumed)) = self.try_passthrough(remaining, &buf) {
                    flush(&mut buf, &mut out);
                    out.push(inline);
                    self.pos += consumed;
                    continue;
                }
                if let Some((inlines, consumed)) = self.try_smart_quote(remaining, &buf) {
                    flush(&mut buf, &mut out);
                    out.extend(inlines);
                    self.pos += consumed;
                    continue;
                }
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
        if let Some(m) = parse_anchor_macro(rem) {
            return Some(m);
        }
        if let Some(m) = parse_kbd_macro(rem) {
            return Some(m);
        }
        if let Some(m) = parse_btn_macro(rem) {
            return Some(m);
        }
        if let Some(m) = parse_menu_macro(rem) {
            return Some(m);
        }
        if let Some(m) = parse_prefix_macro(rem, "link:", self.attrs) {
            return Some(m);
        }
        if let Some(m) = parse_prefix_macro(rem, "mailto:", self.attrs) {
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
        if let Some(m) = parse_footnote_macro(rem, self.attrs, self.subs) {
            return Some(m);
        }
        if let Some(m) = parse_pass_macro(rem) {
            return Some(m);
        }
        None
    }

    // --- passthroughs ---
    //
    // Both forms emit raw text with HTML-escape but no further inline
    // substitution. Handled outside [`try_quote`] because the inner text
    // is not recursively parsed.

    fn try_passthrough(&self, rem: &str, buf: &str) -> Option<(Inline, usize)> {
        // Unconstrained `++text++` — try first so `+` constrained doesn't
        // greedily match a single `+`.
        if rem.starts_with("++") {
            if let Some(inner_len) = find_closing(rem, "++", 2, false) {
                let value = rem[2..2 + inner_len].to_string();
                return Some((Inline::Passthrough { value }, 2 + inner_len + 2));
            }
        }
        // Constrained `+text+` — single `+` with word-boundary rule.
        if rem.starts_with('+') && !rem.starts_with("++") && is_constrained_open(rem, "+", buf) {
            if let Some(inner_len) = find_closing(rem, "+", 1, true) {
                let value = rem[1..1 + inner_len].to_string();
                return Some((Inline::Passthrough { value }, 1 + inner_len + 1));
            }
        }
        None
    }

    fn try_autolink(&self, rem: &str) -> Option<(Inline, usize)> {
        for scheme in ["https://", "http://", "ftp://"] {
            if !rem.starts_with(scheme) {
                continue;
            }
            let end = rem
                .find(|c: char| c.is_whitespace() || matches!(c, '[' | ']' | '<' | '>'))
                .unwrap_or(rem.len());
            if end <= scheme.len() {
                continue;
            }
            let url = &rem[..end];
            let trimmed = url.trim_end_matches(|c: char| ".,;:!?".contains(c));
            // Optional `[label]` form: `https://url[label]` becomes a
            // labelled link rather than a raw autolink.
            if rem[trimmed.len()..].starts_with('[') {
                let after = &rem[trimmed.len()..];
                if let Some(close_rel) = find_unescaped(after, ']') {
                    let label_src = &after[1..close_rel];
                    let consumed = trimmed.len() + close_rel + 1;
                    let label = label_src.split(',').next().unwrap_or("").trim();
                    let text_value = if label.is_empty() {
                        trimmed.to_string()
                    } else {
                        substitute_attrs(label, self.attrs)
                    };
                    return Some((
                        Inline::Link {
                            href: trimmed.to_string(),
                            text: vec![Inline::Text { value: text_value }],
                        },
                        consumed,
                    ));
                }
            }
            // Fall back to a bare autolink.
            let text = trimmed.to_string();
            return Some((
                Inline::Link {
                    href: text.clone(),
                    text: vec![Inline::Text { value: text }],
                },
                trimmed.len(),
            ));
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

    // --- smart quotes ---

    /// Recognise the AsciiDoc smart-quote forms and emit them as a sequence
    /// of inlines: opening Unicode quote, parsed inner content, closing
    /// Unicode quote.
    ///
    /// * `"\`text\`"` → `\u{201C}text\u{201D}`  (curly double quotes)
    /// * `'\`text\`'` → `\u{2018}text\u{2019}`  (curly single quotes)
    ///
    /// The opening marker requires word-boundary semantics (the char before
    /// the `"` or `'` must not be alphanumeric, mirroring constrained-quote
    /// rules) so common contractions like `it's` and quoted speech inside
    /// a word don't trigger.
    fn try_smart_quote(&self, rem: &str, buf: &str) -> Option<(Vec<Inline>, usize)> {
        for (open, close, lq, rq) in SMART_QUOTE_PAIRS {
            if !rem.starts_with(open) {
                continue;
            }
            // Word-boundary on the LEFT.
            let prev = buf.chars().last();
            let left_ok = prev
                .map(|c| !c.is_alphanumeric() && c != '_')
                .unwrap_or(true);
            if !left_ok {
                continue;
            }
            // The first content char must not be whitespace.
            let after_open = &rem[open.len()..];
            if after_open.chars().next().is_none_or(|c| c.is_whitespace()) {
                continue;
            }
            // Find the closing pair somewhere ahead.
            let Some(close_idx) = after_open.find(close) else {
                continue;
            };
            // Closing pair must not be immediately preceded by whitespace.
            let inner = &after_open[..close_idx];
            if inner
                .chars()
                .last()
                .map(char::is_whitespace)
                .unwrap_or(true)
            {
                continue;
            }
            let parsed = parse(inner, self.attrs, self.subs);
            let mut out = Vec::with_capacity(parsed.len() + 2);
            out.push(Inline::Text {
                value: lq.to_string(),
            });
            out.extend(parsed);
            out.push(Inline::Text {
                value: rq.to_string(),
            });
            let consumed = open.len() + close_idx + close.len();
            return Some((out, consumed));
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
        out.push(Inline::Text {
            value: std::mem::take(buf),
        });
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

/// Smart-quote pairs: (open marker, close marker, opening Unicode quote,
/// closing Unicode quote). Tried in order so the longer/double form wins
/// before the single form when both could match.
const SMART_QUOTE_PAIRS: &[(&str, &str, &str, &str)] = &[
    ("\"`", "`\"", "\u{201C}", "\u{201D}"),
    ("'`", "`'", "\u{2018}", "\u{2019}"),
];

// Constrained quotes: single-char markers, require word boundary at edges.
const CONSTRAINED_QUOTES: &[(&str, fn(Vec<Inline>) -> Inline)] = &[
    ("*", make_strong),
    ("_", make_emphasis),
    ("`", make_monospace),
    ("#", make_highlight),
];

// Unconstrained quotes: longer (or marker-class-specific) markers, with
// only inner-edge whitespace rejection — no outer word-boundary rule.
// Subscript (`~`) and superscript (`^`) are single-char but unconstrained
// per spec, so they live here.
const UNCONSTRAINED_QUOTES: &[(&str, fn(Vec<Inline>) -> Inline)] = &[
    ("**", make_strong),
    ("__", make_emphasis),
    ("``", make_monospace),
    ("##", make_highlight),
    ("~", make_subscript),
    ("^", make_superscript),
];

// Variant ctors named so they can be used as `fn` pointers in const
// tables — the struct-shaped variants can't be referenced directly the
// way tuple variants can.
fn make_strong(children: Vec<Inline>) -> Inline {
    Inline::Strong { children }
}
fn make_emphasis(children: Vec<Inline>) -> Inline {
    Inline::Emphasis { children }
}
fn make_monospace(children: Vec<Inline>) -> Inline {
    Inline::Monospace { children }
}
fn make_highlight(children: Vec<Inline>) -> Inline {
    Inline::Highlight { children }
}
fn make_subscript(children: Vec<Inline>) -> Inline {
    Inline::Subscript { children }
}
fn make_superscript(children: Vec<Inline>) -> Inline {
    Inline::Superscript { children }
}

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
            if inner
                .chars()
                .next_back()
                .map_or(true, |c| c.is_whitespace())
            {
                i += 1;
                continue;
            }
            if constrained {
                // Char after the closing marker must not be a word char.
                let after = &hay[i + mb.len()..];
                if after.chars().next().map_or(true, |c| !is_word_char(c)) {
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

fn parse_prefix_macro(rem: &str, prefix: &str, attrs: &Attributes) -> Option<(Inline, usize)> {
    if !rem.starts_with(prefix) {
        return None;
    }
    let after = &rem[prefix.len()..];
    let target_end = after.find(|c: char| c == '[' || c.is_whitespace())?;
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
    // Attribute references in the target are resolved at macro time; the
    // 6-group substitution pipeline runs attributes *after* macros, so the
    // macro itself has to do this lookup or `link:{homepage}[...]` would
    // emit `{homepage}` as the href.
    let target = substitute_attrs(target, attrs);
    let href = if prefix == "mailto:" {
        format!("mailto:{target}")
    } else {
        target.clone()
    };
    let text_src = attrs_str.split(',').next().unwrap_or("").trim();
    let text_value = if text_src.is_empty() {
        target
    } else {
        substitute_attrs(text_src, attrs)
    };
    let text = vec![Inline::Text { value: text_value }];
    Some((Inline::Link { href, text }, consumed))
}

/// `anchor:id[]` and `anchor:id[reftext]` — emits an empty `<a>` so the
/// id becomes a link target. The optional reftext is currently dropped
/// (a doc-wide xref registry would consume it for label substitution).
fn parse_anchor_macro(rem: &str) -> Option<(Inline, usize)> {
    let prefix = "anchor:";
    if !rem.starts_with(prefix) {
        return None;
    }
    let after = &rem[prefix.len()..];
    let target_end = after.find('[')?;
    let id = &after[..target_end];
    if id.is_empty() || !is_attribute_name(id) {
        return None;
    }
    let attrs_end = find_unescaped(&after[target_end..], ']')?;
    let consumed = prefix.len() + target_end + attrs_end + 1;
    Some((
        Inline::RawHtml {
            value: format!(r#"<a id="{}"></a>"#, escape_html_attr(id)),
        },
        consumed,
    ))
}

/// `kbd:[Ctrl+Alt+Del]` — emit each `+`-separated key in its own `<kbd>`.
/// `kbd:[Enter]` produces a single `<kbd>Enter</kbd>`.
fn parse_kbd_macro(rem: &str) -> Option<(Inline, usize)> {
    let prefix = "kbd:[";
    if !rem.starts_with(prefix) {
        return None;
    }
    let after = &rem[prefix.len()..];
    let close = find_unescaped(&rem[prefix.len() - 1..], ']')?;
    let inner = &after[..close - 1];
    let consumed = prefix.len() + close;
    let keys: Vec<&str> = inner
        .split('+')
        .map(str::trim)
        .filter(|k| !k.is_empty())
        .collect();
    if keys.is_empty() {
        return None;
    }
    let mut html = String::from(r#"<span class="keyseq">"#);
    for (i, key) in keys.iter().enumerate() {
        if i > 0 {
            html.push('+');
        }
        let _ = write!(html, "<kbd>{}</kbd>", escape_html_attr(key));
    }
    html.push_str("</span>");
    Some((Inline::RawHtml { value: html }, consumed))
}

/// `btn:[Save]` — UI button label.
fn parse_btn_macro(rem: &str) -> Option<(Inline, usize)> {
    let prefix = "btn:[";
    if !rem.starts_with(prefix) {
        return None;
    }
    let after = &rem[prefix.len()..];
    let close = find_unescaped(&rem[prefix.len() - 1..], ']')?;
    let inner = &after[..close - 1];
    let consumed = prefix.len() + close;
    Some((
        Inline::RawHtml {
            value: format!(r#"<b class="button">[{}]</b>"#, escape_html_attr(inner)),
        },
        consumed,
    ))
}

/// `menu:File[Save As]` and `menu:File[Save > Save As]` — menu path.
/// Items are split on `>` and joined with a typographic right-pointing
/// guillemet (`›`).
fn parse_menu_macro(rem: &str) -> Option<(Inline, usize)> {
    let prefix = "menu:";
    if !rem.starts_with(prefix) {
        return None;
    }
    let after = &rem[prefix.len()..];
    let bracket = after.find('[')?;
    let target = &after[..bracket];
    if target.is_empty() || target.contains(char::is_whitespace) {
        return None;
    }
    let attrs_end = find_unescaped(&after[bracket..], ']')?;
    let inner = &after[bracket + 1..bracket + attrs_end];
    let consumed = prefix.len() + bracket + attrs_end + 1;
    let mut parts = vec![target.to_string()];
    parts.extend(inner.split('>').map(|s| s.trim().to_string()));
    let parts: Vec<String> = parts.into_iter().filter(|s| !s.is_empty()).collect();
    let inner_html = parts
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let class = if i == 0 { "menu" } else { "menuitem" };
            format!(r#"<span class="{class}">{}</span>"#, escape_html_attr(p))
        })
        .collect::<Vec<_>>()
        .join(" \u{203A} ");
    Some((
        Inline::RawHtml {
            value: format!(r#"<span class="menuseq">{inner_html}</span>"#),
        },
        consumed,
    ))
}

fn escape_html_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Resolve `{name}` attribute references in a flat string. Used by macro
/// helpers that run before the regular attribute-substitution pass.
/// Unknown names are left literal.
fn substitute_attrs(s: &str, attrs: &Attributes) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'{' {
            if let Some(close_rel) = s[i + 1..].find('}') {
                let name = &s[i + 1..i + 1 + close_rel];
                if is_attribute_name(name) {
                    if let Some(AttributeValue::String(v)) = attrs.get(name) {
                        out.push_str(v);
                        i += 1 + close_rel + 1;
                        continue;
                    }
                }
            }
        }
        let ch_len = utf8_char_len(bytes[i]);
        out.push_str(&s[i..i + ch_len]);
        i += ch_len;
    }
    out
}

fn utf8_char_len(b: u8) -> usize {
    match b {
        0..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        _ => 4,
    }
}

fn parse_image_macro(rem: &str) -> Option<(Inline, usize)> {
    let prefix = "image:";
    if !rem.starts_with(prefix) {
        return None;
    }
    // Reject the block form (`image::`) here — the block parser handles
    // those before falling through to inline parsing.
    let after = &rem[prefix.len()..];
    if after.starts_with(':') {
        return None;
    }
    let target_end = after.find(|c: char| c == '[' || c.is_whitespace())?;
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
    let parts: Vec<&str> = attrs_str.split(',').map(str::trim).collect();
    let alt = parts.first().copied().unwrap_or("").to_string();
    let width = parts
        .get(1)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let height = parts
        .get(2)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
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

/// `footnote:[text]` (anonymous) or `footnote:id[text]` (named). The
/// inner text is parsed for the active substitutions so it can carry
/// formatting, links, etc.
fn parse_footnote_macro(rem: &str, attrs: &Attributes, subs: Subs) -> Option<(Inline, usize)> {
    let prefix = "footnote:";
    if !rem.starts_with(prefix) {
        return None;
    }
    let after = &rem[prefix.len()..];
    let bracket = after.find('[')?;
    let id_str = &after[..bracket];
    let id = if id_str.is_empty() {
        None
    } else if is_attribute_name(id_str) {
        Some(id_str.to_string())
    } else {
        return None;
    };
    let attrs_end = find_unescaped(&after[bracket..], ']')?;
    let inner = &after[bracket + 1..bracket + attrs_end];
    let consumed = prefix.len() + bracket + attrs_end + 1;
    let text = parse(inner, attrs, subs);
    Some((Inline::Footnote { id, text }, consumed))
}

/// `pass:[text]` — verbatim insertion (no escape, no further subs).
/// Modifier letters (`pass:c[]`, `pass:n[]`, etc.) are not yet honoured;
/// for now the body is always treated as raw HTML.
fn parse_pass_macro(rem: &str) -> Option<(Inline, usize)> {
    let prefix = "pass:";
    if !rem.starts_with(prefix) {
        return None;
    }
    let after = &rem[prefix.len()..];
    let bracket = after.find('[')?;
    // Reject letters that aren't ascii alpha (so we don't accidentally
    // consume `pass:foo bar` etc.) — only modifiers should sit here.
    if !after[..bracket].chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    let attrs_end = find_unescaped(&after[bracket..], ']')?;
    let inner = &after[bracket + 1..bracket + attrs_end];
    let consumed = prefix.len() + bracket + attrs_end + 1;
    Some((
        Inline::RawHtml {
            value: inner.to_string(),
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
