//! Render context: state derived from the document attributes via a
//! single pre-walk over the section tree. Threaded through every render
//! function that recurses into the block dispatcher.
//!
//! This is the place where `:toc:`, `:sectnums:`, and `:sectanchors:`
//! turn into concrete data — section numbers keyed by id, an in-order
//! TOC entry list, and the booleans for whether to show each.

use std::collections::BTreeMap;
use std::fmt::Write;

use crate::ast::{
    inlines_to_plain, AttributeValue, Block, DelimitedContent, Document, IdRegistry, Inline,
    Location,
};
use crate::diag::{Diagnostic, Diagnostics};

use super::escape::{escape, escape_attr};

pub(crate) struct RenderCtx {
    /// `:toc:` flag — render a TOC at the top of the body.
    pub(crate) toc: bool,
    /// `:toc-placement:` — where in the document the TOC renders.
    /// `auto` (default) puts it at the top of `<main>`; `preamble`
    /// puts it after the preamble div; `macro` (and `left`/`right`,
    /// which require sidebar layout) fall back to `auto` in v1.
    pub(crate) toc_placement: TocPlacement,
    /// `:sectnums:` flag — prepend "1.2.3" to each section heading.
    #[allow(dead_code)]
    pub(crate) sectnums: bool,
    /// `:sectanchors:` flag — emit a `<a class="anchor">` next to each heading.
    pub(crate) sectanchors: bool,
    /// Section ID → numbering string (e.g. `"1.2.3"`). Empty string when
    /// `sectnums` is off.
    pub(crate) section_numbers: BTreeMap<String, String>,
    /// In-order TOC entries (one per section).
    pub(crate) toc_entries: Vec<TocEntry>,
    /// Doc-wide ID registry — used by the inline renderer to validate
    /// xref targets and warn (via `tracing`) on dangling references.
    pub(crate) ids: IdRegistry,
}

/// Where in the document the TOC is inserted. `Auto` is the v1 default
/// (top of `<main id="content">`); `Preamble` puts it right after the
/// preamble div, between the intro prose and the first section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TocPlacement {
    Auto,
    Preamble,
}

pub(crate) struct TocEntry {
    pub(crate) level: u8,
    pub(crate) id: String,
    pub(crate) number: String,
    pub(crate) title_plain: String,
}

impl RenderCtx {
    pub(crate) fn new(doc: &Document) -> Self {
        let toc = is_truthy(doc.attributes.get("toc"));
        let toc_placement = match doc
            .attributes
            .get("toc-placement")
            .and_then(AttributeValue::as_str)
            .map(|s| s.to_ascii_lowercase())
            .as_deref()
        {
            Some("preamble") => TocPlacement::Preamble,
            // Anything else — including the unsupported `macro` /
            // `left` / `right` — falls back to the v1 default.
            _ => TocPlacement::Auto,
        };
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
        let ids = IdRegistry::collect(doc);
        Self {
            toc,
            toc_placement,
            sectnums,
            sectanchors,
            section_numbers,
            toc_entries,
            ids,
        }
    }

    pub(crate) fn section_number(&self, id: Option<&str>) -> Option<&str> {
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

pub(crate) fn render_toc(out: &mut String, ctx: &RenderCtx) {
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

pub(crate) fn is_truthy(v: Option<&AttributeValue>) -> bool {
    matches!(v, Some(AttributeValue::Bool(true)))
        || matches!(v, Some(AttributeValue::String(s)) if !s.is_empty() && !s.eq_ignore_ascii_case("false"))
}

/// Walk every inline in the document and append a [`Diagnostic`] for
/// each `<<…>>` / `xref:…[]` whose target id isn't in the registry.
/// Run once during `convert()`; the inline renderer stays target-blind.
///
/// Diagnostics are span-pointing where possible: each xref is attributed
/// to its containing block's [`Location`], so miette can render a
/// snippet of the surrounding source. Output is unaffected — the
/// rendered HTML still emits the dangling href.
pub(crate) fn validate_xrefs(doc: &Document, ids: &IdRegistry, diags: &mut Diagnostics) {
    walk_validate_blocks(&doc.blocks, ids, diags);
}

fn walk_validate_blocks(blocks: &[Block], ids: &IdRegistry, diags: &mut Diagnostics) {
    for b in blocks {
        match b {
            Block::Section(s) => {
                walk_validate_inlines(&s.title, ids, &s.location, diags);
                walk_validate_blocks(&s.blocks, ids, diags);
            }
            Block::Paragraph(p) => walk_validate_inlines(&p.inlines, ids, &p.location, diags),
            Block::List(l) => {
                for item in &l.items {
                    walk_validate_inlines(&item.principal, ids, &l.location, diags);
                    walk_validate_blocks(&item.blocks, ids, diags);
                }
            }
            Block::DescriptionList(d) => {
                for item in &d.items {
                    walk_validate_inlines(&item.term, ids, &d.location, diags);
                    walk_validate_blocks(&item.description, ids, diags);
                }
            }
            Block::Delimited(d) => {
                if let DelimitedContent::Blocks { blocks } = &d.content {
                    walk_validate_blocks(blocks, ids, diags);
                }
            }
            Block::Table(t) => {
                for row in &t.rows {
                    for cell in &row.cells {
                        walk_validate_inlines(&cell.inlines, ids, &t.location, diags);
                        walk_validate_blocks(&cell.blocks, ids, diags);
                    }
                }
            }
            Block::Colist(c) => {
                for item in &c.items {
                    walk_validate_inlines(&item.inlines, ids, &c.location, diags);
                }
            }
            Block::DiscreteHeading(d) => walk_validate_inlines(&d.title, ids, &d.location, diags),
        }
    }
}

fn walk_validate_inlines(
    inlines: &[Inline],
    ids: &IdRegistry,
    container: &Location,
    diags: &mut Diagnostics,
) {
    for i in inlines {
        match i {
            Inline::Xref { target, text } => {
                if !target.is_empty() && !ids.contains(target) {
                    diags.push(
                        Diagnostic::warning(
                            "adoc::xref::dangling",
                            format!("cross-reference target `{target}` not found"),
                            container.clone(),
                        )
                        .with_label("no anchor with this id is defined")
                        .with_help(suggest_ids(target, ids)),
                    );
                }
                if let Some(t) = text {
                    walk_validate_inlines(t, ids, container, diags);
                }
            }
            Inline::Strong { children }
            | Inline::Emphasis { children }
            | Inline::Monospace { children }
            | Inline::Subscript { children }
            | Inline::Superscript { children }
            | Inline::Highlight { children } => {
                walk_validate_inlines(children, ids, container, diags)
            }
            Inline::Link { text, .. } | Inline::Footnote { text, .. } => {
                walk_validate_inlines(text, ids, container, diags);
            }
            _ => {}
        }
    }
}

/// Build a help string for a dangling xref. Lists up to a handful of
/// known ids so the user can spot a typo.
fn suggest_ids(_target: &str, ids: &IdRegistry) -> String {
    let mut sample: Vec<&str> = ids.ids().take(8).collect();
    if sample.is_empty() {
        return "no ids are defined in this document".to_string();
    }
    sample.sort_unstable();
    format!("known ids include: {}", sample.join(", "))
}
