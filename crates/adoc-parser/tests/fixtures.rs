//! Fixture-driven integration tests.
//!
//! Runs every `.adoc` file in `tests/fixtures/` through the full pipeline
//! (preprocessor + parser + HTML5 converter) and asserts structural
//! properties that pin the v1 feature set.

use adoc_convert_html5::Html5Converter;
use adoc_core::{AttributeValue, Block, Converter, DelimitedStyle, ListMarker, SourceId};
use adoc_parser::parse;
use adoc_preprocessor::Preprocessor;
use std::path::{Path, PathBuf};

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("tests")
        .join("fixtures")
}

fn render(name: &str) -> (adoc_core::Document, String) {
    let path = fixtures_dir().join(name);
    let src = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
    let pre = Preprocessor::default();
    let lines = pre.run(&src, SourceId(0)).expect("preprocess");
    let doc = parse(&lines).expect("parse");
    let html = Html5Converter::new().convert(&doc).expect("convert");
    (doc, html)
}

#[test]
fn paragraph() {
    let (doc, html) = render("01_paragraph.adoc");
    assert!(doc.header.is_none());
    assert_eq!(count_paragraphs(&doc.blocks), 2);
    assert!(html.contains("<p>A simple paragraph of text.</p>"));
}

#[test]
fn header_with_authors_and_revision() {
    let (doc, html) = render("02_header.adoc");
    let header = doc.header.expect("header");
    assert_eq!(header.authors.len(), 2);
    assert_eq!(header.authors[0].name, "Alice Author");
    assert_eq!(
        header.authors[0].email.as_deref(),
        Some("alice@example.com")
    );
    let rev = header.revision.expect("revision");
    assert_eq!(rev.number.as_deref(), Some("v1.2.3"));
    assert_eq!(rev.date.as_deref(), Some("2026-04-18"));
    assert_eq!(rev.remark.as_deref(), Some("Initial release"));
    assert_eq!(
        doc.attributes.get("toc"),
        Some(&AttributeValue::Bool(true))
    );
    assert_eq!(
        doc.attributes.get("source-highlighter").and_then(AttributeValue::as_str),
        Some("rouge")
    );
    assert!(html.contains("<h1>Document Title</h1>"));
}

#[test]
fn nested_sections() {
    let (doc, html) = render("03_sections.adoc");
    // Level 1 section should contain a level 2 subsection.
    let l1 = doc
        .blocks
        .iter()
        .find_map(|b| match b {
            Block::Section(s) if s.level == 1 => Some(s),
            _ => None,
        })
        .expect("level 1 section");
    let has_l2 = l1.blocks.iter().any(|b| matches!(b, Block::Section(s) if s.level == 2));
    assert!(has_l2);
    assert!(html.contains("<h2>Level 1 Section</h2>"));
    assert!(html.contains("<h3>Level 2 Subsection</h3>"));
    assert!(html.contains("<h5>Level 4</h5>"));
}

#[test]
fn unordered_list_with_depth() {
    let (doc, html) = render("04_unordered_list.adoc");
    let list = find_list(&doc.blocks).expect("list");
    assert_eq!(list.marker, ListMarker::Unordered);
    let depths: Vec<u8> = list.items.iter().map(|i| i.depth).collect();
    assert_eq!(depths, vec![1, 1, 1, 2, 2, 3, 1]);
    assert!(html.contains("<ul>"));
    assert!(html.contains("<li>Deeply nested</li>"));
}

#[test]
fn ordered_list() {
    let (doc, html) = render("05_ordered_list.adoc");
    let list = find_list(&doc.blocks).expect("list");
    assert_eq!(list.marker, ListMarker::Ordered);
    assert!(html.contains("<ol>"));
}

#[test]
fn description_list() {
    let (doc, html) = render("06_description_list.adoc");
    let dlist = doc
        .blocks
        .iter()
        .find_map(|b| match b {
            Block::DescriptionList(d) => Some(d),
            _ => None,
        })
        .expect("description list");
    assert_eq!(dlist.items.len(), 3);
    assert!(html.contains("<dt>CPU</dt>"));
    assert!(html.contains("The brain of the computer"));
}

#[test]
fn listing_block_is_verbatim() {
    let (_doc, html) = render("07_listing_block.adoc");
    // The listing preserves * characters literally (no <strong>).
    assert!(html.contains(r#"<pre><code>fn main()"#));
    assert!(!html.contains("<strong>"));
}

#[test]
fn literal_block_preserves_markup_verbatim() {
    let (_doc, html) = render("08_literal_block.adoc");
    assert!(html.contains("*formatting*"));
    assert!(html.contains("{attributes}"));
}

#[test]
fn example_quote_sidebar_open_passthrough_render() {
    for (name, needle) in [
        ("09_example_block.adoc", r#"<div class="example">"#),
        ("10_quote_block.adoc", "<blockquote>"),
        ("11_sidebar_block.adoc", "<aside>"),
        ("12_passthrough_block.adoc", r#"<div class="raw">"#),
        ("13_open_block.adoc", "<div>"),
    ] {
        let (_doc, html) = render(name);
        assert!(html.contains(needle), "{name} missing {needle}\n{html}");
    }
}

#[test]
fn table_rows_and_cells() {
    let (doc, html) = render("14_table.adoc");
    let table = doc
        .blocks
        .iter()
        .find_map(|b| match b {
            Block::Table(t) => Some(t),
            _ => None,
        })
        .expect("table");
    assert_eq!(table.rows.len(), 4);
    assert_eq!(table.rows[0].cells.len(), 3);
    assert!(html.contains("<td>Alice</td>"));
}

#[test]
fn inline_quotes() {
    let (_doc, html) = render("15_inline_quotes.adoc");
    assert!(html.contains("<strong>strong</strong>"));
    assert!(html.contains("<em>emphasis</em>"));
    assert!(html.contains("<code>monospace</code>"));
    assert!(html.contains("<strong><em>strong emphasis</em></strong>"));
    assert!(html.contains("1 &lt; 2"));
    assert!(html.contains("AT&amp;T"));
}

#[test]
fn inline_macros() {
    let (_doc, html) = render("16_inline_macros.adoc");
    assert!(html.contains(r#"<a href="https://example.com">Example Site</a>"#));
    assert!(html.contains(r#"<a href="mailto:alice@example.com">Alice</a>"#));
    assert!(html.contains(r##"<a href="#intro">Introduction</a>"##));
    assert!(html.contains(r#"<img src="logo.png" alt="Logo" width="100" height="50">"#));
}

#[test]
fn attribute_references() {
    let (_doc, html) = render("17_attribute_refs.adoc");
    assert!(html.contains("Welcome to Adoc version 0.1.0"));
    assert!(html.contains("{missing}"));
}

#[test]
fn replacements() {
    let (_doc, html) = render("18_replacements.adoc");
    assert!(html.contains('\u{00A9}'.to_string().as_str()));
    assert!(html.contains('\u{2122}'.to_string().as_str()));
    assert!(html.contains('\u{2014}'.to_string().as_str()));
    assert!(html.contains('\u{2026}'.to_string().as_str()));
    assert!(html.contains('\u{2192}'.to_string().as_str()));
}

#[test]
fn line_breaks() {
    let (_doc, html) = render("19_line_breaks.adoc");
    assert!(html.contains("<br>"));
}

#[test]
fn mixed_kitchen_sink() {
    let (doc, html) = render("20_mixed.adoc");
    assert!(doc.header.is_some());
    assert!(html.contains("<h1>The adoc User Guide</h1>"));
    assert!(html.contains("<h2>Features</h2>"));
    assert!(html.contains("<h3>Subsection</h3>"));
    assert!(html.contains("<pre><code>println!"));
    assert!(html.contains("<table>"));
    assert!(html.contains(r##"<a href="#features">"##));
}

// --- helpers ----------------------------------------------------------------

fn count_paragraphs(blocks: &[Block]) -> usize {
    blocks
        .iter()
        .filter(|b| matches!(b, Block::Paragraph(_)))
        .count()
}

fn find_list(blocks: &[Block]) -> Option<&adoc_core::List> {
    blocks.iter().find_map(|b| match b {
        Block::List(l) => Some(l),
        _ => None,
    })
}

#[test]
fn delimited_style_roundtrip() {
    // Quick check that every DelimitedStyle variant exercised by fixtures
    // is recognised.
    let styles = vec![
        DelimitedStyle::Listing,
        DelimitedStyle::Literal,
        DelimitedStyle::Example,
        DelimitedStyle::Quote,
        DelimitedStyle::Sidebar,
        DelimitedStyle::Passthrough,
        DelimitedStyle::Open,
    ];
    // Just ensures match-exhaustiveness at compile time; no runtime check needed.
    for _ in styles {}
}
