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

### Full-feature showcase

[`docs/showcase.adoc`](docs/showcase.adoc) is a single document that exercises every feature `adoc` implements today — header metadata, every delimited block, all admonitions, source blocks, tables with cell formatters, inline extras (sub/sup/highlight/passthrough/footnote), block metadata, preprocessor directives with `include::` arguments, section IDs, TOC, sectnums, and sectanchors. Build the binary, then:

```bash
make showcase                        # renders docs/showcase.adoc → docs/showcase.html
```

The rendered output is checked into [`docs/showcase.html`](docs/showcase.html) so it can be browsed directly on GitHub.

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
      --emit-ast                Emit serialized AST (JSON) to stdout
      --from-ast                Read serialized AST from stdin or input file
      --emit-ast-schema         Emit JSON Schema for the AST and exit
      --emit-chunks             Emit block-level retrieval chunks (JSON) and exit
      --check                   Run the pipeline without writing output
      --werror                  Treat warnings as errors (non-zero exit)
      --diagnostic-format <F>   plain|json  (default: plain)
  -v, --verbose                 Increase log verbosity (repeatable)
  -q, --quiet                   Suppress warnings
  -h, --help
  -V, --version

Subcommands:
  init-genai [TARGET]   Drop AI-author instruction templates (AGENTS.md,
                        Copilot, Claude Skill, system prompt) into a
                        downstream project. See `--help` for --tools,
                        --force, --dry-run.
```

Exit codes: `0` success · `1` usage error · `2` parse/convert error · `3` I/O error.

### Using `adoc` in AI document pipelines

A handful of features make this tool a good fit for genai workflows where an LLM produces or modifies content and a build script validates it.

**Lint loop (generate → validate → fix).** `--check` runs the full pipeline without writing output; `--diagnostic-format=json` emits one NDJSON object per warning so the LLM can patch byte spans:

```bash
adoc draft.adoc --check --diagnostic-format=json 2> diags.ndjson
```

Each diagnostic carries a stable `code` (`adoc::xref::dangling`, `adoc::preprocess::include_cycle`, …), a `filename`, and `labels[].span.{offset,length}`. `--werror` flips the exit status to non-zero so a CI step can refuse to publish until warnings are zero:

```bash
adoc draft.adoc --check --werror   # exits 1 if any warning fires
```

**Structured-output target (skip prose generation).** `--emit-ast-schema` prints a JSON Schema for the [`Document`] type so models with structured-output modes (OpenAI `response_format`, Anthropic tool-use, …) can be constrained to produce valid AST directly:

```bash
adoc --emit-ast-schema > ast-schema.json   # feed to the model
echo "$model_output" | adoc --from-ast -o out.html
```

The AST round-trips through `serde_json` (`--emit-ast | jq … | --from-ast`) so transformation pipelines can manipulate the tree without touching the parser.

**Retrieval chunks for RAG.** `--emit-chunks` walks the document and produces one entry per leaf block — paragraphs, lists, tables, listings, admonitions, callout descriptions — with the containing-section path, plain-text body, and a SHA-256 content hash:

```bash
adoc handbook.adoc --emit-chunks --quiet > chunks.json
```

```jsonc
[
  {
    "section_path": ["_methods", "_setup"],
    "section_title": "Setup",
    "block_index": 2,
    "kind": "paragraph",
    "text": "Each run starts from a clean checkout...",
    "hash": "sha256:9f3c...e1"
  }
]
```

Two re-runs over the same source produce identical hashes; an edit to a single block changes only that block's hash, so an embedding-index can re-embed only the changed entries. `section_path` is the chain of section ids — the same identifiers anchored sections render to in HTML, so a chunk and its source render share `#`-fragment links.

**Sandboxing untrusted generated content.** `--safe-mode safe --base-dir <dir>` rejects any `include::` that escapes the directory tree or uses an absolute path, so a hallucinated `include::../../etc/passwd[]` is denied at the preprocessor — even if the LLM produces it. `secure` disables `include::` entirely.

**Author-side instruction templates.** [`docs/genai/`](docs/genai/) ships drop-in instruction files for the major AI authoring tools — `AGENTS.md` (Codex / Cursor / aider), `.github/copilot-instructions.md` (Copilot), a `claude-skill/` folder (Claude Code / Claude Desktop), and a flat `system-prompt.md` for ad-hoc API calls. They cover the working subset of AsciiDoc, the lint-loop, the unsupported constructs to avoid, and the `<<id>>`-in-backticks footgun.

The `init-genai` subcommand drops the templates straight into a downstream project — no clone-and-copy ritual:

```bash
adoc init-genai [TARGET]                  # all four files; default target = "."
adoc init-genai --dry-run                 # preview the actions
adoc init-genai --tools=agents,claude     # selective install
adoc init-genai --force                   # overwrite existing files
```

Without `--force`, existing files at the target paths are left alone and reported as skipped. The templates are baked into the binary so the subcommand works in any directory without needing the `adoc` source tree on disk.

## What works today

- Document header: title, multiple authors with optional emails, revision (`vN, date: remark`), attribute entries.
- Sections to level 5 with nested section parsing.
- Paragraphs and hard line breaks (` +`).
- Ordered, unordered, and description lists with depth and `+` continuation.
- All seven delimited block styles: listing, literal, example, quote, sidebar, passthrough, open.
- Constrained and unconstrained inline quotes: `*strong*`, `_emphasis_`, `` `monospace` ``, plus `**`/`__`/```` `` ```` forms.
- Inline macros: `link:`, `mailto:`, `xref:`, `image:`, `anchor:id[]`, `kbd:[Ctrl+C]`, `btn:[OK]`, `menu:File[Save > Save As]`, shorthand `<<xref>>`, and `http(s)`/`ftp` autolinks. `link:{attr}[label]` substitutes attribute references in the URL; `https://url[label]` is also accepted (bare URL with explicit text).
- Index terms — `(((primary)))`, `(((primary, secondary)))`, `(((primary, secondary, tertiary)))` render as invisible `<span class="indexterm">` markers carrying `data-primary` / `data-secondary` / `data-tertiary` attributes. No visible flow text — downstream tooling can scan the markers to build an index.
- Bibliography entries — `[[[id]]]` (triple-bracket) at the start of any inline run emits `<a id="…"></a>[id]`, so a bibliography list (`* [[[knuth1968]]] Knuth, Donald. …`) gets an anchor target plus a visible `[knuth1968]` label. `<<knuth1968>>` resolves via the same xref machinery.
- `:toc-placement:` — `auto` (default; TOC at the top of `<main id="content">`) or `preamble` (TOC right after the preamble div, between the intro prose and the first section). `macro` / `left` / `right` fall back to `auto` in v1.
- Block image — `image::path[alt, width, height]` on its own line renders as `<div class="imageblock">` with optional `.Title` caption.
- Block video / audio — `video::url[width, height, poster, autoplay, loop, muted, playsinline]` and `audio::url[loop]` on their own line render as `<div class="videoblock">` / `<div class="audioblock">` wrapping a native HTML5 `<video>` / `<audio>` element. `controls` is on by default.
- Derived header attributes — `{doctitle}`, `{author}`, `{authors}`, `{firstname}`, `{middlename}`, `{lastname}`, `{authorinitials}`, `{email}`, `{author_2}` / `{email_2}` for additional authors, and `{revnumber}` (with leading `v` stripped), `{revdate}`, `{revremark}`. User-supplied attribute entries take precedence.
- `[discrete]` headings — same `==`-style title syntax, but the heading doesn't open a new section: rendered as `<hN class="discrete">` and the surrounding blocks remain siblings.
- Attribute references (`{name}`) with the document attribute context.
- Character replacements: `(C)`, `(R)`, `(TM)`, `...`, `--`, `->`, `=>`, `<-`, `<=`.
- Smart quotes — `"`text`"` renders curly double quotes (`\u{201C}…\u{201D}`), `'`text`'` renders curly single quotes (`\u{2018}…\u{2019}`). Word-boundary rules mean apostrophes in contractions (`it's`, `don't`) stay literal.
- `[verse]` quote blocks — `[verse, Author, Source]` on a `____` block preserves whitespace and line breaks (rendered as `<pre class="verseblock">`). The `[quote, …]` and `[verse, …]` positionals also produce a trailing `<div class="attribution">— Author<br><cite>Source</cite></div>` line.
- Block metadata — `[source,rust]`, `[NOTE]`, `[#id.role%opt]`, `[caption="…"]`, `.Title` lines — attached to the following block. The HTML5 backend emits `id`, `class`, and a `<div class="title">` accordingly.
- Preprocessor directives — `include::path[]` (relative to the including file's directory, cycle detection, 64-deep limit), `ifdef::name[]` / `ifndef::name[]` (block + inline form, `,` any-of, `+` all-of), `ifeval::[expr]` over numbers / quoted strings / attribute refs with `==`, `!=`, `<`, `<=`, `>`, `>=`, and `endif::[]`.
- `include::` arguments — `lines=` (single line, range, open-ended `..-1`, multiple `;`-separated ranges), `tags=` / `tag=` (comment-leader-agnostic `tag::name[]` / `end::name[]` markers, multiple tags, `!name` negation, `*` matches any tagged region, `**` matches every line), `leveloffset=` (signed, clamped to `1..=6`), `indent=` (strips the common leading whitespace and prepends N spaces — `indent=0` strips entirely), `encoding=` (accepted for spec compatibility; the loader always reads UTF-8 in v1, so the value is currently a no-op).
- `:doctype:` — surfaces as a body class (`<body class="doctype-book">` etc.) so themes can target the doctype. Default `article` keeps the body unclassed. Level-0 part parsing for `book` (the user-facing structural difference) is queued.
- Safe modes — `unsafe` (no checks), `safe` / `server` (rejects absolute include paths and any path that escapes `--base-dir` after canonicalisation), `secure` (disables `include::` entirely).
- Section IDs — auto-derived from titles (`_` prefix, lowercase, non-alphanumeric collapsed to `_`, deduped with `_2`/`_3`/… suffixes), or supplied explicitly via the `[#id]` shorthand or the legacy `[[id]]` / `[[id, reftext]]` block-anchor line. Anchored blocks render with `id="…"` so `xref:` targets resolve.
- Doc-wide ID registry — built once per document via `adoc::ast::IdRegistry::collect(&doc)`. Picks up section IDs, block IDs from `[#…]` shorthand, inline anchors (`anchor:id[]`), bibliography anchors (`[[[id]]]`), and `<a id="…">` emitted from `Inline::RawHtml`. The HTML5 converter validates every `<<…>>` / `xref:…[]` target against the registry and produces span-pointing diagnostics for unresolved references — see *Diagnostics* below.
- Diagnostics — span-pointing warnings and errors with source-snippet rendering. The HTML5 converter exposes `convert_with_diagnostics(&doc) -> Result<(String, Diagnostics), _>` alongside the trait-level `convert`. Each diagnostic carries a stable code (`adoc::xref::dangling`, `adoc::preprocess::include_cycle`, `adoc::preprocess::stray_endif`, `adoc::preprocess::secure_mode`, `adoc::preprocess::absolute_include`, `adoc::preprocess::include_path`, `adoc::preprocess::include_depth`), a [`Location`], an optional help line, and a label for the underlined span. The CLI feeds them through miette so users see the file path, surrounding source, and an underlined span for every problem. Source context comes from a [`SourceMap`] populated by the preprocessor as it walks `include::` directives. `--quiet` suppresses warning rendering; errors still abort. `--diagnostic-format=json` emits one NDJSON object per diagnostic for tooling consumption (GitHub annotations, SARIF converters, IDE plugins).
- `<<Section Title>>` xref resolution — a post-parse pass rewrites any `xref:` / `<<>>` whose target string matches a section's plain-text title to instead point at that section's id (auto-derived or explicit), and keeps the original target as the visible link text when no `[label]` was supplied. Explicit ids take precedence; targets that match neither pass through and trigger the dangling-xref warning.
- Admonitions — paragraph form (`NOTE: text`, `TIP: …`, `IMPORTANT: …`, `WARNING: …`, `CAUTION: …`) and block form (`[NOTE]` on any paragraph or `====` example block) render as `<div class="admonitionblock kw">` with a labelled body. Bundled CSS gives each variant a coloured side-rule.
- Source blocks with language — `[source,LANG]` on a `----` listing emits `<pre data-lang="LANG"><code class="language-LANG">…</code></pre>`. The `data-lang` attribute drives a small CSS-only language pill in the corner; the `language-LANG` class is the integration point for any client-side highlighter.
- Inline extras — subscript `~text~`, superscript `^text^`, highlight `#text#` / `##text##`, constrained passthrough `+text+` and unconstrained `++text++` (HTML-escaped, no inline subs), `pass:[…]` macro (raw HTML), and inline footnotes `footnote:[…]` / `footnote:id[…]`. The converter rewrites each inline footnote into a numbered `<sup class="footnote">[<a href="#_footnotedef_N">N</a>]</sup>` ref and gathers the bodies into a `<div id="footnotes">` section at the end of the body, with back-links to the inline ref via `_footnoteref_N` / `_footnotedef_N`.
- TOC, sectnums, sectanchors — `:toc:` emits a nested `<div id="toc">` with title links above the body; `:sectnums:` prepends `1.2.3` numbering to section headings (and TOC entries); `:sectanchors:` adds a hover-revealed `<a class="anchor">` next to each heading. All three are computed in a single pre-walk.
- `--emit-ast` / `--from-ast` — the AST round-trips through `serde_json`, locking in the JSON shape as a public contract. The stdio extension model now works:
  ```
  adoc --emit-ast doc.adoc | jq … | adoc --from-ast -o out.html
  ```
  Variants are internally tagged: `{"kind": "text", "value": "hi"}`, `{"kind": "section", "level": 1, "title": [...], …}`, etc. Unit variants serialize as `{"kind": "line_break"}`.
- Tables — `<thead>`/`<tbody>` split, header rows detected via `[%header]` / `options="header"` or the spec's blank-line-after-first-row heuristic; cell formatters `m|` (monospace), `s|` (strong), `e|` (emphasis), `l|` (literal `<pre>`), `h|` (forced `<th>` even in body rows), and `a|` (AsciiDoc — content recursively parses as nested blocks). `cols="…"` parses widths (`1,2,1`), alignments (`<,^,>`), repeats (`3*<`), bare-integer shorthand (`3` ⇒ three equal columns), and width+alignment combos (`<2,^1,>1`); widths emit a `<colgroup>` with normalised percentages and alignments paint `class="halign-left|center|right"` on each cell. Per-cell formatter prefixes combine repeat (`3*|`), span (`2+|`, `.3+|`, `2.3+|`), alignment (`<|` / `^|` / `>|`), and style letter — e.g. `2+a|`, `^m|`, `<s|`, `3*|` — and rows can begin with the formatter alone (e.g. `a| content`). Cells are parsed flat-then-grouped: a cell can span multiple source lines (essential for `a|` blocks), rowspan correctly skips occupied slots in subsequent rows, and `[format=csv]` / `[format=dsv]` (with optional `separator=`) replace the `|`-based parser with a comma- or character-separated reader (CSV mode honours `"…"` quoted cells with `""` escapes).
- Source-block syntax highlighting — opt-in via `:source-highlighter: prism` or `:source-highlighter: highlightjs`. The converter loads a light + dark theme pair from the matching CDN with `media="(prefers-color-scheme: …)"`, so code rendering follows the document's color scheme. Defaults: `prism` / `prism-tomorrow` and `github` / `github-dark`. Each side overridable via `:prism-theme:` / `:prism-dark-theme:` / `:highlightjs-theme:` / `:highlightjs-dark-theme:`; set the dark variant to `!:` to suppress it. A small inline `<style>` re-asserts `--adoc-code-bg` / `--adoc-code-fg` on the highlighter's surface so backgrounds match the rest of the document.
- Source-block callouts — `<1>` / `<2>` markers inside listing or literal blocks render as `<b class="conum">(N)</b>`, and a sibling `<N> description` block — one or more adjacent callout lines after the listing — becomes an `<ol class="colist">` with each `<li value="N">` carrying the matching description. Markers render whether or not a colist follows; a colist after a non-listing block is treated as ordinary text.

## What's missing

The big-ticket items, in roughly the order they're queued:

- **Level-0 parts under `:doctype: book`** — the body-class is set today, but `= Part Title` mid-document isn't yet recognised as a part wrapper.
- **Stem / math** (`stem:[]`, `latexmath`, `asciimath`) — deliberately out of v1 scope; would need MathJax/KaTeX integration along the same pattern as `:source-highlighter:`.

### Spec footguns worth knowing about

These aren't bugs — they're spec-compliant — but they trip new users:

- **`<<id>>` inside `` `monospace` ``**. Backticks apply every substitution, including macros, so `` `<<intro>>` `` renders as a clickable xref styled as monospace, not as literal `<<intro>>` text. Use the constrained-passthrough `+` to keep it literal: `` `+<<intro>>+` ``.

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

Two test corpora work together. The structural suite under `tests/fixtures/` (41+ `.adoc` inputs) drives `tests/fixtures.rs` and asserts *shape* — node counts, presence of particular tags. The conformance suite under `tests/conformance/<entry>/` asserts *byte-identity*: each entry has `input.adoc`, `expected.ast.json`, and `expected.html` (rendered with no stylesheet so CSS edits don't dominate diffs). Bless intentional changes with `ADOC_CONFORMANCE_BLESS=1 cargo test --test conformance`. Add a conformance entry alongside any new spec feature.

## Contributing

Issues and pull requests welcome. Some house rules:

- Read [DESIGN.md](DESIGN.md) before proposing architectural changes — there are load-bearing constraints (every node carries a `Location`; the AST is the public contract for extensions; converters never depend on the parser).
- New language features must ship with a fixture in `tests/fixtures/` (and, eventually, a matching `tests/conformance/` entry).
- The spec is the oracle. Asciidoctor is a sanity check, not the source of truth.

## License

Licensed under the [Apache License, Version 2.0](https://www.apache.org/licenses/LICENSE-2.0).
