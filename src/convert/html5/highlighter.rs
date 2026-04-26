//! `:source-highlighter:` integration. Loads a light + dark theme pair
//! from a CDN and emits a small surface-override `<style>` so the
//! document's `--adoc-code-bg` / `--adoc-code-fg` tokens win over the
//! highlighter's hardcoded colors. Keeps code rendering in sync with
//! the document's `prefers-color-scheme`.

use std::fmt::Write;

use crate::ast::{AttributeValue, Document};

use super::escape::escape_attr;

/// Pick a recognised `:source-highlighter:` value, normalised to lowercase.
/// Returns `None` for unset, empty, or unsupported (e.g. `rouge`, `pygments`)
/// values — those need toolchains we don't bundle.
fn source_highlighter(doc: &Document) -> Option<&'static str> {
    let raw = doc
        .attributes
        .get("source-highlighter")
        .and_then(AttributeValue::as_str)?;
    match raw.to_ascii_lowercase().as_str() {
        "prism" | "prismjs" => Some("prism"),
        "highlightjs" | "highlight.js" => Some("highlightjs"),
        _ => None,
    }
}

fn highlighter_attr<'a>(doc: &'a Document, name: &str, default: &'a str) -> &'a str {
    doc.attributes
        .get(name)
        .and_then(AttributeValue::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or(default)
}

/// Read an attribute, falling back to `default`. Returns `None` if the
/// attribute is set to an explicit falsy value (`AttributeValue::Bool(false)`),
/// so callers can suppress an output by setting `:name!:`.
fn optional_attr<'a>(doc: &'a Document, name: &str, default: &'a str) -> Option<&'a str> {
    match doc.attributes.get(name) {
        Some(AttributeValue::Bool(false)) => None,
        Some(AttributeValue::String(s)) if s.is_empty() => Some(default),
        Some(AttributeValue::String(s)) => Some(s.as_str()),
        Some(AttributeValue::Bool(true)) | None => Some(default),
    }
}

pub(crate) fn render_highlighter_head(out: &mut String, doc: &Document) {
    match source_highlighter(doc) {
        Some("prism") => {
            // Light + dark theme pair, each gated by prefers-color-scheme so
            // the highlighter follows the document's color scheme. Defaults:
            // `prism` (light) and `prism-tomorrow` (dark). Either can be
            // overridden with `:prism-theme:` / `:prism-dark-theme:`. Set
            // `:prism-dark-theme!:` (falsy) to suppress the dark variant
            // entirely if you want a single fixed theme.
            let light = highlighter_attr(doc, "prism-theme", "prism");
            let _ = write!(
                out,
                "<link rel=\"stylesheet\" media=\"(prefers-color-scheme: light)\" href=\"https://cdn.jsdelivr.net/npm/prismjs@1/themes/{}.min.css\">\n",
                escape_attr(light)
            );
            if let Some(dark) = optional_attr(doc, "prism-dark-theme", "prism-tomorrow") {
                let _ = write!(
                    out,
                    "<link rel=\"stylesheet\" media=\"(prefers-color-scheme: dark)\" href=\"https://cdn.jsdelivr.net/npm/prismjs@1/themes/{}.min.css\">\n",
                    escape_attr(dark)
                );
            }
            // Hand off the surface (background, padding, border-radius)
            // to our own tokens so the document and the highlighter agree.
            out.push_str(PRISM_SURFACE_OVERRIDE);
        }
        Some("highlightjs") => {
            let light = highlighter_attr(doc, "highlightjs-theme", "github");
            let _ = write!(
                out,
                "<link rel=\"stylesheet\" media=\"(prefers-color-scheme: light)\" href=\"https://cdn.jsdelivr.net/npm/highlight.js@11/styles/{}.min.css\">\n",
                escape_attr(light)
            );
            if let Some(dark) = optional_attr(doc, "highlightjs-dark-theme", "github-dark") {
                let _ = write!(
                    out,
                    "<link rel=\"stylesheet\" media=\"(prefers-color-scheme: dark)\" href=\"https://cdn.jsdelivr.net/npm/highlight.js@11/styles/{}.min.css\">\n",
                    escape_attr(dark)
                );
            }
            out.push_str(HLJS_SURFACE_OVERRIDE);
        }
        _ => {}
    }
}

pub(crate) fn render_highlighter_body(out: &mut String, doc: &Document) {
    match source_highlighter(doc) {
        Some("prism") => {
            out.push_str(
                "<script src=\"https://cdn.jsdelivr.net/npm/prismjs@1/prism.min.js\"></script>\n",
            );
            out.push_str(
                "<script src=\"https://cdn.jsdelivr.net/npm/prismjs@1/plugins/autoloader/prism-autoloader.min.js\"></script>\n",
            );
        }
        Some("highlightjs") => {
            out.push_str(
                "<script src=\"https://cdn.jsdelivr.net/npm/highlight.js@11/lib/common.min.js\"></script>\n",
            );
            out.push_str("<script>hljs.highlightAll();</script>\n");
        }
        _ => {}
    }
}

/// Inline override that lets our `--adoc-code-bg` / `--adoc-code-fg`
/// tokens show through Prism's default theme rules and keeps the
/// document's mono stack + size in charge of code rendering. Without
/// this, Prism paints its own hardcoded surface (Consolas/Monaco at a
/// shrunken size) and the document's light/dark mode stops applying.
const PRISM_SURFACE_OVERRIDE: &str = r#"<style>
pre[class*="language-"], code[class*="language-"] {
  background: var(--adoc-code-bg) !important;
  color: var(--adoc-code-fg) !important;
  text-shadow: none !important;
  font-family: var(--adoc-font-mono) !important;
  font-size: 1em !important;
}
pre[class*="language-"] *, code[class*="language-"] * {
  /* Prism's per-token spans inherit the typeface but some themes
     still set `font-family` on `.token` — re-assert. */
  font-family: inherit !important;
}
:not(pre) > code[class*="language-"] {
  padding: 0.15em 0.4em;
  border-radius: var(--adoc-radius-sm);
}
</style>
"#;

const HLJS_SURFACE_OVERRIDE: &str = r#"<style>
.hljs {
  background: var(--adoc-code-bg) !important;
  color: var(--adoc-code-fg) !important;
  font-family: var(--adoc-font-mono) !important;
  font-size: 1em !important;
}
.hljs * {
  font-family: inherit !important;
}
</style>
"#;
