//! Document header parser.
//!
//! Recognises the optional title (`= Title`), author line, revision line,
//! and leading attribute entries (`:name: value` / `:!name:`). Author and
//! revision lines only count if a title is present.

use crate::ast::{AttributeValue, Attributes, Author, Header, Revision};

use super::cursor::Cursor;
use super::inline;
use super::subs::Subs;

pub fn try_parse_header(cursor: &mut Cursor, attrs: &mut Attributes) -> Option<Header> {
    cursor.skip_blank_lines();
    let line = cursor.peek()?;
    // A document title is `= ` followed by text (single `=` only).
    if !is_title_line(&line.text) {
        // Still consume any leading attribute entries as document attributes.
        consume_attribute_entries(cursor, attrs);
        return None;
    }
    let title_text = line.text[2..].trim().to_string();
    let location = line.location.clone();
    cursor.advance();

    let authors = if let Some(next) = cursor.peek() {
        if is_author_line(&next.text) {
            let parsed = parse_author_line(&next.text);
            cursor.advance();
            parsed
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };

    let revision = if !authors.is_empty() {
        if let Some(next) = cursor.peek() {
            if is_revision_line(&next.text) {
                let r = parse_revision_line(&next.text);
                cursor.advance();
                Some(r)
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };

    // Attribute entries after the header apply to the document.
    consume_attribute_entries(cursor, attrs);

    let title = inline::parse(&title_text, attrs, Subs::HEADER);
    Some(Header {
        title,
        authors,
        revision,
        location,
    })
}

fn is_title_line(text: &str) -> bool {
    // Exactly one leading `=` (not `==`) followed by space and content.
    let bytes = text.as_bytes();
    bytes.len() >= 3 && bytes[0] == b'=' && bytes[1] == b' ' && !text[2..].trim().is_empty()
}

fn is_author_line(text: &str) -> bool {
    // Non-empty, not a section marker, not an attribute entry.
    let t = text.trim();
    !t.is_empty()
        && !t.starts_with('=')
        && !t.starts_with(':')
        && !t.starts_with('[')
        && !starts_with_marker(t, '*')
        && !starts_with_marker(t, '.')
}

fn starts_with_marker(t: &str, marker: char) -> bool {
    let mut chars = t.chars();
    let first = chars.next();
    if first != Some(marker) {
        return false;
    }
    chars.next().map_or(false, |c| c == ' ' || c == marker)
}

fn parse_author_line(text: &str) -> Vec<Author> {
    text.split(';')
        .map(|part| parse_one_author(part.trim()))
        .filter(|a| !a.name.is_empty())
        .collect()
}

fn parse_one_author(part: &str) -> Author {
    if let Some(lt) = part.find('<') {
        if let Some(gt) = part.rfind('>') {
            if gt > lt {
                let name = part[..lt].trim().to_string();
                let email = part[lt + 1..gt].trim().to_string();
                return Author {
                    name,
                    email: if email.is_empty() { None } else { Some(email) },
                };
            }
        }
    }
    Author {
        name: part.to_string(),
        email: None,
    }
}

fn is_revision_line(text: &str) -> bool {
    let t = text.trim();
    !t.is_empty() && !t.starts_with(':') && !t.starts_with('=')
}

fn parse_revision_line(text: &str) -> Revision {
    // Pattern: "v<number>, <date>: <remark>" — any part optional.
    let t = text.trim();
    let (head, remark) = match t.split_once(':') {
        Some((h, r)) => (h.trim(), Some(r.trim().to_string())),
        None => (t, None),
    };
    let (number, date) = match head.split_once(',') {
        Some((n, d)) => (Some(n.trim().to_string()), Some(d.trim().to_string())),
        None => (Some(head.to_string()), None),
    };
    Revision {
        number,
        date,
        remark,
    }
}

/// Consumes consecutive attribute-entry lines (`:name: value`, `:!name:`).
pub fn consume_attribute_entries(cursor: &mut Cursor, attrs: &mut Attributes) {
    loop {
        cursor.skip_blank_lines();
        let line = match cursor.peek() {
            Some(l) => l,
            None => return,
        };
        match parse_attribute_entry(&line.text) {
            Some((name, value)) => {
                attrs.insert(name, value);
                cursor.advance();
            }
            None => return,
        }
    }
}

pub fn parse_attribute_entry(text: &str) -> Option<(String, AttributeValue)> {
    let t = text.trim_end();
    if !t.starts_with(':') {
        return None;
    }
    let rest = &t[1..];
    let (name_part, value_part) = rest.split_once(':')?;
    let (name, negate) = if let Some(stripped) = name_part.strip_prefix('!') {
        (stripped.to_string(), true)
    } else if let Some(stripped) = name_part.strip_suffix('!') {
        (stripped.to_string(), true)
    } else {
        (name_part.to_string(), false)
    };
    if name.is_empty() || !is_attribute_name(&name) {
        return None;
    }
    let value = value_part.trim();
    if negate {
        Some((name, AttributeValue::Bool(false)))
    } else if value.is_empty() {
        Some((name, AttributeValue::Bool(true)))
    } else {
        Some((name, AttributeValue::String(value.to_string())))
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
