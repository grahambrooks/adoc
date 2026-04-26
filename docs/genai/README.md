# Author instructions for GenAI tools

If you're shipping a project that uses `adoc` to render AsciiDoc, drop
the right file from this directory into your project so AI authoring
tools (Copilot, Claude, Codex, Cursor, aider, ...) pick up consistent,
project-specific instructions without you having to maintain four
copies.

## What's in here

| File | Where it goes | Read by |
| --- | --- | --- |
| `AGENTS.md` | Project root: `./AGENTS.md` | OpenAI Codex, Cursor (via `Project Rules`), aider, increasingly the cross-vendor convention. |
| `copilot-instructions.md` | `.github/copilot-instructions.md` | GitHub Copilot Chat / inline. |
| `claude-skill/` | `.claude/skills/asciidoc-author/` (or `~/.claude/skills/` for personal use) | Claude Code, Claude Desktop. The folder is one Claude Skill. |
| `system-prompt.md` | Pasted into the "system" / "developer" message of any ad-hoc API call. | Any model. |

All four target the same audience — an AI author writing AsciiDoc that
will be processed by `adoc` — and say roughly the same things in the
shape each tool expects. Pick the ones whose tools you use; the others
are inert clutter you can leave on the cutting-room floor.

## Why ship them at all

Without these, an AI tool generating AsciiDoc tends to:

- Emit Asciidoctor-isms `adoc` doesn't implement (level-0 book parts,
  stem/math, custom inline macros).
- Produce dangling `<<xref>>` references because it can't see the id
  registry.
- Forget to wrap monospace passthrough around `<<id>>` examples.
- Use the wrong escape for `[label]` after a URL.

These instructions cover the working subset, the lint loop
(`adoc input.adoc --check --werror`), and the JSON-shaped feedback
that closes the loop. A model with these in its system prompt
produces clean documents on the first or second iteration; a model
without typically takes five or six.

## Updating

Treat `AGENTS.md` as the source of truth. The other files cross-link
to it. When you add a feature to `adoc` that affects authors (a new
inline macro, a new safe-mode behaviour, a new diagnostic), update
`AGENTS.md` first; the others are short pointers and rarely need
edits.

## See also

- `docs/showcase.adoc` — every feature exercised in one document.
- The repo's `README.md` *Using `adoc` in AI document pipelines*
  section — focused on the build-time / pipeline flags
  (`--emit-ast-schema`, `--emit-chunks`, `--check`, `--werror`).
- `tests/conformance/` — frozen byte-identity oracles per feature, the
  shape spec compliance ends up looking like.
