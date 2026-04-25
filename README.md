# adoc

A Rust command-line processor for the [AsciiDoc Language specification](https://docs.asciidoctor.org/asciidoc/latest/), shipping HTML5 output and a serializable AST.

> **Status:** early development. The block parser, inline parser, and HTML5 converter cover the core language; the preprocessor (`include::`, `ifdef`/`ifeval`), block-attribute lines (`[source,rust]`, admonitions), and section IDs are not yet implemented. See [DESIGN.md](DESIGN.md) for the full status matrix.

## Why another AsciiDoc tool

- **Spec-first.** The [AsciiDoc Language specification](https://docs.asciidoctor.org/asciidoc/latest/) is the authority. Where it diverges from Asciidoctor, this tool follows the spec and documents the divergence.
- **Single static binary.** No Ruby, no Java, no runtime — one `adoc` executable.
- **Serializable AST.** Every node round-trips through `serde`. The eventual extension model is Unix-shaped: `adoc --emit-ast doc.adoc | my-filter | adoc --from-ast --to html5`.
- **Precise diagnostics.** Every AST node carries a source location so errors and warnings can point back to the byte range that caused them.

## Install

Requires Rust 1.80 or newer.

```bash
git clone https://github.com/grahambrooks/adoc
cd adoc
cargo build --release
# binary: target/release/adoc
```

A published crate is not yet available.

## Usage

```bash
adoc input.adoc                      # emit input.html next to input.adoc
adoc -o out.html input.adoc          # explicit output path
adoc -D dist input.adoc              # output into dist/
adoc -a toc -a sectnums input.adoc   # set document attributes
adoc input.adoc > input.html         # render to stdout
```

### Stylesheets

The HTML5 backend ships with a built-in stylesheet (light/dark aware) and mirrors Asciidoctor's attribute-driven model.

| Attribute              | Effect                                                  |
| ---------------------- | ------------------------------------------------------- |
| _(default)_            | Inline the built-in stylesheet via `<style>`.           |
| `:linkcss:`            | Link to `adoc.css` instead of inlining.                 |
| `:stylesheet: x.css`   | Inline a custom CSS file resolved against `:stylesdir:`.|
| `:stylesheet!:`        | Emit no stylesheet at all.                              |
| `:copycss:`            | Copy the resolved CSS next to the output file.          |

### CLI reference

```
adoc [OPTIONS] <INPUT>...

Options:
  -o, --out <FILE>              Output path (default: stem + .html)
  -b, --backend <BACKEND>       html5 (default)
  -a, --attribute <NAME[=VAL]>  Set document attribute (repeatable)
  -D, --destination-dir <DIR>   Output directory
      --safe-mode <MODE>        unsafe|safe|server|secure  (default: safe)
      --base-dir <DIR>          Base directory for includes
      --emit-ast                Emit serialized AST (JSON) to stdout  [planned]
      --from-ast                Read serialized AST from stdin         [planned]
  -v, --verbose                 Increase log verbosity (repeatable)
  -q, --quiet                   Suppress warnings
  -h, --help
  -V, --version
```

Exit codes: `0` success · `1` usage error · `2` parse/convert error · `3` I/O error.

## What works today

- Document header: title, multiple authors with optional emails, revision (`vN, date: remark`), attribute entries.
- Sections to level 5 with nested section parsing.
- Paragraphs and hard line breaks (` +`).
- Ordered, unordered, and description lists with depth and `+` continuation.
- All seven delimited block styles: listing, literal, example, quote, sidebar, passthrough, open.
- Simple tables (one cell per `|` delimiter; column specs and cell formatters are not yet handled).
- Constrained and unconstrained inline quotes: `*strong*`, `_emphasis_`, `` `monospace` ``, plus `**`/`__`/```` `` ```` forms.
- Inline macros: `link:`, `mailto:`, `xref:`, `image:`, shorthand `<<xref>>`, and `http(s)`/`ftp` autolinks.
- Attribute references (`{name}`) with the document attribute context.
- Character replacements: `(C)`, `(R)`, `(TM)`, `...`, `--`, `->`, `=>`, `<-`, `<=`.
- Block metadata — `[source,rust]`, `[NOTE]`, `[#id.role%opt]`, `[caption="…"]`, `.Title` lines — attached to the following block. The HTML5 backend emits `id`, `class`, and a `<div class="title">` accordingly.
- Preprocessor directives — `include::path[]` (relative to the including file's directory, cycle detection, 64-deep limit), `ifdef::name[]` / `ifndef::name[]` (block + inline form, `,` any-of, `+` all-of), `ifeval::[expr]` over numbers / quoted strings / attribute refs with `==`, `!=`, `<`, `<=`, `>`, `>=`, and `endif::[]`.
- `include::` arguments — `lines=` (single line, range, open-ended `..-1`, multiple `;`-separated ranges), `tags=` / `tag=` (comment-leader-agnostic `tag::name[]` / `end::name[]` markers, multiple tags, `!name` negation), `leveloffset=` (signed, clamped to `1..=6`).
- Safe modes — `unsafe` (no checks), `safe` / `server` (rejects absolute include paths and any path that escapes `--base-dir` after canonicalisation), `secure` (disables `include::` entirely).
- Section IDs — auto-derived from titles (`_` prefix, lowercase, non-alphanumeric collapsed to `_`, deduped with `_2`/`_3`/… suffixes), or supplied explicitly via the `[#id]` shorthand or the legacy `[[id]]` / `[[id, reftext]]` block-anchor line. Anchored blocks render with `id="…"` so `xref:` targets that previously dangled now resolve.
- Admonitions — paragraph form (`NOTE: text`, `TIP: …`, `IMPORTANT: …`, `WARNING: …`, `CAUTION: …`) and block form (`[NOTE]` on any paragraph or `====` example block) render as `<div class="admonitionblock kw">` with a labelled body. Bundled CSS gives each variant a coloured side-rule.
- Source blocks with language — `[source,LANG]` on a `----` listing emits `<pre><code class="language-LANG">…</code></pre>` so downstream highlighters (Prism, Highlight.js, Rouge) can take over without conflict.
- Inline extras — subscript `~text~`, superscript `^text^`, highlight `#text#` / `##text##`, constrained passthrough `+text+` and unconstrained `++text++` (HTML-escaped, no inline subs), `pass:[…]` macro (raw HTML), and inline footnotes `footnote:[…]` / `footnote:id[…]` (rendered inline as `<span class="footnote">`; numbered end-of-doc section is queued).
- TOC, sectnums, sectanchors — `:toc:` emits a nested `<div id="toc">` with title links above the body; `:sectnums:` prepends `1.2.3` numbering to section headings (and TOC entries); `:sectanchors:` adds a hover-revealed `<a class="anchor">` next to each heading. All three are computed in a single pre-walk.

## What's missing

The big-ticket items, in roughly the order they're queued:

- **`include::` argument tail** — `indent=`, `encoding=`, and tag wildcards (`*`, `**`). The common arguments (`lines=`, `tags=`, `leveloffset=`) are in.
- **`[discrete]` headings** and `:toc-placement:` (always top for now).
- **Doc-wide ID registry + xref validation** — section IDs land on the AST nodes today, but there's no centralised registry yet, so dangling xrefs render silently (the `<a href>` is emitted but the target doesn't exist). Validation belongs with the diagnostics work.
- **Built-in syntax highlighting** — we ship the `language-LANG` class only; no Rouge/Pygments/Prism integration. Ship of choice is BYO highlighter via a custom stylesheet.
- **Source-block callouts** — `<1>` / `<2>` markers and matching colist sibling block.
- **Full tables** — `cols=`, header rows, cell formatters (`a`, `m`, `s`, `e`, `l`, `h`), CSV/DSV separators.
- **Inline anchors** (`anchor:id[]`), bibliography entries (`[[[id]]]`), and the numbered end-of-doc footnote section.
- **TOC, discrete headings, `sectnums`, `sectanchors`.**
- **`--emit-ast` / `--from-ast`** wiring for the stdio extension model.
- **Real `miette::Diagnostic` errors** with span pointers (locations are already plumbed; error types just don't carry them yet).

See [DESIGN.md](DESIGN.md) for the full inventory and rationale.

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

Single Cargo crate, four pipeline modules plus a binary:

| Module                  | Role                                                                                  |
| ----------------------- | ------------------------------------------------------------------------------------- |
| `adoc::ast`             | AST types, `Location`, `Converter` trait, attribute model. No I/O.                    |
| `adoc::preprocessor`    | Line-level: includes, conditionals, attribute entries.                                |
| `adoc::parser`          | Hand-written recursive-descent block parser + inline substitution pipeline.           |
| `adoc::convert::html5`  | Implements the `Converter` trait for HTML5; owns the built-in stylesheet.             |
| `src/main.rs`           | Binary `adoc`. Parses CLI args, wires the pipeline.                                   |

Dependency direction inside the crate: `main → {parser, preprocessor, convert::html5} → ast`. Future converters (DocBook, manpage) sit beside `html5` under `src/convert/`.

## Development

```bash
make build            # cargo build
make test             # cargo test
make lint             # cargo clippy --all-targets -- -D warnings
make fmt              # cargo fmt --all
make examples         # render tests/fixtures/*.adoc to docs/examples/*.html
make ci               # fmt-check + lint + test
```

The integration corpus lives in `tests/fixtures/` (twenty `.adoc` inputs). A spec-derived conformance suite under `tests/conformance/` (with expected AST + HTML per feature) is queued behind block-metadata work.

## Contributing

Issues and pull requests welcome. Some house rules:

- Read [DESIGN.md](DESIGN.md) before proposing architectural changes — there are load-bearing constraints (every node carries a `Location`; the AST is the public contract for extensions; converters never depend on the parser).
- New language features must ship with a fixture in `tests/fixtures/` (and, eventually, a matching `tests/conformance/` entry).
- The spec is the oracle. Asciidoctor is a sanity check, not the source of truth.

## License

Licensed under the [Apache License, Version 2.0](https://www.apache.org/licenses/LICENSE-2.0).
