//! HTML5 converter.
//!
//! v1 scope: enough HTML to see every block and inline the parser produces.
//! Matching Asciidoctor's exact HTML (class names, wrappers, TOC, etc.) is
//! conformance work — deliberately out of scope here.

use std::fmt::Write;

use crate::ast::{
    AttributeValue, Block, ConvertError, Converter, DelimitedContent, DelimitedStyle,
    DescriptionList, Document, Inline, List, ListMarker, Paragraph, Section, Table,
};

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

impl Converter for Html5Converter {
    fn convert(&self, doc: &Document) -> Result<String, ConvertError> {
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
        write!(out, "<title>{}</title>\n", escape(&title_text))
            .map_err(|e| ConvertError::Message(e.to_string()))?;
        render_stylesheet(&mut out, &self.options.stylesheet);
        out.push_str("</head>\n<body>\n");

        if let Some(header) = &doc.header {
            out.push_str("<header>\n");
            write!(out, "<h1>{}</h1>\n", render_inlines(&header.title))
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

        for block in &doc.blocks {
            render_block(&mut out, block)?;
        }

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
            let _ = write!(
                out,
                "<link rel=\"stylesheet\" href=\"{}\">\n",
                escape_attr(href)
            );
        }
    }
}

fn render_block(out: &mut String, block: &Block) -> Result<(), ConvertError> {
    match block {
        Block::Section(s) => render_section(out, s),
        Block::Paragraph(p) => render_paragraph(out, p),
        Block::List(l) => render_list(out, l),
        Block::DescriptionList(d) => render_description_list(out, d),
        Block::Delimited(d) => render_delimited(out, d),
        Block::Table(t) => render_table(out, t),
    }
}

fn render_section(out: &mut String, s: &Section) -> Result<(), ConvertError> {
    let tag = match s.level {
        1 => "h2",
        2 => "h3",
        3 => "h4",
        4 => "h5",
        _ => "h6",
    };
    write!(
        out,
        "<section>\n<{tag}>{}</{tag}>\n",
        render_inlines(&s.title)
    )
    .map_err(|e| ConvertError::Message(e.to_string()))?;
    for b in &s.blocks {
        render_block(out, b)?;
    }
    out.push_str("</section>\n");
    Ok(())
}

fn render_paragraph(out: &mut String, p: &Paragraph) -> Result<(), ConvertError> {
    write!(out, "<p>{}</p>\n", render_inlines(&p.inlines))
        .map_err(|e| ConvertError::Message(e.to_string()))
}

fn render_list(out: &mut String, l: &List) -> Result<(), ConvertError> {
    // Re-nest by depth.
    let tag = match l.marker {
        ListMarker::Unordered => "ul",
        ListMarker::Ordered => "ol",
    };
    let mut current_depth = 0u8;
    let mut open = 0u32;
    for item in &l.items {
        while current_depth < item.depth {
            out.push_str(&format!("<{tag}>\n"));
            current_depth += 1;
            open += 1;
        }
        while current_depth > item.depth {
            out.push_str(&format!("</{tag}>\n"));
            current_depth -= 1;
            open = open.saturating_sub(1);
        }
        write!(out, "<li>{}", render_inlines(&item.principal))
            .map_err(|e| ConvertError::Message(e.to_string()))?;
        for b in &item.blocks {
            out.push('\n');
            render_block(out, b)?;
        }
        out.push_str("</li>\n");
    }
    for _ in 0..open {
        out.push_str(&format!("</{tag}>\n"));
    }
    Ok(())
}

fn render_description_list(out: &mut String, d: &DescriptionList) -> Result<(), ConvertError> {
    out.push_str("<dl>\n");
    for item in &d.items {
        write!(out, "<dt>{}</dt>\n", render_inlines(&item.term))
            .map_err(|e| ConvertError::Message(e.to_string()))?;
        out.push_str("<dd>");
        for b in &item.description {
            render_block(out, b)?;
        }
        out.push_str("</dd>\n");
    }
    out.push_str("</dl>\n");
    Ok(())
}

fn render_delimited(out: &mut String, d: &crate::ast::DelimitedBlock) -> Result<(), ConvertError> {
    match (&d.style, &d.content) {
        (DelimitedStyle::Listing, DelimitedContent::Raw { text }) => {
            write!(out, "<pre><code>{}</code></pre>\n", escape(text))
                .map_err(|e| ConvertError::Message(e.to_string()))
        }
        (DelimitedStyle::Literal, DelimitedContent::Raw { text }) => {
            write!(out, "<pre>{}</pre>\n", escape(text))
                .map_err(|e| ConvertError::Message(e.to_string()))
        }
        (DelimitedStyle::Passthrough, DelimitedContent::Raw { text }) => {
            out.push_str(text);
            out.push('\n');
            Ok(())
        }
        (DelimitedStyle::Example, DelimitedContent::Blocks { blocks }) => {
            out.push_str(r#"<div class="example">"#);
            out.push('\n');
            for b in blocks {
                render_block(out, b)?;
            }
            out.push_str("</div>\n");
            Ok(())
        }
        (DelimitedStyle::Quote, DelimitedContent::Blocks { blocks }) => {
            out.push_str("<blockquote>\n");
            for b in blocks {
                render_block(out, b)?;
            }
            out.push_str("</blockquote>\n");
            Ok(())
        }
        (DelimitedStyle::Sidebar, DelimitedContent::Blocks { blocks }) => {
            out.push_str(r#"<aside>"#);
            out.push('\n');
            for b in blocks {
                render_block(out, b)?;
            }
            out.push_str("</aside>\n");
            Ok(())
        }
        (DelimitedStyle::Open, DelimitedContent::Blocks { blocks }) => {
            out.push_str("<div>\n");
            for b in blocks {
                render_block(out, b)?;
            }
            out.push_str("</div>\n");
            Ok(())
        }
        _ => Err(ConvertError::Message(
            "delimited block style/content mismatch".into(),
        )),
    }
}

fn render_table(out: &mut String, t: &Table) -> Result<(), ConvertError> {
    out.push_str("<table>\n");
    for row in &t.rows {
        out.push_str("<tr>");
        for cell in &row.cells {
            write!(out, "<td>{}</td>", render_inlines(&cell.inlines))
                .map_err(|e| ConvertError::Message(e.to_string()))?;
        }
        out.push_str("</tr>\n");
    }
    out.push_str("</table>\n");
    Ok(())
}

// --- inline rendering ------------------------------------------------------

fn render_inlines(inlines: &[Inline]) -> String {
    let mut out = String::new();
    for i in inlines {
        render_inline(&mut out, i);
    }
    out
}

fn render_inline(out: &mut String, i: &Inline) {
    match i {
        Inline::Text(s) => out.push_str(&escape(s)),
        Inline::Strong(children) => {
            out.push_str("<strong>");
            for c in children {
                render_inline(out, c);
            }
            out.push_str("</strong>");
        }
        Inline::Emphasis(children) => {
            out.push_str("<em>");
            for c in children {
                render_inline(out, c);
            }
            out.push_str("</em>");
        }
        Inline::Monospace(children) => {
            out.push_str("<code>");
            for c in children {
                render_inline(out, c);
            }
            out.push_str("</code>");
        }
        Inline::Link { href, text } => {
            let _ = write!(out, r#"<a href="{}">"#, escape_attr(href));
            for c in text {
                render_inline(out, c);
            }
            out.push_str("</a>");
        }
        Inline::Xref { target, text } => {
            let _ = write!(out, r##"<a href="#{}">"##, escape_attr(target));
            match text {
                Some(t) => {
                    for c in t {
                        render_inline(out, c);
                    }
                }
                None => out.push_str(&escape(target)),
            }
            out.push_str("</a>");
        }
        Inline::Image {
            target,
            alt,
            width,
            height,
        } => {
            let _ = write!(
                out,
                r#"<img src="{}" alt="{}""#,
                escape_attr(target),
                escape_attr(alt)
            );
            if let Some(w) = width {
                let _ = write!(out, r#" width="{}""#, escape_attr(w));
            }
            if let Some(h) = height {
                let _ = write!(out, r#" height="{}""#, escape_attr(h));
            }
            out.push_str(">");
        }
        Inline::AttributeRef(name) => {
            let _ = write!(out, "{{{name}}}");
        }
        Inline::LineBreak => out.push_str("<br>"),
        Inline::RawHtml(html) => out.push_str(html),
    }
}

fn inlines_to_plain(inlines: &[Inline]) -> String {
    let mut out = String::new();
    for i in inlines {
        inline_to_plain(&mut out, i);
    }
    out
}

fn inline_to_plain(out: &mut String, i: &Inline) {
    match i {
        Inline::Text(s) => out.push_str(s),
        Inline::Strong(c) | Inline::Emphasis(c) | Inline::Monospace(c) => {
            for child in c {
                inline_to_plain(out, child);
            }
        }
        Inline::Link { text, .. } => {
            for child in text {
                inline_to_plain(out, child);
            }
        }
        Inline::Xref { target, text } => {
            if let Some(t) = text {
                for child in t {
                    inline_to_plain(out, child);
                }
            } else {
                out.push_str(target);
            }
        }
        Inline::Image { alt, .. } => out.push_str(alt),
        Inline::AttributeRef(name) => {
            let _ = write!(out, "{{{name}}}");
        }
        Inline::LineBreak => out.push(' '),
        Inline::RawHtml(_) => {}
    }
}

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(c),
        }
    }
    out
}

fn escape_attr(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}
