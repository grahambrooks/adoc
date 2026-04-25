//! Block parser.
//!
//! Recognises sections, paragraphs, ordered/unordered/description lists,
//! all seven delimited block styles, and basic tables. Inline text inside
//! blocks is resolved immediately using the accumulated attribute context.

use crate::ast::{
    Attributes, Block, BlockMeta, DelimitedBlock, DelimitedContent, DelimitedStyle,
    DescriptionList, DescriptionListItem, Inline, List, ListItem, ListMarker, Location, Paragraph,
    Section, Table, TableCell, TableRow,
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
        // immediately precedes the upcoming block.
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
                break;
            }
            let section = parse_section(cursor, attrs, level, meta);
            out.push(Block::Section(section));
            continue;
        }
        if let Some(block) = parse_one_block(cursor, attrs, meta) {
            out.push(block);
        } else {
            break;
        }
    }
    out
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

fn parse_paragraph(cursor: &mut Cursor, attrs: &Attributes, meta: BlockMeta) -> Paragraph {
    let location = cursor.current_location();
    let mut lines: Vec<String> = Vec::new();
    while let Some(line) = cursor.peek() {
        let t = line.text.as_str();
        if t.trim().is_empty() {
            break;
        }
        if is_block_boundary(t) {
            break;
        }
        lines.push(line.text.clone());
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
            out.push(Inline::Text(" ".to_string()));
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
            (Some(Inline::Text(a)), Inline::Text(b)) => a.push_str(&b),
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
    let rows = parse_table_rows(&lines, attrs);
    Table {
        rows,
        location,
        meta,
    }
}

fn parse_table_rows(lines: &[String], attrs: &Attributes) -> Vec<TableRow> {
    let mut rows: Vec<TableRow> = Vec::new();
    for raw in lines {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        // A row line starts with `|`; split on ` | ` boundaries.
        if let Some(rest) = trimmed.strip_prefix('|') {
            let cells: Vec<TableCell> = split_table_cells(rest)
                .into_iter()
                .map(|src| TableCell {
                    inlines: inline::parse(src.trim(), attrs, Subs::NORMAL),
                })
                .collect();
            rows.push(TableRow { cells });
        }
    }
    rows
}

fn split_table_cells(row: &str) -> Vec<String> {
    // Split on unescaped `|`. Does not yet honour cell specs (a|, m|, etc.).
    let mut cells: Vec<String> = Vec::new();
    let mut current = String::new();
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
            cells.push(std::mem::take(&mut current));
            continue;
        }
        current.push(ch);
    }
    cells.push(current);
    cells
}

// Re-export for crate root use.
pub(crate) use self::parse_blocks as parse_block_sequence;

// Suppress unused-warning for helpers referenced only internally.
#[allow(dead_code)]
fn _unused(_: Location) {}
