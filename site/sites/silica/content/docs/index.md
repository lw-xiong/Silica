---
title: Manual contents
description: The Silica manual — installing it, opening a vault, the panes and views, every piece of Markdown syntax it understands, the search grammar, the settings it reads, and reference material on Obsidian compatibility.
---

# The Silica manual

Silica is a native knowledge base that opens an [Obsidian](https://obsidian.md) vault
unchanged. A vault is a folder of Markdown files; Silica reads that folder, reads the
`.obsidian` configuration beside it, and opens. Nothing is imported and nothing is moved.

This manual covers what Silica does today. Where something is not implemented yet it says
so in place, rather than describing Obsidian's behavior and leaving you to find out.

> **Note** Silica is an independent implementation and is not affiliated with Obsidian.
> Obsidian is a trademark of Dynalist Inc.

> **Warning** This is version 0.3.2. Reading a vault is well covered; writing one is the
> half being built now. Keep a backup of any vault you open with it, and see
> [what differs from Obsidian](/docs/reference/differences/) for the current gaps.

## Getting started

| Page | What it covers |
|---|---|
| [What Silica is](/docs/what-silica-is/) | What it is, what it is not, and who it is for |
| [Install and run](/docs/install/) | Getting a binary, the command line, NixOS |
| [Open a vault](/docs/open-a-vault/) | What a vault is, what happens on first open, what gets read |
| [Coming from Obsidian](/docs/from-obsidian/) | What carries over, what to expect, how to try it safely |

## Using Silica

| Page | What it covers |
|---|---|
| [The window](/docs/app/window/) | The three panes, tabs, history, the ribbon, what is where |
| [The Files view](/docs/app/files/) | The file tree, the header's sort orders, the keyboard, bookmarks |
| [Reading view](/docs/app/reading-view/) | How a finished note is displayed, and what is not drawn yet |
| [Editing a note](/docs/app/editing/) | Live Preview and Source mode, smart lists, auto-pairing, saving, undo |
| [Find and replace](/docs/app/find/) | The in-note search panel, regex, multiple cursors |
| [Backlinks and the side panes](/docs/app/panes/) | Backlinks, unlinked mentions, Outgoing links, Tags, All properties, Outline |
| [Search](/docs/app/search/) | Searching the whole vault, reading the results, explain |
| [Switcher and palette](/docs/app/switcher/) | The quick switcher and the command palette |
| [The graph view](/docs/app/graph/) | The global and local graph, filters, color groups, forces |
| [Managing notes](/docs/app/manage/) | New, rename, move, copy, delete — and the link rewriting |

## Writing notes

Silica understands Obsidian Flavored Markdown: CommonMark, the GitHub extensions Obsidian
uses, and Obsidian's own syntax. These pages are the syntax reference.

| Page | What it covers |
|---|---|
| [Basic formatting](/docs/format/basic/) | Headings, emphasis, lists, quotes, rules, line breaks |
| [Internal links](/docs/format/links/) | `[[Wikilinks]]`, Markdown links, headings, blocks, aliases |
| [Embedding files](/docs/format/embeds/) | `![[Embeds]]` of notes, sections, blocks and images |
| [Callouts](/docs/format/callouts/) | `> [!note]`, the built-in types, folding |
| [Properties and tags](/docs/format/properties/) | YAML frontmatter, the properties editor, `#tags`, aliases |
| [Block ids and footnotes](/docs/format/blocks/) | `^block-ids`, footnote definitions, inline footnotes |
| [Tables, code and math](/docs/format/code-math/) | Tables and table editing, fenced code, `$math$`, `%%comments%%` |

## Settings

| Page | What it covers |
|---|---|
| [Settings](/docs/settings/) | The tabs Silica shows, where each value comes from, which controls write |
| [Hotkeys](/docs/hotkeys/) | How `hotkeys.json` is applied, rebinding, unbinding |

## Reference

| Page | What it covers |
|---|---|
| [Search syntax](/docs/reference/search-syntax/) | Every operator in the query grammar |
| [Commands and hotkeys](/docs/reference/commands/) | The command ids, their names, and the default bindings |
| [Obsidian compatibility](/docs/reference/compatibility/) | The four tiers, and the conformance numbers |
| [What differs from Obsidian](/docs/reference/differences/) | The honest list of gaps and deliberate differences |
| [Vault files and formats](/docs/reference/formats/) | `.obsidian/`, Canvas, Bases, what Silica reads and writes |
| [How Silica works](/docs/reference/architecture/) | The crates, the one-parser rule, the cache, the renderer |
| [Performance](/docs/reference/performance/) | Measured indexing and editor numbers, and what costs time |
| [Troubleshooting](/docs/reference/troubleshooting/) | The failures people actually hit |
| [Glossary](/docs/reference/glossary/) | The vocabulary this manual uses |

## Conventions in this manual

Command ids are written as `editor:toggle-source`, because that is the string your
`hotkeys.json` uses and the thing you can search for in the command palette. Keys are
written as Obsidian labels them — `Ctrl + Shift + F` — and on macOS, Ctrl means Cmd unless
a page says otherwise. Settings are named as the Settings window names them, with the
`.obsidian` key in code beside them, because the file is often the faster way to find out
what a setting actually is.

Where a feature is unfinished, the manual says **not yet** and, where it is useful, why.
