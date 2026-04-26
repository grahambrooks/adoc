//! Post-render pass that turns inline `<span class="footnote">…</span>`
//! markers into numbered `<sup>` references and collects the bodies for
//! the end-of-doc footnote section.
//!
//! Lives outside `inlines.rs` because the rewrite has to operate on the
//! already-rendered HTML — the inline renderer doesn't carry document
//! ordering across siblings, so numbering can't happen in-place.

use std::fmt::Write;

/// Walk the rendered body string, replacing each inline footnote span
/// with a numbered `<sup>` reference and collecting the bodies. Returns
/// `(rewritten_body, footnote_bodies)` where the bodies appear in
/// document order.
///
/// Recognises both the anonymous and id'd forms emitted by the inline
/// renderer:
/// * `<span class="footnote">body</span>`
/// * `<span class="footnote" id="...">body</span>`
///
/// Doesn't try to dedupe by id — every span turns into its own
/// numbered reference. v1 limitation.
pub(crate) fn number_footnotes(body: &str) -> (String, Vec<String>) {
    const PREFIX: &str = r#"<span class="footnote""#;
    const CLOSE: &str = "</span>";
    let mut out = String::with_capacity(body.len());
    let mut bodies: Vec<String> = Vec::new();
    let mut i = 0;
    while i < body.len() {
        let Some(rel) = body[i..].find(PREFIX) else {
            out.push_str(&body[i..]);
            break;
        };
        let span_start = i + rel;
        out.push_str(&body[i..span_start]);
        // Find the end of the opening tag — the first `>` after the prefix.
        let after_prefix = &body[span_start + PREFIX.len()..];
        let Some(open_end_rel) = after_prefix.find('>') else {
            out.push_str(&body[span_start..]);
            break;
        };
        let body_start = span_start + PREFIX.len() + open_end_rel + 1;
        let Some(close_rel) = body[body_start..].find(CLOSE) else {
            out.push_str(&body[span_start..]);
            break;
        };
        let inner = &body[body_start..body_start + close_rel];
        let n = bodies.len() + 1;
        bodies.push(inner.to_string());
        let _ = write!(
            out,
            r##"<sup class="footnote" id="_footnoteref_{n}">[<a href="#_footnotedef_{n}">{n}</a>]</sup>"##
        );
        i = body_start + close_rel + CLOSE.len();
    }
    (out, bodies)
}
