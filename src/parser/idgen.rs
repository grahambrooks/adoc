//! Section-ID derivation and post-parse ID assignment.
//!
//! After block parsing, every section needs an `id` so cross-references
//! resolve. Authors can supply an ID three ways:
//!
//! - `[#name]` shorthand on a `[attrlist]` metadata line (handled in
//!   [`crate::parser::meta`]).
//! - `[[name]]` legacy block-anchor line (also handled in `meta`).
//! - Implicitly — derive one from the section title here.
//!
//! Auto-derivation: lowercase, replace each run of non-alphanumeric
//! characters with `_`, trim trailing `_`, prepend `_`. On collision with
//! an existing ID, append `_2`, `_3`, … This mirrors Asciidoctor's
//! defaults (`idprefix=_`, `idseparator=_`).

use std::collections::BTreeSet;

use crate::ast::{Block, Inline};

/// Walk `blocks`, collect every explicit ID, then assign auto-generated
/// IDs to sections that don't have one. Auto IDs are de-duped against
/// the explicit set as well as against earlier auto IDs.
pub fn assign_ids(blocks: &mut [Block]) {
    let mut existing = BTreeSet::new();
    collect_explicit_ids(blocks, &mut existing);
    auto_assign(blocks, &mut existing);
}

fn collect_explicit_ids(blocks: &[Block], out: &mut BTreeSet<String>) {
    for b in blocks {
        if let Some(id) = block_id(b) {
            out.insert(id.to_string());
        }
        if let Block::Section(s) = b {
            collect_explicit_ids(&s.blocks, out);
        }
    }
}

fn block_id(block: &Block) -> Option<&str> {
    match block {
        Block::Section(s) => s.meta.id.as_deref(),
        Block::Paragraph(p) => p.meta.id.as_deref(),
        Block::List(l) => l.meta.id.as_deref(),
        Block::DescriptionList(d) => d.meta.id.as_deref(),
        Block::Delimited(d) => d.meta.id.as_deref(),
        Block::Table(t) => t.meta.id.as_deref(),
    }
}

fn auto_assign(blocks: &mut [Block], existing: &mut BTreeSet<String>) {
    for b in blocks {
        if let Block::Section(s) = b {
            if s.meta.id.is_none() {
                let plain = inlines_to_plain(&s.title);
                let id = derive_section_id(&plain, existing);
                existing.insert(id.clone());
                s.meta.id = Some(id);
            }
            auto_assign(&mut s.blocks, existing);
        }
    }
}

/// Derive an ID from `title` (already-plain text), avoiding any name
/// already in `existing`.
pub fn derive_section_id(title: &str, existing: &BTreeSet<String>) -> String {
    let prefix = "_";
    let separator = '_';
    let mut body = String::new();
    let mut prev_sep = true;
    for ch in title.chars() {
        if ch.is_ascii_alphanumeric() {
            body.push(ch.to_ascii_lowercase());
            prev_sep = false;
        } else if !prev_sep {
            body.push(separator);
            prev_sep = true;
        }
    }
    let trimmed = body.trim_end_matches(separator);
    let candidate = if trimmed.is_empty() {
        format!("{prefix}section")
    } else {
        format!("{prefix}{trimmed}")
    };

    if !existing.contains(&candidate) {
        return candidate;
    }
    let mut n: u32 = 2;
    loop {
        let with_suffix = format!("{candidate}_{n}");
        if !existing.contains(&with_suffix) {
            return with_suffix;
        }
        n += 1;
    }
}

/// Render an inline sequence to plain text suitable for ID derivation.
/// Drops formatting markup, preserves text content of links/xrefs/images.
pub fn inlines_to_plain(inlines: &[Inline]) -> String {
    let mut out = String::new();
    for i in inlines {
        write_plain(&mut out, i);
    }
    out
}

fn write_plain(out: &mut String, inline: &Inline) {
    match inline {
        Inline::Text { value } => out.push_str(value),
        Inline::Strong { children }
        | Inline::Emphasis { children }
        | Inline::Monospace { children }
        | Inline::Subscript { children }
        | Inline::Superscript { children }
        | Inline::Highlight { children } => {
            for child in children {
                write_plain(out, child);
            }
        }
        Inline::Link { text, .. } => {
            for child in text {
                write_plain(out, child);
            }
        }
        Inline::Xref {
            target,
            text: Some(t),
        } => {
            if t.is_empty() {
                out.push_str(target);
            } else {
                for child in t {
                    write_plain(out, child);
                }
            }
        }
        Inline::Xref { target, text: None } => out.push_str(target),
        Inline::Image { alt, .. } => out.push_str(alt),
        Inline::Footnote { text, .. } => {
            for child in text {
                write_plain(out, child);
            }
        }
        Inline::AttributeRef { name } => {
            out.push('{');
            out.push_str(name);
            out.push('}');
        }
        Inline::LineBreak => out.push(' '),
        Inline::Passthrough { value } => out.push_str(value),
        Inline::RawHtml { .. } => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(name: &str) -> String {
        name.to_string()
    }

    #[test]
    fn ascii_titles_lowercase_and_separate() {
        let none = BTreeSet::new();
        assert_eq!(derive_section_id("Introduction", &none), "_introduction");
        assert_eq!(derive_section_id("Hello, World!", &none), "_hello_world");
        assert_eq!(derive_section_id("API v2", &none), "_api_v2");
        assert_eq!(
            derive_section_id("--leading-- punctuation", &none),
            "_leading_punctuation"
        );
    }

    #[test]
    fn empty_or_punctuation_only_falls_back_to_section() {
        let none = BTreeSet::new();
        assert_eq!(derive_section_id("", &none), "_section");
        assert_eq!(derive_section_id("?!?", &none), "_section");
    }

    #[test]
    fn collisions_get_numeric_suffixes() {
        let mut existing = BTreeSet::new();
        existing.insert(s("_intro"));
        existing.insert(s("_intro_2"));
        assert_eq!(derive_section_id("Intro", &existing), "_intro_3");
    }

    #[test]
    fn explicit_ids_block_auto_use() {
        // If an explicit `_introduction` is already taken, the next
        // section titled "Introduction" must skip past it.
        let mut existing = BTreeSet::new();
        existing.insert(s("_introduction"));
        assert_eq!(
            derive_section_id("Introduction", &existing),
            "_introduction_2"
        );
    }

    #[test]
    fn inline_to_plain_strips_formatting() {
        let title = vec![
            Inline::Text {
                value: "API ".into(),
            },
            Inline::Strong {
                children: vec![Inline::Text { value: "v2".into() }],
            },
            Inline::Text {
                value: " Reference".into(),
            },
        ];
        assert_eq!(inlines_to_plain(&title), "API v2 Reference");
    }
}
