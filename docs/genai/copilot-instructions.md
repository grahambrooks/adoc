# GitHub Copilot — AsciiDoc authoring with `adoc`

When generating or editing AsciiDoc documents in this project, follow
the conventions in [`AGENTS.md`](../AGENTS.md). The headlines:

* The build tool is `adoc` — a Rust AsciiDoc processor that follows
  the AsciiDoc Language spec and **diverges from Asciidoctor where
  the spec requires it**. Don't generate Asciidoctor-isms by reflex.
* Validate with `adoc <file>.adoc --check --werror
  --diagnostic-format=json` after every batch of edits and patch the
  reported byte spans until clean.
* Use `<<Section Title>>` (auto-resolves to the derived id) for
  forward references. Use explicit `[#id]` only for anchors users
  will link to from outside.
* For literal `<<id>>` styled as monospace, wrap in a constrained
  passthrough: `` `+<<id>>+` ``. Bare backticks apply macro
  substitution.
* Always set `cols=` on multi-column tables.
* Always tag `[source,LANG]` with a language. Use `<N>` callouts +
  a colist for code-with-explanation.
* Use `include::` skeletons with the `safe` safe-mode and a
  `--base-dir` to sandbox AI-generated content.

## Unsupported constructs (don't emit)

* `:doctype: book` mid-document `= Part Title` syntax.
* `stem:[]`, `latexmath`, `asciimath`.
* Single-paren `((flow text))` index terms (only triple-paren
  invisible markers are supported).

## Diagnostics codes

| Code | Common cause |
| --- | --- |
| `adoc::xref::dangling` | Cross-reference target isn't defined. |
| `adoc::preprocess::include_cycle` | `include::` chain loops. |
| `adoc::preprocess::stray_endif` | Unmatched `endif::[]`. |
| `adoc::preprocess::absolute_include` | Include path is absolute under safe mode. |
| `adoc::preprocess::include_path` | Include path escapes `--base-dir`. |

The full set is documented in [`AGENTS.md`](../AGENTS.md). When in
doubt, defer to that file.
