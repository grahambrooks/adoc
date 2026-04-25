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

#[derive(Debug, thiserror::Error)]
pub enum ConvertError {
    #[error("conversion failed: {0}")]
    Message(String),
}
