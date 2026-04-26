//! AsciiDoc parser.
//!
//! Consumes preprocessed lines and produces an [`crate::ast::Document`].
//! The block parser is hand-written recursive descent; the inline parser
//! is a single-pass character walker implementing the spec's six substitution
//! groups. Attributes accumulate through the document — header attribute
//! entries become part of the document attribute context used to resolve
//! `{name}` references in subsequent inline content.

mod block;
mod cursor;
mod header;
mod idgen;
mod inline;
mod meta;
mod subs;

use crate::ast::{Attributes, Document};
use crate::preprocessor::PreprocessedLine;

pub use subs::Subs;

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("parse error: {0}")]
    Message(String),
    /// Span-carrying parse error — call sites that know the offending
    /// source location build a [`crate::diag::Diagnostic`] and wrap it
    /// here. Rendered by the CLI through miette so users see file:line:
    /// col + a snippet, not just the message string.
    #[error("{}", .0.message)]
    Diagnostic(Box<crate::diag::Diagnostic>),
}

impl ParseError {
    /// Build a span-carrying error from a [`crate::diag::Diagnostic`].
    pub fn diagnostic(d: crate::diag::Diagnostic) -> Self {
        Self::Diagnostic(Box::new(d))
    }

    /// If this error carries a [`Diagnostic`] payload, return it. Used
    /// by the CLI to render with miette's graphical handler instead of
    /// a plain message.
    pub fn as_diagnostic(&self) -> Option<&crate::diag::Diagnostic> {
        match self {
            Self::Diagnostic(d) => Some(d),
            _ => None,
        }
    }
}

pub fn parse(lines: &[PreprocessedLine]) -> Result<Document, ParseError> {
    parse_with(lines, Attributes::new())
}

/// Parse with a seeded attribute set (typically CLI-provided attributes).
/// Document-level entries currently override seeded values — if we later
/// need CLI-wins semantics, track provenance here.
pub fn parse_with(lines: &[PreprocessedLine], initial: Attributes) -> Result<Document, ParseError> {
    let mut cursor = cursor::Cursor::new(lines);
    let mut attributes = initial;
    let header = header::try_parse_header(&mut cursor, &mut attributes);
    let mut blocks = block::parse_block_sequence(&mut cursor, &mut attributes, 0);
    idgen::assign_ids(&mut blocks);
    let mut doc = Document {
        header,
        attributes,
        blocks,
    };
    // Final post-pass: rewrite `<<Title Text>>` xrefs whose target
    // string matches a section title to use that section's id. Runs
    // after id-generation so derived ids are in place.
    crate::ast::resolve_title_xrefs(&mut doc);
    Ok(doc)
}
