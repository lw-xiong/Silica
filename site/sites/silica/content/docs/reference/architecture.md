---
title: How Silica works
description: Silica's internals — the crates and what each owns, the one-parser rule and why it matters, the metadata cache, how the reading view and editor share a renderer, and the clean-room method.
---

# How Silica works

Useful if you are filing a bug, reading the source, or deciding how much to trust the thing.

## The crates

| Crate | Owns |
|---|---|
| `silica` | The GPUI application: window, workspace shell, every view |
| `silica-vault` | A vault is a folder — scanning, file kinds, `.obsidian` config reads, file operations |
| `silica-markdown` | Obsidian Flavored Markdown: CommonMark + GFM + Obsidian syntax into one spanned tree, and the HTML renderer |
| `silica-cache` | The metadata cache and link resolution with Obsidian's rules |
| `silica-formats` | On-disk formats: file kinds, `app.json` and friends, JSON Canvas 1.0, Bases YAML |
| `silica-commands` | The 244 command ids, names and default hotkeys |
| `silica-search` | The search query grammar, the matcher, and fuzzy search |
| `silica-bench` | Benchmarks: the synthetic vault generator and the index timer |

## The one-parser rule

**There is exactly one Markdown parser, and every node in its tree carries its byte span.**
The editor's syntax highlighting, Live Preview's display transform, the reading view, the
metadata cache and the link rewriter are all built from that one tree.

This is the single most consequential decision in the project, and it is enforced as a
project rule rather than a preference: adding a second tokenizer for any of those is not
allowed.

What it buys:

- **What you read and what the cache believes cannot drift.** A link the reading view draws
  is the same link the backlinks pane counted, because they came from the same parse.
- **Rewriting is surgical.** A rename knows the byte range of each link that needs changing,
  so it can replace those bytes and leave the rest of the file alone — which is how
  frontmatter links get updated without re-serialising your YAML.
- **One place to be right.** Obsidian's Markdown has a lot of corners
  ([basic formatting](/docs/format/basic/) lists a dozen), and each one is implemented once.

The parser follows CommonMark's two-phase design: a line-by-line block pass with a stack of
open containers — lazy continuation, tab stops, list-item indentation — then an inline pass
per paragraph, heading, callout title and table cell, using a delimiter stack for emphasis
and a bracket stack for links. Obsidian's constructs are extra block starts and inline
scanners in the *same* two passes, each behind a flag, so the CommonMark suite can run with
them all switched off and still pass 652 of 652.

## The metadata cache

`silica-cache` is the equivalent of Obsidian's `CachedMetadata`. Per note it records links,
embeds, tags, headings, sections with Obsidian's type names, list items with Obsidian's
`parent` encoding, blocks, footnotes, and frontmatter links with their key paths.

Vault-wide it keeps `resolvedLinks` and `unresolvedLinks` counts per source, backlinks per
occurrence, tags merged case-insensitively across inline and frontmatter, and aliases.

[Link resolution](/docs/format/links/) lives here, including the ambiguous-basename order
that a probe against a running Obsidian had to settle.

## Rendering, and why both views share it

The reading view turns the parse tree into an owned render tree — styled runs with clickable
ranges, resolved embeds, footnotes in reference order, frontmatter as properties — and draws
it with a lazily measured list, one item per top-level block.

Live Preview is **the same editor and buffer** as Source mode, with a per-line display
transform: from the line's highlight spans it hides markers or substitutes glyphs and returns
display text plus a piece map between source and display offsets. The editor shapes the
display text and maps clicks, the cursor, the selection and IME bounds back through the
pieces. Source mode is the identity transform.

Block widgets — a top-level table, math block, or embed-only paragraph — are rendered by the
*reading view's* renderer, laid out as a child element inside the editor's element. That is
why an embedded note looks the same while you are writing around it as it does when you
finish.

## Threads

- **Scanning and indexing** run on a background thread at startup, so the window appears
  before the vault is finished.
- **Parsing after an edit** runs on a background thread, one parse in flight and one queued,
  with results swapped in when ready. Typing never blocks on the parser.
- **Searching** runs on the background executor with rayon across the indexed note text.
- **The graph's force simulation** steps on a background thread.
- **File-watcher events** are drained every 300 ms, so a burst of writes is one batch.

## The clean-room method

No code, CSS or icons come from Obsidian's bundle. The behavior was written down first, as a
functional specification derived from observing a real Obsidian, its file formats and its
public documentation. The code implements the specification.

Where the specification and Obsidian disagree, the specification is what gets corrected —
and the way that happens is a throwaway plugin in a scratch vault that dumps what Obsidian
actually does. That is how the command ids, names and default hotkeys were captured, how the
settings tabs were enumerated, and how the link-rewriting and link-resolution rules were
established. The "same folder first" rule for ambiguous basenames was *wrong* in the
specification, and a probe is what showed it.

## The ratchets

Two test suites are not allowed to go down:

- `tests/commonmark.rs` — 652 of 652 CommonMark 0.31.2 examples, byte-identical HTML.
- `tests/corpus.rs` — a vault of 1,000 generated notes plus nine hand-written edge cases,
  against the HTML Obsidian 1.13.7 produced for each. Both sides are reduced to a canonical
  token stream — no copy buttons, icons, MathJax output or `dir` attributes — before
  comparison. 1,007 of 1,009 match.

A third, `rename_corpus.rs`, renames every note in the corpus under every link format, moves
its folders, and renames its attachments, asserting each time that every resolved link still
resolves to the same file, that no link appears or disappears, and that nothing outside the
links changed.

When Obsidian and the parser disagree on something new, the procedure is: add a probe note to
the corpus vault's `edge` folder, re-render it with the plugin in Obsidian, match the observed
behavior, and write it down in the specification.

## GPUI

The interface is [GPUI](https://gpui.rs), the GPU-accelerated toolkit built for the Zed
editor. The editor and the reading view are custom GPUI elements that shape and paint only
what is visible.

`gpui` and `gpui_platform` are git dependencies pinned to a Zed release commit, because the
crates.io release is stale and crashes on NVIDIA. The pin is bumped deliberately, one release
at a time, and the app is run on a real GPU before the bump is committed.

## Where the source is

[github.com/ragtux/silica](https://github.com/ragtux/silica) and
[git.ragtux.com/ragtux/silica](https://git.ragtux.com/ragtux/silica). The specification and
the development notes live in `docs/silica`, which is itself an Obsidian vault — dogfooded by
Silica, so anything the two apps disagree about in it is a bug or a specification gap.

## Next

- [Performance](/docs/reference/performance/)
- [What differs from Obsidian](/docs/reference/differences/)
