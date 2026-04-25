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

use crate::ast::{AttributeValue, Attributes, Location, SourceId};

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
}

/// Mirrors the CLI safe-mode flag enough to gate filesystem-touching
/// constructs. Only `Secure` (which disables `include::`) is enforced
/// today; `Safe` and `Server` are accepted but otherwise treated as
/// permissive — full enforcement is queued behind diagnostics work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SafeMode {
    Unsafe,
    #[default]
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
        let top_id = self.register_source(top_path.to_owned());
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

    fn register_source(&mut self, path: Utf8PathBuf) -> SourceId {
        let id = SourceId(self.sources.len() as u32);
        self.sources.push(path);
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
                    return Err(PreprocessError::Message(format!(
                        "endif:: at line {} without a matching ifdef/ifndef/ifeval",
                        location.line
                    )));
                }
                state.cond_stack.pop();
                Ok(())
            }
            Directive::Include { target } => {
                if !state.emitting() {
                    return Ok(());
                }
                self.handle_include(&target, source_path, state, output)
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
        current_path: &Utf8Path,
        state: &mut ProcessState,
        output: &mut Vec<PreprocessedLine>,
    ) -> Result<(), PreprocessError> {
        if matches!(self.safe_mode, SafeMode::Secure) {
            return Err(PreprocessError::Message(
                "include:: is disabled in safe mode 'secure'".into(),
            ));
        }

        let resolved = self.resolve_include_path(target.trim(), current_path);

        if state.include_chain.iter().any(|p| p == &resolved) {
            return Err(PreprocessError::Message(format!(
                "include cycle: {resolved}"
            )));
        }
        if state.include_chain.len() as u32 >= self.max_include_depth {
            return Err(PreprocessError::Message(format!(
                "include depth limit reached ({})",
                self.max_include_depth
            )));
        }

        let source = read_file(&resolved)?;
        let source_id = self.register_source(resolved.clone());
        state.include_chain.push(resolved.clone());
        let owned = resolved;
        self.process_source(&source, source_id, &owned, state, output)?;
        state.include_chain.pop();
        Ok(())
    }

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
        assert!(matches!(err, PreprocessError::Message(_)));
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
        assert!(matches!(err, PreprocessError::Message(m) if m.contains("cycle")));
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
        assert!(matches!(err, PreprocessError::Message(m) if m.contains("secure")));
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
