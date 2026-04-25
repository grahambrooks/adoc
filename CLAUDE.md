# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

`adoc` is a Rust command-line tool implementing the [AsciiDoc Language specification](https://docs.asciidoctor.org/asciidoc/latest/). The spec is the authority, not Asciidoctor's behavior — where the two diverge, follow the spec and document the divergence.

Full design rationale, phasing, and conformance strategy live in `DESIGN.md`. Read it before making architectural changes.

## Common commands

```bash
cargo build                         # build the project
cargo build --release               # release binary at target/release/adoc
cargo run -- <file>                 # run the CLI against an .adoc file
cargo test                          # run all tests
cargo test <name>                   # a single test by (sub)name
cargo test --test fixtures          # one integration test target
cargo clippy --all-targets -- -D warnings
cargo fmt
```

`make help` lists higher-level targets (`make ci` runs fmt-check + lint + test).

The binary is `adoc`, defined by `src/main.rs` alongside the library at `src/lib.rs`.

## Architecture

Single Cargo crate. Pipeline: **Loader → Preprocessor → Parser → Document (AST) → Converter → Writer**.

The pipeline is realised as four sibling modules under `src/`:

| Module | Role |
| --- | --- |
| `adoc::ast` | AST types (`Document`, `Block`, `Inline`), `Location`, `Converter` trait, `Attributes`. No I/O. All types `serde`-serializable. |
| `adoc::preprocessor` | Line-level: `include::`, `ifdef`/`ifndef`/`ifeval`/`endif`, attribute entries. Produces `PreprocessedLine` with source spans. |
| `adoc::parser` | Hand-written recursive-descent block parser + inline parser with the spec's six-group substitution pipeline. Consumes preprocessed lines, produces `Document`. |
| `adoc::convert::html5` | Implements `adoc::ast::Converter` for HTML5. Owns the built-in stylesheet asset (`assets/adoc.css`). |

The `src/main.rs` binary depends on the library through `adoc::*` paths and wires the pipeline.

Dependency direction inside the crate: `main → {parser, preprocessor, convert::html5} → ast`. Never introduce cycles; converters must not depend on the parser. (Future backends — DocBook, manpage — go under `src/convert/`.)

### Load-bearing design constraints

- **AST is the interchange format.** Extensions will be stdio-based external processes that read/write the AST as JSON (`adoc --emit-ast` / `--from-ast`). Keep every `adoc::ast` type `Serialize + Deserialize`, and treat the serialized form as a public interface — breaking changes need intent.
- **Every AST node carries a `Location`** (source id + byte range + line/column). Diagnostics via `miette` depend on this; don't drop spans when transforming the tree.
- **Substitutions are a pipeline, not ad-hoc.** The six spec-defined groups (specialchars, quotes, attributes, replacements, macros, post-replacements) are applied in order; each block type declares its applicable groups. Implement new inline features inside this pipeline, not as one-off passes.
- **Parser is hand-written recursive descent.** AsciiDoc's context-sensitive, line-oriented grammar made parser combinators a non-starter. Don't reach for `nom`/`chumsky`.
- **HTML5 is currently the only backend, but the `Converter` trait exists to keep that reversible.** Don't hard-code HTML assumptions into `adoc::ast` or the parser.

### What's not here yet

`adoc::preprocessor` is currently a faithful line-splitter — directive handling (`include::`, `ifdef`, `ifeval`) is not yet implemented. Block metadata lines (`[source,rust]`, `.Title`), section IDs, and admonitions are also pending. See `DESIGN.md` for the full status matrix and phasing.

## Conformance

Compliance is measured against a conformance suite under `tests/conformance/` (not yet populated) — one `.adoc` input with expected AST (JSON) and expected HTML5 per feature. When adding a language feature, add a conformance fixture in the same change. The interim corpus is `tests/fixtures/` (twenty `.adoc` inputs driving structural assertions through the full pipeline). Asciidoctor is a sanity check, not the oracle.
