//! Conformance suite driver.
//!
//! Walks `tests/conformance/<entry>/` directories and, for each entry,
//! runs `input.adoc` through the full pipeline and compares the
//! serialized AST and rendered HTML against `expected.ast.json` and
//! `expected.html` respectively.
//!
//! Layout per entry:
//!
//!     tests/conformance/<entry>/
//!         input.adoc           — AsciiDoc source under test
//!         expected.ast.json    — pretty-printed [`adoc::ast::Document`]
//!         expected.html        — HTML5 output, rendered with no stylesheet
//!
//! ## Workflow
//!
//! * Run the suite: `cargo test --test conformance`. Failures print a
//!   per-entry summary with diff hints and the suite as a whole panics
//!   so CI catches regressions.
//! * Bless intentional changes:
//!   `ADOC_CONFORMANCE_BLESS=1 cargo test --test conformance`.
//!   The driver overwrites `expected.ast.json` / `expected.html` for
//!   every entry instead of comparing. Review the diff in git before
//!   committing.
//! * Add a new entry: create the directory + `input.adoc`, run with
//!   `ADOC_CONFORMANCE_BLESS=1` to populate the expected files, then
//!   review and commit.
//!
//! Why a separate suite when `tests/fixtures.rs` exists already: the
//! fixture suite asserts *structural* properties (counts, presence of
//! particular tags) and is the place where new features earn their
//! `#[test]`. The conformance suite asserts *byte-for-byte* output
//! against a frozen oracle, which is the form spec compliance ends up
//! looking like — much stricter, much easier to bless.

use adoc::ast::{Converter, Document};
use adoc::convert::html5::{Html5Converter, Html5Options, Stylesheet};
use adoc::parser::parse_with;
use adoc::preprocessor::Preprocessor;
use camino::Utf8Path;
use std::fs;
use std::path::{Path, PathBuf};

const BLESS_ENV: &str = "ADOC_CONFORMANCE_BLESS";
const AST_FILE: &str = "expected.ast.json";
const HTML_FILE: &str = "expected.html";
const INPUT_FILE: &str = "input.adoc";

fn conformance_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("conformance")
}

#[test]
fn conformance_suite() {
    let blessing = std::env::var(BLESS_ENV).is_ok();
    let entries =
        collect_entries(&conformance_dir()).expect("failed to enumerate tests/conformance/");

    if entries.is_empty() {
        eprintln!(
            "tests/conformance/ is empty — nothing to do. Add an entry with an `{INPUT_FILE}`."
        );
        return;
    }

    let mut failures: Vec<String> = Vec::new();
    for entry in &entries {
        match run_entry(entry, blessing) {
            Ok(()) => {}
            Err(msg) => failures.push(format!("[{}] {msg}", entry.name)),
        }
    }

    if !failures.is_empty() {
        let summary = failures.join("\n\n");
        panic!(
            "{n} conformance entr{plural} failed:\n\n{summary}\n\n\
            Hint: run with `{BLESS_ENV}=1 cargo test --test conformance` \
            to regenerate `{AST_FILE}` / `{HTML_FILE}` and review the diff.",
            n = failures.len(),
            plural = if failures.len() == 1 { "y" } else { "ies" },
        );
    }
}

#[derive(Debug)]
struct Entry {
    name: String,
    dir: PathBuf,
}

fn collect_entries(dir: &Path) -> std::io::Result<Vec<Entry>> {
    let mut out = Vec::new();
    if !dir.exists() {
        return Ok(out);
    }
    for child in fs::read_dir(dir)? {
        let child = child?;
        let path = child.path();
        if !path.is_dir() {
            continue;
        }
        // Each entry directory must contain an input.adoc — anything
        // else is metadata (README, etc.) and is ignored.
        if !path.join(INPUT_FILE).is_file() {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?")
            .to_string();
        out.push(Entry { name, dir: path });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

fn run_entry(entry: &Entry, blessing: bool) -> Result<(), String> {
    let input_path = entry.dir.join(INPUT_FILE);
    let src = fs::read_to_string(&input_path).map_err(|e| format!("read {INPUT_FILE}: {e}"))?;
    let utf8 = Utf8Path::from_path(&input_path).ok_or("input path not UTF-8")?;

    let mut pre = Preprocessor::default();
    let lines = pre
        .run(&src, utf8)
        .map_err(|e| format!("preprocess: {e}"))?;
    let doc: Document =
        parse_with(&lines, Default::default()).map_err(|e| format!("parse: {e}"))?;

    let ast_actual =
        serde_json::to_string_pretty(&doc).map_err(|e| format!("AST serialize: {e}"))? + "\n";

    // No stylesheet so HTML diffs reflect *content* changes, not
    // unrelated CSS edits.
    let converter = Html5Converter::with_options(Html5Options {
        stylesheet: Stylesheet::None,
    });
    let html_actual = converter
        .convert(&doc)
        .map_err(|e| format!("convert: {e}"))?;

    let ast_path = entry.dir.join(AST_FILE);
    let html_path = entry.dir.join(HTML_FILE);

    if blessing {
        fs::write(&ast_path, &ast_actual).map_err(|e| format!("write {AST_FILE}: {e}"))?;
        fs::write(&html_path, &html_actual).map_err(|e| format!("write {HTML_FILE}: {e}"))?;
        return Ok(());
    }

    let ast_expected =
        fs::read_to_string(&ast_path).map_err(|e| format!("read {AST_FILE}: {e}"))?;
    if ast_actual != ast_expected {
        return Err(format!(
            "AST drift in {AST_FILE}\n--- expected ---\n{}\n--- actual ---\n{}",
            preview(&ast_expected),
            preview(&ast_actual),
        ));
    }

    let html_expected =
        fs::read_to_string(&html_path).map_err(|e| format!("read {HTML_FILE}: {e}"))?;
    if html_actual != html_expected {
        return Err(format!(
            "HTML drift in {HTML_FILE}\n--- expected ---\n{}\n--- actual ---\n{}",
            preview(&html_expected),
            preview(&html_actual),
        ));
    }
    Ok(())
}

/// Compress a long expected/actual blob into something the panic
/// handler can render readably without dominating CI logs.
fn preview(s: &str) -> String {
    const MAX_LINES: usize = 60;
    let lines: Vec<&str> = s.lines().collect();
    if lines.len() <= MAX_LINES {
        s.to_string()
    } else {
        let head = lines[..MAX_LINES / 2].join("\n");
        let tail = lines[lines.len() - MAX_LINES / 2..].join("\n");
        format!(
            "{head}\n…\n[{} more lines]\n…\n{tail}",
            lines.len() - MAX_LINES,
        )
    }
}
