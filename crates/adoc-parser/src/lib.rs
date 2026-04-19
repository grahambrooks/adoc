//! AsciiDoc parser.
//!
//! Consumes preprocessed lines and produces an [`adoc_core::Document`].
//! The block parser is hand-written recursive descent; the inline parser
//! is a single-pass character walker implementing the spec's six substitution
//! groups. Attributes accumulate through the document — header attribute
//! entries become part of the document attribute context used to resolve
//! `{name}` references in subsequent inline content.

mod block;
mod cursor;
mod header;
mod inline;
mod subs;

use adoc_core::{Attributes, Document};
use adoc_preprocessor::PreprocessedLine;

pub use subs::Subs;

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("parse error: {0}")]
    Message(String),
}

pub fn parse(lines: &[PreprocessedLine]) -> Result<Document, ParseError> {
    parse_with(lines, Attributes::new())
}

/// Parse with a seeded attribute set (typically CLI-provided attributes).
/// Document-level entries currently override seeded values — if we later
/// need CLI-wins semantics, track provenance here.
pub fn parse_with(
    lines: &[PreprocessedLine],
    initial: Attributes,
) -> Result<Document, ParseError> {
    let mut cursor = cursor::Cursor::new(lines);
    let mut attributes = initial;
    let header = header::try_parse_header(&mut cursor, &mut attributes);
    let blocks = block::parse_block_sequence(&mut cursor, &mut attributes, 0);
    Ok(Document {
        header,
        attributes,
        blocks,
    })
}
