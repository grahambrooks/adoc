# adoc — Design

A Rust command-line tool implementing the [AsciiDoc Language specification](https://docs.asciidoctor.org/asciidoc/latest/).

## Goals and non-goals

**Goals**
- Conform to the AsciiDoc Language specification as the authority — not Asciidoctor-specific behavior.
- Ship a single static binary: `adoc`.
- Emit HTML5 as the first and reference output backend.
- Produce diagnostics with precise source locations.
- Keep the AST serializable so external tools can consume and transform it.

**Non-goals (for now)**
- Asciidoctor-specific extensions, Ruby ERB templates, or bug-for-bug parity with Asciidoctor where it diverges from the spec.
- In-process extension API. Extensibility will be delivered later as **stdio-based filters** (Unix pipeline model: serialize AST to stdout, let an external process transform it, read it back).
- Backends beyond HTML5 in the first milestone. The converter layer is trait-based so DocBook, man page, etc. can be added without reworking the core.
- PDF output. That belongs downstream of DocBook or a dedicated renderer.

## Architecture

```
adoc (CLI)
  └─ Loader → Preprocessor → Parser → Document (AST) → Converter → Writer
                  │             │           │               │
                  │             │           │               └─ HTML5 (v1)
                  │             │           │                  DocBook, manpage (later)
                  │             │           └─ blocks, inlines, attributes, xrefs, TOC
                  │             └─ block & inline grammar, substitution pipeline
                  └─ includes, ifdef/ifndef/ifeval, attribute entries
```

### Crate layout (Cargo workspace)

| Crate | Responsibility |
|---|---|
| `adoc-cli` | Binary crate. `clap` argument parsing, file I/O, exit codes, wires the pipeline. |
| `adoc-core` | Shared types: `Document`, `Block`, `Inline`, `Attributes`, `Location`. `serde` serializable. No I/O. |
| `adoc-preprocessor` | Include resolution, conditional directives (`ifdef`, `ifndef`, `ifeval`, `endif`), attribute entries. Line-level. |
| `adoc-parser` | Block parser (line-oriented recursive descent) and inline parser (substitution pipeline). Produces `adoc-core::Document`. |
| `adoc-convert-html5` | Visits the AST, emits HTML5. Implements a `Converter` trait defined in `adoc-core`. |

Future crates: `adoc-convert-docbook`, `adoc-convert-manpage`, `adoc-ext-stdio`.

## Key design choices

### AST, not streaming

Section nesting, cross-references, auto-generated TOC, and block IDs all require resolution after the whole document is read. A full tree is the right default. Memory cost is acceptable for realistic document sizes.

### Hand-written parser

AsciiDoc's grammar is line-oriented and context-sensitive (block delimiters, nested blocks, list continuations, substitution groups per block type). Parser combinators (`nom`, `chumsky`) get unreadable fast here. Hand-written recursive descent gives us control over error recovery and source spans.

### Substitutions as a pipeline

The spec defines six substitution groups applied in order:

1. Special characters
2. Quotes (constrained/unconstrained formatting)
3. Attribute references
4. Replacements (entity-like: `(C)`, `--`, etc.)
5. Macros (inline)
6. Post-replacements (line breaks)

Each block type declares which groups apply. We model substitutions as a composable pipeline; block parsers configure it per block. This matches the spec's structure and makes overrides (via `subs` attribute) straightforward.

### Diagnostics via `miette`

Every AST node carries a `Location { source: SourceId, byte_range, line, column }`. Errors and warnings report precise spans with `miette`-style formatted output. Include resolution preserves the include chain so errors in included files point through the chain.

### Serializable AST

The AST round-trips through `serde`. This is load-bearing for the future stdio extension model: `adoc --emit-ast doc.adoc | my-filter | adoc --from-ast --to html5`. Getting this right in v1 costs little and unlocks extensions later without a core rework.

### Unicode-correct by default

Source is UTF-8. Column offsets are character-based, not byte-based, for diagnostics. String slicing uses char boundaries.

## Conformance strategy

"Spec-compliant" is only meaningful if it's measurable. From the first milestone:

- A **conformance suite** under `tests/conformance/` — one `.adoc` input plus expected AST (JSON) and expected HTML5 output per feature.
- Fixtures derived from the spec's own examples where the normative text provides them.
- Asciidoctor's behavior is a sanity check, not the oracle. Where the spec is silent, we document our interpretation; where Asciidoctor diverges from the spec, we follow the spec and record the divergence.

## CLI surface (v1)

```
adoc [OPTIONS] <INPUT>...

Options:
  -o, --out <FILE>              Output path (default: stem + .html)
  -b, --backend <BACKEND>       html5 (default)
  -a, --attribute <NAME[=VAL]>  Set document attribute (repeatable)
  -D, --destination-dir <DIR>   Output directory
      --safe-mode <MODE>        unsafe|safe|server|secure  (default: safe)
      --base-dir <DIR>          Base directory for includes
      --emit-ast                Emit serialized AST (JSON) to stdout
      --from-ast                Read serialized AST from stdin instead of parsing
  -v, --verbose                 Increase log verbosity (repeatable)
  -q, --quiet                   Suppress warnings
  -h, --help
  -V, --version
```

Exit codes: `0` success, `1` usage error, `2` parse/convert error, `3` I/O error.

## Phasing

1. **Skeleton** — workspace scaffold, CLI reads a file and emits a `<body>`-wrapped paragraph. End-to-end pipeline shape proven.
2. **Block parser** — paragraphs, sections, lists (ordered/unordered/description), delimited blocks (listing, literal, example, quote, sidebar, passthrough, open), tables.
3. **Inline parser** — all quote forms, attribute references, cross-references, inline macros, passthroughs.
4. **Preprocessor** — include directives, conditional directives, attribute entries.
5. **HTML5 converter** — passes the conformance corpus.
6. **Stdio extension model** — `--emit-ast` / `--from-ast` stabilized, documented.
7. **Additional backends** — DocBook, man page.

## Dependencies (initial)

- `clap` (derive) — CLI parsing
- `miette` + `thiserror` — diagnostics and errors
- `serde` + `serde_json` — AST serialization
- `camino` — UTF-8 path handling
- `unicode-segmentation` — grapheme/column accounting
- `tracing` — structured logging
- Dev: `insta` — snapshot testing for the conformance suite
