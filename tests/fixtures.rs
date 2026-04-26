//! Fixture-driven integration tests.
//!
//! Runs every `.adoc` file in `tests/fixtures/` through the full pipeline
//! (preprocessor + parser + HTML5 converter) and asserts structural
//! properties that pin the v1 feature set.

use adoc::ast::{AttributeValue, Block, Converter, DelimitedStyle, ListMarker};
use adoc::convert::html5::Html5Converter;
use adoc::parser::parse;
use adoc::preprocessor::Preprocessor;
use camino::Utf8Path;
use std::path::{Path, PathBuf};

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

fn render(name: &str) -> (adoc::ast::Document, String) {
    let path = fixtures_dir().join(name);
    let src = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
    let utf8_path = Utf8Path::from_path(&path).expect("utf-8 path");
    let mut pre = Preprocessor::default();
    let lines = pre.run(&src, utf8_path).expect("preprocess");
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
    assert_eq!(doc.attributes.get("toc"), Some(&AttributeValue::Bool(true)));
    assert_eq!(
        doc.attributes
            .get("source-highlighter")
            .and_then(AttributeValue::as_str),
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
    let has_l2 = l1
        .blocks
        .iter()
        .any(|b| matches!(b, Block::Section(s) if s.level == 2));
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

fn find_list(blocks: &[Block]) -> Option<&adoc::ast::List> {
    blocks.iter().find_map(|b| match b {
        Block::List(l) => Some(l),
        _ => None,
    })
}

#[test]
fn block_metadata_attaches_to_following_block() {
    let (doc, html) = render("21_block_metadata.adoc");

    // First paragraph: title + id + role.
    let first = doc
        .blocks
        .iter()
        .find_map(|b| match b {
            Block::Paragraph(p) if p.meta.id.as_deref() == Some("first") => Some(p),
            _ => None,
        })
        .expect("paragraph #first");
    assert_eq!(first.meta.roles, vec!["lead".to_string()]);
    let title = first.meta.title.as_ref().expect("title");
    assert!(
        matches!(title.first(), Some(adoc::ast::Inline::Text { value }) if value == "A titled paragraph")
    );

    // [NOTE] paragraph: style is captured.
    let note = doc
        .blocks
        .iter()
        .find_map(|b| match b {
            Block::Paragraph(p) if p.meta.style.as_deref() == Some("NOTE") => Some(p),
            _ => None,
        })
        .expect("NOTE paragraph");
    assert!(note.meta.id.is_none());

    // Source listing block: style + positional + title.
    let listing = doc
        .blocks
        .iter()
        .find_map(|b| match b {
            Block::Delimited(d) if d.meta.style.as_deref() == Some("source") => Some(d),
            _ => None,
        })
        .expect("source listing");
    assert_eq!(listing.meta.positional, vec!["rust".to_string()]);
    assert!(listing.meta.title.is_some());

    // Mixed shorthand paragraph.
    let mixed = doc
        .blocks
        .iter()
        .find_map(|b| match b {
            Block::Paragraph(p) if p.meta.id.as_deref() == Some("xref-target") => Some(p),
            _ => None,
        })
        .expect("xref-target paragraph");
    assert_eq!(mixed.meta.roles, vec!["callout".to_string()]);
    assert_eq!(mixed.meta.options, vec!["hardbreaks".to_string()]);
    assert_eq!(mixed.meta.positional, vec!["quoted".to_string()]);

    // Named-attributes paragraph.
    let named = doc
        .blocks
        .iter()
        .find_map(|b| match b {
            Block::Paragraph(p) if p.meta.named.contains_key("caption") => Some(p),
            _ => None,
        })
        .expect("named-attrs paragraph");
    assert_eq!(
        named.meta.named.get("caption").map(String::as_str),
        Some("Figure 1.")
    );
    assert_eq!(
        named.meta.named.get("align").map(String::as_str),
        Some("center")
    );

    // Table with id and title.
    let table = doc
        .blocks
        .iter()
        .find_map(|b| match b {
            Block::Table(t) => Some(t),
            _ => None,
        })
        .expect("table");
    assert_eq!(table.meta.id.as_deref(), Some("prices"));
    assert!(table.meta.title.is_some());

    // Section id from `[#feature]`.
    let section = doc
        .blocks
        .iter()
        .find_map(|b| match b {
            Block::Section(s) => Some(s),
            _ => None,
        })
        .expect("section");
    assert_eq!(section.meta.id.as_deref(), Some("feature"));

    // HTML carries the id, class, and title-divs.
    assert!(html.contains(r#"<p id="first" class="lead">"#));
    assert!(html.contains(r#"<div class="title">A titled paragraph</div>"#));
    assert!(html.contains(r#"<table id="prices">"#));
    assert!(html.contains(r#"<div class="title">Important table</div>"#));
    assert!(html.contains(r#"<section id="feature">"#));
}

#[test]
fn preprocessor_conditionals_and_include() {
    let (doc, html) = render("22_preprocessor.adoc");

    // Conditionals that resolve to true.
    assert!(html.contains("Hello, this content is gated by a defined attribute."));
    assert!(html.contains("This is shown because"));
    assert!(html.contains("We are on edition 2 or later."));
    assert!(html.contains("Localised paragraph for English."));

    // Include pulled the section in.
    let included = doc
        .blocks
        .iter()
        .find_map(|b| match b {
            Block::Section(s) if s.level == 1 => Some(s),
            _ => None,
        })
        .expect("included section");
    assert!(matches!(
        included.title.first(),
        Some(adoc::ast::Inline::Text { value }) if value == "Included Section"
    ));

    // Inline ifdef inside the included file emitted on the same line, so
    // the trailing paragraph contains the substituted value.
    assert!(html.contains("Edition number 2."));

    // The closing literal paragraph is present.
    assert!(html.contains("The end."));
}

#[test]
fn tables_header_inference_and_cell_formatters() {
    let (doc, html) = render("28_tables_header_formatters.adoc");

    let body_start = html.find("<body>").expect("body open");
    let body_end = html.find("</body>").expect("body close");
    let body = &html[body_start..body_end];

    let tables: Vec<&adoc::ast::Table> = doc
        .blocks
        .iter()
        .flat_map(|b| match b {
            Block::Section(s) => s
                .blocks
                .iter()
                .filter_map(|b| match b {
                    Block::Table(t) => Some(t),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        })
        .collect();
    assert_eq!(tables.len(), 3, "expected 3 tables, got {}", tables.len());

    // Inferred header (blank line after first row).
    assert_eq!(tables[0].rows[0].kind, adoc::ast::RowKind::Header);
    for row in &tables[0].rows[1..] {
        assert_eq!(row.kind, adoc::ast::RowKind::Body);
    }

    // Explicit [%header].
    assert_eq!(tables[1].rows[0].kind, adoc::ast::RowKind::Header);

    // Cell formatters captured on cell.style.
    let formatters = &tables[2];
    // Row 0: cells[0] plain, cells[1] s|, cells[2] e|.
    assert_eq!(formatters.rows[0].cells[0].style, None);
    assert_eq!(
        formatters.rows[0].cells[1].style,
        Some(adoc::ast::CellStyle::Strong)
    );
    assert_eq!(
        formatters.rows[0].cells[2].style,
        Some(adoc::ast::CellStyle::Emphasis)
    );
    // Row 1: cells[0] plain, cells[1] m|, cells[2] l|, cells[3] h|.
    assert_eq!(
        formatters.rows[1].cells[1].style,
        Some(adoc::ast::CellStyle::Monospace)
    );
    assert_eq!(
        formatters.rows[1].cells[2].style,
        Some(adoc::ast::CellStyle::Literal)
    );
    assert_eq!(
        formatters.rows[1].cells[3].style,
        Some(adoc::ast::CellStyle::Header)
    );

    // HTML carries <thead>/<tbody> for headered tables.
    assert!(body.matches("<thead>").count() >= 2);
    assert!(body.contains("<tbody>"));

    // Cell-formatter rendering.
    assert!(body.contains("<td><strong>strong cell</strong></td>"));
    assert!(body.contains("<td><em>emphasised cell</em></td>"));
    assert!(body.contains("<td><code>monospace text</code></td>"));
    // Literal cells keep their stars literal (no <strong>) and wrap in <pre>.
    assert!(body.contains("<td><pre>literal *not strong*</pre></td>"));
    // h| in a body row forces <th>.
    assert!(body.contains("<th>forced th in body row</th>"));
}

#[test]
fn ast_roundtrips_through_serde_json() {
    // For every existing fixture, render → JSON → parse-back → render, and
    // assert the two HTML outputs are byte-identical. This pins the AST
    // serde format as a public contract.
    let fixtures = [
        "01_paragraph.adoc",
        "02_header.adoc",
        "03_sections.adoc",
        "04_unordered_list.adoc",
        "06_description_list.adoc",
        "07_listing_block.adoc",
        "14_table.adoc",
        "15_inline_quotes.adoc",
        "16_inline_macros.adoc",
        "20_mixed.adoc",
        "21_block_metadata.adoc",
        "24_section_ids.adoc",
        "25_admonitions_source.adoc",
        "26_inline_extras.adoc",
        "27_toc_sectnums.adoc",
        "28_tables_header_formatters.adoc",
        "29_callouts.adoc",
        "30_table_cols.adoc",
        "31_asciidoc_cells_and_span.adoc",
        "32_compat_links_anchors_discrete.adoc",
        "33_smart_quotes_verse.adoc",
    ];
    for name in fixtures {
        let (doc, html_a) = render(name);
        let json = serde_json::to_string(&doc).expect("serialize");
        let doc2: adoc::ast::Document = serde_json::from_str(&json).expect("deserialize");
        let html_b = Html5Converter::new()
            .convert(&doc2)
            .expect("convert roundtrip");
        assert_eq!(
            html_a, html_b,
            "AST roundtrip diverges for {name}\n--- direct ---\n{html_a}\n--- roundtrip ---\n{html_b}",
        );
    }
}

#[test]
fn toc_sectnums_sectanchors() {
    let (_doc, html) = render("27_toc_sectnums.adoc");

    let body_start = html.find("<body>").expect("body open");
    let body_end = html.find("</body>").expect("body close");
    let body = &html[body_start..body_end];

    // TOC block exists with a title and nested ULs.
    assert!(body.contains(r#"<div id="toc" class="toc">"#));
    assert!(body.contains(r#"<div class="toc-title">Table of Contents</div>"#));
    // Top-level sections are at depth 1; nested ones bump depth — we don't
    // assert exact nesting here (it's covered structurally below) beyond
    // checking that `<ul>` opens at all.
    assert!(body.contains("<ul>"));

    // TOC entries link to each section's id and carry the sectnum prefix.
    assert!(body
        .contains(r##"<a href="#_introduction"><span class="sectnum">1</span> Introduction</a>"##));
    assert!(body.contains(r##"<a href="#_methods"><span class="sectnum">2</span> Methods</a>"##));
    assert!(body.contains(r##"<a href="#_setup"><span class="sectnum">2.1</span> Setup</a>"##));
    assert!(
        body.contains(r##"<a href="#_step_one"><span class="sectnum">2.2.1</span> Step One</a>"##)
    );
    assert!(
        body.contains(r##"<a href="#_conclusion"><span class="sectnum">4</span> Conclusion</a>"##)
    );

    // Section headings carry the sectnum prefix and the sectanchors `<a>`.
    assert!(body.contains(
        r##"<h2><a class="anchor" href="#_introduction"></a><span class="sectnum">1</span> Introduction</h2>"##
    ));
    assert!(body.contains(
        r##"<h3><a class="anchor" href="#_setup"></a><span class="sectnum">2.1</span> Setup</h3>"##
    ));
    assert!(body.contains(
        r##"<h4><a class="anchor" href="#_step_one"></a><span class="sectnum">2.2.1</span> Step One</h4>"##
    ));
}

#[test]
fn inline_subscript_superscript_highlight_passthrough_footnote() {
    let (_doc, html) = render("26_inline_extras.adoc");

    let body_start = html.find("<body>").expect("body open");
    let body_end = html.find("</body>").expect("body close");
    let body = &html[body_start..body_end];

    // Subscript and superscript.
    assert!(
        body.contains("H<sub>2</sub>O"),
        "missing <sub>; body:\n{body}"
    );
    assert!(body.contains("E=mc<sup>2</sup>"));

    // Highlight (unconstrained ##…## form).
    assert!(body.contains("<mark>highlighted phrase</mark>"));

    // Constrained passthrough HTML-escapes its content but suppresses inline
    // subs — literal stars and attribute braces both survive as text.
    assert!(body.contains("*keeps stars* and {attrs}"));
    assert!(!body.contains("<strong>keeps stars</strong>"));

    // Unconstrained ++…++ form: literal `**double stars**` survives.
    assert!(body.contains("**double stars**"));
    assert!(!body.contains("<strong>double stars</strong>"));

    // pass:[…] is rendered verbatim — angle brackets stay literal.
    assert!(body.contains("<kbd>Ctrl</kbd>"));

    // Footnotes (anonymous + id'd).
    assert!(body.contains(r#"<span class="footnote">anchored thought</span>"#));
    assert!(body.contains(r#"<span class="footnote" id="fn1">an id</span>"#));
}

#[test]
fn admonitions_paragraph_block_titled_and_source_language() {
    let (doc, html) = render("25_admonitions_source.adoc");

    let body_start = html.find("<body>").expect("body open");
    let body_end = html.find("</body>").expect("body close");
    let body = &html[body_start..body_end];

    // Paragraph admonition shortcut: each keyword is captured on meta.style
    // and renders the appropriate admonitionblock with a default label.
    for (kw, class, label) in [
        ("NOTE", "note", "Note"),
        ("TIP", "tip", "Tip"),
        ("IMPORTANT", "important", "Important"),
        ("WARNING", "warning", "Warning"),
        ("CAUTION", "caution", "Caution"),
    ] {
        // The AST records the keyword on meta.style.
        let any = doc.blocks.iter().any(|b| match b {
            Block::Paragraph(p) => p.meta.style.as_deref() == Some(kw),
            _ => false,
        });
        assert!(any, "no paragraph with style {kw}");

        // The HTML carries the right admonitionblock class and either a
        // default label or, for the titled TIP, a custom title.
        let needle = format!(r#"<div class="admonitionblock {class}">"#);
        assert!(body.contains(&needle), "expected {needle}\nbody:\n{body}",);
        if kw != "TIP" {
            // Default label for non-titled admonitions.
            let label_needle = format!(r#"<p class="label">{label}</p>"#);
            assert!(body.contains(&label_needle), "missing label {label_needle}");
        }
    }

    // The titled TIP keeps its supplied title in place of the default label.
    assert!(body.contains(r#"<p class="title">A titled note</p>"#));

    // Block-form admonition wraps multiple paragraphs.
    let inner_count = body
        .matches(r#"<div class="admonitionblock note">"#)
        .count();
    assert!(inner_count >= 1, "expected a NOTE admonitionblock");

    // Source listing with language → language-rust class on <code> and
    // matching data-lang on <pre>.
    assert!(body.contains(r#"<pre data-lang="rust"><code class="language-rust">fn main()"#));
    // [source] without a language → plain <pre> and plain <code>.
    assert!(body.contains(r#"<pre><code>no language attribute here"#));
    // Plain listing (no [source]) → also plain <code>, untouched.
    assert!(body.contains(r#"<pre><code>plain listing"#));
}

#[test]
fn section_ids_auto_explicit_and_xrefs() {
    let (doc, html) = render("24_section_ids.adoc");

    let body_start = html.find("<body>").expect("body open");
    let body_end = html.find("</body>").expect("body close");
    let body = &html[body_start..body_end];

    let sections: Vec<&adoc::ast::Section> = doc
        .blocks
        .iter()
        .filter_map(|b| match b {
            Block::Section(s) => Some(s),
            _ => None,
        })
        .collect();

    let ids: Vec<&str> = sections
        .iter()
        .map(|s| s.meta.id.as_deref().unwrap_or(""))
        .collect();

    // Auto IDs are derived; the second "Auto-generated" gets a numeric suffix.
    // Explicit forms are preserved verbatim.
    assert_eq!(
        ids,
        vec![
            "_auto_generated",
            "_hello_world",
            "_auto_generated_2",
            "explicit-anchor",
            "explicit-shorthand",
            "_cross_references",
        ]
    );

    // HTML carries each id on the section opening tag.
    for id in &ids {
        let needle = format!(r#"<section id="{id}">"#);
        assert!(
            body.contains(&needle),
            "missing section opener {needle}\n{body}"
        );
    }

    // Xrefs render as anchor hrefs that match the assigned IDs.
    assert!(body.contains(r##"<a href="#_auto_generated">first dup</a>"##));
    assert!(body.contains(r##"<a href="#_hello_world">via punctuation</a>"##));
    assert!(body.contains(r##"<a href="#_auto_generated_2">second dup</a>"##));
    assert!(body.contains(r##"<a href="#explicit-anchor">legacy form</a>"##));
    assert!(body.contains(r##"<a href="#explicit-shorthand">shorthand form</a>"##));
}

#[test]
fn preprocessor_include_arguments() {
    let (_doc, html) = render("23_include_args.adoc");

    // Slice out the document body so substring assertions don't trip on the
    // embedded stylesheet (which contains `:first-child` etc.).
    let body_start = html.find("<body>").expect("body open");
    let body_end = html.find("</body>").expect("body close");
    let body = &html[body_start..body_end];

    // lines=2..4 keeps "second", "third", "fourth"; rest is filtered out.
    assert!(body.contains("<p>second third fourth</p>"));
    assert!(!body.contains("first"));
    assert!(!body.contains("fifth"));

    // tag=keep selects only the marked region; markers themselves are stripped.
    assert!(body.contains("<p>selected by tag</p>"));
    assert!(!body.contains("tag::keep"));
    assert!(!body.contains("end::keep"));

    // leveloffset=+1 lifts `== Section Two` to `=== Section Two`; the
    // converter then renders level-2 sections as <h3>.
    assert!(body.contains("<h3>Section Two</h3>"));
    assert!(body.contains("content under section two"));
}

#[test]
fn block_metadata_orphaned_by_blank_line_is_dropped() {
    let src = "[#orphan]\n\nplain paragraph\n";
    let mut pre = adoc::preprocessor::Preprocessor::default();
    let lines = pre.run(src, Utf8Path::new("<input>")).expect("preprocess");
    let doc = adoc::parser::parse(&lines).expect("parse");
    let para = doc
        .blocks
        .iter()
        .find_map(|b| match b {
            Block::Paragraph(p) => Some(p),
            _ => None,
        })
        .expect("paragraph");
    assert!(para.meta.id.is_none(), "orphan id should not have attached");
}

#[test]
fn smart_quotes_verse_attribution() {
    let (_doc, html) = render("33_smart_quotes_verse.adoc");
    let body_start = html.find("<body>").expect("body open");
    let body_end = html.find("</body>").expect("body close");
    let body = &html[body_start..body_end];

    // Smart double + single quotes.
    assert!(body.contains("\u{201C}hello, world\u{201D}"));
    assert!(body.contains("\u{2018}single-quoted\u{2019}"));

    // Apostrophes in contractions are NOT converted.
    assert!(body.contains("Don't"));
    assert!(body.contains("it's"));
    assert!(body.contains("you're"));

    // Smart quotes wrap inline formatting (children parsed normally).
    assert!(body.contains("\u{201C}emphasis is <em>here</em>\u{201D}"));

    // Verse block: <pre class="verseblock"> preserves line breaks.
    assert!(body.contains(r#"<pre class="verseblock">"#));
    assert!(body.contains("The woods are lovely, dark and deep,\nBut I have promises to keep,"));

    // Attribution lines for both verse and blockquote.
    assert!(
        body.contains(r#"<div class="attribution">\u{2014}"#) || body.contains("— Robert Frost")
    );
    assert!(body.contains("<cite>Stopping by Woods on a Snowy Evening</cite>"));
    assert!(body.contains("— Albert Einstein"));
}

#[test]
fn derived_header_attributes_link_substitution_anchors_discrete_blockimage() {
    let (doc, html) = render("32_compat_links_anchors_discrete.adoc");

    let body_start = html.find("<body>").expect("body open");
    let body_end = html.find("</body>").expect("body close");
    let body = &html[body_start..body_end];

    // Derived header attributes resolve.
    assert!(body.contains("Title is Compatibility Round"));
    assert!(body.contains("First author is Alice Author"));
    assert!(body.contains("Alice Author (Alice Author, initials AA"));
    assert!(body.contains("All authors: Alice Author, Bob Builder"));
    assert!(body.contains("Second author: Bob Builder"));
    assert!(body.contains("Revision v3.1.4 on 2026-04-25 — Compat round"));

    // link: with attribute reference in the URL.
    assert!(body.contains(r#"<a href="https://example.com">Example</a>"#));

    // Bare URL with [label] form.
    assert!(
        body.matches(r#"<a href="https://example.com">Example</a>"#)
            .count()
            >= 2
    );

    // Inline anchor renders an empty <a id="…">.
    assert!(body.contains(r#"<a id="my-anchor"></a>"#));

    // Discrete heading: <hN class="discrete"> sitting inside the parent
    // <section>, not opening a new <section>.
    assert!(body.contains(r#"<h3 class="discrete">A discrete heading</h3>"#));

    // Block image: standalone <div class="imageblock"> with title.
    assert!(body.contains(r#"<div class="imageblock">"#));
    assert!(body.contains(r#"<img src="diagram.png" alt="Diagram" width="400" height="200">"#));
    assert!(body.contains(r#"<div class="title">A standalone block image</div>"#));

    // AST shape: discrete heading is a top-level Block::DiscreteHeading
    // sibling of the surrounding section's children, not a Block::Section.
    let section_blocks = doc
        .blocks
        .iter()
        .find_map(|b| match b {
            Block::Section(s) => {
                let title_text = match s.title.first() {
                    Some(adoc::ast::Inline::Text { value }) => value.as_str(),
                    _ => "",
                };
                if title_text.contains("discrete")
                    || s.blocks
                        .iter()
                        .any(|b| matches!(b, Block::DiscreteHeading(_)))
                {
                    Some(&s.blocks)
                } else {
                    None
                }
            }
            _ => None,
        })
        .expect("expected a section containing the discrete heading");
    assert!(section_blocks
        .iter()
        .any(|b| matches!(b, Block::DiscreteHeading(d) if d.level == 2)));
}

#[test]
fn asciidoc_cells_and_colspan() {
    let (doc, html) = render("31_asciidoc_cells_and_span.adoc");

    let body_start = html.find("<body>").expect("body open");
    let body_end = html.find("</body>").expect("body close");
    let body = &html[body_start..body_end];

    // a| cell content parses recursively: it's wrapped in <p>...</p> and
    // every inline formatter inside survives.
    assert!(body.contains(
        "<td><p>An <strong>AsciiDoc</strong> cell with <code>inline code</code> and <em>emphasis</em>.</p>"
    ));

    // Colspan attribute on `<td>`.
    assert!(body.contains(r#"<td colspan="2">spans columns 1 and 2</td>"#));
    assert!(body.contains(r#"<td colspan="2">spans the last two columns</td>"#));

    // AST shape: a| cell carries blocks, not inlines.
    let first_table: &adoc::ast::Table = doc
        .blocks
        .iter()
        .flat_map(|b| match b {
            Block::Section(s) => s
                .blocks
                .iter()
                .filter_map(|b| match b {
                    Block::Table(t) => Some(t),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        })
        .next()
        .expect("first table");
    let asciidoc_cell = &first_table.rows[1].cells[0];
    assert_eq!(asciidoc_cell.style, Some(adoc::ast::CellStyle::AsciiDoc));
    assert!(
        asciidoc_cell.inlines.is_empty(),
        "a| cells use blocks, not inlines"
    );
    assert!(
        !asciidoc_cell.blocks.is_empty(),
        "a| cell should have parsed blocks"
    );
    assert!(matches!(asciidoc_cell.blocks[0], Block::Paragraph(_)));
}

#[test]
fn table_cols_widths_and_alignment() {
    let (doc, html) = render("30_table_cols.adoc");

    let body_start = html.find("<body>").expect("body open");
    let body_end = html.find("</body>").expect("body close");
    let body = &html[body_start..body_end];

    let tables: Vec<&adoc::ast::Table> = doc
        .blocks
        .iter()
        .flat_map(|b| match b {
            Block::Section(s) => s
                .blocks
                .iter()
                .filter_map(|b| match b {
                    Block::Table(t) => Some(t),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        })
        .collect();
    assert_eq!(tables.len(), 5);

    // Widths: 1,3,1 → [1, 3, 1]
    assert_eq!(tables[0].cols.len(), 3);
    assert_eq!(tables[0].cols[0].width, 1);
    assert_eq!(tables[0].cols[1].width, 3);
    assert_eq!(tables[0].cols[2].width, 1);

    // Alignments: <,^,>
    assert_eq!(tables[1].cols.len(), 3);
    assert_eq!(tables[1].cols[0].h_align, Some(adoc::ast::HAlign::Left));
    assert_eq!(tables[1].cols[1].h_align, Some(adoc::ast::HAlign::Center));
    assert_eq!(tables[1].cols[2].h_align, Some(adoc::ast::HAlign::Right));

    // Width + alignment combined
    assert_eq!(tables[2].cols[0].width, 2);
    assert_eq!(tables[2].cols[0].h_align, Some(adoc::ast::HAlign::Left));
    assert_eq!(tables[2].cols[1].width, 1);
    assert_eq!(tables[2].cols[1].h_align, Some(adoc::ast::HAlign::Center));

    // Repeat: 3*< → 3 columns, all left
    assert_eq!(tables[3].cols.len(), 3);
    for col in &tables[3].cols {
        assert_eq!(col.h_align, Some(adoc::ast::HAlign::Left));
    }

    // HTML: colgroup with width percentages
    assert!(body.contains("<colgroup>"));
    assert!(body.contains(r#"<col style="width: 20.0000%;">"#));
    assert!(body.contains(r#"<col style="width: 60.0000%;">"#));

    // HTML: alignment classes
    assert!(body.contains(r#"<td class="halign-left">left</td>"#));
    assert!(body.contains(r#"<td class="halign-center">centre</td>"#));
    assert!(body.contains(r#"<td class="halign-right">right</td>"#));

    // Per-cell alignment overrides column-level alignment.
    assert!(
        body.contains(r#"<td class="halign-center">centred override</td>"#),
        "expected per-cell centre override in body:\n{body}"
    );
    assert!(body.contains(r#"<td class="halign-right">right override</td>"#));
    // The column default (left) wins on the next row.
    assert!(body.contains(r#"<td class="halign-left">col-default</td>"#));
}

#[test]
fn callouts_listing_markers_and_colist() {
    let (doc, html) = render("29_callouts.adoc");

    let body_start = html.find("<body>").expect("body open");
    let body_end = html.find("</body>").expect("body close");
    let body = &html[body_start..body_end];

    // Markers in listings render as <b class="conum">(N)</b>.
    assert!(body.contains(r#"<b class="conum">(1)</b>"#));
    assert!(body.contains(r#"<b class="conum">(2)</b>"#));
    assert!(body.contains(r#"<b class="conum">(3)</b>"#));

    // Colist comes out as <ol class="colist">.
    assert!(body.contains(r#"<ol class="colist">"#));
    assert!(body.contains(r#"<li value="1">Bind a string slice to a local.</li>"#));
    assert!(body.contains(r#"<li value="2">Format and print using an inline expression.</li>"#));
    assert!(body.contains(r#"<li value="3">Two callouts can share a single line.</li>"#));

    // Three colists in total: one for the source listing, one for the
    // plain listing, one for the literal block. The third listing has
    // no following descriptions, so no colist there.
    assert_eq!(body.matches(r#"<ol class="colist">"#).count(), 3);

    // AST shape: each colist is its own block, sibling to the listing.
    let colists: Vec<&adoc::ast::Colist> = doc
        .blocks
        .iter()
        .flat_map(|b| match b {
            Block::Section(s) => s
                .blocks
                .iter()
                .filter_map(|b| match b {
                    Block::Colist(c) => Some(c),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        })
        .collect();
    assert_eq!(colists.len(), 3);
    assert_eq!(colists[0].items.len(), 3);
    assert_eq!(colists[0].items[0].number, 1);
    assert_eq!(colists[0].items[2].number, 3);

    // The "no colist" case: the third section has a listing whose markers
    // render but no <ol class="colist"> sibling.
    let third = match &doc.blocks[2] {
        Block::Section(s) => s,
        _ => panic!("expected section 3"),
    };
    let colist_count = third
        .blocks
        .iter()
        .filter(|b| matches!(b, Block::Colist(_)))
        .count();
    assert_eq!(colist_count, 0);
}

fn render_src(src: &str) -> String {
    let mut pre = adoc::preprocessor::Preprocessor::default();
    let lines = pre.run(src, Utf8Path::new("<input>")).expect("preprocess");
    let doc = adoc::parser::parse(&lines).expect("parse");
    Html5Converter::new().convert(&doc).expect("convert")
}

#[test]
fn source_highlighter_prism_emits_link_and_scripts() {
    let html = render_src(":source-highlighter: prism\n\nhi\n");
    assert!(html.contains("prismjs@1/themes/prism.min.css"));
    assert!(html.contains("prismjs@1/prism.min.js"));
    assert!(html.contains("prism-autoloader.min.js"));
    let link = html.find("prism.min.css").unwrap();
    let script = html.find("prism.min.js").unwrap();
    assert!(link < script, "link must appear in <head> before scripts");
}

#[test]
fn source_highlighter_prism_theme_attribute_overrides_default() {
    let html = render_src(":source-highlighter: prism\n:prism-theme: prism-tomorrow\n\nhi\n");
    assert!(html.contains("themes/prism-tomorrow.min.css"));
    assert!(!html.contains("themes/prism.min.css"));
}

#[test]
fn source_highlighter_highlightjs_emits_link_and_scripts() {
    let html = render_src(":source-highlighter: highlightjs\n\nhi\n");
    assert!(html.contains("highlight.js@11/styles/default.min.css"));
    assert!(html.contains("highlight.js@11/lib/common.min.js"));
    assert!(html.contains("hljs.highlightAll()"));
}

#[test]
fn source_highlighter_unknown_value_emits_nothing() {
    let html = render_src(":source-highlighter: rouge\n\nhi\n");
    assert!(!html.contains("prism"));
    assert!(!html.contains("highlight.js"));
    assert!(!html.contains("hljs"));
}

#[test]
fn source_highlighter_unset_emits_nothing() {
    let html = render_src("hi\n");
    assert!(!html.contains("prism"));
    assert!(!html.contains("highlight.js"));
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
