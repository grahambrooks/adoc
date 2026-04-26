//! `adoc` — AsciiDoc command-line processor.

use std::fs;
use std::io::Write;

use std::io::Read;

use adoc::ast::{AttributeValue, Attributes, Document, SourceMap};
use adoc::convert::html5::{
    Html5Converter, Html5Options, Stylesheet, BUILTIN_CSS, BUILTIN_FILENAME,
};
use adoc::parser::parse_with;
use adoc::preprocessor::{Preprocessor, SafeMode as PreprocSafeMode};
use camino::{Utf8Path, Utf8PathBuf};
use clap::{Parser, ValueEnum};
use miette::{miette, IntoDiagnostic};

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Backend {
    Html5,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum SafeMode {
    Unsafe,
    Safe,
    Server,
    Secure,
}

/// How to render diagnostics on stderr.
///
/// `plain` (default) — graphical / narratable text, with colour when
/// stderr is a TTY. `json` — one JSON object per diagnostic, suitable
/// for SARIF converters / GitHub annotations / IDE consumption.
#[derive(Debug, Clone, Copy, ValueEnum)]
enum DiagnosticFormat {
    Plain,
    Json,
}

#[derive(Debug, Parser)]
#[command(name = "adoc", version, about = "Rust AsciiDoc processor")]
struct Cli {
    /// Input AsciiDoc files.
    #[arg(required_unless_present = "from_ast")]
    inputs: Vec<Utf8PathBuf>,

    /// Output file (default: stem + .html, or stdout if neither -o nor -D).
    #[arg(short = 'o', long = "out")]
    out: Option<Utf8PathBuf>,

    /// Output backend.
    #[arg(short = 'b', long = "backend", default_value = "html5")]
    backend: Backend,

    /// Set document attribute (repeatable). `NAME`, `NAME=VALUE`, `!NAME`, or `NAME!`.
    #[arg(short = 'a', long = "attribute")]
    attributes: Vec<String>,

    /// Output directory. When set without -o, output is <stem>.html inside this dir.
    #[arg(short = 'D', long = "destination-dir")]
    destination_dir: Option<Utf8PathBuf>,

    /// Safe mode.
    #[arg(long = "safe-mode", default_value = "safe")]
    safe_mode: SafeMode,

    /// Base directory for includes and stylesheet resolution (default: input's dir).
    #[arg(long = "base-dir")]
    base_dir: Option<Utf8PathBuf>,

    /// Emit the serialized AST (JSON) to stdout instead of rendering.
    #[arg(long = "emit-ast")]
    emit_ast: bool,

    /// Read a serialized AST from stdin instead of parsing an input file.
    #[arg(long = "from-ast")]
    from_ast: bool,

    /// Increase log verbosity.
    #[arg(short = 'v', long = "verbose", action = clap::ArgAction::Count)]
    verbose: u8,

    /// Suppress warnings.
    #[arg(short = 'q', long = "quiet")]
    quiet: bool,

    /// Diagnostic output format. `plain` (default) renders the
    /// graphical text form; `json` emits one JSON object per
    /// diagnostic (one per line) for tooling consumption.
    #[arg(long = "diagnostic-format", default_value = "plain")]
    diagnostic_format: DiagnosticFormat,
}

fn main() -> miette::Result<()> {
    let cli = Cli::parse();
    init_tracing(cli.verbose, cli.quiet);

    let cli_attrs = parse_cli_attributes(&cli.attributes)?;
    let input_ref: Option<&Utf8Path> = cli.inputs.first().map(Utf8PathBuf::as_path);

    // Build the AST either by parsing source or by deserializing JSON.
    // Also capture the SourceMap for diagnostics — the preprocessor
    // populates it as it walks includes; the AST-from-stdin path has
    // no source map.
    let (doc, source_map): (Document, SourceMap) = if cli.from_ast {
        (load_ast_input(input_ref)?, SourceMap::new())
    } else {
        let input = input_ref.ok_or_else(|| miette!("input required"))?;
        let source = fs::read_to_string(input.as_std_path())
            .map_err(|e| miette!("failed to read {input}: {e}"))?;
        let base_dir = preprocessor_base_dir(&cli, Some(input));
        let mut preproc = Preprocessor::with_attributes(cli_attrs.clone())
            .with_base_dir(base_dir)
            .with_safe_mode(map_safe_mode(cli.safe_mode));
        let lines = preproc.run(&source, input).map_err(|e| {
            // Promote a span-carrying preprocess error to a real
            // miette report so the user gets file:line:col + snippet.
            let map = preproc.source_map();
            preprocess_error_to_report(e, &map)
        })?;
        let doc = parse_with(&lines, cli_attrs.clone()).map_err(|e| miette!("{e}"))?;
        (doc, preproc.source_map())
    };

    let out_path = resolve_output_path(&cli, input_ref);

    // `--emit-ast` short-circuits the converter and writes JSON.
    if cli.emit_ast {
        let json =
            serde_json::to_string_pretty(&doc).map_err(|e| miette!("AST serialize failed: {e}"))?;
        return write_output(
            out_path.as_deref(),
            json.as_bytes(),
            /*trailing_newline=*/ true,
        );
    }

    let base_dir = preprocessor_base_dir(&cli, input_ref);
    let stylesheet = resolve_stylesheet(&doc.attributes, &base_dir).map_err(|e| miette!("{e}"))?;

    let (output_html, diagnostics) = match cli.backend {
        Backend::Html5 => {
            let converter = Html5Converter::with_options(Html5Options {
                stylesheet: stylesheet.clone(),
            });
            converter
                .convert_with_diagnostics(&doc)
                .map_err(|e| miette!("{e}"))?
        }
    };

    write_output(out_path.as_deref(), output_html.as_bytes(), false)?;

    if is_truthy(doc.attributes.get("copycss")) {
        copy_stylesheet(&stylesheet, &doc.attributes, &base_dir, out_path.as_deref())?;
    }

    // Render any diagnostics collected during conversion (dangling
    // xrefs etc.) to stderr through miette's default handler so users
    // get colour, source snippets, and span underlines. `--quiet`
    // suppresses them.
    if !cli.quiet {
        render_diagnostics(diagnostics, &source_map, cli.diagnostic_format);
    }

    Ok(())
}

/// Promote a [`adoc::preprocessor::PreprocessError`] into a `miette`
/// report. Span-carrying variants become rich source-pointing reports;
/// the message-only fallback uses the existing flat-string form.
fn preprocess_error_to_report(
    err: adoc::preprocessor::PreprocessError,
    source_map: &SourceMap,
) -> miette::Report {
    if let Some(diag) = err.as_diagnostic() {
        return diag.clone().into_report(source_map);
    }
    miette!("{err}")
}

fn render_diagnostics(
    diagnostics: adoc::diag::Diagnostics,
    source_map: &SourceMap,
    format: DiagnosticFormat,
) {
    use std::io::IsTerminal;
    let mut stderr = std::io::stderr().lock();
    let mut buf = String::new();
    match format {
        DiagnosticFormat::Plain => {
            let to_terminal = std::io::stderr().is_terminal();
            let handler = if to_terminal {
                miette::GraphicalReportHandler::new()
            } else {
                miette::GraphicalReportHandler::new_themed(miette::GraphicalTheme::unicode_nocolor())
            };
            for diag in diagnostics {
                let report = diag.into_report(source_map);
                buf.clear();
                let _ = handler.render_report(&mut buf, report.as_ref());
                let _ = std::io::Write::write_all(&mut stderr, buf.as_bytes());
            }
        }
        DiagnosticFormat::Json => {
            // miette's JSONReportHandler emits one report per call. We
            // emit them as NDJSON (one object per line) so a stream
            // consumer can split on \n without parsing.
            let handler = miette::JSONReportHandler::new();
            for diag in diagnostics {
                let report = diag.into_report(source_map);
                buf.clear();
                let _ = handler.render_report(&mut buf, report.as_ref());
                let _ = std::io::Write::write_all(&mut stderr, buf.as_bytes());
                let _ = std::io::Write::write_all(&mut stderr, b"\n");
            }
        }
    }
}

/// Read a serialized [`Document`] from `path` if given, otherwise from stdin.
fn load_ast_input(path: Option<&Utf8Path>) -> miette::Result<Document> {
    let json = match path {
        Some(p) => fs::read_to_string(p.as_std_path())
            .map_err(|e| miette!("failed to read AST {p}: {e}"))?,
        None => {
            let mut buf = String::new();
            std::io::stdin()
                .read_to_string(&mut buf)
                .map_err(|e| miette!("failed to read AST from stdin: {e}"))?;
            buf
        }
    };
    serde_json::from_str(&json).map_err(|e| miette!("AST deserialize failed: {e}"))
}

fn preprocessor_base_dir(cli: &Cli, input: Option<&Utf8Path>) -> Utf8PathBuf {
    cli.base_dir.clone().unwrap_or_else(|| {
        input
            .and_then(Utf8Path::parent)
            .map(Utf8PathBuf::from)
            .unwrap_or_else(|| Utf8PathBuf::from("."))
    })
}

fn write_output(
    out_path: Option<&Utf8Path>,
    bytes: &[u8],
    add_trailing_newline: bool,
) -> miette::Result<()> {
    if let Some(path) = out_path {
        if let Some(parent) = path.parent() {
            if !parent.as_str().is_empty() {
                fs::create_dir_all(parent.as_std_path()).into_diagnostic()?;
            }
        }
        fs::write(path.as_std_path(), bytes).into_diagnostic()?;
        if add_trailing_newline {
            // The fs::write above replaces; append a newline only when writing
            // raw text (e.g. AST JSON) to keep line-oriented tools happy.
            if !bytes.ends_with(b"\n") {
                fs::OpenOptions::new()
                    .append(true)
                    .open(path.as_std_path())
                    .and_then(|mut f| std::io::Write::write_all(&mut f, b"\n"))
                    .into_diagnostic()?;
            }
        }
    } else {
        let mut stdout = std::io::stdout().lock();
        stdout.write_all(bytes).into_diagnostic()?;
        if add_trailing_newline && !bytes.ends_with(b"\n") {
            stdout.write_all(b"\n").into_diagnostic()?;
        }
    }
    Ok(())
}

fn map_safe_mode(mode: SafeMode) -> PreprocSafeMode {
    match mode {
        SafeMode::Unsafe => PreprocSafeMode::Unsafe,
        SafeMode::Safe => PreprocSafeMode::Safe,
        SafeMode::Server => PreprocSafeMode::Server,
        SafeMode::Secure => PreprocSafeMode::Secure,
    }
}

// --- attribute parsing -----------------------------------------------------

fn parse_cli_attributes(raw: &[String]) -> miette::Result<Attributes> {
    let mut attrs = Attributes::new();
    for entry in raw {
        let (name, value) = parse_one_cli_attribute(entry)
            .ok_or_else(|| miette!("invalid -a attribute: {entry}"))?;
        attrs.insert(name, value);
    }
    Ok(attrs)
}

fn parse_one_cli_attribute(raw: &str) -> Option<(String, AttributeValue)> {
    // Strip leading `!` for negation.
    let (name_part, value_part, negate) = if let Some(rest) = raw.strip_prefix('!') {
        (rest, None, true)
    } else if let Some((n, v)) = raw.split_once('=') {
        if let Some(n) = n.strip_suffix('!') {
            (n, Some(v), true)
        } else {
            (n, Some(v), false)
        }
    } else if let Some(n) = raw.strip_suffix('!') {
        (n, None, true)
    } else {
        (raw, None, false)
    };
    if name_part.is_empty() {
        return None;
    }
    let value = if negate {
        AttributeValue::Bool(false)
    } else {
        match value_part {
            None => AttributeValue::Bool(true),
            Some(s) => AttributeValue::String(s.to_string()),
        }
    };
    Some((name_part.to_string(), value))
}

// --- stylesheet resolution -------------------------------------------------

fn resolve_stylesheet(attrs: &Attributes, base_dir: &Utf8Path) -> std::io::Result<Stylesheet> {
    // Treat `:stylesheet!:` or `:stylesheet:` empty as disabled.
    match attrs.get("stylesheet") {
        Some(AttributeValue::Bool(false)) => return Ok(Stylesheet::None),
        Some(AttributeValue::String(s)) if s.is_empty() => return Ok(Stylesheet::None),
        _ => {}
    }
    let linkcss = is_truthy(attrs.get("linkcss"));
    let explicit_name = attrs
        .get("stylesheet")
        .and_then(AttributeValue::as_str)
        .filter(|s| !s.is_empty());
    let stylesdir = attrs.get("stylesdir").and_then(AttributeValue::as_str);

    match (explicit_name, linkcss) {
        (None, false) => Ok(Stylesheet::BuiltinEmbed),
        (None, true) => {
            let href = match stylesdir {
                Some(dir) => format!("{}/{}", dir.trim_end_matches('/'), BUILTIN_FILENAME),
                None => BUILTIN_FILENAME.to_string(),
            };
            Ok(Stylesheet::BuiltinLink { href })
        }
        (Some(name), false) => {
            let path = resolve_style_path(base_dir, stylesdir, name);
            let css = fs::read_to_string(path.as_std_path())?;
            Ok(Stylesheet::CustomEmbed { css })
        }
        (Some(name), true) => {
            let href = match stylesdir {
                Some(dir) => format!("{}/{}", dir.trim_end_matches('/'), name),
                None => name.to_string(),
            };
            Ok(Stylesheet::CustomLink { href })
        }
    }
}

fn resolve_style_path(base_dir: &Utf8Path, stylesdir: Option<&str>, name: &str) -> Utf8PathBuf {
    let rel = match stylesdir {
        Some(dir) => format!("{}/{}", dir.trim_end_matches('/'), name),
        None => name.to_string(),
    };
    let rel_path = Utf8PathBuf::from(rel);
    if rel_path.is_absolute() {
        rel_path
    } else {
        base_dir.join(rel_path)
    }
}

// --- copycss ---------------------------------------------------------------

fn copy_stylesheet(
    stylesheet: &Stylesheet,
    attrs: &Attributes,
    base_dir: &Utf8Path,
    out_path: Option<&Utf8Path>,
) -> miette::Result<()> {
    let (contents, filename) = match stylesheet {
        Stylesheet::None => return Ok(()),
        Stylesheet::BuiltinEmbed | Stylesheet::BuiltinLink { .. } => {
            (BUILTIN_CSS.to_string(), BUILTIN_FILENAME.to_string())
        }
        Stylesheet::CustomEmbed { css } => {
            let name = attrs
                .get("stylesheet")
                .and_then(AttributeValue::as_str)
                .unwrap_or(BUILTIN_FILENAME)
                .to_string();
            (css.clone(), leaf(&name))
        }
        Stylesheet::CustomLink { href } => {
            let name = leaf(href);
            let path = resolve_style_path(base_dir, None, href);
            let css = fs::read_to_string(path.as_std_path())
                .map_err(|e| miette!("reading custom stylesheet {path}: {e}"))?;
            (css, name)
        }
    };

    let target_dir: Utf8PathBuf = out_path
        .and_then(Utf8Path::parent)
        .map(Utf8PathBuf::from)
        .unwrap_or_else(|| Utf8PathBuf::from("."));
    fs::create_dir_all(target_dir.as_std_path()).into_diagnostic()?;
    let target = target_dir.join(&filename);
    fs::write(target.as_std_path(), contents).into_diagnostic()?;
    Ok(())
}

fn leaf(path_like: &str) -> String {
    Utf8Path::new(path_like)
        .file_name()
        .unwrap_or(path_like)
        .to_string()
}

// --- output path -----------------------------------------------------------

fn resolve_output_path(cli: &Cli, input: Option<&Utf8Path>) -> Option<Utf8PathBuf> {
    match (&cli.out, &cli.destination_dir) {
        (Some(out), Some(dir)) if out.is_relative() => Some(dir.join(out)),
        (Some(out), _) => Some(out.clone()),
        (None, Some(dir)) => {
            let stem = input.and_then(Utf8Path::file_stem).unwrap_or("out");
            Some(dir.join(format!("{stem}.html")))
        }
        (None, None) => None,
    }
}

// --- shared helpers --------------------------------------------------------

fn is_truthy(v: Option<&AttributeValue>) -> bool {
    matches!(v, Some(AttributeValue::Bool(true)))
        || matches!(v, Some(AttributeValue::String(s)) if !s.is_empty() && !s.eq_ignore_ascii_case("false"))
}

fn init_tracing(verbose: u8, quiet: bool) {
    use tracing_subscriber::{fmt, EnvFilter};
    let level = if quiet {
        "error"
    } else {
        match verbose {
            0 => "warn",
            1 => "info",
            2 => "debug",
            _ => "trace",
        }
    };
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(level));
    fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();
}
