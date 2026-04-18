//! `adoc` — AsciiDoc command-line processor.

use adoc_convert_html5::Html5Converter;
use adoc_core::{Attributes, Converter};
use adoc_parser::parse;
use adoc_preprocessor::Preprocessor;
use camino::Utf8PathBuf;
use clap::{Parser, ValueEnum};

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

#[derive(Debug, Parser)]
#[command(name = "adoc", version, about = "Rust AsciiDoc processor")]
struct Cli {
    /// Input AsciiDoc files.
    #[arg(required_unless_present = "from_ast")]
    inputs: Vec<Utf8PathBuf>,

    /// Output file (default: stem + .html).
    #[arg(short = 'o', long = "out")]
    out: Option<Utf8PathBuf>,

    /// Output backend.
    #[arg(short = 'b', long = "backend", default_value = "html5")]
    backend: Backend,

    /// Set document attribute (repeatable). `NAME` or `NAME=VALUE`.
    #[arg(short = 'a', long = "attribute")]
    attributes: Vec<String>,

    /// Output directory.
    #[arg(short = 'D', long = "destination-dir")]
    destination_dir: Option<Utf8PathBuf>,

    /// Safe mode.
    #[arg(long = "safe-mode", default_value = "safe")]
    safe_mode: SafeMode,

    /// Base directory for includes.
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
}

fn main() -> miette::Result<()> {
    let cli = Cli::parse();
    init_tracing(cli.verbose, cli.quiet);

    let source = std::fs::read_to_string(
        cli.inputs
            .first()
            .expect("clap enforces presence unless --from-ast"),
    )
    .map_err(|e| miette::miette!("failed to read input: {e}"))?;

    let preproc = Preprocessor::new(Attributes::new());
    let lines = preproc
        .run(&source)
        .map_err(|e| miette::miette!("{e}"))?;
    let doc = parse(&lines).map_err(|e| miette::miette!("{e}"))?;

    let output = match cli.backend {
        Backend::Html5 => Html5Converter
            .convert(&doc)
            .map_err(|e| miette::miette!("{e}"))?,
    };

    print!("{output}");
    Ok(())
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
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(level));
    fmt().with_env_filter(filter).with_writer(std::io::stderr).init();
}
