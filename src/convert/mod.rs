//! Output backends.
//!
//! HTML5 is currently the only backend; DocBook and man page are planned.
//! Each backend implements [`crate::ast::Converter`] and depends only on
//! [`crate::ast`] — never on [`crate::parser`].

pub mod html5;
