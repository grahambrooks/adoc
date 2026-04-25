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

Status: flags are parsed and the happy path renders to file or stdout. The following are accepted but not yet honoured by the pipeline:

- `--emit-ast` / `--from-ast` — the binary always parses from disk and always renders.
- `--safe-mode` — parsed; no enforcement applied.
- Multiple input files — accepted; only the first is processed.

## Implementation status

The original phasing assumed a strict left-to-right walk; in practice the block parser, inline parser, and HTML5 converter were built in parallel against a fixture suite, and the preprocessor was deferred. Current state:

| Area | Status |
|---|---|
| Workspace skeleton, CLI shell, end-to-end pipeline | done |
| Block parser: paragraphs, sections (levels 1–5), ordered/unordered/description lists with depth and `+` continuation, all seven delimited styles, simple tables | done |
| Inline parser: constrained + unconstrained quotes (strong/em/mono), attribute references, `link:`/`mailto:`/`xref:`/`image:` macros, http/https/ftp autolinks, shorthand `<<xref>>`, eight character replacements, hard line break (` +`) | done |
| Header parser: title, multiple authors with optional emails, revision (`vN, date: remark`), leading/trailing attribute entries | done |
| HTML5 converter: every current AST node renders; document title, authors, revision in `<header>`; debug `<!-- attributes: ... -->` trailer | done |
| Stylesheet resolution (five modes) and `:copycss:` | done |
| Block metadata: `[attrlist]` (positional/named/shorthand `#id.role%opt`) and `.Title` lines collected and attached to the next block via `BlockMeta`; HTML5 emits `id`/`class` and a `<div class="title">` ahead of titled blocks | done |
| Preprocessor: `include::` (with cycle detection and a depth limit), `ifdef::`/`ifndef::` (block + inline forms, `,` any-of, `+` all-of), `ifeval::` (numeric or string compare on attribute refs / literals), `endif::`; attribute entries evaluated at preprocess time so conditionals see them | done |
| `include::` arguments: `lines=` (ranges, open-ended, multiple), `tags=`/`tag=` (multi, with `!name` negation), `leveloffset=` (signed, clamped) | done |
| `include::` arguments: `indent=`, `encoding=`, tag wildcards (`*`, `**`) | **not started** |
| Safe-mode enforcement: `safe`/`server` reject absolute paths and paths escaping `base_dir` after canonicalisation; `secure` disables `include::` | done |
| Section IDs — `[#id]` shorthand (via block metadata), `[[id]]` / `[[id, reftext]]` legacy anchor lines, and auto-generation from titles (lowercase, non-alphanumeric → `_`, deduped). Block parser rolls back metadata that turned out to belong to an outer scope (so `[[anchor]]` above a sibling-level section header attaches to the right section). | done |
| Doc-wide ID registry + xref validation (warn on dangling, resolve `<<title text>>` to derived IDs) | **not started** — sits with the diagnostics phase |
| Admonitions: paragraph shortcut (`NOTE: …`) and block-form (`[NOTE]` on any paragraph or `====` example) render as `<div class="admonitionblock kw">` with a default label or supplied title | done |
| Source blocks with language: `[source,LANG]` adds `language-LANG` class on the inner `<code>`; downstream-highlighter friendly | done |
| Inline extras: subscript `~`, superscript `^`, highlight `#`/`##`, passthrough `+`/`++` (HTML-escape, no subs), `pass:[]` macro (raw HTML), `footnote:[]` / `footnote:id[]` (rendered inline) | done |
| TOC, sectnums, sectanchors driven by document attributes; computed in a single pre-walk and rendered ahead of the body | done |
| `[discrete]` headings, `:toc-placement:`, custom TOC titles | **not started** |
| Inline anchors (`anchor:id[]`), bibliography entries (`[[[id]]]`), numbered end-of-doc footnote section | **not started** |
| Admonition blocks and admonition paragraphs | **not started** |
| Source blocks with language attribute (callouts, syntax-highlighter hint) | **not started** |
| Tables: column specs (`cols=`), header rows, cell formatters (`a\|`, `m\|`, `s\|`, `e\|`, `l\|`, `h\|`), `psv`/`csv`/`dsv` separators | **not started** (every row is a body row of plain inline cells) |
| Inline subscript/superscript/highlight (`~x~`, `^x^`, `#x#`), inline passthroughs (`+text+`, `pass:[]`), inline footnotes, inline anchors, bibliography entries | **not started** |
| TOC generation, discrete headings, `sectnums`/`sectanchors` honoured | **not started** |
| `doctype` (article/book/manpage/inline) influencing output | **not started** |
| `--emit-ast` / `--from-ast` wiring in `src/main.rs` | **not started** |
| Real `miette::Diagnostic` errors with span pointers (locations exist; error types don't carry them yet) | **not started** |
| Conformance suite under `tests/conformance/` (expected AST + HTML5 per spec example) | **not started** — `tests/fixtures/` covers the v1 surface in the meantime |

## AST gaps

The current `adoc::ast` types cover what the parser produces. Several spec constructs need new node shapes (or new fields) before the parser can emit them:

- `Block` needs an `Admonition` variant (or a derived view over `BlockMeta::style`) carrying `note` / `tip` / `important` / `warning` / `caution`. The metadata is already captured (`meta.style = Some("NOTE")`); the next bullet just needs to render it as an admonition.
- `Table` needs column specs, a separator kind, and per-cell `format`/`halign`/`valign`/`colspan`/`rowspan`. Header/footer row distinction belongs at the table level, not the row level. (`BlockMeta::named` already carries `cols=`, so the wiring exists.)
- `Inline` has `Subscript`, `Superscript`, `Highlight`, `Footnote`, and `Passthrough`. Still missing: `Anchor` (for `anchor:id[]`), `IndexTerm`, and bibliography entries (`[[[id]]]`). `Inline::RawHtml` is reached today via `pass:[]`.
- Cross-reference resolution needs a doc-wide ID registry built after parse, before convert. The registry's home is `adoc::ast` (so `Converter` impls can consult it), but populating it is a parser pass.

## CLI / pipeline gaps

- `--emit-ast` should emit `serde_json::to_string_pretty(&doc)` and exit before the converter runs. `--from-ast` should bypass the preprocessor and parser entirely and `serde_json::from_reader(stdin)`.
- Multi-input handling: either iterate (one output per input) or document that only the first input is processed and reject the rest at parse time.
- Safe modes need a real implementation: `unsafe` permits arbitrary include paths; `safe` rejects absolute paths; `server` additionally rejects `..`; `secure` disables `include::` and any macro that touches the filesystem.

## Phasing (revised)

1. ~~**Skeleton** — project scaffold, CLI reads a file and emits a `<body>`-wrapped paragraph.~~ ✓
2. ~~**Block parser** — paragraphs, sections, lists, delimited blocks, basic tables.~~ ✓
3. ~~**Inline parser** — quotes, attribute references, cross-references, inline macros, replacements, line breaks.~~ ✓
4. ~~**Block metadata** — parse `[attr]` lines and `.Title` lines, attach to the following block via `BlockMeta`. HTML5 renders `id`/`class`/title.~~ ✓
5. ~~**Preprocessor** — `include::`, `ifdef`/`ifndef`/`ifeval`/`endif`, attribute entries evaluated before the parser sees them.~~ ✓
6. ~~**Section IDs** — auto-generate from titles, parse the legacy `[[anchor]]` form, populate `meta.id` on every section.~~ ✓ (Doc-wide ID registry + xref validation deferred to the diagnostics phase.)
7. ~~**`include::` argument forms** — `lines=`, `tags=`, `leveloffset=` over the existing include path.~~ ✓
8. **HTML5 conformance** — match the spec's expected output for the conformance corpus: TOC, section anchors, admonition markup, source-block markup with language class, full table model. Stand up `tests/conformance/`.
9. **Diagnostics polish** — `miette::Diagnostic` for `ParseError`/`PreprocessError`/`ConvertError` with span pointers; promote warnings (dangling xref, unknown attribute reference) into the diagnostic stream.
10. **Stdio extension model** — implement `--emit-ast` / `--from-ast`; freeze and document the JSON schema; ship a trivial example filter.
11. **Additional backends** — DocBook, man page.

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
