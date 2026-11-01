//! Inline formatting spans line breaks inside a paragraph, as in Asciidoctor.

use adoc::convert::html5::Html5Converter;

fn html(src: &str) -> String {
    let lines = adoc::preprocessor::Preprocessor::default()
        .run(src, camino::Utf8Path::new("t.adoc"))
        .unwrap();
    let doc = adoc::parser::parse(&lines).unwrap();
    Html5Converter::new()
        .convert_with_diagnostics(&doc)
        .unwrap()
        .0
}

#[test]
fn constrained_bold_across_lines() {
    let out = html("variants are *back-reference to\ndifference* described below.\n");
    assert!(
        out.contains("<strong>back-reference to difference</strong> described"),
        "{out}"
    );
}

#[test]
fn emphasis_and_monospace_across_lines() {
    let out = html("An _emphasised\nphrase_ and `some\ncode` here.\n");
    assert!(out.contains("<em>emphasised phrase</em>"), "{out}");
    assert!(out.contains("<code>some code</code>"), "{out}");
}

#[test]
fn lines_still_join_with_a_space() {
    let out = html("First line\nsecond line.\n");
    assert!(out.contains("<p>First line second line.</p>"), "{out}");
}

#[test]
fn hard_line_break_mid_paragraph() {
    let out = html("Roses are red, +\nviolets are blue.\n");
    assert!(
        out.contains("<p>Roses are red,<br> violets are blue.</p>"),
        "{out}"
    );
}

#[test]
fn unclosed_marker_stays_literal() {
    let out = html("A *star\non its own.\n");
    assert!(out.contains("A *star on its own."), "{out}");
}
