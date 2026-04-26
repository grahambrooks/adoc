//! HTML5 converter.
//!
//! v1 scope: enough HTML to see every block and inline the parser produces.
//! Matching Asciidoctor's exact HTML (class names, wrappers, TOC, etc.) is
//! conformance work — deliberately out of scope here.
//!
//! ## Module layout
//!
//! * [`escape`] — `escape` / `escape_attr` for element content / attribute values.
//! * [`inlines`] — `render_inlines` / `render_inline` for every `Inline` variant.
//! * [`ctx`] — `RenderCtx`, the TOC pre-walk, section-number map, `render_toc`.
//! * [`blocks`] — `render_block` dispatcher plus per-variant block renderers
//!   (sections, paragraphs, lists, delimited, admonitions, callouts,
//!   discrete headings) and shared block-meta helpers.
//! * [`tables`] — `render_table` + colgroup + cell dispatch.
//! * [`highlighter`] — `:source-highlighter:` integration (Prism, highlight.js).
//! * [`footnotes`] — post-render rewrite of inline footnote spans into
//!   numbered `<sup>` refs + the end-of-doc footnote section.
//!
//! This top-level module owns only the public surface: [`Html5Converter`],
//! [`Html5Options`], [`Stylesheet`], the [`Converter`] impl, and the
//! `render_stylesheet` helper.

mod blocks;
mod ctx;
mod escape;
mod footnotes;
mod highlighter;
mod inlines;
mod tables;

use std::fmt::Write;

use crate::ast::{inlines_to_plain, AttributeValue, Block, ConvertError, Converter, Document};
use crate::diag::Diagnostics;

use self::blocks::render_block;
use self::ctx::{render_toc, validate_xrefs, RenderCtx, TocPlacement};
use self::escape::{escape, escape_attr};
use self::footnotes::number_footnotes;
use self::highlighter::{render_highlighter_body, render_highlighter_head};
use self::inlines::render_inlines;

/// The built-in default stylesheet, compiled into the binary.
pub const BUILTIN_CSS: &str = include_str!("assets/adoc.css");

/// Default filename used when linking or copying the built-in stylesheet.
pub const BUILTIN_FILENAME: &str = "adoc.css";

/// How the stylesheet should appear in the generated HTML.
///
/// Mirrors Asciidoctor's attribute-driven model (`stylesheet`, `linkcss`,
/// `stylesdir`). Resolution happens at the CLI boundary; the converter
/// just renders what it's handed.
#[derive(Debug, Clone, Default)]
pub enum Stylesheet {
    /// Inline the built-in stylesheet (default).
    #[default]
    BuiltinEmbed,
    /// Emit `<link>` to the built-in stylesheet at `href`.
    BuiltinLink { href: String },
    /// Inline the supplied CSS content.
    CustomEmbed { css: String },
    /// Emit `<link>` to an arbitrary href.
    CustomLink { href: String },
    /// Emit no stylesheet at all.
    None,
}

#[derive(Debug, Clone, Default)]
pub struct Html5Options {
    pub stylesheet: Stylesheet,
}

#[derive(Debug, Clone, Default)]
pub struct Html5Converter {
    pub options: Html5Options,
}

impl Html5Converter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_options(options: Html5Options) -> Self {
        Self { options }
    }
}

impl Html5Converter {
    /// Render `doc` and return the HTML alongside any diagnostics
    /// (warnings) produced during conversion. The `Converter::convert`
    /// trait method discards the diagnostics for callers that only
    /// want the HTML — use this entry point when you want to render
    /// span-pointing warnings (dangling xrefs, etc.) to the user.
    pub fn convert_with_diagnostics(
        &self,
        doc: &Document,
    ) -> Result<(String, Diagnostics), ConvertError> {
        let ctx = RenderCtx::new(doc);
        let mut diagnostics = Diagnostics::new();
        validate_xrefs(doc, &ctx.ids, &mut diagnostics);
        let html = self.render_html(doc, &ctx)?;
        Ok((html, diagnostics))
    }
}

impl Converter for Html5Converter {
    fn convert(&self, doc: &Document) -> Result<String, ConvertError> {
        let (html, _diagnostics) = self.convert_with_diagnostics(doc)?;
        Ok(html)
    }
}

impl Html5Converter {
    /// The actual rendering. Split out from the public entry points so
    /// `convert` and `convert_with_diagnostics` share one implementation.
    fn render_html(&self, doc: &Document, ctx: &RenderCtx) -> Result<String, ConvertError> {
        let mut out = String::new();
        out.push_str("<!doctype html>\n<html lang=\"en\">\n<head>\n");
        out.push_str(r#"<meta charset="utf-8">"#);
        out.push('\n');
        out.push_str(r#"<meta name="viewport" content="width=device-width, initial-scale=1">"#);
        out.push('\n');
        let title_text = doc
            .header
            .as_ref()
            .map(|h| inlines_to_plain(&h.title))
            .unwrap_or_else(|| "Untitled".to_string());
        writeln!(out, "<title>{}</title>", escape(&title_text))
            .map_err(|e| ConvertError::Message(e.to_string()))?;
        render_stylesheet(&mut out, &self.options.stylesheet);
        render_highlighter_head(&mut out, doc);
        // Surface a non-default doctype as a body class so themes can
        // target `body.doctype-book` / `.doctype-manpage` etc. The
        // `article` default keeps the output unchanged. Level-0 part
        // parsing for `book` is the bigger user-facing feature and is
        // queued.
        let doctype = doc
            .attributes
            .get("doctype")
            .and_then(AttributeValue::as_str)
            .filter(|s| !s.is_empty() && *s != "article");
        match doctype {
            Some(d) => {
                let _ = write!(
                    out,
                    "</head>\n<body class=\"doctype-{}\">\n",
                    escape_attr(d)
                );
            }
            None => out.push_str("</head>\n<body>\n"),
        }

        if let Some(header) = &doc.header {
            out.push_str("<header>\n");
            writeln!(out, "<h1>{}</h1>", render_inlines(&header.title))
                .map_err(|e| ConvertError::Message(e.to_string()))?;
            if !header.authors.is_empty() {
                out.push_str(r#"<p class="authors">"#);
                let names: Vec<String> = header.authors.iter().map(|a| escape(&a.name)).collect();
                out.push_str(&names.join(", "));
                out.push_str("</p>\n");
            }
            if let Some(rev) = &header.revision {
                out.push_str(r#"<p class="revision">"#);
                let parts: Vec<String> = [
                    rev.number.as_deref(),
                    rev.date.as_deref(),
                    rev.remark.as_deref(),
                ]
                .into_iter()
                .flatten()
                .map(escape)
                .collect();
                out.push_str(&parts.join(" · "));
                out.push_str("</p>\n");
            }
            out.push_str("</header>\n");
        }

        // The whole document body lives inside `<main id="content">` so
        // every block has the same enclosing context — the TOC and any
        // pre-section prose share the section content's surface, instead
        // of sitting as bare body children.
        out.push_str(r#"<main id="content">"#);
        out.push('\n');

        // `:toc-placement: auto` (the default) puts the TOC at the top
        // of `<main>`. `preamble` puts it right after the preamble div.
        if ctx.toc && ctx.toc_placement == TocPlacement::Auto {
            render_toc(&mut out, ctx);
        }

        let body_start = out.len();
        // Group leading non-section blocks (the "preamble") into a single
        // `<div id="preamble">` so the renderer matches Asciidoctor's
        // structural convention.
        let first_section_idx = doc
            .blocks
            .iter()
            .position(|b| matches!(b, Block::Section(_)));
        let preamble_end = first_section_idx.unwrap_or(doc.blocks.len());
        if preamble_end > 0 {
            out.push_str(r#"<div id="preamble">"#);
            out.push('\n');
            for block in &doc.blocks[..preamble_end] {
                render_block(&mut out, block, ctx)?;
            }
            out.push_str("</div>\n");
        }
        if ctx.toc && ctx.toc_placement == TocPlacement::Preamble {
            render_toc(&mut out, ctx);
        }
        for block in &doc.blocks[preamble_end..] {
            render_block(&mut out, block, ctx)?;
        }

        // After the body is rendered, walk it and turn each inline
        // `<span class="footnote">…</span>` into a numbered `<sup>` ref,
        // then append a `<div id="footnotes">` section gathering the
        // bodies. Footnotes inside footnotes are rare and not handled.
        let body = out.split_off(body_start);
        let (rewritten, footnotes) = number_footnotes(&body);
        out.push_str(&rewritten);
        if !footnotes.is_empty() {
            out.push_str(r#"<div id="footnotes">"#);
            out.push_str("<hr>\n");
            for (i, body) in footnotes.iter().enumerate() {
                let n = i + 1;
                let _ = writeln!(
                    out,
                    r##"<div class="footnote" id="_footnotedef_{n}"><a href="#_footnoteref_{n}">{n}</a>. {body}</div>"##
                );
            }
            out.push_str("</div>\n");
        }
        out.push_str("</main>\n");

        if !doc.attributes.is_empty() {
            // Emit document attributes as an HTML comment so round-tripping
            // preserves them visibly in the output for debugging.
            out.push_str("<!-- attributes: ");
            for (k, v) in &doc.attributes {
                match v {
                    AttributeValue::String(s) => {
                        let _ = write!(out, "{k}={s}; ");
                    }
                    AttributeValue::Bool(b) => {
                        let _ = write!(out, "{k}={b}; ");
                    }
                }
            }
            out.push_str("-->\n");
        }

        render_highlighter_body(&mut out, doc);
        out.push_str("</body>\n</html>\n");
        Ok(out)
    }
}

fn render_stylesheet(out: &mut String, stylesheet: &Stylesheet) {
    match stylesheet {
        Stylesheet::None => {}
        Stylesheet::BuiltinEmbed => {
            out.push_str("<style>\n");
            out.push_str(BUILTIN_CSS);
            out.push_str("</style>\n");
        }
        Stylesheet::CustomEmbed { css } => {
            out.push_str("<style>\n");
            out.push_str(css);
            if !css.ends_with('\n') {
                out.push('\n');
            }
            out.push_str("</style>\n");
        }
        Stylesheet::BuiltinLink { href } | Stylesheet::CustomLink { href } => {
            let _ = writeln!(
                out,
                "<link rel=\"stylesheet\" href=\"{}\">",
                escape_attr(href)
            );
        }
    }
}
