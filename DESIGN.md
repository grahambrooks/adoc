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

### Module layout

`adoc` is a single Cargo crate. The pipeline is realised as four sibling modules under `src/`, plus a binary at `src/main.rs`:

| Module | Responsibility |
| --- | --- |
| `adoc::ast` | Shared types: `Document`, `Block`, `Inline`, `Attributes`, `Location`, plus the `Converter` trait. `serde` serializable. No I/O. |
| `adoc::preprocessor` | Include resolution, conditional directives (`ifdef`, `ifndef`, `ifeval`, `endif`), attribute entries. Line-level. |
| `adoc::parser` | Block parser (line-oriented recursive descent) and inline parser (substitution pipeline). Produces `adoc::ast::Document`. |
| `adoc::convert::html5` | Visits the AST, emits HTML5. Implements `adoc::ast::Converter`. Owns the built-in stylesheet asset (`assets/adoc.css`). |
| `src/main.rs` | Binary `adoc`. `clap` argument parsing, file I/O, exit codes, wires the pipeline. |

Future siblings under `src/convert/`: `docbook`, `manpage`. Future top-level modules: `ext` (stdio extension model).

Dependency direction inside the crate: `main → {parser, preprocessor, convert::html5} → ast`. Cycles are forbidden; converters must not depend on the parser.

The project lived as a five-crate Cargo workspace (`adoc-cli`, `adoc-core`, `adoc-preprocessor`, `adoc-parser`, `adoc-convert-html5`) early on; it was consolidated into a single crate once the dependency graph stabilised. Workspace boundaries were enforcing what plain module boundaries already enforce (the dependency direction above), at the cost of five separate `Cargo.toml`s and slower compile cycles.

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

Each block type declares which groups apply. The pipeline is modeled as a `Subs` flag struct; block parsers configure it per block. This matches the spec's structure and makes overrides (via `subs` attribute) straightforward.

Implementation note: special-character escaping is currently performed at the HTML5 render boundary rather than as a parse-time substitution. The AST holds raw UTF-8 text in `Inline::Text`; converters are responsible for the appropriate escape. This keeps the AST language-neutral but means the special-characters group is implicit, not a configurable step. Revisit if a backend needs to opt out.

### Diagnostics via `miette`

Every AST node carries a `Location { source: SourceId, byte_range, line, column }`. Errors and warnings are intended to report precise spans through `miette`. Include resolution preserves the include chain so errors in included files point through the chain.

Current state: locations are populated, but `ParseError`/`PreprocessError` are flat `Message(String)` enums. Promoting them to span-bearing `miette::Diagnostic` impls is queued behind preprocessor work — there's little to report on until directives can fail.

### Serializable AST

The AST round-trips through `serde`. This is load-bearing for the future stdio extension model: `adoc --emit-ast doc.adoc | my-filter | adoc --from-ast --to html5`. Getting this right in v1 costs little and unlocks extensions later without a core rework.

The serialized JSON form is a public interface — once `--emit-ast` ships, breaking changes to AST node shapes need intent and a versioning story.

### Stylesheet model

HTML5 output ships with a built-in stylesheet (`adoc.css`, embedded into the binary). Resolution mirrors Asciidoctor's attribute-driven model and happens at the CLI boundary; the converter receives a fully-resolved `Stylesheet` enum and renders it.

| Attribute combination | Result |
|---|---|
| (default) | Inline the built-in stylesheet via `<style>`. |
| `:linkcss:` | Emit `<link rel="stylesheet" href="adoc.css">`. |
| `:stylesheet: theme.css` | Read `theme.css` from `:stylesdir:` (default: input dir) and inline it. |
| `:stylesheet: theme.css` + `:linkcss:` | Emit `<link>` to the supplied href; do not read the file. |
| `:stylesheet!:` or empty | Emit no stylesheet. |
| `:copycss:` | Copy the resolved CSS next to the output file. |

Five modes total: `BuiltinEmbed` (default), `BuiltinLink`, `CustomEmbed`, `CustomLink`, `None`.

### Unicode-correct by default

Source is UTF-8. Column offsets are character-based, not byte-based, for diagnostics. String slicing uses char boundaries.

## Conformance strategy

"Spec-compliant" is only meaningful if it's measurable. The plan is a layered test corpus:

- **Now:** integration fixtures under `tests/fixtures/` — 20 `.adoc` inputs each driving a structural assertion through the full pipeline (parser + HTML5). These pin the v1 feature set during development.
- **Next:** a **conformance suite** under `tests/conformance/` — one `.adoc` input plus expected AST (JSON, snapshotted with `insta`) and expected HTML5 output per feature, derived from the spec's normative examples where available.
- Asciidoctor's behavior is a sanity check, not the oracle. Where the spec is silent, document the interpretation; where Asciidoctor diverges from the spec, follow the spec and record the divergence.

The fixture set bootstraps confidence and gets retired as the conformance suite grows.

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

Status: the happy path renders to file or stdout; `--emit-ast` and
`--from-ast` are wired (round-trip is exercised by the integration
suite); `--safe-mode` is enforced for `include::` (`safe`/`server`
reject path escapes, `secure` disables includes entirely). Multiple
input files are accepted at the CLI but only the first is processed —
queued behind the diagnostics phase.

## Implementation status

The original phasing assumed a strict left-to-right walk; in practice the block parser, inline parser, HTML5 converter, and preprocessor were built in parallel against a fixture suite. Current state, grouped by subsystem:

### Pipeline

| Area | Status |
|---|---|
| Workspace skeleton, CLI shell, end-to-end pipeline | done |
| Stylesheet resolution (five modes) and `:copycss:` | done |
| `--emit-ast` / `--from-ast` wiring; AST roundtrips through `serde_json` (verified by a per-fixture render → JSON → parse-back → render byte-identity test) | done |
| Multi-input handling — currently only the first input is processed | **not started** |

### Header & block parser

| Area | Status |
|---|---|
| Header: title, multiple authors with optional emails, revision (`vN, date: remark`), leading/trailing attribute entries | done |
| Derived header attributes: `{doctitle}`, `{author}` / `{authors}` / `{firstname}` / `{middlename}` / `{lastname}` / `{authorinitials}` / `{email}`, `{author_N}` / `{email_N}` for additional authors, `{revnumber}` (leading `v` stripped) / `{revdate}` / `{revremark}`. User entries take precedence | done |
| Sections (levels 1–5) with nested parsing | done |
| Lists (ordered, unordered, description) with depth and `+` continuation | done |
| All seven delimited styles (listing, literal, example, quote, sidebar, passthrough, open) | done |
| `[verse]` quote blocks — raw inner text rendered in `<pre class="verseblock">`; `[quote, A, S]` / `[verse, A, S]` emit a `<div class="attribution">` | done |
| Block metadata: `[attrlist]` (positional/named/shorthand `#id.role%opt`), `.Title` lines, `[NOTE]`-style admonition styles, attached to the next block via `BlockMeta` | done |
| `[discrete]` headings — section title syntax that doesn't open a new section | done |
| Block image (`image::path[alt]`) and block video / audio (`video::url[…]`, `audio::url[…]`) on their own line render as `<div class="imageblock|videoblock|audioblock">` with optional `.Title` caption | done |
| Section IDs — `[#id]` shorthand, `[[id]]`/`[[id, reftext]]` legacy anchor lines, and auto-derivation from titles (lowercase, non-alphanumeric → `_`, deduped) | done |
| Doc-wide ID registry (`adoc::ast::IdRegistry`) — collects section / block / inline-anchor / bibliography ids in one walk; HTML5 converter validates every xref against it and emits `tracing::warn!` for dangling targets | done |
| `<<title text>>` xref resolution — a post-parse pass rewrites targets matching a section title to that section's id; explicit ids win over title matches | done |

### Inline parser

| Area | Status |
|---|---|
| Constrained + unconstrained quotes (`*strong*`, `_em_`, `` `mono` ``, plus the `**`/`__`/`` `` `` `` `` forms) | done |
| Attribute references (`{name}`); macros run before the attribute pass, so `link:` / `mailto:` / `xref:` macro arguments call into the same resolver explicitly | done |
| Macros: `link:`, `mailto:`, `xref:`, `image:`, `anchor:id[]`, `kbd:[Ctrl+C]`, `btn:[OK]`, `menu:File[Save > Save As]`, shorthand `<<xref>>`, `pass:[]`, `footnote:[]` / `footnote:id[]` | done |
| Index terms `(((primary)))` / `(((primary, secondary)))` / `(((p, s, t)))` — invisible `<span class="indexterm">` with `data-primary`/`-secondary`/`-tertiary` attributes | done |
| Bibliography entries `[[[id]]]` and citations via `<<id>>` (single-id) | done |
| HTTP/HTTPS/FTP autolinks; bare-URL-with-label form (`https://url[label]`) | done |
| Eight character replacements: `(C)`, `(R)`, `(TM)`, `...`, `--`, `->`, `=>`, `<-`, `<=` | done |
| Smart quotes — `"`text`"` and `'`text`'` render as curly Unicode quotes; word-boundary rules keep contractions literal | done |
| Subscript `~`, superscript `^`, highlight `#`/`##`, passthrough `+`/`++` (HTML-escape, no subs), hard line break (` +`) | done |
| Inline footnotes get rewritten by the converter into numbered `<sup>` refs and gathered into a `<div id="footnotes">` end-of-doc section | done |

### Preprocessor

| Area | Status |
|---|---|
| `include::` with cycle detection and depth limit | done |
| `ifdef::` / `ifndef::` (block + inline forms, `,` any-of, `+` all-of) | done |
| `ifeval::` (numeric or string compare on attribute refs / literals) | done |
| Attribute entries evaluated at preprocess time so conditionals see them | done |
| `include::` arguments: `lines=`, `tags=`/`tag=` (with `!name` negation, `*` any-tagged, `**` all), `leveloffset=`, `indent=`, `encoding=` (accepted; v1 always reads UTF-8) | done |
| Safe-mode enforcement (`safe`/`server` reject absolute paths and `..`-escapes; `secure` disables `include::`) | done |

### HTML5 converter

| Area | Status |
|---|---|
| Every current AST node renders; document title, authors, revision in `<header>`; body wrapped in `<main id="content">`; preamble blocks grouped in `<div id="preamble">` | done |
| TOC, sectnums, sectanchors driven by document attributes; computed in a single pre-walk | done |
| `:toc-placement:` — `auto` (default) or `preamble`; `macro` / `left` / `right` fall back to `auto` | done |
| Admonitions: paragraph shortcut (`NOTE: …`) and block-form render as `<div class="admonitionblock kw">` with default label or supplied title; default stylesheet ships SVG icons per kind | done |
| Source blocks with language: `[source,LANG]` ⇒ `<pre data-lang="LANG"><code class="language-LANG">…</code></pre>`; corner-pill language label via CSS | done |
| Source-block syntax highlighting: `:source-highlighter: prism|highlightjs` loads a light + dark theme pair gated by `prefers-color-scheme`, plus a surface override so code background follows the document tokens | done |
| Source-block callouts (`<1>`, `<2>` …) render as `<b class="conum">(N)</b>`; sibling `<N> description` lines parse to a `Block::Colist` and render as `<ol class="colist">` | done |
| Tables: `cols=` widths/alignments/repeats/bare-integer-N, header rows, cell formatters (`a\|`, `m\|`, `s\|`, `e\|`, `l\|`, `h\|`), per-cell alignment / span / repeat (`<m\|`, `2+\|`, `.3+\|`, `2.3+\|`, `3*\|`), AsciiDoc cells, multi-line cell continuation, rowspan-aware row chunking, CSV/DSV separators (`format=csv\|dsv` with `separator=`) | done |
| `:doctype:` — `article` (default) / `book` / `manpage` / `inline` surfaces as a body class so themes can target it; level-0 part parsing for `book` is queued | partial |

### Diagnostics & conformance

| Area | Status |
|---|---|
| `miette::Diagnostic` for span-pointing errors and warnings — `adoc::diag::{Diagnostic, Diagnostics}` collector; `Preprocessor::source_map()` keeps file content alongside SourceId; `PreprocessError`/`ParseError` gain a `Diagnostic(...)` variant that carries a `Location`; the converter's xref pre-walk produces structured warnings; CLI renders via miette's graphical or JSON handler (`--diagnostic-format=plain\|json`) | done |
| Conformance suite under `tests/conformance/` (expected AST + HTML5 per spec example) | **not started** — the 37-fixture corpus under `tests/fixtures/` plus `docs/showcase.adoc` cover the v1 surface in the meantime |

## AST gaps

The current `adoc::ast` types cover what the parser produces. The remaining constructs that need new node shapes (or new fields) before the parser can emit them:

- `Inline` covers every form the parser emits today; index terms and bibliography anchors land via `Inline::RawHtml` rather than typed variants. Promoting them to `Inline::IndexTerm` / `Inline::BibAnchor` would let an index-page or bibliography backend consume them directly without re-scanning HTML; queued behind diagnostics work.
- Stem/math (`stem:[]`, `latexmath::[]`, `asciimath::[]`) blocks/inlines — out of v1 scope; would attach via the same pattern as `:source-highlighter:` (load MathJax/KaTeX from a CDN).

## CLI / pipeline gaps

- Multi-input handling: either iterate (one output per input) or document that only the first input is processed and reject the rest at parse time.

## Phasing (revised)

1. ~~**Skeleton** — project scaffold, CLI reads a file and emits a `<body>`-wrapped paragraph.~~ ✓
2. ~~**Block parser** — paragraphs, sections, lists, delimited blocks, basic tables.~~ ✓
3. ~~**Inline parser** — quotes, attribute references, cross-references, inline macros, replacements, line breaks.~~ ✓
4. ~~**Block metadata** — parse `[attr]` lines and `.Title` lines, attach to the following block via `BlockMeta`. HTML5 renders `id`/`class`/title.~~ ✓
5. ~~**Preprocessor** — `include::`, `ifdef`/`ifndef`/`ifeval`/`endif`, attribute entries evaluated before the parser sees them.~~ ✓
6. ~~**Section IDs** — auto-generate from titles, parse the legacy `[[anchor]]` form, populate `meta.id` on every section.~~ ✓ (Doc-wide ID registry + xref validation deferred to the diagnostics phase.)
7. ~~**`include::` argument forms** — `lines=`, `tags=`, `leveloffset=`, `indent=`.~~ ✓ (`encoding=` and tag wildcards still queued.)
8. ~~**HTML5 conformance** — TOC, section anchors, admonition markup, source-block markup with language class, full table model (cols, cell formatters, span, repeat, multi-line cells, CSV/DSV).~~ ✓ Conformance suite under `tests/conformance/` still queued.
9. ~~**HTML5 polish** — `<main id="content">` wrapper, preamble grouping, smart quotes, verse blocks with attribution, block image / video / audio, kbd/btn/menu UI macros, derived header attributes, `[discrete]` headings, inline anchors, light/dark-aware syntax-highlighter integration, end-of-doc footnote section.~~ ✓
10. **Diagnostics polish** — `miette::Diagnostic` for `ParseError`/`PreprocessError`/`ConvertError` with span pointers; doc-wide ID registry that powers xref validation; promote warnings (dangling xref, unknown attribute reference) into the diagnostic stream.
11. **Stdio extension model** — freeze and document the JSON schema (the wiring is done; documentation isn't); ship a trivial example filter.
12. **Additional backends** — DocBook, man page.

## Dependencies

In use:
- `clap` (derive) — CLI parsing
- `miette` + `thiserror` — diagnostics and errors
- `serde` + `serde_json` — AST serialization
- `camino` — UTF-8 path handling
- `unicode-segmentation` — grapheme/column accounting (added; not yet used in column reporting)
- `tracing` + `tracing-subscriber` — structured logging
- Dev: `insta` — snapshot testing, queued for the conformance suite

No new dependencies are anticipated for phases 4–7. Phase 8's extension model may pull in `jsonschema` for AST validation at the `--from-ast` boundary.
