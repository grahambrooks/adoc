---
name: asciidoc-author
description: Use this skill when authoring or editing AsciiDoc documents in projects that use the `adoc` Rust CLI to render them. Covers the working subset of AsciiDoc that `adoc` supports, common pitfalls, the lint-loop workflow, and the AST/chunks/schema entry points for tooling integrations. Invoke whenever the user's task involves writing `.adoc` files, validating AsciiDoc, generating chunks for retrieval, or producing AST-shaped output for downstream tools.
---

# Authoring AsciiDoc for `adoc`

You are helping the user produce or edit AsciiDoc documents that will
be processed by the `adoc` Rust CLI. `adoc` follows the AsciiDoc
Language specification and **diverges from Asciidoctor where the spec
requires it** — don't generate Asciidoctor-isms by reflex.

## Lint loop (the closing of every authoring task)

After every batch of edits, validate:

```bash
adoc <file>.adoc --check --werror --diagnostic-format=json 2> diags.ndjson
```

* Empty `diags.ndjson` and exit 0 ⇒ done.
* Otherwise each line is a JSON object with `code`, `filename`,
  `labels[].span.{offset,length}`, and `help`. Patch the byte range
  the span identifies; iterate.

Common diagnostic codes and how to address them:

| Code | Action |
| --- | --- |
| `adoc::xref::dangling` | The target id doesn't exist. Use one of the ids the `help` line suggests, or add `[#id]` above the section being referenced. For forward refs, `<<Section Title>>` auto-resolves to the derived id. |
| `adoc::preprocess::stray_endif` | Add the missing `ifdef::name[]` / `ifndef::name[]` / `ifeval::[expr]`, or delete the orphan `endif::[]`. |
| `adoc::preprocess::include_cycle` | Refactor so each file appears at most once in the include chain. The chain is in the message. |
| `adoc::preprocess::absolute_include` / `include_path` | Use a relative path under `--base-dir`. |

## Authoring conventions

### Sections and ids

* Document title is `= Title` once, at the top.
* Sections use `==` (level 1) through `=====` (level 4), rendered as
  `<h2>`–`<h5>`.
* Auto-derived ids: title is lowercased, non-alphanumeric chars
  collapse to `_`. "Methods Setup" ⇒ `_methods_setup`.
* For an explicit id, write `[#stable-id]` *immediately above* the
  section line.

### Cross-references

Three valid forms:

* `<<id>>` — shortest; uses the id verbatim as the link text.
* `xref:id[Custom Label]` — explicit text.
* `<<Section Title>>` — title-text resolution; the post-parse pass
  rewrites the target to the derived id. Prefer this for forward
  refs in prose.

### Inline formatting

* Constrained: `*strong*`, `_em_`, `` `mono` ``.
* Unconstrained mid-word: `**bo**ld`, `__it__alic`, `` ``mo``no ``.
* Smart quotes: `"`text`"` and `'`text`'`. Apostrophes in
  contractions stay literal.
* Highlight: `#text#`, `##text##`.
* Sub/sup: `H~2~O`, `E=mc^2^`.

### The `<<id>>`-in-backticks footgun

`` `<<intro>>` `` is parsed as a clickable xref styled as monospace,
not as literal `<<intro>>` text. To keep the literal text, wrap the
inner part in a constrained passthrough:

```adoc
The `+<<intro>>+` shorthand is the xref form.
```

This is spec-compliant — backticks apply every substitution including
macros — but it surprises users every time.

### Tables

Always set `cols=` on multi-column tables to avoid mis-grouping when
rows aren't uniform:

```adoc
[cols="1,2,1"]
|===
| Name | Description | Type
| age  | Years lived | int
|===
```

For machine data, use `[%header,format=csv]` and let the CSV reader
handle quoted commas:

```adoc
[%header,format=csv]
|===
Name,Age,Role
Alice,30,Engineer
"Bob, with comma",25,Designer
|===
```

### Source blocks and callouts

Always tag the language. For step-by-step code, use callouts:

```adoc
[source,rust]
----
fn main() {
    let x = 1; <1>
    println!("{x}"); <2>
}
----
<1> Bind a value.
<2> Print it.
```

### Skeleton-and-fill with includes

When generating a multi-section document, write a skeleton and fill
the leaves separately. Use tag wildcards so partial regens don't
clobber human edits:

```adoc
include::generated/intro.adoc[tags=ai-generated]
```

```adoc
// tag::ai-generated[]
This paragraph is regenerable.
// end::ai-generated[]

This paragraph below the close marker is human-edited.
```

Build under safe mode so a hallucinated escape is blocked at the
preprocessor:

```bash
adoc skeleton.adoc --safe-mode safe --base-dir generated/
```

## Tooling entry points (when the task is "produce data, not docs")

These commands turn the same parser into a tooling target:

```bash
# JSON Schema for the AST — feed to structured-output modes
adoc --emit-ast-schema > schema.json

# Round-trip the AST — manipulate the tree, then render
adoc input.adoc --emit-ast | jq '...' | adoc --from-ast -o out.html

# Block-level retrieval chunks (section_path, plain text, SHA-256)
adoc input.adoc --emit-chunks --quiet > chunks.json
```

When the user says "give me the document as structured data", they
almost always want one of these — the AST for full fidelity, the
chunks for embeddings, the schema to constrain a separate model run.

## What `adoc` does not yet support

Don't emit these — they'll fall back to literal text or fire
diagnostics:

* `:doctype: book` level-0 parts (`= Part Title` mid-document).
* `stem:[]`, `latexmath`, `asciimath` — math.
* Single-paren `((flow text))` visible index terms — only the
  triple-paren invisible form `(((primary, secondary)))` is in.
* `include::` non-UTF-8 `encoding=` — value parses but is ignored.

## When in doubt

* Read [`AGENTS.md`](../AGENTS.md) at the project root — it's the
  same content but cross-vendor and lives next to the files the user
  is editing.
* The repo's `docs/showcase.adoc` exercises every supported feature
  in one document; cross-reference it when you need to confirm that
  a construct works.
* Run `adoc <file>.adoc --check --werror`. If it exits 0, you're done.
