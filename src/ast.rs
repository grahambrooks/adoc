//! AST types for the adoc AsciiDoc toolchain.
//!
//! The AST produced by [`crate::parser`] and consumed by converters lives here.
//! Every node carries a [`Location`] so diagnostics can point back to source.
//! All types are `serde`-serializable: the JSON form is the contract for
//! future stdio-based extensions.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    pub header: Option<Header>,
    pub attributes: Attributes,
    pub blocks: Vec<Block>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Header {
    pub title: Vec<Inline>,
    pub authors: Vec<Author>,
    pub revision: Option<Revision>,
    pub location: Location,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Author {
    pub name: String,
    pub email: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Revision {
    pub number: Option<String>,
    pub date: Option<String>,
    pub remark: Option<String>,
}

pub type Attributes = BTreeMap<String, AttributeValue>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AttributeValue {
    Bool(bool),
    String(String),
}

impl AttributeValue {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            AttributeValue::String(s) => Some(s.as_str()),
            AttributeValue::Bool(_) => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Block {
    Section(Section),
    Paragraph(Paragraph),
    List(List),
    DescriptionList(DescriptionList),
    Delimited(DelimitedBlock),
    Table(Table),
    Colist(Colist),
    /// A `[discrete]` heading — uses the section title syntax but doesn't
    /// open a new section: the renderer emits the heading and the surrounding
    /// blocks remain siblings of the heading rather than being nested under it.
    DiscreteHeading(DiscreteHeading),
}

/// Metadata attached to a block via `.Title` and `[...]` lines.
///
/// The serialized form skips empty fields, so a metadata-free block round-trips
/// through JSON as `"meta": {}` (or omits the field entirely with future
/// `#[serde(skip_serializing_if = "BlockMeta::is_empty")]` on the parent).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BlockMeta {
    /// Explicit block ID (`[#myid]` shorthand or `id="myid"` attribute).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Block title (`.Title` line). Stored inline-parsed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<Vec<Inline>>,
    /// Block style — first positional attribute (e.g., `source`, `NOTE`, `quote`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
    /// Roles (`.role` shorthand or whitespace-separated `role="..."`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roles: Vec<String>,
    /// Options (`%opt` shorthand or comma-separated `options="..."`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
    /// Remaining positional values after the style (e.g., `rust` in `[source,rust]`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub positional: Vec<String>,
    /// Named attributes (`name=value`), excluding ones folded into the fields above.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub named: BTreeMap<String, String>,
}

impl BlockMeta {
    pub fn is_empty(&self) -> bool {
        self.id.is_none()
            && self.title.is_none()
            && self.style.is_none()
            && self.roles.is_empty()
            && self.options.is_empty()
            && self.positional.is_empty()
            && self.named.is_empty()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Section {
    pub level: u8,
    pub title: Vec<Inline>,
    pub blocks: Vec<Block>,
    pub location: Location,
    #[serde(default, skip_serializing_if = "BlockMeta::is_empty")]
    pub meta: BlockMeta,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Paragraph {
    pub inlines: Vec<Inline>,
    pub location: Location,
    #[serde(default, skip_serializing_if = "BlockMeta::is_empty")]
    pub meta: BlockMeta,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct List {
    pub marker: ListMarker,
    pub items: Vec<ListItem>,
    pub location: Location,
    #[serde(default, skip_serializing_if = "BlockMeta::is_empty")]
    pub meta: BlockMeta,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ListMarker {
    Unordered,
    Ordered,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListItem {
    pub depth: u8,
    pub principal: Vec<Inline>,
    pub blocks: Vec<Block>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DescriptionList {
    pub items: Vec<DescriptionListItem>,
    pub location: Location,
    #[serde(default, skip_serializing_if = "BlockMeta::is_empty")]
    pub meta: BlockMeta,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DescriptionListItem {
    pub term: Vec<Inline>,
    pub description: Vec<Block>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DelimitedBlock {
    pub style: DelimitedStyle,
    pub content: DelimitedContent,
    pub location: Location,
    #[serde(default, skip_serializing_if = "BlockMeta::is_empty")]
    pub meta: BlockMeta,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DelimitedStyle {
    Listing,
    Literal,
    Example,
    Quote,
    Sidebar,
    Passthrough,
    Open,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "form", rename_all = "snake_case")]
pub enum DelimitedContent {
    /// Raw text with substitutions suppressed (listing, literal, passthrough).
    Raw { text: String },
    /// Nested blocks (example, quote, sidebar, open).
    Blocks { blocks: Vec<Block> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Table {
    pub rows: Vec<TableRow>,
    /// Column specifications parsed from `cols="…"` on the block-attribute
    /// line. Empty when no `cols=` was supplied — the renderer then falls
    /// back to equal column widths, no alignment override.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cols: Vec<ColumnSpec>,
    pub location: Location,
    #[serde(default, skip_serializing_if = "BlockMeta::is_empty")]
    pub meta: BlockMeta,
}

/// One entry in the `cols=` spec. A spec like `"1,2,<3"` produces three of
/// these — the width number is a relative weight (0 ⇒ unspecified) and
/// `h_align` carries the optional `<`/`^`/`>` from the spec.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ColumnSpec {
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub width: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub h_align: Option<HAlign>,
}

fn is_zero_u32(n: &u32) -> bool {
    *n == 0
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HAlign {
    Left,
    Center,
    Right,
}

/// A `[discrete]` heading: semantically a leaf — same syntax as a section
/// title but flat in the document tree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscreteHeading {
    pub level: u8,
    pub title: Vec<Inline>,
    pub location: Location,
    #[serde(default, skip_serializing_if = "BlockMeta::is_empty")]
    pub meta: BlockMeta,
}

/// Callout list — the `<N> description` siblings of a `[source]` / listing /
/// literal block. Each item carries the marker number that pairs it with a
/// `<N>` callout in the preceding block.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Colist {
    pub items: Vec<ColistItem>,
    pub location: Location,
    #[serde(default, skip_serializing_if = "BlockMeta::is_empty")]
    pub meta: BlockMeta,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColistItem {
    pub number: u32,
    pub inlines: Vec<Inline>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableRow {
    pub cells: Vec<TableCell>,
    #[serde(default, skip_serializing_if = "RowKind::is_default")]
    pub kind: RowKind,
}

/// Whether a row is part of the table header, body, or footer. Body is
/// the default; header is set by the `[%header]` option or the
/// "first-row-then-blank-line" heuristic. Footer is reserved for `[%footer]`
/// (parser support pending).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RowKind {
    Header,
    #[default]
    Body,
    Footer,
}

impl RowKind {
    pub fn is_default(&self) -> bool {
        matches!(self, RowKind::Body)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableCell {
    pub inlines: Vec<Inline>,
    /// Recursively-parsed blocks for `a|` (AsciiDoc) cells. Empty for any
    /// other cell style. When non-empty, renderers should ignore `inlines`
    /// and walk this list instead.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocks: Vec<Block>,
    /// Style override from a cell-formatter prefix (`a|`, `m|`, etc.).
    /// `None` means default rendering — plain inline content in a `<td>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<CellStyle>,
    /// Per-cell horizontal alignment from the `<`/`^`/`>` formatter prefix
    /// (e.g. `<m|content`). `None` falls back to the column-level alignment
    /// from `cols=`, which itself defaults to left.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub h_align: Option<HAlign>,
    /// Column span from a `N+|` cell-formatter prefix. Default is 1.
    #[serde(default = "one_u32", skip_serializing_if = "is_one_u32")]
    pub colspan: u32,
    /// Row span from a `.N+|` (or `M.N+|`) cell-formatter prefix. Default is 1.
    #[serde(default = "one_u32", skip_serializing_if = "is_one_u32")]
    pub rowspan: u32,
}

fn one_u32() -> u32 {
    1
}
fn is_one_u32(n: &u32) -> bool {
    *n == 1
}

/// Cell style from a formatter prefix on the `|` cell separator.
///
/// `AsciiDoc` cells re-parse their content as nested AsciiDoc blocks; the
/// parser does not yet honour that — `a|` cells currently degrade to
/// default style. `Header` forces `<th>` regardless of row kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CellStyle {
    AsciiDoc,
    Monospace,
    Strong,
    Emphasis,
    Header,
    Literal,
}

/// Inline content node.
///
/// All variants are struct-shaped so the AST round-trips cleanly through
/// internally-tagged JSON (`{"kind": "text", "value": "..."}`) — serde
/// requires struct or unit variants for that tagging mode, since the tag
/// is merged into the variant's serialized object.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Inline {
    Text {
        value: String,
    },
    Strong {
        children: Vec<Inline>,
    },
    Emphasis {
        children: Vec<Inline>,
    },
    Monospace {
        children: Vec<Inline>,
    },
    /// `~text~` — renders as `<sub>`.
    Subscript {
        children: Vec<Inline>,
    },
    /// `^text^` — renders as `<sup>`.
    Superscript {
        children: Vec<Inline>,
    },
    /// `#text#` (constrained) or `##text##` (unconstrained) — renders as `<mark>`.
    Highlight {
        children: Vec<Inline>,
    },
    Link {
        href: String,
        text: Vec<Inline>,
    },
    Xref {
        target: String,
        text: Option<Vec<Inline>>,
    },
    Image {
        target: String,
        alt: String,
        width: Option<String>,
        height: Option<String>,
    },
    /// `footnote:[text]` (anonymous) or `footnote:id[text]` (named).
    /// Numbering / end-of-doc footnote section is deferred — the renderer
    /// emits the text inline today.
    Footnote {
        #[serde(skip_serializing_if = "Option::is_none", default)]
        id: Option<String>,
        text: Vec<Inline>,
    },
    AttributeRef {
        name: String,
    },
    LineBreak,
    /// `+text+` (constrained) or `++text++` (unconstrained) — emits the
    /// text with HTML special-character escaping but no further inline
    /// substitutions applied.
    Passthrough {
        value: String,
    },
    /// `pass:[text]` — emitted verbatim, no escaping. Use to inject raw HTML.
    RawHtml {
        value: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SourceId(pub u32);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Location {
    pub source: SourceId,
    pub byte_start: u32,
    pub byte_end: u32,
    pub line: u32,
    pub column: u32,
}

impl Location {
    pub fn synthetic() -> Self {
        Self {
            source: SourceId(0),
            byte_start: 0,
            byte_end: 0,
            line: 0,
            column: 0,
        }
    }
}

pub trait Converter {
    fn convert(&self, doc: &Document) -> Result<String, ConvertError>;
}

/// Doc-wide ID registry — every section / block / inline anchor /
/// bibliography ID, collected in a single AST walk so cross-reference
/// resolution and validation can run without re-walking the tree.
///
/// Built once per document (after parsing, before converting). Membership
/// is checked via [`IdRegistry::contains`].
#[derive(Debug, Default, Clone)]
pub struct IdRegistry {
    ids: std::collections::BTreeSet<String>,
}

impl IdRegistry {
    pub fn collect(doc: &Document) -> Self {
        let mut reg = IdRegistry::default();
        reg.walk_blocks(&doc.blocks);
        reg
    }

    pub fn contains(&self, id: &str) -> bool {
        self.ids.contains(id)
    }

    pub fn ids(&self) -> impl Iterator<Item = &str> {
        self.ids.iter().map(String::as_str)
    }

    fn walk_blocks(&mut self, blocks: &[Block]) {
        for b in blocks {
            self.walk_block(b);
        }
    }

    fn walk_block(&mut self, b: &Block) {
        match b {
            Block::Section(s) => {
                self.add_meta(&s.meta);
                self.walk_inlines(&s.title);
                self.walk_blocks(&s.blocks);
            }
            Block::Paragraph(p) => {
                self.add_meta(&p.meta);
                self.walk_inlines(&p.inlines);
            }
            Block::List(l) => {
                self.add_meta(&l.meta);
                for item in &l.items {
                    self.walk_inlines(&item.principal);
                    self.walk_blocks(&item.blocks);
                }
            }
            Block::DescriptionList(d) => {
                self.add_meta(&d.meta);
                for item in &d.items {
                    self.walk_inlines(&item.term);
                    self.walk_blocks(&item.description);
                }
            }
            Block::Delimited(d) => {
                self.add_meta(&d.meta);
                if let DelimitedContent::Blocks { blocks } = &d.content {
                    self.walk_blocks(blocks);
                }
            }
            Block::Table(t) => {
                self.add_meta(&t.meta);
                for row in &t.rows {
                    for cell in &row.cells {
                        self.walk_inlines(&cell.inlines);
                        self.walk_blocks(&cell.blocks);
                    }
                }
            }
            Block::Colist(c) => {
                self.add_meta(&c.meta);
                for item in &c.items {
                    self.walk_inlines(&item.inlines);
                }
            }
            Block::DiscreteHeading(d) => {
                self.add_meta(&d.meta);
                self.walk_inlines(&d.title);
            }
        }
    }

    fn add_meta(&mut self, meta: &BlockMeta) {
        if let Some(id) = &meta.id {
            self.ids.insert(id.clone());
        }
    }

    fn walk_inlines(&mut self, inlines: &[Inline]) {
        for i in inlines {
            self.walk_inline(i);
        }
    }

    fn walk_inline(&mut self, i: &Inline) {
        match i {
            Inline::Strong { children }
            | Inline::Emphasis { children }
            | Inline::Monospace { children }
            | Inline::Subscript { children }
            | Inline::Superscript { children }
            | Inline::Highlight { children } => self.walk_inlines(children),
            Inline::Link { text, .. } => self.walk_inlines(text),
            Inline::Footnote { text, .. } => self.walk_inlines(text),
            Inline::Xref { text: Some(t), .. } => self.walk_inlines(t),
            Inline::RawHtml { value } => {
                // `anchor:id[]`, `[[[id]]]`, and similar inline macros
                // emit `<a id="…">` directly. Pull every `id="…"` out
                // of the rendered fragment so they're discoverable from
                // the registry without a typed AST variant.
                self.scan_id_attrs(value);
            }
            Inline::Text { .. }
            | Inline::Xref { text: None, .. }
            | Inline::Image { .. }
            | Inline::AttributeRef { .. }
            | Inline::LineBreak
            | Inline::Passthrough { .. } => {}
        }
    }

    fn scan_id_attrs(&mut self, html: &str) {
        let mut i = 0;
        let bytes = html.as_bytes();
        while i + 4 <= bytes.len() {
            if &bytes[i..i + 4] == b"id=\"" {
                let start = i + 4;
                if let Some(end_rel) = html[start..].find('"') {
                    let id = &html[start..start + end_rel];
                    if !id.is_empty() {
                        self.ids.insert(id.to_string());
                    }
                    i = start + end_rel + 1;
                    continue;
                }
            }
            i += 1;
        }
    }
}

/// Render an inline sequence to plain text.
///
/// Drops formatting markup, preserves the text content of links / xrefs /
/// footnotes / images. Used by ID derivation, the `<title>` element, the
/// TOC pre-walk, and any other site that needs the human-readable form
/// of inline content. Centralised here so id-generation and the converter
/// can't drift.
pub fn inlines_to_plain(inlines: &[Inline]) -> String {
    let mut out = String::new();
    for i in inlines {
        write_plain(&mut out, i);
    }
    out
}

fn write_plain(out: &mut String, inline: &Inline) {
    use std::fmt::Write;
    match inline {
        Inline::Text { value } => out.push_str(value),
        Inline::Strong { children }
        | Inline::Emphasis { children }
        | Inline::Monospace { children }
        | Inline::Subscript { children }
        | Inline::Superscript { children }
        | Inline::Highlight { children } => {
            for child in children {
                write_plain(out, child);
            }
        }
        Inline::Link { text, .. } => {
            for child in text {
                write_plain(out, child);
            }
        }
        Inline::Xref {
            target,
            text: Some(t),
        } => {
            if t.is_empty() {
                out.push_str(target);
            } else {
                for child in t {
                    write_plain(out, child);
                }
            }
        }
        Inline::Xref { target, text: None } => out.push_str(target),
        Inline::Image { alt, .. } => out.push_str(alt),
        Inline::Footnote { text, .. } => {
            for child in text {
                write_plain(out, child);
            }
        }
        Inline::AttributeRef { name } => {
            let _ = write!(out, "{{{name}}}");
        }
        Inline::LineBreak => out.push(' '),
        Inline::Passthrough { value } => out.push_str(value),
        Inline::RawHtml { .. } => {}
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConvertError {
    #[error("conversion failed: {0}")]
    Message(String),
}
