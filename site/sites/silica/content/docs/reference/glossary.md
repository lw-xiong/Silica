---
title: Glossary
description: The vocabulary this manual uses — vault, note, wikilink, embed, block id, callout, property, metadata cache, Live Preview, ratchet, round-trip, and the rest.
---

# Glossary

Terms as this manual and Silica's source use them. Most are Obsidian's words, kept
deliberately so that one vocabulary covers both applications.

**Alias** — another name for a note, declared in its `aliases`
[property](/docs/format/properties/). Resolvable by `[[alias]]`, matched by the quick
switcher, and counted in unlinked mentions.

**Attachment** — a file in the vault that is not a note: an image, audio, video or PDF.

**Backlink** — a link *to* the note you are reading, from somewhere else. Counted per
occurrence and grouped by the note it comes from, so a note linking here three times is one
group of three matches. See [the side panes](/docs/app/panes/).

**Base** — a `.base` file, Obsidian's Bases format. The format is implemented; there is no
view yet.

**Block** — the parser's unit of structure: a paragraph, a list item, a table, a code block,
a quote. A [block id](/docs/format/blocks/) names one so that it can be linked to.

**Block id** — `^an-id` attached to a block, making `[[Note#^an-id]]` possible.

**Callout** — a block quote with a declared type: `> [!warning]`. See
[callouts](/docs/format/callouts/).

**Canvas** — a `.canvas` file, [JSON Canvas 1.0](https://jsoncanvas.org). The format is
implemented and one opens in a canvas view of its own, which autosaves in Obsidian's bytes.

**Clean room** — implementing from an observed and documented specification rather than from
another program's code. No code, CSS or icons in Silica come from Obsidian's bundle. See
[how Silica works](/docs/reference/architecture/).

**Command id** — the stable string naming a command, like `editor:toggle-source`. What your
`hotkeys.json` keys on. Silica registers all 244 of Obsidian's. See [commands and
hotkeys](/docs/reference/commands/).

**CommonMark** — the Markdown specification Obsidian Flavored Markdown builds on. Silica
passes all 652 of its spec examples.

**Core plugin** — one of the 35 features Obsidian ships as a built-in plugin: the file
explorer, search, the graph, backlinks, and so on. Silica implements these as part of the
application rather than as plugins.

**Embed** — a link with a `!` in front, which renders the target in place:
`![[Note#Heading]]`. See [embedding files](/docs/format/embeds/).

**Frontmatter** — the YAML block at the very top of a note, line 1 only. Obsidian calls its
contents *properties*.

**Gate** — a condition a development phase must meet before the next begins, written as a
claim about a person rather than a feature list — "a daily user browses their real vault
with no missing content". See [compatibility](/docs/reference/compatibility/).

**GPUI** — the GPU-accelerated interface toolkit from the Zed editor, which Silica's
interface is built on.

**Linked mention** — a real link, as opposed to an unlinked mention. The first group in the
Backlinks pane.

**Live Preview** — the editing mode that hides a construct's markers unless your cursor
touches it. The same buffer as Source mode, with a display transform. See [editing a
note](/docs/app/editing/).

**Metadata cache** — what Silica knows about every note: links, embeds, tags, headings,
sections, list items, blocks, footnotes and frontmatter, plus the resolved link graph. Built
from the one parse of each note, held in memory, never written to disk.

**Note** — a `.md` file in the vault.

**Obsidian Flavored Markdown (OFM)** — CommonMark, plus the GitHub extensions Obsidian uses
(tables, strikethrough, task lists, footnotes, bare URLs), plus Obsidian's own syntax
(wikilinks, embeds, tags, block ids, callouts, highlights, comments, math).

**Property** — a key in a note's frontmatter. `aliases` and `tags` have special meaning;
everything else is yours, and searchable with `[key:value]`.

**Ratchet** — a test suite that is not allowed to score lower than it currently does. Silica
has two: the CommonMark suite at 652/652, and the Obsidian corpus at 1,007/1,009.

**Reading view** — the mode that shows a note as a finished document, with no markers and no
cursor.

**Resolved link** — a link that finds a file. An **unresolved link** is a `[[Name]]` with no
file behind it; Silica tracks these and clicking one creates the note.

**Round-trip** — opening a vault in one application, editing it, opening it in the other, and
finding nothing changed but what you changed. Byte-identical round-tripping is the gate for
Silica's writer phase.

**Section** — the span from one heading to the next heading of the same or a higher level.
What `![[Note#Heading]]` embeds, and what `section:` searches.

**Source mode** — the editing mode that shows every character of the file, with syntax
highlighting and nothing hidden.

**Span** — a byte range in the source. Every node in Silica's parse tree carries one, which
is what makes surgical rewriting possible.

**Tier** — one of the four levels of Obsidian compatibility the project aims at: vault and
format, feature parity, themes, plugins. See
[compatibility](/docs/reference/compatibility/).

**Unlinked mention** — a note's name or alias appearing as plain text where it could have
been a link. Found whole-word and case-insensitively, never inside a link or code.

**Vault** — a folder of notes. That is the entire definition; there is no vault file.

**Wikilink** — `[[Note]]`, Obsidian's link syntax. Its body grammar is
`path#heading#heading|alias` or `path#^block-id`. See [internal
links](/docs/format/links/).

## Next

- [Manual contents](/docs/)
