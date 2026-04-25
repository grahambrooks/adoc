//! Block parser.
//!
//! Recognises sections, paragraphs, ordered/unordered/description lists,
//! all seven delimited block styles, and basic tables. Inline text inside
//! blocks is resolved immediately using the accumulated attribute context.

use crate::ast::{
    Attributes, Block, BlockMeta, CellStyle, Colist, ColistItem, ColumnSpec, DelimitedBlock,
    DelimitedContent, DelimitedStyle, DescriptionList, DescriptionListItem, HAlign, Inline, List,
    ListItem, ListMarker, Location, Paragraph, RowKind, Section, Table, TableCell, TableRow,
};

use super::cursor::Cursor;
use super::header::{consume_attribute_entries, parse_attribute_entry};
use super::inline;
use super::meta::collect_block_meta;
use super::subs::Subs;

pub fn parse_blocks(cursor: &mut Cursor, attrs: &mut Attributes, section_level: u8) -> Vec<Block> {
    let mut out = Vec::new();
    loop {
        cursor.skip_blank_lines();
        if cursor.at_end() {
            break;
        }
        // Opportunistic mid-document attribute entries.
        if let Some(line) = cursor.peek() {
            if parse_attribute_entry(&line.text).is_some() {
                consume_attribute_entries(cursor, attrs);
                continue;
            }
        }
        // Collect any block metadata (.Title and [attrlist] lines) that
        // immediately precedes the upcoming block. Save the cursor first
        // so we can roll back if the metadata turns out to belong to an
        // outer scope (e.g. it sits above a same-or-higher-level section
        // header that ends this nested call).
        let pre_meta_pos = cursor.pos();
        let meta = collect_block_meta(cursor, attrs);
        // A blank line between metadata and the block detaches the metadata.
        // Drop and start over rather than attaching across the gap.
        match cursor.peek() {
            None => break,
            Some(line) if line.text.trim().is_empty() => continue,
            _ => {}
        }
        // Section headers end the enclosing section if they are same-or-higher level.
        if let Some(level) = peek_section_level(cursor) {
            if level <= section_level {
                // Hand the meta lines back to the outer scope.
                cursor.seek(pre_meta_pos);
                break;
            }
            let section = parse_section(cursor, attrs, level, meta);
            out.push(Block::Section(section));
            continue;
        }
        if let Some(block) = parse_one_block(cursor, attrs, meta) {
            // A listing or literal block can be followed by a colist
            // (one or more `<N> description` lines pairing markers in the
            // block above with descriptions).
            let listing_or_literal = matches!(
                &block,
                Block::Delimited(d)
                    if matches!(d.style, DelimitedStyle::Listing | DelimitedStyle::Literal)
            );
            out.push(block);
            if listing_or_literal {
                cursor.skip_blank_lines();
                if let Some(colist) = try_parse_colist(cursor, attrs) {
                    out.push(Block::Colist(colist));
                }
            }
        } else {
            break;
        }
    }
    out
}

/// Recognise a callout list: one or more adjacent lines of the form
/// `<N> description`. Stops at the first non-matching line.
fn try_parse_colist(cursor: &mut Cursor, attrs: &Attributes) -> Option<Colist> {
    let first = cursor.peek()?;
    let (n, rest) = strip_callout_prefix(&first.text)?;
    let location = first.location.clone();
    let mut items = vec![ColistItem {
        number: n,
        inlines: inline::parse(rest, attrs, Subs::NORMAL),
    }];
    cursor.advance();
    while let Some(line) = cursor.peek() {
        let Some((n, rest)) = strip_callout_prefix(&line.text) else {
            break;
        };
        items.push(ColistItem {
            number: n,
            inlines: inline::parse(rest, attrs, Subs::NORMAL),
        });
        cursor.advance();
    }
    Some(Colist {
        items,
        location,
        meta: BlockMeta::default(),
    })
}

fn strip_callout_prefix(text: &str) -> Option<(u32, &str)> {
    let trimmed = text.trim_start();
    let rest = trimmed.strip_prefix('<')?;
    let close = rest.find('>')?;
    let num: u32 = rest[..close].parse().ok()?;
    let body = rest[close + 1..].trim_start();
    if body.is_empty() {
        return None;
    }
    Some((num, body))
}

fn peek_section_level(cursor: &Cursor) -> Option<u8> {
    let text = cursor.peek_text()?;
    section_level_of(text)
}

fn section_level_of(text: &str) -> Option<u8> {
    let bytes = text.as_bytes();
    let mut eq_count = 0usize;
    while eq_count < bytes.len() && bytes[eq_count] == b'=' {
        eq_count += 1;
    }
    if eq_count == 0 || eq_count > 6 {
        return None;
    }
    if bytes.get(eq_count) != Some(&b' ') {
        return None;
    }
    if text[eq_count + 1..].trim().is_empty() {
        return None;
    }
    // level 0 is the doc title (consumed by the header parser); here we only
    // recognise nested section headers (level >= 1).
    if eq_count == 1 {
        None
    } else {
        Some((eq_count - 1) as u8)
    }
}

fn parse_section(
    cursor: &mut Cursor,
    attrs: &mut Attributes,
    level: u8,
    meta: BlockMeta,
) -> Section {
    let line = cursor.advance().expect("caller peeked a section header");
    let location = line.location.clone();
    let title_src = line.text[(level as usize + 1) + 1..].trim();
    let title = inline::parse(title_src, attrs, Subs::NORMAL);
    let blocks = parse_blocks(cursor, attrs, level);
    Section {
        level,
        title,
        blocks,
        location,
        meta,
    }
}

fn parse_one_block(cursor: &mut Cursor, attrs: &mut Attributes, meta: BlockMeta) -> Option<Block> {
    let line = cursor.peek()?;
    let text = line.text.as_str();

    if let Some(style) = delimited_style(text) {
        return Some(Block::Delimited(parse_delimited(
            cursor, attrs, style, meta,
        )));
    }
    if text.trim_start() == "|===" {
        return Some(Block::Table(parse_table(cursor, attrs, meta)));
    }
    if let Some(marker) = list_marker(text) {
        return Some(parse_list_kind(cursor, attrs, marker, meta));
    }
    if is_description_item(text) {
        return Some(Block::DescriptionList(parse_description_list(
            cursor, attrs, meta,
        )));
    }
    Some(Block::Paragraph(parse_paragraph(cursor, attrs, meta)))
}

// --- paragraphs -------------------------------------------------------------

fn parse_paragraph(cursor: &mut Cursor, attrs: &Attributes, mut meta: BlockMeta) -> Paragraph {
    let location = cursor.current_location();
    let mut lines: Vec<String> = Vec::new();
    let mut first = true;
    while let Some(line) = cursor.peek() {
        let t = line.text.as_str();
        if t.trim().is_empty() {
            break;
        }
        if is_block_boundary(t) {
            break;
        }
        let pushed = if first {
            // Paragraph admonition shortcut: `NOTE: text`, `TIP: …`, etc.
            // The first line's keyword + colon + space is stripped and
            // promoted to `meta.style`. Block-level metadata wins if it
            // already set a style.
            if let Some((kw, rest)) = strip_admonition_prefix(t) {
                meta.style.get_or_insert_with(|| kw.to_string());
                rest.to_string()
            } else {
                line.text.clone()
            }
        } else {
            line.text.clone()
        };
        lines.push(pushed);
        first = false;
        cursor.advance();
    }
    let text = lines.join("\n");
    let inlines = parse_inlines_multiline(&text, attrs, Subs::NORMAL);
    Paragraph {
        inlines,
        location,
        meta,
    }
}

const ADMONITION_KEYWORDS: &[&str] = &["NOTE", "TIP", "IMPORTANT", "WARNING", "CAUTION"];

fn strip_admonition_prefix(line: &str) -> Option<(&'static str, &str)> {
    for kw in ADMONITION_KEYWORDS {
        if let Some(rest) = line.strip_prefix(kw) {
            if let Some(content) = rest.strip_prefix(": ") {
                return Some((kw, content));
            }
        }
    }
    None
}

fn is_block_boundary(text: &str) -> bool {
    delimited_style(text).is_some()
        || text.trim_start() == "|==="
        || list_marker(text).is_some()
        || is_description_item(text)
        || section_level_of(text).is_some()
}

/// Parse inline content that spans multiple source lines.
/// Post-replacements (line breaks) apply per line; lines are joined with spaces
/// otherwise.
pub fn parse_inlines_multiline(text: &str, attrs: &Attributes, subs: Subs) -> Vec<Inline> {
    let mut out: Vec<Inline> = Vec::new();
    for (idx, line) in text.split('\n').enumerate() {
        if idx > 0 {
            out.push(Inline::Text {
                value: " ".to_string(),
            });
        }
        let parsed = inline::parse(line, attrs, subs);
        out.extend(parsed);
    }
    merge_adjacent_text(out)
}

fn merge_adjacent_text(inlines: Vec<Inline>) -> Vec<Inline> {
    let mut out: Vec<Inline> = Vec::with_capacity(inlines.len());
    for item in inlines {
        match (out.last_mut(), item) {
            (Some(Inline::Text { value: a }), Inline::Text { value: b }) => a.push_str(&b),
            (_, other) => out.push(other),
        }
    }
    out
}

// --- lists ------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ListMarkerInfo {
    marker: ListMarker,
    depth: u8,
    prefix_len: usize,
}

fn list_marker(text: &str) -> Option<ListMarkerInfo> {
    let bytes = text.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    // Unordered: 1+ asterisks.
    if bytes[0] == b'*' {
        let mut n = 0;
        while n < bytes.len() && bytes[n] == b'*' {
            n += 1;
        }
        if n <= 5 && bytes.get(n) == Some(&b' ') {
            return Some(ListMarkerInfo {
                marker: ListMarker::Unordered,
                depth: n as u8,
                prefix_len: n + 1,
            });
        }
    }
    // Ordered: 1+ dots.
    if bytes[0] == b'.' {
        let mut n = 0;
        while n < bytes.len() && bytes[n] == b'.' {
            n += 1;
        }
        if n <= 5 && bytes.get(n) == Some(&b' ') {
            return Some(ListMarkerInfo {
                marker: ListMarker::Ordered,
                depth: n as u8,
                prefix_len: n + 1,
            });
        }
    }
    // Ordered numeric: `<digits>. `
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i > 0 && bytes.get(i) == Some(&b'.') && bytes.get(i + 1) == Some(&b' ') {
        return Some(ListMarkerInfo {
            marker: ListMarker::Ordered,
            depth: 1,
            prefix_len: i + 2,
        });
    }
    None
}

fn parse_list_kind(
    cursor: &mut Cursor,
    attrs: &mut Attributes,
    first: ListMarkerInfo,
    meta: BlockMeta,
) -> Block {
    let location = cursor.current_location();
    let mut items: Vec<ListItem> = Vec::new();
    loop {
        let line = match cursor.peek() {
            Some(l) => l,
            None => break,
        };
        let text = line.text.clone();
        if text.trim().is_empty() {
            // A blank line could end the list unless the next non-blank line is a
            // continuation marker `+` or another list item. For v1, terminate.
            break;
        }
        let info = match list_marker(&text) {
            Some(i) if i.marker == first.marker => i,
            _ => break,
        };
        cursor.advance();
        let principal_src = text[info.prefix_len..].trim_end().to_string();
        let principal = inline::parse(&principal_src, attrs, Subs::NORMAL);
        let blocks = parse_list_item_attachments(cursor, attrs);
        items.push(ListItem {
            depth: info.depth,
            principal,
            blocks,
        });
    }
    Block::List(List {
        marker: first.marker,
        items,
        location,
        meta,
    })
}

fn parse_list_item_attachments(cursor: &mut Cursor, attrs: &mut Attributes) -> Vec<Block> {
    // Handles the `+` continuation: a single `+` line followed by another block
    // attaches that block to the current list item.
    let mut out = Vec::new();
    loop {
        let line = match cursor.peek() {
            Some(l) => l,
            None => return out,
        };
        if line.text.trim() != "+" {
            return out;
        }
        cursor.advance();
        cursor.skip_blank_lines();
        if cursor.at_end() {
            return out;
        }
        // Allow metadata on continuation blocks too.
        let inner_meta = collect_block_meta(cursor, attrs);
        if let Some(block) = parse_one_block(cursor, attrs, inner_meta) {
            out.push(block);
        }
    }
}

// --- description lists ------------------------------------------------------

fn is_description_item(text: &str) -> bool {
    description_term_end(text).is_some()
}

/// Returns the byte index of the `::` that ends the term, if this line begins
/// a description-list item.
fn description_term_end(text: &str) -> Option<usize> {
    // The simplest rule: line contains `::` and what precedes is non-empty text
    // that isn't itself a list/section/block marker.
    let idx = text.find("::")?;
    if idx == 0 {
        return None;
    }
    // After `::` must be end-of-line or whitespace.
    let after = &text[idx + 2..];
    if !(after.is_empty() || after.starts_with(' ') || after.starts_with('\t')) {
        return None;
    }
    let term = &text[..idx];
    // Reject if the "term" is actually a known block prefix.
    if list_marker(text).is_some() {
        return None;
    }
    if term.trim().is_empty() {
        return None;
    }
    Some(idx)
}

fn parse_description_list(
    cursor: &mut Cursor,
    attrs: &mut Attributes,
    meta: BlockMeta,
) -> DescriptionList {
    let location = cursor.current_location();
    let mut items: Vec<DescriptionListItem> = Vec::new();
    loop {
        let line = match cursor.peek() {
            Some(l) => l,
            None => break,
        };
        let text = line.text.clone();
        if text.trim().is_empty() {
            break;
        }
        let term_end = match description_term_end(&text) {
            Some(t) => t,
            None => break,
        };
        cursor.advance();
        let term_src = text[..term_end].trim();
        let desc_inline_src = text[term_end + 2..].trim();
        let term = inline::parse(term_src, attrs, Subs::NORMAL);
        let mut description: Vec<Block> = Vec::new();
        if !desc_inline_src.is_empty() {
            description.push(Block::Paragraph(Paragraph {
                inlines: inline::parse(desc_inline_src, attrs, Subs::NORMAL),
                location: line.location.clone(),
                meta: BlockMeta::default(),
            }));
        }
        items.push(DescriptionListItem { term, description });
    }
    DescriptionList {
        items,
        location,
        meta,
    }
}

// --- delimited blocks -------------------------------------------------------

fn delimited_style(text: &str) -> Option<DelimitedStyle> {
    let t = text.trim_end();
    if t == "--" {
        return Some(DelimitedStyle::Open);
    }
    let bytes = t.as_bytes();
    if bytes.len() < 4 {
        return None;
    }
    let first = bytes[0];
    if !matches!(first, b'-' | b'.' | b'=' | b'_' | b'*' | b'+') {
        return None;
    }
    if !bytes.iter().all(|&b| b == first) {
        return None;
    }
    Some(match first {
        b'-' => DelimitedStyle::Listing,
        b'.' => DelimitedStyle::Literal,
        b'=' => DelimitedStyle::Example,
        b'_' => DelimitedStyle::Quote,
        b'*' => DelimitedStyle::Sidebar,
        b'+' => DelimitedStyle::Passthrough,
        _ => unreachable!(),
    })
}

fn parse_delimited(
    cursor: &mut Cursor,
    attrs: &mut Attributes,
    style: DelimitedStyle,
    meta: BlockMeta,
) -> DelimitedBlock {
    let opener = cursor.advance().expect("caller peeked opener");
    let delim = opener.text.trim_end().to_string();
    let location = opener.location.clone();

    let is_raw = matches!(
        style,
        DelimitedStyle::Listing | DelimitedStyle::Literal | DelimitedStyle::Passthrough
    );

    if is_raw {
        let mut raw_lines: Vec<String> = Vec::new();
        while let Some(line) = cursor.peek() {
            if line.text.trim_end() == delim {
                cursor.advance();
                break;
            }
            raw_lines.push(line.text.clone());
            cursor.advance();
        }
        return DelimitedBlock {
            style,
            content: DelimitedContent::Raw {
                text: raw_lines.join("\n"),
            },
            location,
            meta,
        };
    }

    // Container styles: collect inner lines, then recursively parse them.
    let mut inner_lines: Vec<crate::preprocessor::PreprocessedLine> = Vec::new();
    while let Some(line) = cursor.peek() {
        if line.text.trim_end() == delim {
            cursor.advance();
            break;
        }
        inner_lines.push(line.clone());
        cursor.advance();
    }
    let mut inner_cursor = Cursor::new(&inner_lines);
    let blocks = parse_blocks(&mut inner_cursor, attrs, 0);
    DelimitedBlock {
        style,
        content: DelimitedContent::Blocks { blocks },
        location,
        meta,
    }
}

// --- tables -----------------------------------------------------------------

fn parse_table(cursor: &mut Cursor, attrs: &Attributes, meta: BlockMeta) -> Table {
    let opener = cursor.advance().expect("caller peeked |===");
    let location = opener.location.clone();
    let mut lines: Vec<String> = Vec::new();
    while let Some(line) = cursor.peek() {
        if line.text.trim_end() == "|===" {
            cursor.advance();
            break;
        }
        lines.push(line.text.clone());
        cursor.advance();
    }
    let cols = parse_cols_spec(meta.named.get("cols").map(String::as_str));
    let rows = parse_table_rows(&lines, attrs, &meta);
    Table {
        rows,
        cols,
        location,
        meta,
    }
}

/// Parse a `cols="…"` value. Each comma-separated segment is matched as
/// `[N*][<>^][digits][styleletter]` — multiplier expands the entry across
/// N columns, the alignment maps to [`HAlign`], digits set the relative
/// width, and the trailing letter (a default style) is consumed but
/// ignored at the AST level for now (per-column default styles are
/// queued behind cell-level style support).
fn parse_cols_spec(spec: Option<&str>) -> Vec<ColumnSpec> {
    let Some(spec) = spec else {
        return Vec::new();
    };
    let spec = spec.trim();
    if spec.is_empty() {
        return Vec::new();
    }
    let mut cols = Vec::new();
    for raw in spec.split(',') {
        let s = raw.trim();
        if s.is_empty() {
            continue;
        }
        let mut chars = s.chars().peekable();
        // Optional multiplier `N*`
        let mut multiplier: u32 = 1;
        let mut digits = String::new();
        while let Some(&c) = chars.peek() {
            if c.is_ascii_digit() {
                digits.push(c);
                chars.next();
            } else {
                break;
            }
        }
        if chars.peek() == Some(&'*') {
            multiplier = digits.parse().unwrap_or(1).max(1);
            chars.next();
            digits.clear();
        }
        // Optional h-align
        let h_align = match chars.peek() {
            Some(&'<') => {
                chars.next();
                Some(HAlign::Left)
            }
            Some(&'^') => {
                chars.next();
                Some(HAlign::Center)
            }
            Some(&'>') => {
                chars.next();
                Some(HAlign::Right)
            }
            _ => None,
        };
        // Optional width digits (only if we didn't already consume them as a multiplier base)
        if digits.is_empty() {
            while let Some(&c) = chars.peek() {
                if c.is_ascii_digit() {
                    digits.push(c);
                    chars.next();
                } else {
                    break;
                }
            }
        } else if multiplier == 1 {
            // The leading digits weren't followed by `*` so they are the width.
            // (Already in `digits`.)
        }
        // Optional style letter — consume but ignore.
        if let Some(&c) = chars.peek() {
            if matches!(c, 'a' | 'd' | 'e' | 'h' | 'l' | 'm' | 's') {
                chars.next();
            }
        }
        // Trailing junk — skip the segment rather than producing garbage.
        if chars.next().is_some() {
            continue;
        }
        let width: u32 = digits.parse().unwrap_or(0);
        let spec = ColumnSpec { width, h_align };
        for _ in 0..multiplier {
            cols.push(spec.clone());
        }
    }
    cols
}

fn parse_table_rows(lines: &[String], attrs: &Attributes, meta: &BlockMeta) -> Vec<TableRow> {
    // Track which rows are immediately followed by a blank line — used by
    // the spec's "first-row-then-blank-line" header heuristic.
    let mut rows: Vec<(TableRow, bool)> = Vec::new();
    for raw in lines {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            if let Some(last) = rows.last_mut() {
                last.1 = true;
            }
            continue;
        }
        // A row line starts with `|`; cells follow.
        if let Some((leading_prefix, rest)) = match_row_start(trimmed) {
            // Merge the row-leading formatter into cell 0's prefix BEFORE
            // building cells, so a| / m| / etc. picks the right code path
            // (recursive parse for a|, no-subs for l|, …).
            let mut splits = split_table_cells(rest);
            if let Some((prefix0, _)) = splits.first_mut() {
                if prefix0.style.is_none() {
                    prefix0.style = leading_prefix.style;
                }
                if prefix0.h_align.is_none() {
                    prefix0.h_align = leading_prefix.h_align;
                }
                if prefix0.colspan == 1 {
                    prefix0.colspan = leading_prefix.colspan;
                }
                if prefix0.rowspan == 1 {
                    prefix0.rowspan = leading_prefix.rowspan;
                }
            }
            let cells: Vec<TableCell> = splits
                .into_iter()
                .map(|(prefix, src)| build_cell(prefix, src.trim(), attrs))
                .collect();
            rows.push((
                TableRow {
                    cells,
                    kind: RowKind::Body,
                },
                false,
            ));
        }
    }

    // Header inference: explicit via [%header] / options=header, OR via the
    // blank-line-after-first-row heuristic.
    let explicit_header = meta.options.iter().any(|o| o == "header")
        || meta
            .named
            .get("options")
            .map(|s| s.split(',').any(|p| p.trim() == "header"))
            .unwrap_or(false);
    let inferred_header = rows.len() >= 2 && rows[0].1;
    if explicit_header || inferred_header {
        if let Some(first) = rows.first_mut() {
            first.0.kind = RowKind::Header;
        }
    }

    rows.into_iter().map(|(row, _)| row).collect()
}

fn build_cell(prefix: CellPrefix, src: &str, attrs: &Attributes) -> TableCell {
    let (inlines, blocks) = match prefix.style {
        // Literal cells: verbatim text, no inline subs.
        Some(CellStyle::Literal) => (
            vec![Inline::Text {
                value: src.to_string(),
            }],
            Vec::new(),
        ),
        // AsciiDoc cells: recursively parse the cell's text as nested
        // blocks. Inline-substitution attributes are passed through so
        // attribute references still resolve, but attribute *entries*
        // inside the cell don't bleed back to the outer document.
        Some(CellStyle::AsciiDoc) => (Vec::new(), parse_asciidoc_cell(src, attrs)),
        _ => (inline::parse(src, attrs, Subs::NORMAL), Vec::new()),
    };
    TableCell {
        inlines,
        blocks,
        style: prefix.style,
        h_align: prefix.h_align,
        colspan: prefix.colspan,
        rowspan: prefix.rowspan,
    }
}

/// Recognise the start of a table row: either a literal `|` or a
/// `<formatter>|` prefix where `<formatter>` is anything `parse_cell_format`
/// accepts (span, h-align, style, in any combination). Returns the
/// formatter (default for the literal-`|` case) and the slice that
/// follows the `|`. None ⇒ this line doesn't open a row.
fn match_row_start(line: &str) -> Option<(CellPrefix, &str)> {
    if let Some(rest) = line.strip_prefix('|') {
        return Some((CellPrefix::default(), rest));
    }
    let pipe_pos = line.find('|')?;
    let prefix_str = line[..pipe_pos].trim();
    if prefix_str.is_empty() {
        return None;
    }
    let prefix = parse_cell_format(prefix_str)?;
    Some((prefix, &line[pipe_pos + 1..]))
}

fn parse_asciidoc_cell(src: &str, attrs: &Attributes) -> Vec<Block> {
    use crate::preprocessor::PreprocessedLine;
    let location = Location::synthetic();
    let lines: Vec<PreprocessedLine> = src
        .split('\n')
        .map(|line| PreprocessedLine {
            text: line.to_string(),
            location: location.clone(),
        })
        .collect();
    let mut cursor = Cursor::new(&lines);
    let mut local_attrs = attrs.clone();
    parse_blocks(&mut cursor, &mut local_attrs, 0)
}

#[derive(Debug, Clone, Copy)]
struct CellPrefix {
    h_align: Option<HAlign>,
    style: Option<CellStyle>,
    colspan: u32,
    rowspan: u32,
}

impl Default for CellPrefix {
    fn default() -> Self {
        Self {
            h_align: None,
            style: None,
            colspan: 1,
            rowspan: 1,
        }
    }
}

fn split_table_cells(row: &str) -> Vec<(CellPrefix, String)> {
    // Split on unescaped `|`, peeling off any cell-formatter prefix glued
    // to the `|` that opens each cell. The formatter applies to the *next*
    // cell, not the one whose content it sits inside.
    let mut cells: Vec<(CellPrefix, String)> = Vec::new();
    let mut current = String::new();
    let mut next_prefix = CellPrefix::default();
    let mut escape = false;
    for ch in row.chars() {
        if escape {
            current.push(ch);
            escape = false;
            continue;
        }
        if ch == '\\' {
            escape = true;
            continue;
        }
        if ch == '|' {
            let extracted = peel_trailing_formatter(&mut current);
            cells.push((next_prefix, std::mem::take(&mut current)));
            next_prefix = extracted;
            continue;
        }
        current.push(ch);
    }
    cells.push((next_prefix, current));
    cells
}

/// Strip a trailing cell-formatter from `s` (the content of the *previous*
/// cell). The formatter region is delimited on the left by whitespace (or
/// the start of the string) and parsed as
/// `[span][halign][style]` where:
///
/// * `span`   = `[colspan_digits][.rowspan_digits]+` — the `+` is required
///   when span is present; without it, leading digits are not part of the
///   formatter and we back off.
/// * `halign` = one of `<`, `^`, `>`.
/// * `style`  = one of `a`, `e`, `h`, `l`, `m`, `s`.
///
/// Returns `Default` (no prefix; doesn't touch `s`) when nothing matches.
fn peel_trailing_formatter(s: &mut String) -> CellPrefix {
    let trimmed_end_len = s.trim_end_matches(|c: char| c.is_whitespace()).len();
    let trimmed = &s[..trimmed_end_len];
    if trimmed.is_empty() {
        return CellPrefix::default();
    }
    let candidate_start = trimmed
        .rfind(|c: char| c.is_whitespace())
        .map(|p| p + 1)
        .unwrap_or(0);
    let candidate = &trimmed[candidate_start..];
    let Some(prefix) = parse_cell_format(candidate) else {
        return CellPrefix::default();
    };
    s.truncate(candidate_start);
    prefix
}

/// Forward-parse a cell-formatter region. Returns `None` when the input
/// does not match the grammar exactly (any leftover bytes ⇒ reject).
fn parse_cell_format(s: &str) -> Option<CellPrefix> {
    if s.is_empty() {
        return None;
    }
    let bytes = s.as_bytes();
    let mut i = 0usize;

    // span: [digits][.digits]+
    let mut colspan: u32 = 1;
    let mut rowspan: u32 = 1;
    let span_start = i;
    let mut col_str = String::new();
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        col_str.push(bytes[i] as char);
        i += 1;
    }
    let mut row_str = String::new();
    let mut had_dot = false;
    if i < bytes.len() && bytes[i] == b'.' {
        had_dot = true;
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            row_str.push(bytes[i] as char);
            i += 1;
        }
    }
    let had_span_digits = !col_str.is_empty() || had_dot;
    if had_span_digits && i < bytes.len() && bytes[i] == b'+' {
        i += 1;
        if !col_str.is_empty() {
            colspan = col_str.parse::<u32>().unwrap_or(1).max(1);
        }
        if !row_str.is_empty() {
            rowspan = row_str.parse::<u32>().unwrap_or(1).max(1);
        }
    } else {
        // No `+` ⇒ the leading digits aren't a span. Reset.
        i = span_start;
    }

    // h-align
    let mut h_align = None;
    if i < bytes.len() {
        match bytes[i] {
            b'<' => {
                h_align = Some(HAlign::Left);
                i += 1;
            }
            b'^' => {
                h_align = Some(HAlign::Center);
                i += 1;
            }
            b'>' => {
                h_align = Some(HAlign::Right);
                i += 1;
            }
            _ => {}
        }
    }

    // style letter
    let mut style = None;
    if i < bytes.len() {
        if let Some(st) = letter_to_style(bytes[i] as char) {
            style = Some(st);
            i += 1;
        }
    }

    if i != bytes.len() {
        return None;
    }
    if colspan == 1 && rowspan == 1 && h_align.is_none() && style.is_none() {
        return None;
    }
    Some(CellPrefix {
        h_align,
        style,
        colspan,
        rowspan,
    })
}

fn letter_to_style(c: char) -> Option<CellStyle> {
    Some(match c {
        'a' => CellStyle::AsciiDoc,
        'm' => CellStyle::Monospace,
        's' => CellStyle::Strong,
        'e' => CellStyle::Emphasis,
        'h' => CellStyle::Header,
        'l' => CellStyle::Literal,
        _ => return None,
    })
}

// Re-export for crate root use.
pub(crate) use self::parse_blocks as parse_block_sequence;

// Suppress unused-warning for helpers referenced only internally.
#[allow(dead_code)]
fn _unused(_: Location) {}
