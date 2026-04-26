//! Table rendering: `<colgroup>`, `<thead>`/`<tbody>`/`<tfoot>` grouping,
//! cell-level alignment / span / style dispatch, and the recursive case
//! for `a|` AsciiDoc cells. Header rows force `<th>` regardless of cell
//! style.

use std::fmt::Write;

use crate::ast::{
    inlines_to_plain, CellStyle, ColumnSpec, ConvertError, HAlign, RowKind, Table, TableCell,
};

use super::blocks::{render_block, render_block_title};
use super::ctx::RenderCtx;
use super::escape::escape;
use super::inlines::render_inlines;

pub(crate) fn render_table(
    out: &mut String,
    t: &Table,
    ctx: &RenderCtx,
) -> Result<(), ConvertError> {
    render_block_title(out, &t.meta);
    writeln!(out, "<table{}>", super::blocks::meta_attrs(&t.meta))
        .map_err(|e| ConvertError::Message(e.to_string()))?;
    render_colgroup(out, t);

    // Group rows by kind so we can emit <thead>/<tbody>/<tfoot> sections.
    let mut i = 0;
    while i < t.rows.len() {
        let kind = t.rows[i].kind;
        let mut j = i;
        while j < t.rows.len() && t.rows[j].kind == kind {
            j += 1;
        }
        let (open, close) = match kind {
            RowKind::Header => ("<thead>\n", "</thead>\n"),
            RowKind::Body => ("<tbody>\n", "</tbody>\n"),
            RowKind::Footer => ("<tfoot>\n", "</tfoot>\n"),
        };
        out.push_str(open);
        for row in &t.rows[i..j] {
            out.push_str("<tr>");
            for (idx, cell) in row.cells.iter().enumerate() {
                let col = t.cols.get(idx);
                render_table_cell(out, cell, kind, col, ctx)?;
            }
            out.push_str("</tr>\n");
        }
        out.push_str(close);
        i = j;
    }

    out.push_str("</table>\n");
    Ok(())
}

/// Emit a `<colgroup>` based on `cols=` widths. The widths are relative
/// weights; we normalise them to percentages so the renderer doesn't have
/// to know the table's container width. Skipped when no `cols=` was given
/// or when every entry has width 0.
fn render_colgroup(out: &mut String, t: &Table) {
    if t.cols.is_empty() {
        return;
    }
    let total: u32 = t.cols.iter().map(|c| c.width).sum();
    out.push_str("<colgroup>\n");
    for col in &t.cols {
        if total > 0 && col.width > 0 {
            let pct = (col.width as f64) * 100.0 / (total as f64);
            let _ = writeln!(out, r#"<col style="width: {pct:.4}%;">"#);
        } else {
            out.push_str("<col>\n");
        }
    }
    out.push_str("</colgroup>\n");
}

fn h_align_class(a: HAlign) -> &'static str {
    match a {
        HAlign::Left => "halign-left",
        HAlign::Center => "halign-center",
        HAlign::Right => "halign-right",
    }
}

fn render_table_cell(
    out: &mut String,
    cell: &TableCell,
    row_kind: RowKind,
    col: Option<&ColumnSpec>,
    ctx: &RenderCtx,
) -> Result<(), ConvertError> {
    // Forced header style or header rows always use <th>.
    let force_th = matches!(cell.style, Some(CellStyle::Header)) || row_kind == RowKind::Header;
    let tag = if force_th { "th" } else { "td" };

    // Effective alignment: cell-level wins, otherwise column-level, otherwise none.
    let effective_align = cell.h_align.or_else(|| col.and_then(|c| c.h_align));
    let mut classes: Vec<&str> = Vec::new();
    if let Some(a) = effective_align {
        classes.push(h_align_class(a));
    }
    let class_attr = if classes.is_empty() {
        String::new()
    } else {
        format!(r#" class="{}""#, classes.join(" "))
    };

    let mut span_attrs = String::new();
    if cell.colspan > 1 {
        let _ = write!(span_attrs, r#" colspan="{}""#, cell.colspan);
    }
    if cell.rowspan > 1 {
        let _ = write!(span_attrs, r#" rowspan="{}""#, cell.rowspan);
    }

    // AsciiDoc cells render their pre-parsed nested blocks; everything
    // else uses the cell's flat inline list.
    let body = if matches!(cell.style, Some(CellStyle::AsciiDoc)) {
        let mut buf = String::new();
        for b in &cell.blocks {
            render_block(&mut buf, b, ctx)?;
        }
        buf
    } else {
        match cell.style {
            Some(CellStyle::Monospace) if !force_th => {
                format!("<code>{}</code>", render_inlines(&cell.inlines))
            }
            Some(CellStyle::Strong) if !force_th => {
                format!("<strong>{}</strong>", render_inlines(&cell.inlines))
            }
            Some(CellStyle::Emphasis) if !force_th => {
                format!("<em>{}</em>", render_inlines(&cell.inlines))
            }
            Some(CellStyle::Literal) => {
                let plain = inlines_to_plain(&cell.inlines);
                format!("<pre>{}</pre>", escape(&plain))
            }
            _ => render_inlines(&cell.inlines),
        }
    };

    write!(out, "<{tag}{span_attrs}{class_attr}>{body}</{tag}>")
        .map_err(|e| ConvertError::Message(e.to_string()))
}
