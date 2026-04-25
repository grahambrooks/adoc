//! AsciiDoc preprocessor.
//!
//! v1 scope: split source into lines with [`Location`] spans. Directive
//! handling (`include::`, `ifdef`, attribute entries) is deferred to the
//! preprocessor phase. The parser handles attribute entries itself during
//! header parsing, so this module remains a faithful line-splitter.

use crate::ast::{Attributes, Location, SourceId};

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

#[derive(Default)]
pub struct Preprocessor {
    pub attributes: Attributes,
}

impl Preprocessor {
    pub fn new(attributes: Attributes) -> Self {
        Self { attributes }
    }

    pub fn run(
        &self,
        source: &str,
        source_id: SourceId,
    ) -> Result<Vec<PreprocessedLine>, PreprocessError> {
        let mut lines = Vec::new();
        let mut byte_start = 0u32;
        for (idx, raw) in source.split('\n').enumerate() {
            // Strip a trailing \r so CRLF sources produce clean text.
            let text = raw.strip_suffix('\r').unwrap_or(raw);
            let byte_end = byte_start + text.len() as u32;
            lines.push(PreprocessedLine {
                text: text.to_string(),
                location: Location {
                    source: source_id,
                    byte_start,
                    byte_end,
                    line: (idx + 1) as u32,
                    column: 1,
                },
            });
            // +1 for the \n consumed by split (except after final empty tail).
            byte_start = byte_end + raw.len().saturating_sub(text.len()) as u32 + 1;
        }
        // `split('\n')` yields a trailing empty string when source ends in \n.
        // Keep it — the parser treats trailing blank lines uniformly.
        Ok(lines)
    }
}
