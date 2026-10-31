---
title: The window
description: A tour of Silica's window — the left dock with Files and Search, the note in the middle, the right dock with Backlinks, Outgoing links, Outline and Tags, the status bar, and how navigation works.
---

# The window

Silica opens one window on one vault, in a three-part shell: a dock on the left, the open
note in the middle, a dock on the right. Both docks can be hidden, which leaves the note
alone on screen.

```
┌─────────────┬────────────────────────────────┬──────────────┐
│ Files       │ ▸ Note.md  ▸ Other.md          │ Backlinks    │
│ Search      │          the open note         │ Outgoing     │
│ Bookmarks   │   (reading, Live Preview or    │ Tags         │
│  ← left     │          Source mode)          │ Properties   │
│    dock     │                                │ Outline   →  │
├─────────────┴────────────────────────────────┴──────────────┤
│ vault                              412 words, 2310 characters│
└─────────────────────────────────────────────────────────────┘
```

## The left dock

Three tabs:

- **[Files](/docs/app/files/)** — the folder tree of the vault. The tab Silica opens on.
- **[Search](/docs/app/search/)** — vault-wide search with the full query grammar.
- **Bookmarks** — the tree in your vault's `bookmarks.json`: files, folders, headings,
  blocks, searches and groups, read and written in Obsidian's bytes.

`Ctrl + Shift + F` opens Search; `file-explorer:open` returns to Files and
`bookmarks:open` to Bookmarks. `Ctrl + E` is not involved — toggling the dock itself is
`app:toggle-left-sidebar`.

## The note

The middle is one note, in one of three modes:

| Mode | What it is | Switch with |
|---|---|---|
| Reading view | The finished note, rendered | `Ctrl + E` (`markdown:toggle-preview`) |
| Live Preview | Editing, with syntax hidden unless the cursor is in it | `Ctrl + E` from reading view |
| Source mode | Editing, with every marker visible | `editor:toggle-source` |

Which one a note opens in follows your vault's settings: `defaultViewMode` decides reading
view or editing, and `livePreview` decides which of the two editing modes. See [editing a
note](/docs/app/editing/).

The middle is a tab strip over one or more tab groups. `Ctrl + T` opens a tab,
`workspace:split-vertical` (Split right) and `workspace:split-horizontal` (Split down)
divide the area into groups, the dividers between groups drag, and a tab drags between
groups or onto a group's edge to make a new one. `Ctrl + Tab` and `Ctrl + 1`…`8` walk the
tabs, and `editor:focus-left` and its three siblings move between groups. The `main`
tree is written back to `workspace.json` in the shape Obsidian writes it.

> **Note** Silica still opens its own default layout rather than restoring the one it
> saved. Popout windows do not exist here.

## The right dock

Five tabs, in Obsidian's order. Three are about the open note; Tags and All properties are
about the whole vault:

- **[Backlinks](/docs/app/panes/)** — notes that link here, and unlinked mentions of this
  note's name or aliases.
- **[Outgoing links](/docs/app/panes/)** — where this note points, resolved and unresolved.
- **[Tags](/docs/app/panes/)** — every tag in the vault with its count.
- **[All properties](/docs/app/panes/)** — every property key in the vault with its type and
  count.
- **[Outline](/docs/app/panes/)** — this note's headings; click one to scroll to it.

`app:toggle-right-sidebar` hides and shows the dock. Each tab also has a command of its own
(`backlink:open`, `outgoing-links:open`, `tag-pane:open`, `properties:open`,
`outline:open`), so you can bind a key straight to the pane you want.

## The status bar

Along the bottom: the vault's status on the left, and the open note's word and character
count on the right. While you are editing, the right-hand side also carries the cursor's
line and column, the number of cursors when there is more than one, and the parser and
shaping timings for the note you are in:

```
Ln 212, Col 7  ·  parse 18.4 ms, 9120 spans  ·  shape 0.21 ms  ·  412 words, 2310 characters
```

Those timings are there on purpose rather than left in a debug build: the editor reparses
the note in the background after each edit, and this is where you can see that it is keeping
up. See [performance](/docs/reference/performance/).

## Navigating

| To | Do |
|---|---|
| Open a note | Click it in Files, or click a link in a note |
| Find a note by name | `Ctrl + O` — the [quick switcher](/docs/app/switcher/) |
| Run anything by name | `Ctrl + P` — the [command palette](/docs/app/switcher/) |
| Go back / forward | `Ctrl + Alt + ←` / `Ctrl + Alt + →` |
| Jump to a heading | Click it in Outline, or follow a `[[Note#Heading]]` link |
| See the vault's shape | `Ctrl + G` — the [graph](/docs/app/graph/) |
| Open Settings | `Ctrl + ,` |

Clicking a link to a note that does not exist creates it — the same as Obsidian, and the
same as pressing Enter in the quick switcher on a name nothing matches. See [managing
notes](/docs/app/manage/).

History is per window and linear: opening a note from the graph, from a link, from Files or
from the switcher all push onto the same back stack.

## Keys

Every shortcut in Silica is an Obsidian command bound from its default hotkey, so your
vault's `hotkeys.json` rebinds or unbinds all of them. The one exception is `Ctrl + Q`,
which quits.

The 127 commands Silica can actually run are the ones the command palette offers; the rest
are still listed in Settings' Hotkeys tab, so you can see your bindings for commands that
do not do anything yet. The full list is in [commands and
hotkeys](/docs/reference/commands/).

## The theme

Colors come from your vault's `appearance.json` — the base theme (dark or light) and the
accent color — and are read before the window opens, so the first frame is already right.
A CSS theme (`cssTheme`) is not applied; that is a later phase.

Fonts come from `appearance.json` too: the interface font, the text font and the monospace
font, with the font size.

> **Note** The default text and interface font is Liberation Sans rather than something more
> modern, because the pinned GPUI cannot pick weights out of a variable font — with a
> single-file variable font, nothing renders bold. Set `textFontFamily` and
> `interfaceFontFamily` in `appearance.json` to any font with real static Bold faces and it
> is honored.

## Next

- [The Files view](/docs/app/files/)
- [Reading view](/docs/app/reading-view/)
