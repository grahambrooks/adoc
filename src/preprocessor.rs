//! AsciiDoc preprocessor.
//!
//! Handles the line-level constructs that need to run before the block
//! parser sees the source:
//!
//! - `include::path[]` — splice a file inline; relative paths resolve
//!   against the including file's directory. Each included source is
//!   assigned its own [`SourceId`], so [`Location`]s on downstream AST
//!   nodes still point through the include chain.
//! - `ifdef::name[]` / `ifndef::name[]` — block-form conditional with a
//!   matching `endif::[]`. Comma-separated names are *any-of* (OR);
//!   `+`-separated names are *all-of* (AND). The single-line form
//!   `ifdef::name[content]` emits the bracketed content only if true.
//! - `ifeval::[expr]` — block-form conditional driven by a comparison
//!   between two values (numbers, quoted strings, or attribute refs)
//!   using `==`, `!=`, `<`, `<=`, `>`, `>=`.
//! - Attribute entries (`:name: value`, `:!name:`, `:name!:`) — applied
//!   to the running attribute set so subsequent conditionals see them.
//!   Entries are also passed through to the parser, which re-applies
//!   them; the two passes converge.
//!
//! Output is a flat stream of [`PreprocessedLine`]s; each line carries
//! a [`SourceId`] indexing into [`Preprocessor::sources`].

use camino::{Utf8Path, Utf8PathBuf};
use std::fs;

use crate::ast::{AttributeValue, Attributes, Location, SourceId, SourceMap};

#[derive(Debug, Clone)]
pub struct PreprocessedLine {
    pub text: String,
    pub location: Location,
}

#[derive(Debug, thiserror::Error)]
pub enum PreprocessError {
    #[error("preprocessor error: {0}")]
    Message(String),
    #[error("io error reading {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    /// Span-carrying error — the call site built a [`crate::diag::Diagnostic`]
    /// pointing at the offending source location. Rendered by the CLI
    /// through miette so users see file:line:col + a snippet, not just
    /// the message string.
    #[error("{}", .0.message)]
    Diagnostic(Box<crate::diag::Diagnostic>),
}

impl PreprocessError {
    /// Build a span-carrying error from a [`crate::diag::Diagnostic`].
    pub fn diagnostic(d: crate::diag::Diagnostic) -> Self {
        Self::Diagnostic(Box::new(d))
    }

    /// If this error carries a [`Diagnostic`] payload, return it. Used
    /// by the CLI to render with miette's graphical handler instead of
    /// a plain message.
    pub fn as_diagnostic(&self) -> Option<&crate::diag::Diagnostic> {
        match self {
            Self::Diagnostic(d) => Some(d),
            _ => None,
        }
    }
}

/// Mirrors the CLI safe-mode flag enough to gate filesystem-touching
/// constructs.
///
/// - `Unsafe` (library default): no path checks.
/// - `Safe` / `Server`: `include::` paths must be relative AND must resolve
///   under [`Preprocessor::with_base_dir`] after canonicalisation.
/// - `Secure`: `include::` is disabled entirely.
///
/// The library defaults to `Unsafe` because a library can't know the
/// caller's threat model; the CLI defaults to `Safe` and explicitly
/// threads its choice through [`Preprocessor::with_safe_mode`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SafeMode {
    #[default]
    Unsafe,
    Safe,
    Server,
    Secure,
}

const DEFAULT_MAX_INCLUDE_DEPTH: u32 = 64;

pub struct Preprocessor {
    pub attributes: Attributes,
    base_dir: Utf8PathBuf,
    safe_mode: SafeMode,
    sources: Vec<Utf8PathBuf>,
    /// Source-text-by-id, kept around so diagnostics can render
    /// span-pointing snippets after the pipeline runs.
    source_contents: Vec<String>,
    max_include_depth: u32,
}

impl Default for Preprocessor {
    fn default() -> Self {
        Self::new()
    }
}

impl Preprocessor {
    pub fn new() -> Self {
        Self {
            attributes: Attributes::new(),
            base_dir: Utf8PathBuf::from("."),
            safe_mode: SafeMode::default(),
            sources: Vec::new(),
            source_contents: Vec::new(),
            max_include_depth: DEFAULT_MAX_INCLUDE_DEPTH,
        }
    }

    pub fn with_attributes(attributes: Attributes) -> Self {
        Self {
            attributes,
            ..Self::new()
        }
    }

    pub fn with_base_dir(mut self, dir: impl Into<Utf8PathBuf>) -> Self {
        self.base_dir = dir.into();
        self
    }

    pub fn with_safe_mode(mut self, mode: SafeMode) -> Self {
        self.safe_mode = mode;
        self
    }

    /// `SourceId(i) -> path` registry. Populated by [`run`] and [`run_file`]
    /// across the top source plus everything pulled in via `include::`.
    pub fn sources(&self) -> &[Utf8PathBuf] {
        &self.sources
    }

    /// Build a [`SourceMap`] from the registered sources and their text.
    /// Consumed by the diagnostics renderer so error spans can resolve
    /// to file path + snippet. Call after [`run`] or [`run_file`].
    pub fn source_map(&self) -> SourceMap {
        let mut map = SourceMap::new();
        for (i, path) in self.sources.iter().enumerate() {
            let content = self.source_contents.get(i).cloned().unwrap_or_default();
            map.push(path.as_str().to_string(), content);
        }
        map
    }

    /// Process a top-level source string. `top_path` is recorded for
    /// `SourceId(0)` and used as the base for resolving relative
    /// `include::` paths. For non-file input, pass a synthetic path
    /// such as `Utf8Path::new("<input>")`.
    pub fn run(
        &mut self,
        source: &str,
        top_path: &Utf8Path,
    ) -> Result<Vec<PreprocessedLine>, PreprocessError> {
        self.sources.clear();
        self.source_contents.clear();
        let top_id = self.register_source(top_path.to_owned(), source.to_string());
        let mut output = Vec::new();
        let mut state = ProcessState::default();
        state.include_chain.push(top_path.to_owned());
        self.process_source(source, top_id, top_path, &mut state, &mut output)?;
        state.include_chain.pop();
        if !state.cond_stack.is_empty() {
            return Err(PreprocessError::Message(
                "unclosed ifdef/ifndef/ifeval at end of input".into(),
            ));
        }
        Ok(output)
    }

    /// Convenience wrapper that reads `path` from disk and processes it.
    pub fn run_file(&mut self, path: &Utf8Path) -> Result<Vec<PreprocessedLine>, PreprocessError> {
        let source = read_file(path)?;
        self.run(&source, path)
    }

    fn register_source(&mut self, path: Utf8PathBuf, content: String) -> SourceId {
        let id = SourceId(self.sources.len() as u32);
        self.sources.push(path);
        self.source_contents.push(content);
        id
    }

    fn process_source(
        &mut self,
        source: &str,
        source_id: SourceId,
        source_path: &Utf8Path,
        state: &mut ProcessState,
        output: &mut Vec<PreprocessedLine>,
    ) -> Result<(), PreprocessError> {
        let mut byte_start = 0u32;
        for (idx, raw) in source.split('\n').enumerate() {
            let text = raw.strip_suffix('\r').unwrap_or(raw);
            let byte_end = byte_start + text.len() as u32;
            let location = Location {
                source: source_id,
                byte_start,
                byte_end,
                line: (idx + 1) as u32,
                column: 1,
            };
            byte_start = byte_end + raw.len().saturating_sub(text.len()) as u32 + 1;
            self.process_line(text, &location, source_path, state, output)?;
        }
        Ok(())
    }

    fn process_line(
        &mut self,
        text: &str,
        location: &Location,
        source_path: &Utf8Path,
        state: &mut ProcessState,
        output: &mut Vec<PreprocessedLine>,
    ) -> Result<(), PreprocessError> {
        if let Some(directive) = parse_directive(text) {
            return self.handle_directive(directive, location, source_path, state, output);
        }

        if !state.emitting() {
            return Ok(());
        }

        // Apply attribute entries to the running set so subsequent
        // conditionals see them. The line is still passed through to
        // the parser, which will re-apply it; the two passes converge.
        if let Some((name, value)) = parse_attribute_entry_line(text) {
            self.attributes.insert(name, value);
        }

        output.push(PreprocessedLine {
            text: text.to_string(),
            location: location.clone(),
        });
        Ok(())
    }

    fn handle_directive(
        &mut self,
        directive: Directive,
        location: &Location,
        source_path: &Utf8Path,
        state: &mut ProcessState,
        output: &mut Vec<PreprocessedLine>,
    ) -> Result<(), PreprocessError> {
        match directive {
            Directive::Ifdef { test, content } => {
                let active = state.emitting() && eval_attr_test(&test, &self.attributes);
                self.handle_conditional(state, output, location, active, content)
            }
            Directive::Ifndef { test, content } => {
                let active = state.emitting() && !eval_attr_test(&test, &self.attributes);
                self.handle_conditional(state, output, location, active, content)
            }
            Directive::Ifeval { expr } => {
                let active = state.emitting() && eval_ifeval(&expr, &self.attributes);
                state.cond_stack.push(active);
                Ok(())
            }
            Directive::Endif => {
                if state.cond_stack.is_empty() {
                    return Err(PreprocessError::diagnostic(
                        crate::diag::Diagnostic::error(
                            "adoc::preprocess::stray_endif",
                            "endif:: without a matching ifdef/ifndef/ifeval",
                            location.clone(),
                        )
                        .with_label("no open conditional to close"),
                    ));
                }
                state.cond_stack.pop();
                Ok(())
            }
            Directive::Include { target, args } => {
                if !state.emitting() {
                    return Ok(());
                }
                self.handle_include(&target, &args, source_path, location, state, output)
            }
        }
    }

    fn handle_conditional(
        &mut self,
        state: &mut ProcessState,
        output: &mut Vec<PreprocessedLine>,
        location: &Location,
        active: bool,
        inline_content: Option<String>,
    ) -> Result<(), PreprocessError> {
        match inline_content {
            Some(content) => {
                if active {
                    output.push(PreprocessedLine {
                        text: content,
                        location: location.clone(),
                    });
                }
                Ok(())
            }
            None => {
                state.cond_stack.push(active);
                Ok(())
            }
        }
    }

    fn handle_include(
        &mut self,
        target: &str,
        args: &str,
        current_path: &Utf8Path,
        directive_loc: &Location,
        state: &mut ProcessState,
        output: &mut Vec<PreprocessedLine>,
    ) -> Result<(), PreprocessError> {
        if matches!(self.safe_mode, SafeMode::Secure) {
            return Err(PreprocessError::diagnostic(
                crate::diag::Diagnostic::error(
                    "adoc::preprocess::secure_mode",
                    "include:: is disabled in safe mode 'secure'",
                    directive_loc.clone(),
                )
                .with_label("include rejected by safe mode")
                .with_help("relax with `--safe-mode safe` (or `unsafe`) if the input is trusted"),
            ));
        }

        let target = target.trim();
        let raw_target = Utf8PathBuf::from(target);
        if matches!(self.safe_mode, SafeMode::Safe | SafeMode::Server) && raw_target.is_absolute() {
            return Err(PreprocessError::diagnostic(
                crate::diag::Diagnostic::error(
                    "adoc::preprocess::absolute_include",
                    format!("absolute include path `{raw_target}` rejected in safe mode"),
                    directive_loc.clone(),
                )
                .with_label("absolute paths are denied by safe mode")
                .with_help("use a relative path under --base-dir, or pass `--safe-mode unsafe`"),
            ));
        }

        let resolved = self.resolve_include_path(target, current_path);

        if matches!(self.safe_mode, SafeMode::Safe | SafeMode::Server) {
            self.ensure_under_base_dir(&resolved)
                .map_err(|e| upgrade_unscoped(e, directive_loc))?;
        }

        if state.include_chain.iter().any(|p| p == &resolved) {
            let chain = state
                .include_chain
                .iter()
                .map(|p| p.as_str())
                .collect::<Vec<_>>()
                .join(" → ");
            return Err(PreprocessError::diagnostic(
                crate::diag::Diagnostic::error(
                    "adoc::preprocess::include_cycle",
                    format!("include cycle detected: {chain} → {resolved}"),
                    directive_loc.clone(),
                )
                .with_label("this include closes a cycle"),
            ));
        }
        if state.include_chain.len() as u32 >= self.max_include_depth {
            return Err(PreprocessError::diagnostic(
                crate::diag::Diagnostic::error(
                    "adoc::preprocess::include_depth",
                    format!("include depth limit reached ({})", self.max_include_depth),
                    directive_loc.clone(),
                )
                .with_label("nesting goes too deep here"),
            ));
        }

        let raw_source = read_file(&resolved)?;
        let parsed_args = parse_include_args(args);
        let filtered = apply_include_args(&raw_source, &parsed_args);

        let source_id = self.register_source(resolved.clone(), raw_source.clone());
        state.include_chain.push(resolved.clone());
        let owned = resolved;
        self.process_source(&filtered, source_id, &owned, state, output)?;
        state.include_chain.pop();
        Ok(())
    }

    fn ensure_under_base_dir(&self, resolved: &Utf8Path) -> Result<(), PreprocessError> {
        let base_canon = self.base_dir.canonicalize_utf8().map_err(|e| {
            PreprocessError::Message(format!("base_dir {} not accessible: {e}", self.base_dir))
        })?;
        let resolved_canon =
            resolved
                .canonicalize_utf8()
                .map_err(|source| PreprocessError::Io {
                    path: resolved.to_string(),
                    source,
                })?;
        if !resolved_canon.starts_with(&base_canon) {
            return Err(PreprocessError::Message(format!(
                "include path {resolved_canon} escapes base_dir {base_canon} (safe mode)"
            )));
        }
        Ok(())
    }
}

/// Promote a `PreprocessError::Message(...)` into a span-carrying
/// `Diagnostic` variant when the caller knows the source location.
/// Other variants pass through unchanged.
fn upgrade_unscoped(e: PreprocessError, location: &Location) -> PreprocessError {
    match e {
        PreprocessError::Message(msg) => PreprocessError::diagnostic(
            crate::diag::Diagnostic::error("adoc::preprocess::include_path", msg, location.clone())
                .with_label("rejected by safe mode"),
        ),
        other => other,
    }
}

// Reopen the impl block — `upgrade_unscoped` lives at module scope so
// it can sit between `Preprocessor` methods and the lower helpers.
impl Preprocessor {
    fn resolve_include_path(&self, target: &str, current_path: &Utf8Path) -> Utf8PathBuf {
        let raw = Utf8PathBuf::from(target);
        if raw.is_absolute() {
            return raw;
        }
        let parent = current_path
            .parent()
            .filter(|p| !p.as_str().is_empty())
            .map(Utf8Path::to_path_buf)
            .unwrap_or_else(|| self.base_dir.clone());
        parent.join(raw)
    }
}

#[derive(Default)]
struct ProcessState {
    /// Stack of "is this scope currently emitting" booleans. The whole
    /// stack must be true for output to flow.
    cond_stack: Vec<bool>,
    /// Sources currently being processed (top + every active `include::`),
    /// for cycle detection.
    include_chain: Vec<Utf8PathBuf>,
}

impl ProcessState {
    fn emitting(&self) -> bool {
        self.cond_stack.iter().all(|&b| b)
    }
}

// --- directive parsing ----------------------------------------------------

#[derive(Debug)]
enum Directive {
    Include {
        target: String,
        args: String,
    },
    Ifdef {
        test: AttrTest,
        content: Option<String>,
    },
    Ifndef {
        test: AttrTest,
        content: Option<String>,
    },
    Ifeval {
        expr: String,
    },
    Endif,
}

#[derive(Debug)]
enum AttrTest {
    AnyOf(Vec<String>),
    AllOf(Vec<String>),
}

fn parse_directive(text: &str) -> Option<Directive> {
    let trimmed = text.trim_end();
    for prefix in ["include::", "ifdef::", "ifndef::", "ifeval::", "endif::"] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            let (head, body) = split_directive_body(rest)?;
            return Some(match prefix {
                "include::" => Directive::Include {
                    target: head.to_string(),
                    args: body.to_string(),
                },
                "ifdef::" => Directive::Ifdef {
                    test: parse_attr_test(head)?,
                    content: nonempty(body),
                },
                "ifndef::" => Directive::Ifndef {
                    test: parse_attr_test(head)?,
                    content: nonempty(body),
                },
                "ifeval::" => {
                    if !head.is_empty() {
                        return None;
                    }
                    Directive::Ifeval {
                        expr: body.to_string(),
                    }
                }
                "endif::" => Directive::Endif,
                _ => unreachable!(),
            });
        }
    }
    None
}

/// Split a directive tail of the form `HEAD[BODY]` into its two halves.
fn split_directive_body(s: &str) -> Option<(&str, &str)> {
    let open = s.find('[')?;
    if !s.ends_with(']') {
        return None;
    }
    Some((&s[..open], &s[open + 1..s.len() - 1]))
}

fn parse_attr_test(s: &str) -> Option<AttrTest> {
    if s.contains('+') {
        let names: Vec<String> = s
            .split('+')
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty())
            .collect();
        if names.is_empty() {
            return None;
        }
        Some(AttrTest::AllOf(names))
    } else {
        let names: Vec<String> = s
            .split(',')
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty())
            .collect();
        if names.is_empty() {
            return None;
        }
        Some(AttrTest::AnyOf(names))
    }
}

fn nonempty(s: &str) -> Option<String> {
    if s.is_empty() {
        None
    } else {
        Some(s.to_string())
    }
}

// --- include arguments ----------------------------------------------------

#[derive(Debug, Default)]
struct IncludeArgs {
    /// Inclusive 1-based line ranges. `-1` means "last line"; an `end` of
    /// `-1` (after resolution) means "to end of file". Multiple ranges are
    /// unioned.
    lines: Option<Vec<LineRange>>,
    tags: Option<TagSelector>,
    /// Signed offset added to every section header level in the included
    /// content. Final level is clamped to `1..=6`.
    leveloffset: Option<i32>,
    /// Re-indent the included content. `0` strips all leading whitespace;
    /// `N>0` strips the common leading whitespace and then prepends N
    /// spaces to every non-empty line.
    indent: Option<u32>,
    /// `encoding=` is accepted for spec compatibility but treated as a
    /// no-op in v1 — the loader always decodes as UTF-8. Stored so a
    /// future diagnostics pass can warn when the user asked for a
    /// non-UTF-8 encoding.
    #[allow(dead_code)]
    encoding: Option<String>,
}

#[derive(Debug, Clone, Copy)]
struct LineRange {
    start: i64,
    end: i64,
}

#[derive(Debug, Default)]
struct TagSelector {
    include: Vec<String>,
    exclude: Vec<String>,
    /// `*` wildcard — match any tagged region (a line inside *some*
    /// `tag::name[]` … `end::name[]` block).
    wildcard_tagged: bool,
    /// `**` wildcard — match every line, tagged or not. Combined with
    /// negative selectors (`!foo`) to exclude specific regions.
    wildcard_all: bool,
}

fn parse_include_args(s: &str) -> IncludeArgs {
    let mut args = IncludeArgs::default();
    for part in split_attrlist_top_level(s) {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let Some((name, value)) = split_attr_pair(part) else {
            continue;
        };
        let v = strip_quotes(value);
        match name {
            "lines" => args.lines = Some(parse_line_ranges(v)),
            "tag" | "tags" => args.tags = Some(parse_tag_selector(v)),
            "leveloffset" => args.leveloffset = parse_leveloffset(v),
            "indent" => args.indent = v.trim().parse::<u32>().ok(),
            "encoding" => args.encoding = Some(v.trim().to_string()),
            _ => {}
        }
    }
    args
}

fn apply_include_args(source: &str, args: &IncludeArgs) -> String {
    let mut current = if let Some(ref tags) = args.tags {
        apply_tags_filter(source, tags)
    } else if let Some(ref ranges) = args.lines {
        apply_lines_filter(source, ranges)
    } else {
        source.to_string()
    };
    if let Some(delta) = args.leveloffset {
        if delta != 0 {
            current = apply_leveloffset(&current, delta);
        }
    }
    if let Some(target) = args.indent {
        current = apply_indent(&current, target);
    }
    current
}

/// Re-indent the included text: strip the common leading whitespace from
/// every non-empty line, then prepend `target` spaces. `target == 0`
/// means "strip leading whitespace entirely" (the spec's documented
/// behaviour for `indent=0`).
fn apply_indent(text: &str, target: u32) -> String {
    let lines: Vec<&str> = text.split('\n').collect();
    let common = lines
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.bytes().take_while(|b| *b == b' ').count())
        .min()
        .unwrap_or(0);
    let pad = " ".repeat(target as usize);
    let mut out = String::with_capacity(text.len());
    for (i, line) in lines.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        if line.trim().is_empty() {
            // Preserve blank lines as fully blank (no trailing spaces).
            continue;
        }
        let stripped = if line.len() >= common {
            &line[common..]
        } else {
            line.trim_start()
        };
        out.push_str(&pad);
        out.push_str(stripped);
    }
    out
}

/// Split a comma-separated attrlist body, treating commas inside double
/// quotes as literal. Mirrors `parser::meta::split_top_level_commas` but
/// kept private to avoid coupling the two modules.
fn split_attrlist_top_level(s: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut in_quote = false;
    let mut escape = false;
    for ch in s.chars() {
        if escape {
            current.push(ch);
            escape = false;
            continue;
        }
        match ch {
            '\\' => escape = true,
            '"' => {
                in_quote = !in_quote;
                current.push(ch);
            }
            ',' if !in_quote => parts.push(std::mem::take(&mut current)),
            _ => current.push(ch),
        }
    }
    parts.push(current);
    parts
}

fn split_attr_pair(part: &str) -> Option<(&str, &str)> {
    let eq = part.find('=')?;
    let name = part[..eq].trim();
    if !is_attr_name(name) {
        return None;
    }
    Some((name, part[eq + 1..].trim()))
}

fn strip_quotes(s: &str) -> &str {
    let s = s.trim();
    if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
        &s[1..s.len() - 1]
    } else {
        s
    }
}

fn parse_line_ranges(s: &str) -> Vec<LineRange> {
    s.split([';', ','])
        .filter_map(|piece| parse_one_line_range(piece.trim()))
        .collect()
}

fn parse_one_line_range(s: &str) -> Option<LineRange> {
    if s.is_empty() {
        return None;
    }
    if let Some(idx) = s.find("..") {
        let lo = s[..idx].trim();
        let hi = s[idx + 2..].trim();
        let start = lo.parse::<i64>().ok()?;
        let end = hi.parse::<i64>().ok()?;
        Some(LineRange { start, end })
    } else {
        let n = s.parse::<i64>().ok()?;
        Some(LineRange { start: n, end: n })
    }
}

fn apply_lines_filter(source: &str, ranges: &[LineRange]) -> String {
    let mut lines: Vec<&str> = source.split('\n').collect();
    // A source ending in '\n' produces a trailing empty element; treat it
    // as the file's terminating newline rather than a numbered line.
    let trailing_newline = lines.last() == Some(&"");
    if trailing_newline {
        lines.pop();
    }
    let total = lines.len() as i64;
    let resolve = |n: i64| -> i64 {
        if n < 0 {
            total + n + 1
        } else {
            n
        }
    };
    let mut keep = vec![false; lines.len()];
    for r in ranges {
        let start = resolve(r.start).max(1);
        let end = if r.end == -1 { total } else { resolve(r.end) };
        if end < start {
            continue;
        }
        for i in start..=end {
            if (1..=total).contains(&i) {
                keep[(i - 1) as usize] = true;
            }
        }
    }
    let kept: Vec<&str> = lines
        .iter()
        .enumerate()
        .filter(|(i, _)| keep[*i])
        .map(|(_, l)| *l)
        .collect();
    let mut result = kept.join("\n");
    if trailing_newline && !result.is_empty() {
        result.push('\n');
    }
    result
}

fn parse_tag_selector(s: &str) -> TagSelector {
    let mut sel = TagSelector::default();
    for piece in s.split([';', ',']) {
        let piece = piece.trim();
        if piece.is_empty() {
            continue;
        }
        if let Some(rest) = piece.strip_prefix('!') {
            let name = rest.trim();
            match name {
                "*" | "**" => {
                    // Negative wildcards aren't meaningful in v1 — we
                    // accept them silently rather than failing.
                }
                "" => {}
                _ => sel.exclude.push(name.to_string()),
            }
        } else {
            match piece {
                "**" => sel.wildcard_all = true,
                "*" => sel.wildcard_tagged = true,
                _ => sel.include.push(piece.to_string()),
            }
        }
    }
    sel
}

fn apply_tags_filter(source: &str, selector: &TagSelector) -> String {
    let mut active: Vec<String> = Vec::new();
    let mut out_lines: Vec<&str> = Vec::new();
    let lines: Vec<&str> = source.split('\n').collect();
    let trailing_newline = lines.last() == Some(&"");

    for (idx, line) in lines.iter().enumerate() {
        if trailing_newline && idx == lines.len() - 1 {
            // Skip the trailing empty pseudo-line; restored at the end.
            continue;
        }
        if let Some(name) = parse_tag_marker(line, "tag::") {
            active.push(name.to_string());
            continue;
        }
        if let Some(name) = parse_tag_marker(line, "end::") {
            if let Some(pos) = active.iter().rposition(|n| n == name) {
                active.remove(pos);
            }
            continue;
        }
        let any_excluded = active
            .iter()
            .any(|a| selector.exclude.iter().any(|e| e == a));
        if any_excluded {
            continue;
        }
        let any_named_included = active
            .iter()
            .any(|a| selector.include.iter().any(|i| i == a));
        // Wildcard rules (Asciidoctor convention):
        //   `*`   — emit lines inside any tagged region
        //   `**`  — emit every line (the `!name` exclusions still apply)
        let wildcard_match = if selector.wildcard_all {
            true
        } else if selector.wildcard_tagged {
            !active.is_empty()
        } else {
            false
        };
        if any_named_included || wildcard_match {
            out_lines.push(line);
        }
    }
    let mut result = out_lines.join("\n");
    if trailing_newline && !result.is_empty() {
        result.push('\n');
    }
    result
}

fn parse_tag_marker<'a>(line: &'a str, kw: &str) -> Option<&'a str> {
    let start = line.find(kw)?;
    if start > 0 {
        let prev = line[..start].chars().last()?;
        if prev.is_alphanumeric() || prev == '_' {
            return None;
        }
    }
    let after = &line[start + kw.len()..];
    let bracket = after.find('[')?;
    if !after[bracket..].starts_with("[]") {
        return None;
    }
    let name = after[..bracket].trim();
    if name.is_empty() {
        return None;
    }
    Some(name)
}

fn parse_leveloffset(s: &str) -> Option<i32> {
    s.trim().parse::<i32>().ok()
}

fn apply_leveloffset(source: &str, delta: i32) -> String {
    let mut out = String::with_capacity(source.len());
    let mut first = true;
    for line in source.split('\n') {
        if !first {
            out.push('\n');
        }
        first = false;
        let bytes = line.as_bytes();
        let mut eq_count = 0;
        while eq_count < bytes.len() && bytes[eq_count] == b'=' {
            eq_count += 1;
        }
        let is_section_header = (1..=6).contains(&eq_count) && bytes.get(eq_count) == Some(&b' ');
        if is_section_header {
            let new_count = (eq_count as i32 + delta).clamp(1, 6) as usize;
            for _ in 0..new_count {
                out.push('=');
            }
            out.push_str(&line[eq_count..]);
        } else {
            out.push_str(line);
        }
    }
    out
}

// --- attribute entries ----------------------------------------------------

/// Recognises `:name: value`, `:name:`, `:!name:`, `:name!:`. Mirrors
/// `parser::header::parse_attribute_entry`; duplicated here so the
/// preprocessor can update its attribute set without depending on the
/// parser.
fn parse_attribute_entry_line(text: &str) -> Option<(String, AttributeValue)> {
    let t = text.trim_end();
    if !t.starts_with(':') {
        return None;
    }
    let rest = &t[1..];
    let (name_part, value_part) = rest.split_once(':')?;
    let (name, negate) = if let Some(stripped) = name_part.strip_prefix('!') {
        (stripped.to_string(), true)
    } else if let Some(stripped) = name_part.strip_suffix('!') {
        (stripped.to_string(), true)
    } else {
        (name_part.to_string(), false)
    };
    if name.is_empty() || !is_attr_name(&name) {
        return None;
    }
    let value = value_part.trim();
    if negate {
        Some((name, AttributeValue::Bool(false)))
    } else if value.is_empty() {
        Some((name, AttributeValue::Bool(true)))
    } else {
        Some((name, AttributeValue::String(value.to_string())))
    }
}

fn is_attr_name(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphanumeric() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

// --- conditional evaluation -----------------------------------------------

fn eval_attr_test(test: &AttrTest, attrs: &Attributes) -> bool {
    match test {
        AttrTest::AnyOf(names) => names.iter().any(|n| is_set(attrs, n)),
        AttrTest::AllOf(names) => names.iter().all(|n| is_set(attrs, n)),
    }
}

fn is_set(attrs: &Attributes, name: &str) -> bool {
    !matches!(attrs.get(name), None | Some(AttributeValue::Bool(false)))
}

fn eval_ifeval(expr: &str, attrs: &Attributes) -> bool {
    let expr = expr.trim();
    let Some((lhs, op, rhs)) = find_top_level_op(expr) else {
        return false;
    };
    let l = resolve_value(lhs, attrs);
    let r = resolve_value(rhs, attrs);

    if let (Ok(ln), Ok(rn)) = (l.parse::<f64>(), r.parse::<f64>()) {
        return apply_num_op(ln, op, rn);
    }
    apply_str_op(&l, op, &r)
}

fn find_top_level_op(s: &str) -> Option<(&str, &str, &str)> {
    let bytes = s.as_bytes();
    let mut in_quote = false;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'"' {
            in_quote = !in_quote;
            i += 1;
            continue;
        }
        if in_quote {
            i += 1;
            continue;
        }
        if i + 1 < bytes.len() {
            let two = &s[i..i + 2];
            if matches!(two, "==" | "!=" | "<=" | ">=") {
                return Some((s[..i].trim(), two, s[i + 2..].trim()));
            }
        }
        if b == b'<' || b == b'>' {
            return Some((s[..i].trim(), &s[i..i + 1], s[i + 1..].trim()));
        }
        i += 1;
    }
    None
}

fn resolve_value(s: &str, attrs: &Attributes) -> String {
    let s = s.trim();
    if s.starts_with('"') && s.ends_with('"') && s.len() >= 2 {
        return substitute_attr_refs(&s[1..s.len() - 1], attrs);
    }
    substitute_attr_refs(s, attrs)
}

fn substitute_attr_refs(s: &str, attrs: &Attributes) -> String {
    let mut out = String::new();
    let mut chars = s.chars();
    while let Some(ch) = chars.next() {
        if ch != '{' {
            out.push(ch);
            continue;
        }
        let mut name = String::new();
        let mut closed = false;
        for inner in chars.by_ref() {
            if inner == '}' {
                closed = true;
                break;
            }
            name.push(inner);
        }
        if !closed {
            out.push('{');
            out.push_str(&name);
            continue;
        }
        if !is_attr_name(&name) {
            out.push('{');
            out.push_str(&name);
            out.push('}');
            continue;
        }
        match attrs.get(&name) {
            Some(AttributeValue::String(v)) => out.push_str(v),
            Some(AttributeValue::Bool(true)) => {}
            Some(AttributeValue::Bool(false)) | None => {
                out.push('{');
                out.push_str(&name);
                out.push('}');
            }
        }
    }
    out
}

fn apply_num_op(l: f64, op: &str, r: f64) -> bool {
    match op {
        "==" => l == r,
        "!=" => l != r,
        "<" => l < r,
        "<=" => l <= r,
        ">" => l > r,
        ">=" => l >= r,
        _ => false,
    }
}

fn apply_str_op(l: &str, op: &str, r: &str) -> bool {
    match op {
        "==" => l == r,
        "!=" => l != r,
        "<" => l < r,
        "<=" => l <= r,
        ">" => l > r,
        ">=" => l >= r,
        _ => false,
    }
}

// --- file I/O -------------------------------------------------------------

fn read_file(path: &Utf8Path) -> Result<String, PreprocessError> {
    fs::read_to_string(path.as_std_path()).map_err(|source| PreprocessError::Io {
        path: path.to_string(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attrs_with(pairs: &[(&str, AttributeValue)]) -> Attributes {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), v.clone()))
            .collect()
    }

    #[test]
    fn directive_parsing_recognises_each_form() {
        assert!(matches!(
            parse_directive("include::file.adoc[]"),
            Some(Directive::Include { .. })
        ));
        assert!(matches!(
            parse_directive("ifdef::flag[]"),
            Some(Directive::Ifdef { content: None, .. })
        ));
        assert!(matches!(
            parse_directive("ifdef::flag[hello]"),
            Some(Directive::Ifdef {
                content: Some(_),
                ..
            })
        ));
        assert!(matches!(
            parse_directive("ifndef::flag[]"),
            Some(Directive::Ifndef { .. })
        ));
        assert!(matches!(
            parse_directive("ifeval::[1 == 1]"),
            Some(Directive::Ifeval { .. })
        ));
        assert!(matches!(
            parse_directive("endif::[]"),
            Some(Directive::Endif)
        ));
        assert!(parse_directive("paragraph text").is_none());
    }

    #[test]
    fn ifdef_anyof_and_allof() {
        let only_a = attrs_with(&[("a", AttributeValue::Bool(true))]);
        let a_and_b = attrs_with(&[
            ("a", AttributeValue::Bool(true)),
            ("b", AttributeValue::Bool(true)),
        ]);
        let any = parse_attr_test("a,b").unwrap();
        let all = parse_attr_test("a+b").unwrap();
        assert!(eval_attr_test(&any, &only_a));
        assert!(!eval_attr_test(&all, &only_a));
        assert!(eval_attr_test(&all, &a_and_b));
    }

    #[test]
    fn ifeval_numeric_and_string() {
        let attrs = attrs_with(&[
            ("count", AttributeValue::String("5".into())),
            ("lang", AttributeValue::String("en".into())),
        ]);
        assert!(eval_ifeval("{count} >= 5", &attrs));
        assert!(!eval_ifeval("{count} > 5", &attrs));
        assert!(eval_ifeval(r#""{lang}" == "en""#, &attrs));
        assert!(!eval_ifeval(r#""{lang}" == "fr""#, &attrs));
        assert!(eval_ifeval(r#""{lang}" != "fr""#, &attrs));
    }

    #[test]
    fn block_conditional_skips_inactive_content() {
        let mut p = Preprocessor::default();
        let src = "before\nifdef::flag[]\nhidden\nendif::[]\nafter\n";
        let lines = p.run(src, Utf8Path::new("<input>")).unwrap();
        let texts: Vec<_> = lines.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(texts, vec!["before", "after", ""]);
    }

    #[test]
    fn block_conditional_emits_when_attribute_set() {
        let mut p =
            Preprocessor::with_attributes(attrs_with(&[("flag", AttributeValue::Bool(true))]));
        let src = "before\nifdef::flag[]\nshown\nendif::[]\nafter\n";
        let lines = p.run(src, Utf8Path::new("<input>")).unwrap();
        let texts: Vec<_> = lines.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(texts, vec!["before", "shown", "after", ""]);
    }

    #[test]
    fn inline_ifdef_emits_content_inline() {
        let mut p =
            Preprocessor::with_attributes(attrs_with(&[("flag", AttributeValue::Bool(true))]));
        let src = "ifdef::flag[on the fly]\n";
        let lines = p.run(src, Utf8Path::new("<input>")).unwrap();
        assert_eq!(lines[0].text, "on the fly");
    }

    #[test]
    fn nested_conditionals_propagate_inactive() {
        let mut p = Preprocessor::default();
        let src = "ifdef::outer[]\nifdef::inner[]\ninside\nendif::[]\nendif::[]\n";
        let lines = p.run(src, Utf8Path::new("<input>")).unwrap();
        let texts: Vec<_> = lines.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(texts, vec![""]);
    }

    #[test]
    fn doc_attribute_entry_then_conditional() {
        let mut p = Preprocessor::default();
        let src = ":lang: en\nifdef::lang[]\nhello\nendif::[]\n";
        let lines = p.run(src, Utf8Path::new("<input>")).unwrap();
        let texts: Vec<_> = lines.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(texts, vec![":lang: en", "hello", ""]);
        assert_eq!(
            p.attributes.get("lang"),
            Some(&AttributeValue::String("en".into()))
        );
    }

    #[test]
    fn endif_without_matching_if_errors() {
        let mut p = Preprocessor::default();
        let err = p.run("endif::[]\n", Utf8Path::new("<input>")).unwrap_err();
        let d = err
            .as_diagnostic()
            .expect("should be a span-carrying error");
        assert_eq!(d.code, "adoc::preprocess::stray_endif");
    }

    #[test]
    fn unclosed_if_at_eof_errors() {
        let mut p = Preprocessor::default();
        let err = p.run("ifdef::x[]\n", Utf8Path::new("<input>")).unwrap_err();
        assert!(matches!(err, PreprocessError::Message(m) if m.contains("unclosed")));
    }

    #[test]
    fn include_resolves_relative_to_top_path() {
        let dir = tempdir();
        let part = dir.join("part.adoc");
        std::fs::write(&part, "from include\n").unwrap();
        let top = dir.join("top.adoc");
        std::fs::write(&top, "before\ninclude::part.adoc[]\nafter\n").unwrap();

        let mut p = Preprocessor::default();
        let lines = p.run_file(Utf8Path::from_path(&top).unwrap()).unwrap();
        let texts: Vec<_> = lines.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(texts, vec!["before", "from include", "", "after", ""]);

        // Sources registry has both files.
        assert_eq!(p.sources().len(), 2);
        // The included content's lines are tagged with SourceId(1).
        assert_eq!(lines[1].location.source.0, 1);
    }

    #[test]
    fn include_cycle_is_rejected() {
        let dir = tempdir();
        let a = dir.join("a.adoc");
        let b = dir.join("b.adoc");
        std::fs::write(&a, "include::b.adoc[]\n").unwrap();
        std::fs::write(&b, "include::a.adoc[]\n").unwrap();
        let mut p = Preprocessor::default();
        let err = p.run_file(Utf8Path::from_path(&a).unwrap()).unwrap_err();
        let d = err
            .as_diagnostic()
            .expect("cycle should produce diagnostic");
        assert_eq!(d.code, "adoc::preprocess::include_cycle");
    }

    #[test]
    fn include_disabled_in_secure_mode() {
        let dir = tempdir();
        let part = dir.join("part.adoc");
        std::fs::write(&part, "x").unwrap();
        let top = dir.join("top.adoc");
        std::fs::write(&top, "include::part.adoc[]\n").unwrap();
        let mut p = Preprocessor::default().with_safe_mode(SafeMode::Secure);
        let err = p.run_file(Utf8Path::from_path(&top).unwrap()).unwrap_err();
        let d = err
            .as_diagnostic()
            .expect("secure-mode include should diag");
        assert_eq!(d.code, "adoc::preprocess::secure_mode");
    }

    #[test]
    fn include_lines_range_filters_content() {
        let dir = tempdir();
        let part = dir.join("part.adoc");
        std::fs::write(&part, "one\ntwo\nthree\nfour\nfive\n").unwrap();
        let top = dir.join("top.adoc");
        std::fs::write(&top, "include::part.adoc[lines=2..4]\n").unwrap();
        let mut p = Preprocessor::default();
        let lines = p.run_file(Utf8Path::from_path(&top).unwrap()).unwrap();
        let texts: Vec<_> = lines.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(texts, vec!["two", "three", "four", "", ""]);
    }

    #[test]
    fn include_lines_supports_open_ended_and_multiple_ranges() {
        let dir = tempdir();
        let part = dir.join("part.adoc");
        std::fs::write(&part, "1\n2\n3\n4\n5\n6\n").unwrap();
        let top = dir.join("top.adoc");
        std::fs::write(&top, "include::part.adoc[lines=1;3..-1]\n").unwrap();
        let mut p = Preprocessor::default();
        let lines = p.run_file(Utf8Path::from_path(&top).unwrap()).unwrap();
        let texts: Vec<_> = lines.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(texts, vec!["1", "3", "4", "5", "6", "", ""]);
    }

    #[test]
    fn include_tag_selects_marked_region() {
        let dir = tempdir();
        let part = dir.join("part.adoc");
        std::fs::write(
            &part,
            "outside\n// tag::keep[]\ninside\n// end::keep[]\nafter\n",
        )
        .unwrap();
        let top = dir.join("top.adoc");
        std::fs::write(&top, "include::part.adoc[tag=keep]\n").unwrap();
        let mut p = Preprocessor::default();
        let lines = p.run_file(Utf8Path::from_path(&top).unwrap()).unwrap();
        let texts: Vec<_> = lines.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(texts, vec!["inside", "", ""]);
    }

    #[test]
    fn include_tags_supports_multiple_and_negation() {
        let dir = tempdir();
        let part = dir.join("part.adoc");
        std::fs::write(
            &part,
            "// tag::a[]\nA\n// end::a[]\n// tag::b[]\nB\n// end::b[]\n// tag::c[]\nC\n// end::c[]\n",
        )
        .unwrap();
        let top = dir.join("top.adoc");
        std::fs::write(&top, "include::part.adoc[tags=a;b;!b]\n").unwrap();
        let mut p = Preprocessor::default();
        let lines = p.run_file(Utf8Path::from_path(&top).unwrap()).unwrap();
        let texts: Vec<_> = lines.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(texts, vec!["A", "", ""]);
    }

    #[test]
    fn include_leveloffset_shifts_section_levels() {
        let dir = tempdir();
        let part = dir.join("part.adoc");
        std::fs::write(&part, "= Top\n== Sub\n=== SubSub\n").unwrap();
        let top = dir.join("top.adoc");
        std::fs::write(&top, "include::part.adoc[leveloffset=+1]\n").unwrap();
        let mut p = Preprocessor::default();
        let lines = p.run_file(Utf8Path::from_path(&top).unwrap()).unwrap();
        let texts: Vec<_> = lines.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(texts, vec!["== Top", "=== Sub", "==== SubSub", "", ""]);
    }

    #[test]
    fn include_leveloffset_clamps_to_six() {
        let dir = tempdir();
        let part = dir.join("part.adoc");
        std::fs::write(&part, "===== Five\n").unwrap();
        let top = dir.join("top.adoc");
        std::fs::write(&top, "include::part.adoc[leveloffset=+5]\n").unwrap();
        let mut p = Preprocessor::default();
        let lines = p.run_file(Utf8Path::from_path(&top).unwrap()).unwrap();
        assert_eq!(lines[0].text, "====== Five");
    }

    #[test]
    fn safe_mode_rejects_absolute_include() {
        let dir = tempdir();
        let part = dir.join("part.adoc");
        std::fs::write(&part, "x\n").unwrap();
        let top = dir.join("top.adoc");
        // Use the absolute path of part.adoc as the include target.
        let abs = part.canonicalize().unwrap();
        let abs_str = Utf8Path::from_path(&abs).unwrap().to_string();
        std::fs::write(&top, format!("include::{abs_str}[]\n")).unwrap();
        let mut p = Preprocessor::default().with_safe_mode(SafeMode::Safe);
        let err = p.run_file(Utf8Path::from_path(&top).unwrap()).unwrap_err();
        let d = err
            .as_diagnostic()
            .expect("safe-mode rejection should diag");
        assert!(
            matches!(
                d.code,
                "adoc::preprocess::absolute_include" | "adoc::preprocess::include_path"
            ),
            "got code={}",
            d.code
        );
    }

    #[test]
    fn safe_mode_rejects_path_escaping_base_dir() {
        // Layout: <root>/inside/{top.adoc, ok.adoc} and <root>/outside/leak.adoc.
        // base_dir is <root>/inside. include::../outside/leak.adoc[] must error.
        let root = tempdir();
        let inside = root.join("inside");
        let outside = root.join("outside");
        std::fs::create_dir_all(&inside).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        let leak = outside.join("leak.adoc");
        std::fs::write(&leak, "leaked\n").unwrap();
        let top = inside.join("top.adoc");
        std::fs::write(&top, "include::../outside/leak.adoc[]\n").unwrap();

        let mut p = Preprocessor::default()
            .with_base_dir(Utf8Path::from_path(&inside).unwrap().to_owned())
            .with_safe_mode(SafeMode::Safe);
        let err = p.run_file(Utf8Path::from_path(&top).unwrap()).unwrap_err();
        let d = err.as_diagnostic().expect("escape rejection should diag");
        assert_eq!(d.code, "adoc::preprocess::include_path");
        assert!(d.message.contains("escapes"), "got message={}", d.message);
    }

    #[test]
    fn safe_mode_allows_include_within_base_dir() {
        let root = tempdir();
        let inside = root.join("inside");
        std::fs::create_dir_all(&inside).unwrap();
        let part = inside.join("part.adoc");
        std::fs::write(&part, "ok\n").unwrap();
        let top = inside.join("top.adoc");
        std::fs::write(&top, "include::part.adoc[]\n").unwrap();

        let mut p = Preprocessor::default()
            .with_base_dir(Utf8Path::from_path(&inside).unwrap().to_owned())
            .with_safe_mode(SafeMode::Safe);
        let lines = p
            .run_file(Utf8Path::from_path(&top).unwrap())
            .expect("safe-mode should allow in-tree include");
        let texts: Vec<_> = lines.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(texts, vec!["ok", "", ""]);
    }

    #[test]
    fn line_range_resolves_negative_indices() {
        assert_eq!(parse_one_line_range("1..-1").unwrap().end, -1);
        let r = parse_one_line_range("3..5").unwrap();
        assert_eq!((r.start, r.end), (3, 5));
        let single = parse_one_line_range("7").unwrap();
        assert_eq!((single.start, single.end), (7, 7));
    }

    #[test]
    fn tag_marker_recognises_common_comment_leaders() {
        assert_eq!(parse_tag_marker("// tag::foo[]", "tag::"), Some("foo"));
        assert_eq!(parse_tag_marker("# tag::foo[]", "tag::"), Some("foo"));
        assert_eq!(
            parse_tag_marker("<!-- tag::foo[] -->", "tag::"),
            Some("foo")
        );
        // Adjacent alphanumeric prefix → not a marker.
        assert!(parse_tag_marker("subtag::foo[]", "tag::").is_none());
    }

    /// Tiny ad-hoc tempdir helper to avoid pulling in `tempfile` for tests.
    fn tempdir() -> std::path::PathBuf {
        let mut d = std::env::temp_dir();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        d.push(format!("adoc-preproc-{nanos}-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }
}
