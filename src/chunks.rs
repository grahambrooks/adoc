//! Block-level chunk manifest for retrieval pipelines.
//!
//! Each entry is one top-level (non-section) block paired with the
//! containing-section path, plain text, and a SHA-256 content hash so
//! consumers can detect re-runs that only changed a subset of blocks.
//!
//! Output shape (one JSON array, pretty-printed):
//!
//! ```json
//! [
//!   {
//!     "section_path": ["_methods"],
//!     "section_title": "Methods",
//!     "block_index": 2,
//!     "kind": "paragraph",
//!     "text": "We compared three approaches...",
//!     "hash": "sha256:abcdef..."
//!   }
//! ]
//! ```
//!
//! * `section_path` — chain of section ids from the document root to the
//!   block's containing section. Empty for preamble blocks.
//! * `section_title` — plain-text title of the immediate containing
//!   section, for display alongside the chunk.
//! * `block_index` — block's position among its immediate siblings, so
//!   ordering is recoverable.
//! * `text` — inline plain-text form for embedding (paragraphs:
//!   `inlines_to_plain`; raw blocks: the verbatim text; tables: cells
//!   joined with ` | `; lists: items joined with `\n`).
//! * `hash` — SHA-256 over the serialized AST node. Stable across runs
//!   that produce the same AST shape; changes whenever the parser
//!   produces different output for the source.

use crate::ast::{inlines_to_plain, Block, DelimitedContent, Document};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chunk {
    pub section_path: Vec<String>,
    pub section_title: String,
    pub block_index: usize,
    pub kind: String,
    pub text: String,
    pub hash: String,
}

/// Walk `doc` and produce a flat list of `Chunk`s. Sections are descended
/// into but don't themselves appear as chunks — only their leaf blocks do.
pub fn collect(doc: &Document) -> Vec<Chunk> {
    let mut out = Vec::new();
    walk(&doc.blocks, &[], "", &mut out);
    out
}

/// Convenience wrapper: collect chunks and pretty-print them as JSON.
pub fn to_json_pretty(doc: &Document) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(&collect(doc))
}

fn walk(blocks: &[Block], section_path: &[String], section_title: &str, out: &mut Vec<Chunk>) {
    for (idx, block) in blocks.iter().enumerate() {
        match block {
            Block::Section(s) => {
                let id = s
                    .meta
                    .id
                    .clone()
                    .unwrap_or_else(|| format!("section-{idx}"));
                let title = inlines_to_plain(&s.title);
                let mut nested = section_path.to_vec();
                nested.push(id);
                walk(&s.blocks, &nested, &title, out);
            }
            _ => {
                let text = block_plain_text(block);
                if text.trim().is_empty() {
                    continue;
                }
                let hash = hash_block(block);
                out.push(Chunk {
                    section_path: section_path.to_vec(),
                    section_title: section_title.to_string(),
                    block_index: idx,
                    kind: block_kind(block).to_string(),
                    text,
                    hash,
                });
            }
        }
    }
}

fn block_kind(b: &Block) -> &'static str {
    match b {
        Block::Section(_) => "section",
        Block::Paragraph(_) => "paragraph",
        Block::List(_) => "list",
        Block::DescriptionList(_) => "description_list",
        Block::Delimited(_) => "delimited",
        Block::Table(_) => "table",
        Block::Colist(_) => "colist",
        Block::DiscreteHeading(_) => "discrete_heading",
    }
}

fn block_plain_text(b: &Block) -> String {
    match b {
        Block::Paragraph(p) => inlines_to_plain(&p.inlines),
        Block::List(l) => l
            .items
            .iter()
            .map(|i| inlines_to_plain(&i.principal))
            .collect::<Vec<_>>()
            .join("\n"),
        Block::DescriptionList(d) => d
            .items
            .iter()
            .map(|i| {
                let term = inlines_to_plain(&i.term);
                let body = i
                    .description
                    .iter()
                    .map(block_plain_text)
                    .collect::<Vec<_>>()
                    .join("\n");
                if body.is_empty() {
                    term
                } else {
                    format!("{term}: {body}")
                }
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Block::Delimited(d) => match &d.content {
            DelimitedContent::Raw { text } => text.clone(),
            DelimitedContent::Blocks { blocks } => blocks
                .iter()
                .map(block_plain_text)
                .collect::<Vec<_>>()
                .join("\n"),
        },
        Block::Table(t) => t
            .rows
            .iter()
            .map(|row| {
                row.cells
                    .iter()
                    .map(|c| {
                        if c.blocks.is_empty() {
                            inlines_to_plain(&c.inlines)
                        } else {
                            c.blocks
                                .iter()
                                .map(block_plain_text)
                                .collect::<Vec<_>>()
                                .join(" ")
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" | ")
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Block::Colist(c) => c
            .items
            .iter()
            .map(|item| format!("({}) {}", item.number, inlines_to_plain(&item.inlines)))
            .collect::<Vec<_>>()
            .join("\n"),
        Block::DiscreteHeading(d) => inlines_to_plain(&d.title),
        Block::Section(s) => inlines_to_plain(&s.title),
    }
}

fn hash_block(b: &Block) -> String {
    // Hash the serialized JSON form. The hash is stable across runs
    // as long as the AST node is structurally equal, and changes
    // whenever the parser produces a different shape for the same
    // source — exactly the property a re-embedding pipeline wants.
    let json = serde_json::to_vec(b).expect("AST blocks always serialize");
    let mut hasher = Sha256::new();
    hasher.update(&json);
    let digest = hasher.finalize();
    let mut out = String::with_capacity(7 + digest.len() * 2);
    out.push_str("sha256:");
    for &byte in digest.as_slice() {
        out.push(hex_nibble(byte >> 4));
        out.push(hex_nibble(byte & 0x0f));
    }
    out
}

fn hex_nibble(n: u8) -> char {
    match n {
        0..=9 => (b'0' + n) as char,
        10..=15 => (b'a' + n - 10) as char,
        _ => unreachable!(),
    }
}
