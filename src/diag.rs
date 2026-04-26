//! Diagnostics — span-pointing warnings produced during the convert
//! pass (and, eventually, parse and preprocess errors).
//!
//! The model:
//!
//! 1. The pipeline collects [`Diagnostic`] records into a [`Diagnostics`]
//!    bag. Each diagnostic carries a [`crate::ast::Location`] and a
//!    severity, plus a code + help text.
//! 2. The CLI later resolves each diagnostic against the
//!    [`crate::ast::SourceMap`] from the preprocessor and turns it into
//!    a [`miette::Report`] that the user sees with file:line:col,
//!    surrounding source, and an underlined span.
//!
//! Why a custom struct rather than `miette::Diagnostic` directly: the
//! AST and converter run without owning the source bytes — they only
//! have a [`SourceId`]. Carrying the raw `String` through every
//! converter call would be invasive and would also drift the AST's
//! serializable form. Building the `miette::Report` at the CLI edge
//! keeps the diagnostic types small and serde-clean.
//!
//! [`SourceId`]: crate::ast::SourceId

use crate::ast::{Location, SourceId, SourceMap};
use miette::{LabeledSpan, MietteDiagnostic, NamedSource, Report, Severity};

/// One diagnostic record. The `code` is a stable identifier (`adoc::
/// xref::dangling`) that surfaces in the rendered output and lets a
/// user look up an explanation. `severity` controls how miette renders
/// it. `help` is an optional one-liner that appears under the snippet.
#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub code: &'static str,
    pub message: String,
    pub help: Option<String>,
    pub severity: Severity,
    /// Where in the document the diagnostic applies. The label text is
    /// shown under the underlined span.
    pub location: Location,
    pub label: String,
}

impl Diagnostic {
    pub fn warning(code: &'static str, message: impl Into<String>, location: Location) -> Self {
        Self {
            code,
            message: message.into(),
            help: None,
            severity: Severity::Warning,
            location,
            label: String::new(),
        }
    }

    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }

    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    pub fn source_id(&self) -> SourceId {
        self.location.source
    }

    /// Convert this diagnostic into a [`miette::Report`] suitable for
    /// printing through `miette`'s default handler. Looks up the source
    /// snippet in `sources`; if the source is missing or the location
    /// is synthetic, returns a span-less report (still readable, just
    /// without the source excerpt).
    pub fn into_report(self, sources: &SourceMap) -> Report {
        let labels = self.span_labels();
        let mut diag = MietteDiagnostic::new(self.message)
            .with_code(self.code)
            .with_severity(self.severity)
            .with_labels(labels);
        if let Some(help) = self.help {
            diag = diag.with_help(help);
        }
        match sources.get(self.location.source) {
            Some(file) if !file.content.is_empty() => {
                let named = NamedSource::new(file.path.clone(), file.content.clone());
                Report::new(diag).with_source_code(named)
            }
            _ => Report::new(diag),
        }
    }

    fn span_labels(&self) -> Vec<LabeledSpan> {
        let (offset, len) = self.location.span();
        if len == 0 && offset == 0 {
            // Synthetic / no-location — let miette render without a
            // span so the message is still useful.
            return Vec::new();
        }
        let label = if self.label.is_empty() {
            None
        } else {
            Some(self.label.clone())
        };
        vec![LabeledSpan::new(label, offset, len.max(1))]
    }
}

/// Append-only collector. The pipeline pushes diagnostics as they're
/// produced; the CLI drains and renders them at the end.
#[derive(Debug, Default, Clone)]
pub struct Diagnostics {
    items: Vec<Diagnostic>,
}

impl Diagnostics {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, diag: Diagnostic) {
        self.items.push(diag);
    }

    pub fn extend(&mut self, other: Diagnostics) {
        self.items.extend(other.items);
    }

    pub fn iter(&self) -> std::slice::Iter<'_, Diagnostic> {
        self.items.iter()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn into_vec(self) -> Vec<Diagnostic> {
        self.items
    }
}

impl IntoIterator for Diagnostics {
    type Item = Diagnostic;
    type IntoIter = std::vec::IntoIter<Diagnostic>;
    fn into_iter(self) -> Self::IntoIter {
        self.items.into_iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::SourceMap;

    fn loc(src: u32, start: u32, end: u32) -> Location {
        Location {
            source: SourceId(src),
            byte_start: start,
            byte_end: end,
            line: 1,
            column: start + 1,
        }
    }

    #[test]
    fn diagnostic_renders_with_named_source() {
        let mut map = SourceMap::new();
        map.push("test.adoc".into(), "hello, world\n".into());
        let diag = Diagnostic::warning("adoc::test", "sample", loc(0, 7, 12))
            .with_label("the world part")
            .with_help("just a test");
        let report = diag.into_report(&map);
        let rendered = format!("{report:?}");
        assert!(rendered.contains("sample"));
    }

    #[test]
    fn synthetic_location_renders_without_source_excerpt() {
        let map = SourceMap::new();
        let diag = Diagnostic::warning("adoc::test", "no source", Location::synthetic());
        // Just shouldn't panic — `into_report` falls back to the
        // span-less form when there's no SourceCode to attach.
        let _ = diag.into_report(&map);
    }

    #[test]
    fn diagnostics_collector_orders_pushes() {
        let mut diags = Diagnostics::new();
        diags.push(Diagnostic::warning(
            "adoc::a",
            "first",
            Location::synthetic(),
        ));
        diags.push(Diagnostic::warning(
            "adoc::b",
            "second",
            Location::synthetic(),
        ));
        let v = diags.into_vec();
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].code, "adoc::a");
        assert_eq!(v[1].code, "adoc::b");
    }
}
