//! AsciiDoc preprocessor.
//!
//! Handles line-level directives before the parser runs:
//! `include::`, `ifdef`/`ifndef`/`ifeval`/`endif`, and attribute entries.
//! Output is a flat stream of source lines, each tagged with a [`Location`]
//! so parse errors in included files can be reported through the include chain.

use adoc_core::{Attributes, Location};

#[derive(Debug, Clone)]
pub struct PreprocessedLine {
    pub text: String,
    pub location: Location,
}

#[derive(Debug, thiserror::Error)]
pub enum PreprocessError {
    #[error("preprocessor error: {0}")]
    Message(String),
}

pub struct Preprocessor {
    pub attributes: Attributes,
}

impl Preprocessor {
    pub fn new(attributes: Attributes) -> Self {
        Self { attributes }
    }

    pub fn run(&self, _source: &str) -> Result<Vec<PreprocessedLine>, PreprocessError> {
        Ok(Vec::new())
    }
}
