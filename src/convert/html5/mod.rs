//! HTML5 converter.
//!
//! v1 scope: enough HTML to see every block and inline the parser produces.
//! Matching Asciidoctor's exact HTML (class names, wrappers, TOC, etc.) is
//! conformance work — deliberately out of scope here.

use std::collections::BTreeMap;
use std::fmt::Write;

use crate::ast::{
    AttributeValue, Block, BlockMeta, CellStyle, Colist, ConvertError, Converter, DelimitedContent,
    DelimitedStyle, DescriptionList, Document, HAlign, Inline, List, ListMarker, Paragraph,
    RowKind, Section, Table, TableCell,
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
        let ctx = RenderCtx::new(doc);
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
        render_highlighter_head(&mut out, doc);
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

        if ctx.toc {
            render_toc(&mut out, &ctx);
        }

        for block in &doc.blocks {
            render_block(&mut out, block, &ctx)?;
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

        render_highlighter_body(&mut out, doc);
        out.push_str("</body>\n</html>\n");
        Ok(out)
    }
}

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

fn render_highlighter_head(out: &mut String, doc: &Document) {
    match source_highlighter(doc) {
        Some("prism") => {
            let theme = highlighter_attr(doc, "prism-theme", "prism");
            let _ = write!(
                out,
                "<link rel=\"stylesheet\" href=\"https://cdn.jsdelivr.net/npm/prismjs@1/themes/{}.min.css\">\n",
                escape_attr(theme)
            );
        }
        Some("highlightjs") => {
            let theme = highlighter_attr(doc, "highlightjs-theme", "default");
            let _ = write!(
                out,
                "<link rel=\"stylesheet\" href=\"https://cdn.jsdelivr.net/npm/highlight.js@11/styles/{}.min.css\">\n",
                escape_attr(theme)
            );
        }
        _ => {}
    }
}

fn render_highlighter_body(out: &mut String, doc: &Document) {
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

// --- render context (TOC, sectnums, sectanchors) ---------------------------

/// State derived from the document attributes and a single pre-walk.
/// Threaded through every render fn that recurses into [`render_block`].
struct RenderCtx {
    /// `:toc:` flag — render a TOC at the top of the body.
    toc: bool,
    /// `:sectnums:` flag — prepend "1.2.3" to each section heading.
    #[allow(dead_code)]
    sectnums: bool,
    /// `:sectanchors:` flag — emit a `<a class="anchor">` next to each heading.
    sectanchors: bool,
    /// Section ID → numbering string (e.g. `"1.2.3"`). Empty string when
    /// `sectnums` is off.
    section_numbers: BTreeMap<String, String>,
    /// In-order TOC entries (one per section).
    toc_entries: Vec<TocEntry>,
}

struct TocEntry {
    level: u8,
    id: String,
    number: String,
    title_plain: String,
}

impl RenderCtx {
    fn new(doc: &Document) -> Self {
        let toc = is_truthy(doc.attributes.get("toc"));
        let sectnums = is_truthy(doc.attributes.get("sectnums"));
        let sectanchors = is_truthy(doc.attributes.get("sectanchors"));
        let mut counter = [0u32; 7];
        let mut section_numbers = BTreeMap::new();
        let mut toc_entries = Vec::new();
        walk_sections(
            &doc.blocks,
            &mut counter,
            sectnums,
            &mut section_numbers,
            &mut toc_entries,
        );
        Self {
            toc,
            sectnums,
            sectanchors,
            section_numbers,
            toc_entries,
        }
    }

    fn section_number(&self, id: Option<&str>) -> Option<&str> {
        let id = id?;
        let n = self.section_numbers.get(id)?;
        if n.is_empty() {
            None
        } else {
            Some(n.as_str())
        }
    }
}

fn walk_sections(
    blocks: &[Block],
    counter: &mut [u32; 7],
    sectnums: bool,
    numbers: &mut BTreeMap<String, String>,
    toc: &mut Vec<TocEntry>,
) {
    for b in blocks {
        if let Block::Section(s) = b {
            let level = (s.level as usize).min(6);
            counter[level] += 1;
            for slot in counter.iter_mut().skip(level + 1) {
                *slot = 0;
            }
            let number = if sectnums {
                (1..=level)
                    .map(|i| counter[i].to_string())
                    .collect::<Vec<_>>()
                    .join(".")
            } else {
                String::new()
            };
            if let Some(id) = s.meta.id.as_deref() {
                numbers.insert(id.to_string(), number.clone());
                toc.push(TocEntry {
                    level: s.level,
                    id: id.to_string(),
                    number,
                    title_plain: inlines_to_plain(&s.title),
                });
            }
            walk_sections(&s.blocks, counter, sectnums, numbers, toc);
        }
    }
}

fn render_toc(out: &mut String, ctx: &RenderCtx) {
    if ctx.toc_entries.is_empty() {
        return;
    }
    out.push_str(r#"<div id="toc" class="toc">"#);
    out.push('\n');
    out.push_str(r#"<div class="toc-title">Table of Contents</div>"#);
    out.push('\n');

    // Build correct nesting: a deeper entry's <ul> goes *inside* the
    // preceding <li>, so we keep the most recent <li> open while we look
    // ahead and only close it when we land back at the same or a
    // shallower level.
    let mut depth: u8 = 0;
    let mut li_open = false;
    for entry in &ctx.toc_entries {
        let level = entry.level;
        if level > depth {
            while depth < level {
                out.push_str("<ul>\n");
                depth += 1;
            }
        } else {
            // li_open is overwritten to `true` at the end of every iteration,
            // so just close the prior <li> without bothering to flip the flag.
            if li_open {
                out.push_str("</li>\n");
            }
            while depth > level {
                out.push_str("</ul>\n");
                depth -= 1;
                out.push_str("</li>\n");
            }
        }
        let prefix = if entry.number.is_empty() {
            String::new()
        } else {
            format!(r#"<span class="sectnum">{}</span> "#, entry.number)
        };
        let _ = write!(
            out,
            r##"<li><a href="#{}">{}{}</a>"##,
            escape_attr(&entry.id),
            prefix,
            escape(&entry.title_plain)
        );
        li_open = true;
    }
    if li_open {
        out.push_str("</li>\n");
    }
    while depth > 0 {
        out.push_str("</ul>\n");
        depth -= 1;
        if depth > 0 {
            out.push_str("</li>\n");
        }
    }
    out.push_str("</div>\n");
}

fn is_truthy(v: Option<&AttributeValue>) -> bool {
    matches!(v, Some(AttributeValue::Bool(true)))
        || matches!(v, Some(AttributeValue::String(s)) if !s.is_empty() && !s.eq_ignore_ascii_case("false"))
}

// ---------------------------------------------------------------------------

fn render_block(out: &mut String, block: &Block, ctx: &RenderCtx) -> Result<(), ConvertError> {
    match block {
        Block::Section(s) => render_section(out, s, ctx),
        Block::Paragraph(p) => render_paragraph(out, p),
        Block::List(l) => render_list(out, l, ctx),
        Block::DescriptionList(d) => render_description_list(out, d, ctx),
        Block::Delimited(d) => render_delimited(out, d, ctx),
        Block::Table(t) => render_table(out, t, ctx),
        Block::Colist(c) => {
            render_colist(out, c);
            Ok(())
        }
        Block::DiscreteHeading(d) => render_discrete_heading(out, d),
    }
}

fn render_discrete_heading(
    out: &mut String,
    d: &crate::ast::DiscreteHeading,
) -> Result<(), ConvertError> {
    let tag = match d.level {
        1 => "h2",
        2 => "h3",
        3 => "h4",
        4 => "h5",
        _ => "h6",
    };
    let id_attr = d
        .meta
        .id
        .as_deref()
        .map(|id| format!(r#" id="{}""#, escape_attr(id)))
        .unwrap_or_default();
    let class_attr = if d.meta.roles.is_empty() {
        r#" class="discrete""#.to_string()
    } else {
        let roles = d
            .meta
            .roles
            .iter()
            .map(|r| escape_attr(r))
            .collect::<Vec<_>>()
            .join(" ");
        format!(r#" class="discrete {roles}""#)
    };
    writeln!(
        out,
        "<{tag}{id_attr}{class_attr}>{}</{tag}>",
        render_inlines(&d.title)
    )
    .map_err(|e| ConvertError::Message(e.to_string()))
}

fn render_section(out: &mut String, s: &Section, ctx: &RenderCtx) -> Result<(), ConvertError> {
    let tag = match s.level {
        1 => "h2",
        2 => "h3",
        3 => "h4",
        4 => "h5",
        _ => "h6",
    };
    write!(out, "<section{}>\n", meta_attrs(&s.meta))
        .map_err(|e| ConvertError::Message(e.to_string()))?;
    render_block_title(out, &s.meta);

    // Optional sectnums prefix and sectanchors `<a class="anchor">` link.
    let prefix = ctx.section_number(s.meta.id.as_deref()).unwrap_or("");
    let prefix_html = if prefix.is_empty() {
        String::new()
    } else {
        format!(r#"<span class="sectnum">{prefix}</span> "#)
    };
    let anchor_html = match (ctx.sectanchors, s.meta.id.as_deref()) {
        (true, Some(id)) => format!(r##"<a class="anchor" href="#{}"></a>"##, escape_attr(id)),
        _ => String::new(),
    };
    writeln!(
        out,
        "<{tag}>{anchor_html}{prefix_html}{}</{tag}>",
        render_inlines(&s.title)
    )
    .map_err(|e| ConvertError::Message(e.to_string()))?;
    for b in &s.blocks {
        render_block(out, b, ctx)?;
    }
    out.push_str("</section>\n");
    Ok(())
}

fn render_paragraph(out: &mut String, p: &Paragraph) -> Result<(), ConvertError> {
    if let Some(kw) = admonition_keyword(&p.meta) {
        return render_admonition_paragraph(out, p, kw);
    }
    if p.meta.style.as_deref() == Some("image") {
        return render_block_image(out, p);
    }
    render_block_title(out, &p.meta);
    writeln!(
        out,
        "<p{}>{}</p>",
        meta_attrs(&p.meta),
        render_inlines(&p.inlines)
    )
    .map_err(|e| ConvertError::Message(e.to_string()))
}

fn render_block_image(out: &mut String, p: &Paragraph) -> Result<(), ConvertError> {
    writeln!(out, "<div{} class=\"imageblock\">", meta_id_only(&p.meta))
        .map_err(|e| ConvertError::Message(e.to_string()))?;
    out.push_str(r#"<div class="content">"#);
    out.push_str(&render_inlines(&p.inlines));
    out.push_str("</div>\n");
    if let Some(title) = &p.meta.title {
        writeln!(out, r#"<div class="title">{}</div>"#, render_inlines(title))
            .map_err(|e| ConvertError::Message(e.to_string()))?;
    }
    out.push_str("</div>\n");
    Ok(())
}

fn render_admonition_paragraph(
    out: &mut String,
    p: &Paragraph,
    kw: &str,
) -> Result<(), ConvertError> {
    let label = admonition_label(kw);
    let class = admonition_class(kw);
    writeln!(
        out,
        r#"<div{} class="admonitionblock {class}">"#,
        meta_id_only(&p.meta)
    )
    .map_err(|e| ConvertError::Message(e.to_string()))?;
    if let Some(title) = &p.meta.title {
        writeln!(out, r#"<p class="title">{}</p>"#, render_inlines(title))
            .map_err(|e| ConvertError::Message(e.to_string()))?;
    } else {
        writeln!(out, r#"<p class="label">{label}</p>"#)
            .map_err(|e| ConvertError::Message(e.to_string()))?;
    }
    writeln!(out, r#"<div class="content">"#).map_err(|e| ConvertError::Message(e.to_string()))?;
    writeln!(out, "<p>{}</p>", render_inlines(&p.inlines))
        .map_err(|e| ConvertError::Message(e.to_string()))?;
    out.push_str("</div>\n</div>\n");
    Ok(())
}

fn render_list(out: &mut String, l: &List, ctx: &RenderCtx) -> Result<(), ConvertError> {
    // Re-nest by depth.
    let tag = match l.marker {
        ListMarker::Unordered => "ul",
        ListMarker::Ordered => "ol",
    };
    render_block_title(out, &l.meta);
    let mut current_depth = 0u8;
    let mut open = 0u32;
    let mut top_attrs: Option<String> = Some(meta_attrs(&l.meta));
    for item in &l.items {
        while current_depth < item.depth {
            // Apply meta only to the outermost list element.
            let attrs = top_attrs.take().unwrap_or_default();
            writeln!(out, "<{tag}{attrs}>").map_err(|e| ConvertError::Message(e.to_string()))?;
            current_depth += 1;
            open += 1;
        }
        while current_depth > item.depth {
            writeln!(out, "</{tag}>").map_err(|e| ConvertError::Message(e.to_string()))?;
            current_depth -= 1;
            open = open.saturating_sub(1);
        }
        write!(out, "<li>{}", render_inlines(&item.principal))
            .map_err(|e| ConvertError::Message(e.to_string()))?;
        for b in &item.blocks {
            out.push('\n');
            render_block(out, b, ctx)?;
        }
        out.push_str("</li>\n");
    }
    for _ in 0..open {
        writeln!(out, "</{tag}>").map_err(|e| ConvertError::Message(e.to_string()))?;
    }
    Ok(())
}

fn render_description_list(
    out: &mut String,
    d: &DescriptionList,
    ctx: &RenderCtx,
) -> Result<(), ConvertError> {
    render_block_title(out, &d.meta);
    writeln!(out, "<dl{}>", meta_attrs(&d.meta))
        .map_err(|e| ConvertError::Message(e.to_string()))?;
    for item in &d.items {
        writeln!(out, "<dt>{}</dt>", render_inlines(&item.term))
            .map_err(|e| ConvertError::Message(e.to_string()))?;
        out.push_str("<dd>");
        for b in &item.description {
            render_block(out, b, ctx)?;
        }
        out.push_str("</dd>\n");
    }
    out.push_str("</dl>\n");
    Ok(())
}

fn render_delimited(
    out: &mut String,
    d: &crate::ast::DelimitedBlock,
    ctx: &RenderCtx,
) -> Result<(), ConvertError> {
    // Block-form admonition (e.g. `[NOTE]\n====\n…\n====`).
    if let (Some(kw), DelimitedContent::Blocks { blocks }) =
        (admonition_keyword(&d.meta), &d.content)
    {
        return render_admonition_block(out, &d.meta, kw, blocks, ctx);
    }
    render_block_title(out, &d.meta);
    let a = meta_attrs(&d.meta);
    match (&d.style, &d.content) {
        (DelimitedStyle::Listing, DelimitedContent::Raw { text }) => {
            // `[source,LANG]` adds a `language-LANG` class on the <code>
            // and a matching `data-lang="LANG"` on the <pre> so the default
            // stylesheet can surface the language as a corner pill.
            let code_class = source_language_class(&d.meta);
            let pre_lang_attr = source_language_data_attr(&d.meta);
            let body = substitute_conums(&escape(text));
            writeln!(
                out,
                "<pre{a}{pre_lang_attr}><code{code_class}>{body}</code></pre>"
            )
            .map_err(|e| ConvertError::Message(e.to_string()))
        }
        (DelimitedStyle::Literal, DelimitedContent::Raw { text }) => {
            let body = substitute_conums(&escape(text));
            writeln!(out, "<pre{a}>{body}</pre>").map_err(|e| ConvertError::Message(e.to_string()))
        }
        (DelimitedStyle::Passthrough, DelimitedContent::Raw { text }) => {
            out.push_str(text);
            out.push('\n');
            Ok(())
        }
        (DelimitedStyle::Example, DelimitedContent::Blocks { blocks }) => {
            // The "example" class is intrinsic to this block style; user roles
            // append after it.
            writeln!(out, "<div{}>", merge_class_attr(&d.meta, "example"))
                .map_err(|e| ConvertError::Message(e.to_string()))?;
            for b in blocks {
                render_block(out, b, ctx)?;
            }
            out.push_str("</div>\n");
            Ok(())
        }
        (DelimitedStyle::Quote, DelimitedContent::Blocks { blocks }) => {
            writeln!(out, "<blockquote{a}>").map_err(|e| ConvertError::Message(e.to_string()))?;
            for b in blocks {
                render_block(out, b, ctx)?;
            }
            out.push_str("</blockquote>\n");
            Ok(())
        }
        (DelimitedStyle::Sidebar, DelimitedContent::Blocks { blocks }) => {
            writeln!(out, "<aside{a}>").map_err(|e| ConvertError::Message(e.to_string()))?;
            for b in blocks {
                render_block(out, b, ctx)?;
            }
            out.push_str("</aside>\n");
            Ok(())
        }
        (DelimitedStyle::Open, DelimitedContent::Blocks { blocks }) => {
            writeln!(out, "<div{a}>").map_err(|e| ConvertError::Message(e.to_string()))?;
            for b in blocks {
                render_block(out, b, ctx)?;
            }
            out.push_str("</div>\n");
            Ok(())
        }
        _ => Err(ConvertError::Message(
            "delimited block style/content mismatch".into(),
        )),
    }
}

fn render_table(out: &mut String, t: &Table, ctx: &RenderCtx) -> Result<(), ConvertError> {
    render_block_title(out, &t.meta);
    writeln!(out, "<table{}>", meta_attrs(&t.meta))
        .map_err(|e| ConvertError::Message(e.to_string()))?;
    render_colgroup(out, t);

    // Group rows by kind so we can emit <thead>/<tbody>/<tfoot> sections.
    let mut i = 0;
    while i < t.rows.len() {
        let kind = t.rows[i].kind;
        let mut j = i;
        while j < t.rows.len() && t.rows[j].kind == kind {
            j += 1;
        }
        let (open, close) = match kind {
            RowKind::Header => ("<thead>\n", "</thead>\n"),
            RowKind::Body => ("<tbody>\n", "</tbody>\n"),
            RowKind::Footer => ("<tfoot>\n", "</tfoot>\n"),
        };
        out.push_str(open);
        for row in &t.rows[i..j] {
            out.push_str("<tr>");
            for (idx, cell) in row.cells.iter().enumerate() {
                let col = t.cols.get(idx);
                render_table_cell(out, cell, kind, col, ctx)?;
            }
            out.push_str("</tr>\n");
        }
        out.push_str(close);
        i = j;
    }

    out.push_str("</table>\n");
    Ok(())
}

/// Emit a `<colgroup>` based on `cols=` widths. The widths are relative
/// weights; we normalise them to percentages so the renderer doesn't have
/// to know the table's container width. Skipped when no `cols=` was given
/// or when every entry has width 0.
fn render_colgroup(out: &mut String, t: &Table) {
    if t.cols.is_empty() {
        return;
    }
    let total: u32 = t.cols.iter().map(|c| c.width).sum();
    out.push_str("<colgroup>\n");
    for col in &t.cols {
        if total > 0 && col.width > 0 {
            let pct = (col.width as f64) * 100.0 / (total as f64);
            let _ = writeln!(out, r#"<col style="width: {pct:.4}%;">"#);
        } else {
            out.push_str("<col>\n");
        }
    }
    out.push_str("</colgroup>\n");
}

fn h_align_class(a: HAlign) -> &'static str {
    match a {
        HAlign::Left => "halign-left",
        HAlign::Center => "halign-center",
        HAlign::Right => "halign-right",
    }
}

fn render_table_cell(
    out: &mut String,
    cell: &TableCell,
    row_kind: RowKind,
    col: Option<&crate::ast::ColumnSpec>,
    ctx: &RenderCtx,
) -> Result<(), ConvertError> {
    // Forced header style or header rows always use <th>.
    let force_th = matches!(cell.style, Some(CellStyle::Header)) || row_kind == RowKind::Header;
    let tag = if force_th { "th" } else { "td" };

    // Effective alignment: cell-level wins, otherwise column-level, otherwise none.
    let effective_align = cell.h_align.or_else(|| col.and_then(|c| c.h_align));
    let mut classes: Vec<&str> = Vec::new();
    if let Some(a) = effective_align {
        classes.push(h_align_class(a));
    }
    let class_attr = if classes.is_empty() {
        String::new()
    } else {
        format!(r#" class="{}""#, classes.join(" "))
    };

    let mut span_attrs = String::new();
    if cell.colspan > 1 {
        let _ = write!(span_attrs, r#" colspan="{}""#, cell.colspan);
    }
    if cell.rowspan > 1 {
        let _ = write!(span_attrs, r#" rowspan="{}""#, cell.rowspan);
    }

    // AsciiDoc cells render their pre-parsed nested blocks; everything
    // else uses the cell's flat inline list.
    let body = if matches!(cell.style, Some(CellStyle::AsciiDoc)) {
        let mut buf = String::new();
        for b in &cell.blocks {
            render_block(&mut buf, b, ctx)?;
        }
        buf
    } else {
        match cell.style {
            Some(CellStyle::Monospace) if !force_th => {
                format!("<code>{}</code>", render_inlines(&cell.inlines))
            }
            Some(CellStyle::Strong) if !force_th => {
                format!("<strong>{}</strong>", render_inlines(&cell.inlines))
            }
            Some(CellStyle::Emphasis) if !force_th => {
                format!("<em>{}</em>", render_inlines(&cell.inlines))
            }
            Some(CellStyle::Literal) => {
                let plain = inlines_to_plain(&cell.inlines);
                format!("<pre>{}</pre>", escape(&plain))
            }
            _ => render_inlines(&cell.inlines),
        }
    };

    write!(out, "<{tag}{span_attrs}{class_attr}>{body}</{tag}>")
        .map_err(|e| ConvertError::Message(e.to_string()))
}

// --- block metadata rendering ---------------------------------------------

/// Build the ` id="..." class="..."` fragment for a block opening tag.
/// Returns an empty string when no id or roles are set.
fn meta_attrs(meta: &BlockMeta) -> String {
    let mut out = String::new();
    if let Some(id) = &meta.id {
        let _ = write!(out, r#" id="{}""#, escape_attr(id));
    }
    if !meta.roles.is_empty() {
        let classes = meta
            .roles
            .iter()
            .map(|r| escape_attr(r))
            .collect::<Vec<_>>()
            .join(" ");
        let _ = write!(out, r#" class="{classes}""#);
    }
    out
}

/// Like [`meta_attrs`] but merges an intrinsic class (e.g., `"example"` for the
/// example block) with any user-supplied roles into a single `class` attribute.
fn merge_class_attr(meta: &BlockMeta, intrinsic: &str) -> String {
    let mut out = String::new();
    if let Some(id) = &meta.id {
        let _ = write!(out, r#" id="{}""#, escape_attr(id));
    }
    let mut classes = vec![intrinsic.to_string()];
    classes.extend(meta.roles.iter().map(|r| escape_attr(r)));
    let _ = write!(out, r#" class="{}""#, classes.join(" "));
    out
}

fn render_block_title(out: &mut String, meta: &BlockMeta) {
    if let Some(title) = &meta.title {
        let _ = writeln!(out, r#"<div class="title">{}</div>"#, render_inlines(title));
    }
}

/// Replace `&lt;N&gt;` markers with conum HTML inside an already-escaped
/// listing/literal body. `N` is one or more ASCII digits. The function is
/// UTF-8-safe — non-marker bytes are forwarded unchanged so multibyte
/// characters in source code (e.g. comments) survive intact.
fn substitute_conums(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_end = 0;
    let bytes = s.as_bytes();
    let mut i = 0;
    while let Some(rel) = s[i..].find("&lt;") {
        let start = i + rel;
        let after = start + 4;
        let mut j = after;
        while j < bytes.len() && bytes[j].is_ascii_digit() {
            j += 1;
        }
        if j > after && s[j..].starts_with("&gt;") {
            out.push_str(&s[last_end..start]);
            let num = &s[after..j];
            let _ = write!(out, r#"<b class="conum">({num})</b>"#);
            last_end = j + 4;
            i = last_end;
        } else {
            i = start + 4;
        }
    }
    out.push_str(&s[last_end..]);
    out
}

fn render_colist(out: &mut String, c: &Colist) {
    render_block_title(out, &c.meta);
    out.push_str("<ol class=\"colist\">\n");
    for item in &c.items {
        let _ = writeln!(
            out,
            r#"<li value="{}">{}</li>"#,
            item.number,
            render_inlines(&item.inlines)
        );
    }
    out.push_str("</ol>\n");
}

/// Like [`meta_attrs`] but emits *only* the `id` attribute. Used by
/// renderers that build the class list themselves (admonitions, etc.).
fn meta_id_only(meta: &BlockMeta) -> String {
    let mut out = String::new();
    if let Some(id) = &meta.id {
        let _ = write!(out, r#" id="{}""#, escape_attr(id));
    }
    out
}

// --- admonitions ----------------------------------------------------------

fn admonition_keyword(meta: &BlockMeta) -> Option<&str> {
    let style = meta.style.as_deref()?;
    matches!(style, "NOTE" | "TIP" | "IMPORTANT" | "WARNING" | "CAUTION").then_some(style)
}

fn admonition_label(kw: &str) -> &'static str {
    match kw {
        "NOTE" => "Note",
        "TIP" => "Tip",
        "IMPORTANT" => "Important",
        "WARNING" => "Warning",
        "CAUTION" => "Caution",
        _ => "Note",
    }
}

fn admonition_class(kw: &str) -> &'static str {
    match kw {
        "NOTE" => "note",
        "TIP" => "tip",
        "IMPORTANT" => "important",
        "WARNING" => "warning",
        "CAUTION" => "caution",
        _ => "note",
    }
}

fn render_admonition_block(
    out: &mut String,
    meta: &BlockMeta,
    kw: &str,
    blocks: &[Block],
    ctx: &RenderCtx,
) -> Result<(), ConvertError> {
    let label = admonition_label(kw);
    let class = admonition_class(kw);
    writeln!(
        out,
        r#"<div{} class="admonitionblock {class}">"#,
        meta_id_only(meta)
    )
    .map_err(|e| ConvertError::Message(e.to_string()))?;
    if let Some(title) = &meta.title {
        writeln!(out, r#"<p class="title">{}</p>"#, render_inlines(title))
            .map_err(|e| ConvertError::Message(e.to_string()))?;
    } else {
        writeln!(out, r#"<p class="label">{label}</p>"#)
            .map_err(|e| ConvertError::Message(e.to_string()))?;
    }
    writeln!(out, r#"<div class="content">"#).map_err(|e| ConvertError::Message(e.to_string()))?;
    for b in blocks {
        render_block(out, b, ctx)?;
    }
    out.push_str("</div>\n</div>\n");
    Ok(())
}

// --- source-block language ------------------------------------------------

/// `[source,rust]` on a listing block becomes ` class="language-rust"` on
/// the inner `<code>`. Returns an empty string if no language is set.
fn source_language_class(meta: &BlockMeta) -> String {
    let Some(lang) = source_language(meta) else {
        return String::new();
    };
    format!(r#" class="language-{}""#, escape_attr(lang))
}

/// `[source,rust]` on a listing block becomes ` data-lang="rust"` on the
/// outer `<pre>`. Lets the default stylesheet surface the language as a
/// corner pill via `attr(data-lang)`.
fn source_language_data_attr(meta: &BlockMeta) -> String {
    let Some(lang) = source_language(meta) else {
        return String::new();
    };
    format!(r#" data-lang="{}""#, escape_attr(lang))
}

fn source_language(meta: &BlockMeta) -> Option<&str> {
    if meta.style.as_deref() != Some("source") {
        return None;
    }
    let lang = meta.positional.first()?.trim();
    if lang.is_empty() {
        None
    } else {
        Some(lang)
    }
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
        Inline::Text { value } => out.push_str(&escape(value)),
        Inline::Strong { children } => {
            out.push_str("<strong>");
            for c in children {
                render_inline(out, c);
            }
            out.push_str("</strong>");
        }
        Inline::Emphasis { children } => {
            out.push_str("<em>");
            for c in children {
                render_inline(out, c);
            }
            out.push_str("</em>");
        }
        Inline::Monospace { children } => {
            out.push_str("<code>");
            for c in children {
                render_inline(out, c);
            }
            out.push_str("</code>");
        }
        Inline::Subscript { children } => {
            out.push_str("<sub>");
            for c in children {
                render_inline(out, c);
            }
            out.push_str("</sub>");
        }
        Inline::Superscript { children } => {
            out.push_str("<sup>");
            for c in children {
                render_inline(out, c);
            }
            out.push_str("</sup>");
        }
        Inline::Highlight { children } => {
            out.push_str("<mark>");
            for c in children {
                render_inline(out, c);
            }
            out.push_str("</mark>");
        }
        Inline::Footnote { id, text } => {
            match id {
                Some(id) => {
                    let _ = write!(out, r#"<span class="footnote" id="{}">"#, escape_attr(id));
                }
                None => out.push_str(r#"<span class="footnote">"#),
            }
            for c in text {
                render_inline(out, c);
            }
            out.push_str("</span>");
        }
        Inline::Passthrough { value } => out.push_str(&escape(value)),
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
        Inline::AttributeRef { name } => {
            let _ = write!(out, "{{{name}}}");
        }
        Inline::LineBreak => out.push_str("<br>"),
        Inline::RawHtml { value } => out.push_str(value),
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
        Inline::Text { value } => out.push_str(value),
        Inline::Strong { children }
        | Inline::Emphasis { children }
        | Inline::Monospace { children }
        | Inline::Subscript { children }
        | Inline::Superscript { children }
        | Inline::Highlight { children } => {
            for child in children {
                inline_to_plain(out, child);
            }
        }
        Inline::Footnote { text, .. } => {
            for child in text {
                inline_to_plain(out, child);
            }
        }
        Inline::Passthrough { value } => out.push_str(value),
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
        Inline::AttributeRef { name } => {
            let _ = write!(out, "{{{name}}}");
        }
        Inline::LineBreak => out.push(' '),
        Inline::RawHtml { .. } => {}
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
