//! Shared types for the adoc AsciiDoc toolchain.
//!
//! The AST produced by `adoc-parser` and consumed by converters lives here.
//! Every node carries a [`Location`] so diagnostics can point back to source.
//! Types derive `serde` so the AST can round-trip through stdio for future
//! external filters.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A parsed AsciiDoc document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    pub header: Option<Header>,
    pub attributes: Attributes,
    pub blocks: Vec<Block>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Header {
    pub title: Option<Vec<Inline>>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AttributeValue {
    Bool(bool),
    String(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Block {
    Section(Section),
    Paragraph(Paragraph),
    List(List),
    Delimited(DelimitedBlock),
    Table(Table),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Section {
    pub level: u8,
    pub title: Vec<Inline>,
    pub id: Option<String>,
    pub blocks: Vec<Block>,
    pub location: Location,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Paragraph {
    pub inlines: Vec<Inline>,
    pub location: Location,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct List {
    pub marker: ListMarker,
    pub items: Vec<ListItem>,
    pub location: Location,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ListMarker {
    Unordered,
    Ordered,
    Description,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListItem {
    pub principal: Vec<Inline>,
    pub blocks: Vec<Block>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DelimitedBlock {
    pub style: DelimitedStyle,
    pub content: String,
    pub location: Location,
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
pub struct Table {
    pub rows: Vec<Vec<Vec<Block>>>,
    pub location: Location,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Inline {
    Text(String),
    Strong(Vec<Inline>),
    Emphasis(Vec<Inline>),
    Monospace(Vec<Inline>),
    Link { href: String, text: Vec<Inline> },
    Xref { target: String, text: Option<Vec<Inline>> },
    AttributeRef(String),
    LineBreak,
}

/// Identifies a source file in a [`SourceMap`]. See [`Location`].
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

/// Output backend trait. Implemented by each `adoc-convert-*` crate.
pub trait Converter {
    fn convert(&self, doc: &Document) -> Result<String, ConvertError>;
}

#[derive(Debug, thiserror::Error)]
pub enum ConvertError {
    #[error("conversion failed: {0}")]
    Message(String),
}
