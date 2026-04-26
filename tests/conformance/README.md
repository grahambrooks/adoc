# Conformance suite

This directory pins **byte-for-byte expected output** for a set of
spec features, so the CI test run catches both AST shape changes and
HTML rendering changes.

## Layout

Each entry is a directory containing three files:

| File | What it is |
| --- | --- |
| `input.adoc` | The AsciiDoc source under test. |
| `expected.ast.json` | The serialized [`adoc::ast::Document`] (pretty-printed). |
| `expected.html` | The HTML the [`Html5Converter`] produces, **with the stylesheet disabled** (`Stylesheet::None`) so diffs show content changes, not unrelated CSS edits. |

The entries are ordered with a numeric prefix (`00_paragraph`,
`01_section_basic`, …) for stable test output. The numbering is
cosmetic — pick the next free index when adding an entry.

## Running

```bash
cargo test --test conformance
```

Failures print the name of every entry that drifted plus a head/tail
preview of the expected vs. actual blob. The hint at the end of the
panic message suggests running in bless mode.

## Adding an entry

1. Create the directory and write `input.adoc`.
2. Run with the bless flag set to populate `expected.ast.json` and
   `expected.html` from the current pipeline output:
   ```bash
   ADOC_CONFORMANCE_BLESS=1 cargo test --test conformance
   ```
3. Review the generated files in your diff. They are the *frozen
   oracle* for that feature — they say "this is what we expect to
   render forever, until someone explicitly blesses a change."
4. Commit `input.adoc`, `expected.ast.json`, and `expected.html`.

## Updating an entry

When a behaviour change is intentional (e.g. you fixed a bug, added a
new wrapping `<div>`, tightened the AST shape), bless mode regenerates
all expected files at once:

```bash
ADOC_CONFORMANCE_BLESS=1 cargo test --test conformance
```

Then `git diff tests/conformance/` to see exactly which entries
changed and how. Reject anything you didn't expect.

## Conformance vs. fixtures

The interim corpus lives in `tests/fixtures/` and asserts *structural*
properties via `tests/fixtures.rs` — counts, presence of particular
tags, AST node shape. Those tests are tolerant: a small wording change
to a label leaves them green.

This conformance suite is the opposite: it asserts *byte-identity*.
The smallest change — a renamed CSS class, a different attribute order,
an extra space — flags as a failure. That's exactly the precision spec
compliance needs.

## What's covered today

The starter set spans the breadth of the spec rather than depth:

- `00_paragraph` — bare paragraphs, no header.
- `01_section_basic` — document title + nested sections.
- `02_unordered_list` — list nesting via marker depth.
- `03_listing_block` — the `----` delimiter.
- `04_inline_quotes` — every constrained / unconstrained marker.
- `05_link_macro` — `link:` with attribute substitution, bare URL with
  label, autolink, mailto.
- `06_admonition_paragraph` — the five paragraph admonition shortcuts.
- `07_source_block` — `[source,LANG]` listing with and without a
  language hint.
- `08_table_simple` — `cols=` widths + inferred header.
- `09_smart_quotes` — `"\`…\`"` and `'\`…\`'` curly quotes.
- `10_callouts` — `<N>` markers + matching colist.
- `11_xref_title` — `<<Section Title>>` resolving to derived id.

Add an entry for each new spec feature in the same change as the
feature itself. The fixture suite remains the place for assertions
about *structure*; this suite is for assertions about *bytes*.
