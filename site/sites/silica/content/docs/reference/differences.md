---
title: What differs from Obsidian
description: The honest list — everything Obsidian does that Silica does not do yet, the places Silica deliberately behaves differently, and what is permanently out of scope.
---

# What differs from Obsidian

Three kinds of difference, kept apart because they mean different things: **not yet**,
**deliberately different**, and **not ever**.

If you are deciding whether Silica can be your daily editor, this is the page to read. It is
written against 0.3.2; the [release notes](/releases/) say what each version moved out of the
first list.

## Not yet

Things Obsidian does that Silica intends to do and does not.

### Rendering

| | |
|---|---|
| **Math** | Typeset with a KaTeX-compatible engine rather than MathJax: the common syntax renders alike, a MathJax-only command shows its source in red. No `\newcommand` across formulas, no `\require` |
| **HTML in notes** | Parsed, shown as literal text |
| **Remote images** | `![](https://…)` does not load. Local images do |
| **Mermaid diagrams** | Not rendered |
| **PDF viewing and export** | A PDF opens in a viewer with Obsidian's toolbar, sidebar and find bar, and `![[x.pdf]]` embeds it; the text cannot be selected or copied, and there is no export |
| **Audio and video players** | Recognized as file kinds; no player |
| **Heading folding** | Headings do not collapse |

### Views

| | |
|---|---|
| **Bases** | The Bases YAML format is implemented and round-trips; there is no base view. Canvas has one |
| **A canvas embedded in a note** | `![[Board.canvas]]` is not drawn; the file opens in a view of its own |
| **Popout windows** | — |
| **Stacked tabs** | Tabs and split groups are in; stacking them is not |
| **Multiple vaults in one window** | Run Silica once per vault |
| **Local graph depth beyond 1**, arrows, timelapse, screenshot export | See [the graph](/docs/app/graph/) |
| **Local graph as a side pane** | The graph is a full view |

### Editing

| | |
|---|---|
| **File recovery** | Obsidian keeps its snapshots in IndexedDB outside the vault, so it is out of the round-trip work rather than next in it |
| **Editing properties in reading view** | The reading view's properties table is read-only; Live Preview's is not |
| **Vim mode** | `vimMode` is read, not implemented |
| **Line numbers, indent guides, inline title, right-to-left** | Read and shown in Settings, not acted on |
| **Spellcheck** | — |
| **`Ctrl + hover` page previews** | — |

### Files and settings

| | |
|---|---|
| **Writing `graph.json`** | Graph forces and filters are read, not written back |
| **Drag and drop in Files** | Use `file-explorer:move-file` |
| **Inline title editing** | Use `F2` |
| **Pasting or dropping attachments** | Put the file in the folder yourself |
| **System trash on Windows and macOS** | Implemented on Linux (freedesktop trash with a `.trashinfo` record); elsewhere it falls back to the vault's `.trash/` |
| **Themes (`cssTheme`)** | Base theme, accent color and fonts are honored; a CSS theme is not. Tier 3 |
| **A few Settings controls** | Most rows write. The font families, spellcheck languages, excluded files, accent color and the theme list do not, because the control Obsidian offers for them is one Silica has not built |
| **Search result context menu** | Sorting, copying, collapsing and the keyboard are in; a result has no right-click menu yet. See [search](/docs/app/search/) |
| **A context menu on a side-pane row** | The Files view's is the only one in the app so far |
| **Vault-wide search and replace** | Replace works within one note |

### Markdown edge cases

Two of the 1,009 corpus notes do not match Obsidian's output, on three constructs: nested
delimiter runs such as `****a****`, `==== four ==x===y==`, and a wikilink inside Markdown
link text (which Obsidian splits the link on). See [compatibility](/docs/reference/compatibility/).

## Deliberately different

Places where Silica does not copy Obsidian, on purpose.

**Frontmatter links are rewritten in place.** When a rename changes a link in your YAML,
Obsidian re-serialises the whole frontmatter block — which can turn `aliases: [Bee]` into a
block list, changing bytes you did not touch. Silica replaces the link text and leaves every
other byte alone. This is a difference in Silica's favor and it is not going to be
"fixed".

**The command palette lists only runnable commands.** This matches how Obsidian hides a
command whose check callback fails, but the effect is more visible here because more commands
are unimplemented. The [Hotkeys tab](/docs/hotkeys/) still lists all 244 ids so your
bindings are visible.

**The default font is Liberation Sans.** Not a taste decision: the pinned GPUI cannot select
weights from a variable font, so with a single-file variable font nothing renders bold.
Liberation Sans ships static Bold faces. Your `appearance.json` fonts override it.

## Not ever

**Obsidian's community plugins will not run in Silica.** They are JavaScript against an
Electron app's API, with Node and DOM access. Silica is a native binary with neither.
Embedding a JavaScript runtime and a DOM to host them would rebuild the thing whose absence
is the entire point of the project.

What is planned instead is a **native plugin API of Silica's own** (tier 4, the last phase).
Your `community-plugins.json` and plugin manifests are read and listed in Settings, so you can
see what your vault has — they are simply not executed.

**Obsidian Sync, Publish and the commercial services** are Obsidian's products, not file
formats. Silica has nothing to do with them. Use any file-level sync — git, Syncthing,
Dropbox, or Obsidian Sync itself: they all move files, and files are all Silica needs.

## Keeping this page honest

This list is maintained against `docs/silica/Development.md` in the repository, which carries
the per-item status and the gate notes. If you find something missing from it — a difference
you hit that is not written here — that is worth
[reporting](https://github.com/ragtux/silica/issues) in its own right.

## Next

- [Obsidian compatibility](/docs/reference/compatibility/) — the tiers and the numbers.
- [Troubleshooting](/docs/reference/troubleshooting/)
