//! Thematic breaks: `'''` and the Markdown-style forms render as <hr>;
//! four-character delimiters still open blocks.

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
fn all_spec_forms_render_as_hr() {
    for form in ["'''", "---", "***", "- - -", "* * *"] {
        let out = html(&format!("Before.\n\n{form}\n\nAfter.\n"));
        assert!(out.contains("<hr>"), "{form:?}: {out}");
        assert!(
            !out.contains(&format!("<p>{form}</p>")),
            "{form:?} leaked as text"
        );
    }
}

#[test]
fn break_after_a_listing_block() {
    let out = html("----\ncode\n----\n\n'''\n\nAfter.\n");
    assert!(out.contains("<hr>"), "{out}");
}

#[test]
fn four_dashes_still_open_a_listing() {
    let out = html("----\nnot a break\n----\n");
    assert!(!out.contains("<hr>"), "{out}");
    assert!(out.contains("not a break"));
}
