//! Block-level rendering. Owns the [`render_block`] dispatcher plus every
//! per-variant render function (sections, paragraphs, lists, description
//! lists, delimited blocks, admonitions, block-level images / video /
//! audio, callouts, discrete headings) and the small block-meta helpers
//! every renderer shares (`meta_attrs`, `meta_id_only`, `merge_class_attr`,
//! `render_block_title`).
//!
//! Tables are big enough to live in their own [`super::tables`] module.

use std::fmt::Write;

use crate::ast::{
    Block, BlockMeta, Colist, ConvertError, DelimitedBlock, DelimitedContent, DelimitedStyle,
    DescriptionList, DiscreteHeading, List, ListMarker, Paragraph, Section,
};

use super::ctx::RenderCtx;
use super::escape::{escape, escape_attr};
use super::inlines::render_inlines;
use super::tables::render_table;

pub(crate) fn render_block(
    out: &mut String,
    block: &Block,
    ctx: &RenderCtx,
) -> Result<(), ConvertError> {
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

// --- sections / discrete headings ----------------------------------------

fn render_section(out: &mut String, s: &Section, ctx: &RenderCtx) -> Result<(), ConvertError> {
    let tag = heading_tag_for_level(s.level);
    writeln!(out, "<section{}>", meta_attrs(&s.meta))
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

fn render_discrete_heading(out: &mut String, d: &DiscreteHeading) -> Result<(), ConvertError> {
    let tag = heading_tag_for_level(d.level);
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

fn heading_tag_for_level(level: u8) -> &'static str {
    match level {
        1 => "h2",
        2 => "h3",
        3 => "h4",
        4 => "h5",
        _ => "h6",
    }
}

// --- paragraphs (incl. block image / av / admonition shortcuts) ---------

fn render_paragraph(out: &mut String, p: &Paragraph) -> Result<(), ConvertError> {
    if let Some(kw) = admonition_keyword(&p.meta) {
        return render_admonition_paragraph(out, p, kw);
    }
    if p.meta.style.as_deref() == Some("image") {
        return render_block_image(out, p);
    }
    if matches!(p.meta.style.as_deref(), Some("video") | Some("audio")) {
        return render_block_av(out, p);
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

fn render_block_av(out: &mut String, p: &Paragraph) -> Result<(), ConvertError> {
    let class = match p.meta.style.as_deref() {
        Some("audio") => "audioblock",
        _ => "videoblock",
    };
    writeln!(out, "<div{} class=\"{class}\">", meta_id_only(&p.meta))
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

// --- lists --------------------------------------------------------------

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

// --- delimited blocks ---------------------------------------------------

fn render_delimited(
    out: &mut String,
    d: &DelimitedBlock,
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
            render_quote_attribution(out, &d.meta);
            out.push_str("</blockquote>\n");
            Ok(())
        }
        // `[verse]` on a quote block: whitespace and line breaks matter
        // (poetry, song lyrics, code-shaped prose), so the inner text
        // is captured raw and rendered escaped inside `<pre class=
        // "verseblock">`. Inline formatting inside verse is intentionally
        // suppressed in v1 — most verse content doesn't use it.
        (DelimitedStyle::Quote, DelimitedContent::Raw { text })
            if d.meta.style.as_deref() == Some("verse") =>
        {
            writeln!(
                out,
                r#"<pre{a} class="verseblock">{}</pre>"#,
                escape(text.trim_end_matches('\n'))
            )
            .map_err(|e| ConvertError::Message(e.to_string()))?;
            render_quote_attribution(out, &d.meta);
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

// --- admonitions -------------------------------------------------------

pub(crate) fn admonition_keyword(meta: &BlockMeta) -> Option<&str> {
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

// --- callouts ----------------------------------------------------------

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

// --- block-meta helpers (id / class) ------------------------------------

/// Build the ` id="..." class="..."` fragment for a block opening tag.
/// Returns an empty string when no id or roles are set.
pub(crate) fn meta_attrs(meta: &BlockMeta) -> String {
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

/// Like [`meta_attrs`] but emits *only* the `id` attribute. Used by
/// renderers that build the class list themselves (admonitions, etc.).
fn meta_id_only(meta: &BlockMeta) -> String {
    let mut out = String::new();
    if let Some(id) = &meta.id {
        let _ = write!(out, r#" id="{}""#, escape_attr(id));
    }
    out
}

pub(crate) fn render_block_title(out: &mut String, meta: &BlockMeta) {
    if let Some(title) = &meta.title {
        let _ = writeln!(out, r#"<div class="title">{}</div>"#, render_inlines(title));
    }
}

// --- quote attribution --------------------------------------------------

/// Render the optional `[quote, Author, Source]` attribution under a
/// `<blockquote>` or verse block. The first positional after the style is
/// the author; the second is the source (book, song, etc.). Empty when no
/// attribution is supplied.
fn render_quote_attribution(out: &mut String, meta: &BlockMeta) {
    let author = meta.positional.first().map(String::as_str).unwrap_or("");
    let source = meta.positional.get(1).map(String::as_str).unwrap_or("");
    if author.is_empty() && source.is_empty() {
        return;
    }
    out.push_str(r#"<div class="attribution">"#);
    out.push_str("\u{2014} ");
    out.push_str(&escape(author));
    if !source.is_empty() {
        out.push_str("<br>\n<cite>");
        out.push_str(&escape(source));
        out.push_str("</cite>");
    }
    out.push_str("</div>\n");
}

// --- source-block language ---------------------------------------------

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
