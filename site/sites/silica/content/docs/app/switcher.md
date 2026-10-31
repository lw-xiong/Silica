---
title: Switcher and palette
description: The quick switcher for opening a note by name and the command palette for running anything by name — how each ranks what it offers, and how pinned and recent entries work.
---

# Switcher and palette

Two modal lists, both fuzzy, both opened with a key and dismissed with `Escape`.

## The quick switcher

`Ctrl + O` (`switcher:open`)

Type part of a note's name and press Enter to open it. The switcher matches against three
things, so you do not have to remember which one you know:

- **File names** — the usual case.
- **Aliases** — from the `aliases` property, so a note titled *RFC 8259* also answers to
  *JSON spec* if you said so in its frontmatter.
- **Paths** — so `arch/dec` finds `architecture/decisions.md`.

Recently opened files come first. Below the matches, **targets that do not exist yet** are
listed too — the `[[Names]]` other notes link to where no file has been created. They are
marked as not created, and pressing Enter on one **creates the note**, which is often the
quickest way to start the note you have been meaning to write.

Pressing Enter on a name nothing at all matches also creates a note with that name, in your
`newFileLocation` folder. See [managing notes](/docs/app/manage/).

### How fuzzy matching ranks

The query's characters have to appear in order, case-insensitively, but not adjacently.
Beyond that, three things raise a candidate's score:

- Matching at the **start of a word** (`qs` beats `uqs` for *Quick Start*).
- Matching **consecutive characters** rather than scattered ones.
- Being a **short candidate**, so an exact short name wins over a long one containing it.

## The command palette

`Ctrl + P` (`command-palette:open`)

Type part of a command's name and press Enter to run it. The order is Obsidian's:

1. **Pinned commands**, from your vault's `command-palette.json`.
2. **Commands you have run this session**, most recent first.
3. **Everything else**, in registration order.

Fuzzy matching is over command *names*, the ones you see — *Toggle reading view*, not
`markdown:toggle-preview`.

> **Note** The palette lists only the commands Silica can actually run — 127 of them today.
> This is the same way Obsidian hides a command whose check callback fails, and it means the
> palette never offers you something that does nothing. Every command id is still listed in
> Settings' [Hotkeys](/docs/hotkeys/) tab, so you can see and keep bindings for features that
> are not implemented yet.

The list also narrows with where you are, as Obsidian's does: 37 of those commands need an
editor on screen with the keyboard in it, the fifteen commands that act on an existing table
appear only while the cursor is in one, and `graph:open-local` is gone with no file open.

What is in the palette today: the palette and switcher themselves, Search, the graph and
local graph, toggling reading view and source mode, save, find and find-replace, the editor's
formatting and list commands, the heading commands, multiple cursors, the sixteen table
commands, the three template commands and the three daily-note ones, back and forward,
Settings, both sidebar toggles, Files and reveal-active-file, new note (three variants), new
folder, move, make a copy, rename, delete, the tab and tab-group commands, the canvas
commands, the seven bookmark commands, and the five right-dock panes.

The full list of ids, names and default keys is in [commands and
hotkeys](/docs/reference/commands/).

## Next

- [The graph view](/docs/app/graph/)
- [Managing notes](/docs/app/manage/)
