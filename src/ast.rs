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
    pub location: Location,
    #[serde(default, skip_serializing_if = "BlockMeta::is_empty")]
    pub meta: BlockMeta,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableRow {
    pub cells: Vec<TableCell>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableCell {
    pub inlines: Vec<Inline>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Inline {
    Text(String),
    Strong(Vec<Inline>),
    Emphasis(Vec<Inline>),
    Monospace(Vec<Inline>),
    /// `~text~` — renders as `<sub>`.
    Subscript(Vec<Inline>),
    /// `^text^` — renders as `<sup>`.
    Superscript(Vec<Inline>),
    /// `#text#` (constrained) or `##text##` (unconstrained) — renders as `<mark>`.
    Highlight(Vec<Inline>),
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
    AttributeRef(String),
    LineBreak,
    /// `+text+` (constrained) or `++text++` (unconstrained) — emits the
    /// text with HTML special-character escaping but no further inline
    /// substitutions applied.
    Passthrough(String),
    /// `pass:[text]` — emitted verbatim, no escaping. Use to inject raw HTML.
    RawHtml(String),
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
