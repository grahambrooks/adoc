//! AsciiDoc parser.
//!
//! Consumes preprocessed lines and produces an [`adoc_core::Document`].
//! Block parser is line-oriented recursive descent; inline parser applies
//! the six-group substitution pipeline per block's declared `subs`.

use adoc_core::{Attributes, Document};
use adoc_preprocessor::PreprocessedLine;

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("parse error: {0}")]
    Message(String),
}

pub fn parse(_lines: &[PreprocessedLine]) -> Result<Document, ParseError> {
    Ok(Document {
        header: None,
        attributes: Attributes::new(),
        blocks: Vec::new(),
    })
}
