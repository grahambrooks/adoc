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

### HTML5 converter layout

`adoc::convert::html5` is split into focused submodules. Reach for the right one when adding rendering features:

| Submodule | Role |
| --- | --- |
| `mod.rs` | `Html5Converter`, `Html5Options`, `Stylesheet`, the `Converter` impl, `render_stylesheet`. Top of the body wiring (header, preamble, `<main id="content">`, footnote section). |
| `blocks.rs` | `render_block` dispatcher and per-variant block renderers (sections, paragraphs, lists, delimited, admonitions, callouts, discrete headings, block image / video / audio). Shared block-meta helpers (`meta_attrs`, `meta_id_only`, `merge_class_attr`, `render_block_title`). |
| `tables.rs` | `render_table`, `render_colgroup`, `render_table_cell`. |
| `inlines.rs` | `render_inlines` / `render_inline` for every `Inline` variant. |
| `ctx.rs` | `RenderCtx`, the section pre-walk, `render_toc`. |
| `highlighter.rs` | `:source-highlighter:` integration for Prism / highlight.js, including the surface-override CSS. |
| `footnotes.rs` | Post-render rewrite of inline footnote spans into numbered refs + the end-of-doc footnote section. |
| `escape.rs` | `escape` / `escape_attr` — used everywhere; one canonical implementation. |

The `crate::ast::inlines_to_plain` helper is the single canonical AST → plain-text converter (used by id-generation, the converter, the `<title>` element). Don't add a third copy.

## Conformance

Two test corpora work together:

* **`tests/fixtures/`** asserts *structural* properties — counts, presence of particular tags, AST node shape — for the 41+ `.adoc` inputs the project drives through the full pipeline. New language features earn their `#[test]` here.
* **`tests/conformance/<entry>/`** asserts *byte-identity*. Each entry has `input.adoc`, `expected.ast.json`, and `expected.html` (rendered with no stylesheet so CSS edits don't dominate diffs). The driver lives in `tests/conformance.rs`; bless intentional changes with `ADOC_CONFORMANCE_BLESS=1 cargo test --test conformance`. New spec features should add a conformance entry in the same change as the feature.

Asciidoctor is a sanity check, not the oracle. The spec is the oracle.
