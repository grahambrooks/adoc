//! Block metadata parser.
//!
//! Recognises the two metadata-line forms that attach to the next block:
//!
//! - `.Title` — a line starting with a single `.` followed by non-space,
//!   non-`.` content, becomes the block title (inline-parsed).
//! - `[attrlist]` — a line wrapped in single brackets carrying positional
//!   values, named attributes (`name=value`), and shorthand markers on the
//!   first positional (`#id`, `.role`, `%option`).
//!
//! Multiple metadata lines may stack above a single block. They must appear
//! immediately above the block — a blank line in between detaches them.
//! Detachment handling lives in [`crate::parser::block`]; this module only
//! parses individual metadata lines and accumulates them into a [`BlockMeta`].

use crate::ast::{Attributes, BlockMeta};

use super::cursor::Cursor;
use super::inline;
use super::subs::Subs;

/// Consume metadata lines (`.Title`, `[attrlist]`, `[[anchor]]`) until a
/// non-metadata line.
///
/// Stops at the first line that is neither a block title, an attribute
/// list, nor a legacy block anchor. Does not skip blank lines — the
/// caller decides what to do with the (potentially orphaned) result.
pub fn collect_block_meta(cursor: &mut Cursor, attrs: &Attributes) -> BlockMeta {
    let mut meta = BlockMeta::default();
    while let Some(line) = cursor.peek() {
        let text = line.text.as_str();
        if let Some(title) = try_parse_block_title(text, attrs) {
            meta.title = Some(title);
            cursor.advance();
            continue;
        }
        if let Some((id, reftext)) = try_parse_block_anchor(text) {
            meta.id = Some(id);
            if let Some(rt) = reftext {
                meta.named.insert("reftext".into(), rt);
            }
            cursor.advance();
            continue;
        }
        if let Some(parts) = try_parse_attrlist(text) {
            apply_attrlist(&mut meta, parts, attrs);
            cursor.advance();
            continue;
        }
        break;
    }
    meta
}

/// Legacy block anchor: `[[name]]` or `[[name, reftext]]`. Must occupy a
/// whole line (modulo trailing whitespace).
fn try_parse_block_anchor(text: &str) -> Option<(String, Option<String>)> {
    let trimmed = text.trim();
    let inner = trimmed.strip_prefix("[[")?.strip_suffix("]]")?;
    let (id, reftext) = match inner.split_once(',') {
        Some((i, r)) => (i.trim(), Some(r.trim().to_string())),
        None => (inner.trim(), None),
    };
    if id.is_empty() {
        return None;
    }
    // Anchor names are conservatively the same shape as attribute names.
    let mut chars = id.chars();
    let first = chars.next()?;
    if !(first.is_ascii_alphanumeric() || first == '_') {
        return None;
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return None;
    }
    Some((id.to_string(), reftext))
}

/// `.Title` lines: single `.` followed by a non-space, non-`.` character.
///
/// Disambiguated from list markers (`. item`), delimited literal blocks
/// (`....`), and bare paragraphs.
fn try_parse_block_title(text: &str, attrs: &Attributes) -> Option<Vec<crate::ast::Inline>> {
    let bytes = text.as_bytes();
    if bytes.first() != Some(&b'.') {
        return None;
    }
    let next = bytes.get(1).copied()?;
    if next == b'.' || next == b' ' || next == b'\t' {
        return None;
    }
    Some(inline::parse(text[1..].trim_end(), attrs, Subs::NORMAL))
}

/// `[…]` attrlist lines. Returns the comma-split top-level pieces.
///
/// Excludes the legacy block anchor form `[[anchor]]`, which belongs with
/// section ID handling and is parsed elsewhere.
fn try_parse_attrlist(text: &str) -> Option<Vec<String>> {
    let trimmed = text.trim();
    if trimmed.starts_with("[[") {
        return None;
    }
    if !trimmed.starts_with('[') || !trimmed.ends_with(']') {
        return None;
    }
    let inner = &trimmed[1..trimmed.len() - 1];
    Some(split_top_level_commas(inner))
}

/// Comma-split, respecting double-quoted spans and backslash escapes.
fn split_top_level_commas(s: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut in_quote = false;
    let mut escape = false;
    for ch in s.chars() {
        if escape {
            current.push(ch);
            escape = false;
            continue;
        }
        match ch {
            '\\' => escape = true,
            '"' => {
                in_quote = !in_quote;
                current.push(ch);
            }
            ',' if !in_quote => {
                parts.push(std::mem::take(&mut current));
            }
            _ => current.push(ch),
        }
    }
    parts.push(current);
    parts
}

fn apply_attrlist(meta: &mut BlockMeta, parts: Vec<String>, attrs: &Attributes) {
    let mut shorthand_consumed = false;
    for raw in parts {
        let part = raw.trim();
        if part.is_empty() {
            continue;
        }
        if let Some((name, value)) = split_named(part) {
            shorthand_consumed = true;
            apply_named(meta, name, &value, attrs);
            continue;
        }
        if !shorthand_consumed {
            shorthand_consumed = true;
            apply_shorthand(meta, part);
        } else {
            meta.positional.push(unquote(part));
        }
    }
}

/// Split `name=value` only at top level (skipping `=` inside double quotes).
/// Returns `None` for plain positional values.
fn split_named(part: &str) -> Option<(&str, String)> {
    let bytes = part.as_bytes();
    let mut in_quote = false;
    let mut escape = false;
    for (idx, &b) in bytes.iter().enumerate() {
        if escape {
            escape = false;
            continue;
        }
        match b {
            b'\\' => escape = true,
            b'"' => in_quote = !in_quote,
            b'=' if !in_quote => {
                let name = part[..idx].trim();
                if !is_attr_name(name) {
                    return None;
                }
                let value = part[idx + 1..].trim().to_string();
                return Some((name, value));
            }
            _ => {}
        }
    }
    None
}

fn is_attr_name(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphanumeric() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn apply_named(meta: &mut BlockMeta, name: &str, value: &str, attrs: &Attributes) {
    let v = unquote(value);
    match name {
        "id" => meta.id = Some(v),
        "role" => meta.roles.extend(v.split_whitespace().map(str::to_string)),
        "options" | "opts" => meta.options.extend(
            v.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
        ),
        "title" => meta.title = Some(inline::parse(&v, attrs, Subs::NORMAL)),
        _ => {
            meta.named.insert(name.to_string(), v);
        }
    }
}

/// Parse the first positional's shorthand: `style.role#id%opt` (any order
/// after the optional leading style).
fn apply_shorthand(meta: &mut BlockMeta, value: &str) {
    let v = unquote(value);
    let mut tokens: Vec<(Option<char>, String)> = Vec::new();
    let mut current_kind: Option<char> = None;
    let mut current = String::new();
    for ch in v.chars() {
        if matches!(ch, '#' | '.' | '%') {
            tokens.push((current_kind, std::mem::take(&mut current)));
            current_kind = Some(ch);
        } else {
            current.push(ch);
        }
    }
    tokens.push((current_kind, current));

    for (kind, val) in tokens {
        if val.is_empty() {
            continue;
        }
        match kind {
            None => meta.style = Some(val),
            Some('#') => meta.id = Some(val),
            Some('.') => meta.roles.push(val),
            Some('%') => meta.options.push(val),
            _ => unreachable!(),
        }
    }
}

fn unquote(s: &str) -> String {
    let s = s.trim();
    if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
        s[1..s.len() - 1].to_string()
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn parse(text: &str) -> BlockMeta {
        let mut meta = BlockMeta::default();
        let parts = try_parse_attrlist(text).expect("attrlist");
        apply_attrlist(&mut meta, parts, &BTreeMap::new());
        meta
    }

    #[test]
    fn source_with_language() {
        let m = parse("[source,rust]");
        assert_eq!(m.style.as_deref(), Some("source"));
        assert_eq!(m.positional, vec!["rust".to_string()]);
    }

    #[test]
    fn admonition_style() {
        let m = parse("[NOTE]");
        assert_eq!(m.style.as_deref(), Some("NOTE"));
        assert!(m.positional.is_empty());
    }

    #[test]
    fn shorthand_id_role_option() {
        let m = parse("[#myid.warning%hardbreaks]");
        assert!(m.style.is_none());
        assert_eq!(m.id.as_deref(), Some("myid"));
        assert_eq!(m.roles, vec!["warning".to_string()]);
        assert_eq!(m.options, vec!["hardbreaks".to_string()]);
    }

    #[test]
    fn shorthand_with_style_and_positional() {
        let m = parse("[source.lang#myid%hardbreaks,rust]");
        assert_eq!(m.style.as_deref(), Some("source"));
        assert_eq!(m.id.as_deref(), Some("myid"));
        assert_eq!(m.roles, vec!["lang".to_string()]);
        assert_eq!(m.options, vec!["hardbreaks".to_string()]);
        assert_eq!(m.positional, vec!["rust".to_string()]);
    }

    #[test]
    fn named_attributes() {
        let m = parse("[caption=\"Listing 1.\",subs=\"-attributes\"]");
        assert_eq!(
            m.named.get("caption").map(String::as_str),
            Some("Listing 1.")
        );
        assert_eq!(m.named.get("subs").map(String::as_str), Some("-attributes"));
    }

    #[test]
    fn named_id_role_options_fold_into_fields() {
        let m = parse("[id=\"foo\",role=\"warn info\",options=\"hardbreaks,nofollow\"]");
        assert_eq!(m.id.as_deref(), Some("foo"));
        assert_eq!(m.roles, vec!["warn".to_string(), "info".to_string()]);
        assert_eq!(
            m.options,
            vec!["hardbreaks".to_string(), "nofollow".to_string()]
        );
        assert!(m.named.is_empty());
    }

    #[test]
    fn quoted_value_with_comma() {
        let m = parse("[source,rust,linenums=\"1,2,3\"]");
        assert_eq!(m.style.as_deref(), Some("source"));
        assert_eq!(m.positional, vec!["rust".to_string()]);
        assert_eq!(m.named.get("linenums").map(String::as_str), Some("1,2,3"));
    }

    #[test]
    fn double_bracket_anchor_rejected() {
        assert!(try_parse_attrlist("[[anchor]]").is_none());
    }

    #[test]
    fn block_title_recognised() {
        let attrs = BTreeMap::new();
        assert!(try_parse_block_title(".My Title", &attrs).is_some());
        assert!(try_parse_block_title(".5 stars", &attrs).is_some());
    }

    #[test]
    fn block_title_rejects_list_marker_and_delimited() {
        let attrs = BTreeMap::new();
        assert!(try_parse_block_title(". item", &attrs).is_none());
        assert!(try_parse_block_title("....", &attrs).is_none());
        assert!(try_parse_block_title(".", &attrs).is_none());
    }
}
