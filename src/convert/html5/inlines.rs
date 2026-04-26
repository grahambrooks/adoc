//! Inline-level rendering. Walks `Vec<Inline>` and emits HTML fragments
//! for each variant. Block-level callers use [`render_inlines`] for
//! string-returning convenience and [`render_inline`] when they're
//! already accumulating into a `&mut String`.

use std::fmt::Write;

use crate::ast::Inline;

use super::escape::{escape, escape_attr};

pub(crate) fn render_inlines(inlines: &[Inline]) -> String {
    let mut out = String::new();
    for i in inlines {
        render_inline(&mut out, i);
    }
    out
}

pub(crate) fn render_inline(out: &mut String, i: &Inline) {
    match i {
        Inline::Text { value } => out.push_str(&escape(value)),
        Inline::Strong { children } => {
            out.push_str("<strong>");
            for c in children {
                render_inline(out, c);
            }
            out.push_str("</strong>");
        }
        Inline::Emphasis { children } => {
            out.push_str("<em>");
            for c in children {
                render_inline(out, c);
            }
            out.push_str("</em>");
        }
        Inline::Monospace { children } => {
            out.push_str("<code>");
            for c in children {
                render_inline(out, c);
            }
            out.push_str("</code>");
        }
        Inline::Subscript { children } => {
            out.push_str("<sub>");
            for c in children {
                render_inline(out, c);
            }
            out.push_str("</sub>");
        }
        Inline::Superscript { children } => {
            out.push_str("<sup>");
            for c in children {
                render_inline(out, c);
            }
            out.push_str("</sup>");
        }
        Inline::Highlight { children } => {
            out.push_str("<mark>");
            for c in children {
                render_inline(out, c);
            }
            out.push_str("</mark>");
        }
        Inline::Footnote { id, text } => {
            match id {
                Some(id) => {
                    let _ = write!(out, r#"<span class="footnote" id="{}">"#, escape_attr(id));
                }
                None => out.push_str(r#"<span class="footnote">"#),
            }
            for c in text {
                render_inline(out, c);
            }
            out.push_str("</span>");
        }
        Inline::Passthrough { value } => out.push_str(&escape(value)),
        Inline::Link { href, text } => {
            let _ = write!(out, r#"<a href="{}">"#, escape_attr(href));
            for c in text {
                render_inline(out, c);
            }
            out.push_str("</a>");
        }
        Inline::Xref { target, text } => {
            let _ = write!(out, r##"<a href="#{}">"##, escape_attr(target));
            match text {
                Some(t) => {
                    for c in t {
                        render_inline(out, c);
                    }
                }
                None => out.push_str(&escape(target)),
            }
            out.push_str("</a>");
        }
        Inline::Image {
            target,
            alt,
            width,
            height,
        } => {
            let _ = write!(
                out,
                r#"<img src="{}" alt="{}""#,
                escape_attr(target),
                escape_attr(alt)
            );
            if let Some(w) = width {
                let _ = write!(out, r#" width="{}""#, escape_attr(w));
            }
            if let Some(h) = height {
                let _ = write!(out, r#" height="{}""#, escape_attr(h));
            }
            out.push('>');
        }
        Inline::AttributeRef { name } => {
            let _ = write!(out, "{{{name}}}");
        }
        Inline::LineBreak => out.push_str("<br>"),
        Inline::RawHtml { value } => out.push_str(value),
    }
}
