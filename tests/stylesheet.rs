//! Covers the five [`Stylesheet`] modes.

use adoc::ast::{Attributes, Converter, Document};
use adoc::convert::html5::{
    Html5Converter, Html5Options, Stylesheet, BUILTIN_CSS, BUILTIN_FILENAME,
};

fn empty_doc() -> Document {
    Document {
        header: None,
        attributes: Attributes::new(),
        blocks: Vec::new(),
    }
}

fn convert(stylesheet: Stylesheet) -> String {
    let converter = Html5Converter::with_options(Html5Options { stylesheet });
    converter.convert(&empty_doc()).expect("convert")
}

#[test]
fn builtin_embed_inlines_the_default_stylesheet() {
    let html = convert(Stylesheet::BuiltinEmbed);
    assert!(html.contains("<style>"));
    // Sentinel substring from the built-in CSS so we know it's really inlined.
    assert!(html.contains("--adoc-bg"));
    assert!(!html.contains(r#"<link rel="stylesheet""#));
}

#[test]
fn builtin_link_emits_link_to_default_name() {
    let html = convert(Stylesheet::BuiltinLink {
        href: BUILTIN_FILENAME.to_string(),
    });
    assert!(html.contains(r#"<link rel="stylesheet" href="adoc.css">"#));
    assert!(!html.contains("<style>"));
}

#[test]
fn custom_embed_inlines_supplied_css() {
    let css = "body{color:red}".to_string();
    let html = convert(Stylesheet::CustomEmbed { css: css.clone() });
    assert!(html.contains("<style>"));
    assert!(html.contains(&css));
    assert!(!html.contains("--adoc-bg"));
}

#[test]
fn custom_link_uses_supplied_href() {
    let html = convert(Stylesheet::CustomLink {
        href: "themes/custom.css".to_string(),
    });
    assert!(html.contains(r#"<link rel="stylesheet" href="themes/custom.css">"#));
}

#[test]
fn stylesheet_none_emits_nothing() {
    let html = convert(Stylesheet::None);
    assert!(!html.contains("<style>"));
    assert!(!html.contains(r#"<link rel="stylesheet""#));
}

#[test]
fn builtin_css_has_dark_mode_tokens() {
    // Guards against regressions in the default palette.
    assert!(BUILTIN_CSS.contains("color-scheme: light dark"));
    assert!(BUILTIN_CSS.contains("light-dark("));
    assert!(BUILTIN_CSS.contains("--adoc-bg"));
}
