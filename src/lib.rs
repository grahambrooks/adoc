//! `adoc` — a Rust AsciiDoc processor.
//!
//! The crate is organised as four modules that together form the pipeline:
//!
//! - [`ast`] — shared AST types ([`ast::Document`], [`ast::Block`], [`ast::Inline`]),
//!   [`ast::Location`], and the [`ast::Converter`] trait. No I/O.
//! - [`preprocessor`] — line-level handling (will own includes, conditionals,
//!   attribute entries; currently a faithful line splitter).
//! - [`parser`] — hand-written recursive-descent block parser plus the inline
//!   substitution pipeline. Consumes preprocessed lines, produces an
//!   [`ast::Document`].
//! - [`convert`] — output backends. [`convert::html5`] is the only one today.
//!
//! The serialized form of the AST is a public interface — it's the contract
//! for the future stdio-based extension model.

pub mod ast;
pub mod convert;
pub mod parser;
pub mod preprocessor;
