# Authoring AsciiDoc for `adoc`

This file gives AI authoring tools (Codex, Cursor, aider, Claude, ...)
the project-specific conventions for writing AsciiDoc documents that
this repository's `adoc` CLI processes. Read it once at the start of
an authoring task; come back to it when the lint loop fires.

## Validate before declaring done

After every batch of edits, run:

```bash
adoc <file>.adoc --check --werror --diagnostic-format=json 2> diags.ndjson
```

* Exit 0 with empty `diags.ndjson` ⇒ document is clean.
* Non-zero exit ⇒ each line of `diags.ndjson` is one diagnostic with
  `code`, `filename`, `labels[].span.{offset,length}`, and `help`.
  Patch the indicated byte range; iterate until empty.

The diagnostic codes you'll see:

| Code | Meaning | Typical fix |
| --- | --- | --- |
| `adoc::xref::dangling` | `<<id>>` or `xref:id[]` target isn't defined. | Use one of the `help` line's known ids, or add `[#id]` to the section being referenced. The `<<Section Title>>` form auto-resolves to the derived id, so prefer it for forward references. |
| `adoc::preprocess::stray_endif` | `endif::[]` without a matching opener. | Add the missing `ifdef::name[]` / `ifndef::name[]` / `ifeval::[expr]`, or delete the stray endif. |
| `adoc::preprocess::include_cycle` | `include::` chain points back at itself. | Refactor so each file appears at most once in the chain. |
| `adoc::preprocess::secure_mode` | `include::` while `:safe-mode: secure`. | The author can't override; flag this to the human reviewer. |
| `adoc::preprocess::absolute_include` / `include_path` | Include path escapes safe-mode bounds. | Use a relative path under `--base-dir`. |

## Authoring conventions

### Sections

* Document title is `= Title` once, at the top.
* Sections are `==`, `===`, `====`, `=====` (levels 1–4 supported,
  rendered as `<h2>`–`<h5>`).
* A section gets an auto-derived id from its title (lowercase,
  non-alphanumeric → `_`). Use `[#id]` *above* the section line to set
  an explicit id when you'll cross-reference it.

### Cross-references

Three forms, all valid:

* `<<id>>` — shortest; uses the id verbatim as link text.
* `xref:id[Custom Label]` — explicit label.
* `<<Section Title>>` — title-text resolution; rewrites to the
  derived id at parse time. Prefer this for forward references where
  you don't want to bother writing `[#id]`.

### Inline formatting

* Constrained: `*strong*`, `_emphasis_`, `` `monospace` ``.
* Unconstrained mid-word: `**bo**ld`, `__it__alic`, `` ``mo``no ``.
* Smart quotes: `"`text`"` and `'`text`'` render curly. Apostrophes
  in contractions (`it's`, `don't`) stay literal.
* Highlight: `#text#` or `##text##` mid-word.
* Subscript / superscript: `H~2~O`, `E=mc^2^`.

### The `<<id>>`-in-backticks footgun

`` `<<intro>>` `` does *not* render as literal `<<intro>>` styled in
monospace. Backticks apply every substitution (including macros), so
that becomes a clickable xref styled as monospace. To keep the
literal text, wrap the inner part in a constrained passthrough:

```adoc
The `+<<intro>>+` shorthand is the xref form.
```

### Tables

Always set `cols=` explicitly when you have more than two columns —
without it, the column count is inferred from the first row, which
can mis-group cells when rows aren't uniform:

```adoc
[cols="1,2,1"]
|===
| Name | Description | Type
| age  | Years lived | int
| ...
|===
```

For machine-readable data, use CSV/DSV form:

```adoc
[%header,format=csv]
|===
Name,Age,Role
Alice,30,Engineer
"Bob, with comma",25,Designer
|===
```

### Source blocks

Always tag the language so highlighters can pick it up:

```adoc
[source,rust]
----
fn main() { println!("hi"); }
----
```

For step-by-step code annotated with explanations, use callouts:

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

### Includes (skeleton-and-fill)

When generating a multi-section document, write a thin skeleton with
`include::` directives and fill the leaves separately. Combine with
tag wildcards so a partial regen of one leaf doesn't re-emit
human-edited prose:

```adoc
include::generated/intro.adoc[tags=ai-generated]
```

In the leaf:

```adoc
// tag::ai-generated[]
This paragraph is regenerable.
// end::ai-generated[]

This paragraph below the close marker is human-edited.
```

The build command `adoc skeleton.adoc --safe-mode safe --base-dir
generated/` will reject any `include::` that escapes the directory —
so a hallucinated `include::../../etc/passwd[]` is denied at the
preprocessor.

## What `adoc` does *not* yet support

Avoid these constructs; they'll produce literal-text fallback or
diagnostics:

* `:doctype: book` level-0 part syntax (`= Part Title` mid-document).
* `stem:[]`, `latexmath`, `asciimath` — math notation.
* `include::` `encoding=` non-UTF-8 (the value parses but is ignored;
  files are always read as UTF-8).
* Inline `((…))` (single-paren visible index term) — only the
  invisible triple-paren `(((primary, secondary)))` is supported.

If you encounter a construct that's mentioned in the AsciiDoc spec
but not in this list and not in `README.md` *What works today*, render
it as plain prose rather than guessing — the parser is permissive but
will not magically infer unsupported syntax.

## Build-time helpers worth knowing

These commands turn the same parser into a tooling target. Use them
when the task is "produce structured data about a document" rather
than "render a document":

```bash
# JSON Schema for the AST — feed to LLM structured-output modes
adoc --emit-ast-schema > schema.json

# Round-trip via the AST — manipulate the tree, then render
adoc input.adoc --emit-ast | jq '...' | adoc --from-ast -o out.html

# Block-level retrieval chunks
adoc input.adoc --emit-chunks --quiet > chunks.json
```

The chunks output is one JSON entry per leaf block with
`section_path`, `section_title`, plain-text `text`, and a SHA-256
content `hash`. Two re-runs over unchanged source produce identical
hashes; an edit to a single block changes only that block's hash.
