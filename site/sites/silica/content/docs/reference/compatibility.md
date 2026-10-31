---
title: Obsidian compatibility
description: Silica's four compatibility tiers, what each one promises, the conformance numbers behind tier 1, and where the project currently stands against each.
---

# Obsidian compatibility

Compatibility with Obsidian is Silica's reason to exist, so it is worth being precise about
what is promised and what is merely intended. The project uses four tiers.

| Tier | Goal | Status |
|---|---|---|
| **1** | Vault and format compatibility — no conversion, no data loss | In scope, and first |
| **2** | Core feature parity with Obsidian's app and its 35 core plugins | In scope |
| **3** | Theme compatibility | A variable-level subset of the 1,047 CSS custom properties |
| **4** | Community plugin compatibility | **Out of scope.** A new native plugin API instead |

## Tier 1: the vault and the formats

This is the tier that actually matters, because it is the one whose failure loses your work.
It means: Silica reads your vault without converting anything, and writes it without
damaging anything.

The evidence for it is two conformance suites, both **ratchets** in CI — they are not
allowed to go down:

| Suite | Result |
|---|---|
| CommonMark 0.31.2 | **652 of 652** spec examples render byte-identical HTML |
| Obsidian 1.13.7 reading view | **1,007 of 1,009** corpus notes render the same canonical HTML |

The corpus is a real 1,009-note vault rendered by Obsidian itself, kept as expected output.
A third suite renames every note in it under every link format, moves its folders, and
renames its attachments, checking each time that every resolved link still resolves to the
same file, that no link appears or disappears, and that nothing outside the links changed.

### The two notes that differ

Honesty about a ratchet means naming what it does not cover. Two corpus notes do not match,
on three constructs:

- Nested delimiter runs such as `****a****`.
- `==== four ==x===y==`.
- A wikilink inside Markdown link text, which Obsidian splits the link on.

All three are degenerate cases that no one writes on purpose. They are tracked rather than
hidden, and the suite is set at 1,007 so that a regression elsewhere cannot hide behind
them.

### What tier 1 covers beyond Markdown

| | |
|---|---|
| File kinds and extensions | Notes, images, audio, video, PDF, Canvas, Base |
| Forbidden file-name characters | `* " \ / < > : \| ?`, and no leading dot |
| Frontmatter | Line 1 only, BOM and CRLF tolerated |
| Link resolution | Extensions, `/absolute`, `./relative`, ambiguous basenames in Obsidian's order |
| The metadata cache | Links, embeds, tags, headings, sections, list items with Obsidian's `parent` encoding, blocks, footnotes, frontmatter links with key paths |
| Config files | Sparse, optional, unknown keys round-trip |
| JSON Canvas 1.0 | Fields, z-order by array order, tab-indented output |
| Bases | Top-level keys and recursive filters |
| Command ids | All 244, with Obsidian's names and default hotkeys |

## Tier 2: feature parity

The app's own features and the 35 core plugins. This is the tier currently being built,
phase by phase:

| Phase | Gate | Status |
|---|---|---|
| 1. Spike | 50k-line editor, parser conformance corpus, 2 s vault index | **Done** |
| 2. Reader | A daily user browses their real vault with no missing content | **Done** |
| 3. Writer | The vault round-trips byte-identical after a week of mixed use in both apps | **In progress** |
| 4. Parity | Core plugin parity | **Started**: the canvas view and writer, Bases and the PDF viewer are done; MathJax, the themes subset, URI and CLI are not |
| 5. Ecosystem | Native plugin SDK | Not started |

Note the shape of those gates: each is a claim about a person using the program, not a
checklist of features. The phase-2 gate was checked by opening every note of a real vault
and reading it, which is how two fidelity bugs — equal-width table columns and wrapping code
blocks — were found and fixed.

What is in and out today is [what differs from
Obsidian](/docs/reference/differences/).

## Tier 3: themes

Obsidian themes are CSS over 1,047 documented custom properties. Silica is not a browser, so
a theme's CSS cannot simply be applied. The plan is a **variable-level subset**: implement the
custom properties that carry the visual identity — colors, fonts, sizes, radii — and read a
theme's values for them.

Today: `appearance.json`'s base theme, accent color and fonts are honored. `cssTheme` is not.

## Tier 4: community plugins

**Obsidian's community plugins cannot run in Silica, and making them run is not a goal.**

They are JavaScript modules written against an Electron application's API, with access to
Node and the DOM. Silica is a native binary with no JavaScript runtime and no DOM. Embedding
one to host them would mean rebuilding the thing whose absence is the point.

Instead, tier 4 is a **native plugin API of Silica's own** — the last planned phase. The
35 core plugins are being implemented as part of the application rather than as plugins.

Your `community-plugins.json` and your plugins' manifests are still read and shown in
[Settings](/docs/settings/), so the tab tells you truthfully what your vault has installed.

## Next

- [What differs from Obsidian](/docs/reference/differences/) — the current gap list.
- [How Silica works](/docs/reference/architecture/)
