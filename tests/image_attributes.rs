//! Image macro attribute lists: named attributes don't become the alt text,
//! and a missing alt falls back to the file name, as in Asciidoctor.

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
fn named_attribute_is_not_the_alt() {
    let out = html("image:/images/cqrs-event_service.svg[align=\"center\"]\n");
    assert!(out.contains("alt=\"cqrs event service\""), "{out}");
    assert!(!out.contains("align="), "{out}");
}

#[test]
fn block_image_with_named_width() {
    let out = html("image::/images/table.gif[align=\"center\", width=\"100%\"]\n");
    assert!(out.contains("alt=\"table\""), "{out}");
    assert!(out.contains("width=\"100%\""), "{out}");
}

#[test]
fn positional_alt_width_height() {
    let out = html("image::a.png[A diagram, 300, 200]\n");
    assert!(out.contains("alt=\"A diagram\""), "{out}");
    assert!(out.contains("width=\"300\""), "{out}");
    assert!(out.contains("height=\"200\""), "{out}");
}

#[test]
fn named_alt_and_quoted_comma() {
    let out = html("image::a.png[alt=\"One, two\", width=50]\n");
    assert!(out.contains("alt=\"One, two\""), "{out}");
    assert!(out.contains("width=\"50\""), "{out}");
}

#[test]
fn empty_list_uses_file_name() {
    let out = html("Inline image:icons/my_icon.png[] here.\n");
    assert!(out.contains("alt=\"my icon\""), "{out}");
}
