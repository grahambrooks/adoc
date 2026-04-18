# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

`adoc` is a Rust command-line tool implementing the [AsciiDoc Language specification](https://docs.asciidoctor.org/asciidoc/latest/). The spec is the authority, not Asciidoctor's behavior — where the two diverge, follow the spec and document the divergence.

Full design rationale, phasing, and conformance strategy live in `DESIGN.md`. Read it before making architectural changes.

## Common commands

```bash
cargo build                         # build the workspace
cargo build --release               # release binary at target/release/adoc
cargo run -p adoc-cli -- <file>     # run the CLI against an .adoc file
cargo test                          # run all tests
cargo test -p adoc-parser           # tests for one crate
cargo test -p adoc-parser <name>    # a single test by (sub)name
cargo clippy --all-targets -- -D warnings
cargo fmt
```

The binary is `adoc` (configured via `[[bin]]` in `crates/adoc-cli/Cargo.toml`), not `adoc-cli`.

## Architecture

Cargo workspace. Pipeline: **Loader → Preprocessor → Parser → Document (AST) → Converter → Writer**.

| Crate | Role |
|---|---|
| `adoc-core` | AST types (`Document`, `Block`, `Inline`), `Location`, `Converter` trait, `Attributes`. No I/O. All types `serde`-serializable. |
| `adoc-preprocessor` | Line-level: `include::`, `ifdef`/`ifndef`/`ifeval`/`endif`, attribute entries. Produces `PreprocessedLine` with source spans. |
| `adoc-parser` | Hand-written recursive-descent block parser + inline parser with the spec's six-group substitution pipeline. Consumes preprocessed lines, produces `Document`. |
| `adoc-convert-html5` | Implements `adoc_core::Converter` for HTML5. |
| `adoc-cli` | Binary `adoc`. `clap` parsing, wires the pipeline. |

Dependency direction: `adoc-cli → {adoc-parser, adoc-preprocessor, adoc-convert-html5} → adoc-core`. Never introduce cycles; converters must not depend on the parser.

### Load-bearing design constraints

- **AST is the interchange format.** Extensions will be stdio-based external processes that read/write the AST as JSON (`adoc --emit-ast` / `--from-ast`). Keep every `adoc-core` type `Serialize + Deserialize`, and treat the serialized form as a public interface — breaking changes need intent.
- **Every AST node carries a `Location`** (source id + byte range + line/column). Diagnostics via `miette` depend on this; don't drop spans when transforming the tree.
- **Substitutions are a pipeline, not ad-hoc.** The six spec-defined groups (specialchars, quotes, attributes, replacements, macros, post-replacements) are applied in order; each block type declares its applicable groups. Implement new inline features inside this pipeline, not as one-off passes.
- **Parser is hand-written recursive descent.** AsciiDoc's context-sensitive, line-oriented grammar made parser combinators a non-starter. Don't reach for `nom`/`chumsky`.
- **HTML5 is currently the only backend, but the `Converter` trait exists to keep that reversible.** Don't hard-code HTML assumptions into `adoc-core` or the parser.

### What's not here yet

The preprocessor and parser are scaffolded stubs returning empty results; the HTML5 converter emits a fixed shell. Phase order per `DESIGN.md`: block parser → inline parser → preprocessor → HTML5 conformance → stdio extension model → additional backends.

## Conformance

Compliance is measured against a conformance suite under `tests/conformance/` (not yet populated) — one `.adoc` input with expected AST (JSON) and expected HTML5 per feature. When adding a language feature, add a conformance fixture in the same change. Asciidoctor is a sanity check, not the oracle.
