---
title: The Files view
description: Silica's file tree — the header's five buttons and six sort orders, which files are shown and hidden, revealing the open note, the keyboard, and the right-click menu for creating, renaming, moving, copying and deleting.
---

# The Files view

The Files tab in the left dock is the vault's folder tree. It is the tab Silica opens on,
and `file-explorer:open` returns to it.

## What you see

- **Folders before files.** Folders keep their name order under every sort order; only the
  files of a folder reorder. This is Obsidian's behavior.
- **Notes without their extension.** `Ideas.md` shows as *Ideas*, because the `.md` on every
  note is noise.
- **Attachments with theirs.** `diagram.png` shows as *diagram.png*.
- **Empty folders.** They exist in the vault, so they are in the tree.

Click a folder to collapse or expand it; the collapse state is kept while the window is
open. Click a file to open it.

## The header

Five icon buttons across the top of the pane, the ones Obsidian puts there:

| Button | Does |
|---|---|
| New note | `file-explorer:new-file`, which follows `newFileLocation` |
| New folder | `file-explorer:new-folder`, at the vault root |
| Change sort order | A menu of the six orders, with a check on the one in force |
| Auto-reveal current file | A toggle; it wears the accent color while on |
| Expand all / Collapse all | The label follows the tree — one folder opened by hand makes it *Collapse all* |

The six orders are Obsidian's own:

| Order | `sortOrder` |
|---|---|
| File name (A to Z) | `alphabetical` |
| File name (Z to A) | `alphabeticalReverse` |
| Modified time (new to old) | `byModifiedTime` |
| Modified time (old to new) | `byModifiedTimeReverse` |
| Created time (new to old) | `byCreatedTime` |
| Created time (old to new) | `byCreatedTimeReverse` |

Names break every tie, so the list is stable across a rescan. Created time is the
filesystem's birth time where it has one and the modified time where it does not.

The order and the auto-reveal toggle live in the file-explorer leaf of
`.obsidian/workspace.json`, never in `app.json`, and Silica reads them at startup and
writes them back in place — in Obsidian's key order, so nothing else in the file moves.

## The keyboard

`file-explorer:open` puts the keyboard in the tree where it last was, and
`file-explorer:reveal-active-file` puts it on the open file. The focused row carries a
border rather than a selection wash.

| Key | Does |
|---|---|
| `↑` / `↓` | Move over the visible rows, opening nothing |
| `Enter` | Open the row, and hand the keyboard to the note |
| `→` | Open a folder, or step into an open one |
| `←` | Climb to the parent folder and close it |
| `Escape` | Give the keyboard back to the note |
| `F2` | Rename |
| `Delete` | Delete, asking first unless `promptDelete` is off |

## What is hidden

| Hidden | Why |
|---|---|
| Dot-folders (`.obsidian`, `.git`, `.trash`) | Configuration and machinery, as in Obsidian |
| Unknown file kinds | Unless `showUnsupportedFiles` is on in `app.json` |

A "known kind" is a note, an image, an audio file, a video, a PDF, a Canvas or a Base. A
`.zip` or a `.xlsx` in your vault is an unknown kind: with `showUnsupportedFiles` off it is
not listed, with it on it is listed but clicking it does nothing — Silica has no handler to
open it with.

## Reveal the open note

`file-explorer:reveal-active-file` expands the tree down to the note you are reading and
highlights it. Opening a note from somewhere else — a link, the switcher, the graph — always
highlights its row; whether the tree also expands the folders above it and scrolls to it
depends on the header's **Auto-reveal current file** toggle. With auto-reveal off the list
stays where you put it, which is also why a write elsewhere in the vault no longer moves
it.

## The right-click menu

Right-click a file or a folder for:

| On a file | On a folder |
|---|---|
| New note | New note (in that folder) |
| New folder | New folder |
| Rename… | Rename… |
| Make a copy | — |
| Delete | Delete |

Moving is not in this menu: run `file-explorer:move-file`, which opens a fuzzy folder
picker. Each of the others is a command too, so it can have a key of its own —
`file-explorer:new-file`, `file-explorer:new-folder`, `workspace:edit-file-title` (`F2`),
`file-explorer:duplicate-file`, `app:delete-file`.

Renaming and moving rewrite the links in the rest of the vault. What exactly that means —
which links change, which are left byte for byte, what happens to aliases — is the subject
of [managing notes](/docs/app/manage/), and worth reading once before you rename anything
in a vault you care about.

## Files that change on disk

The tree follows the folder. A file added, removed, renamed or moved by anything else — git,
a sync client, Obsidian in another window — appears or disappears here within a moment,
without a refresh. A folder moved into the vault is walked and added whole; a folder removed
takes everything under it out of the index.

## Not yet

- **Drag and drop.** Run `file-explorer:move-file` instead, which offers a fuzzy folder
  picker.
- **Inline title editing.** Rename through the menu or `F2`.
- **Pasting or dropping attachments into the vault.** Put the file in the folder yourself;
  the tree will notice.
- **Inline naming of a new folder.** The header's New folder opens a prompt and creates the
  folder at the vault root.
- **A context menu on a row of the other panes.** The Files menu is the only one in the app
  so far.

## Bookmarks

Bookmarking is its own pane rather than a star on a row: the **Bookmarks** tab in the left
dock (`bookmarks:open`, shown while `core-plugins.json` leaves the plugin on) holds the tree
in your vault's `.obsidian/bookmarks.json` — files, folders, headings, blocks, searches,
graphs, URLs and groups — and the seven `bookmarks:` commands add and remove entries. An
item Silica never touched is written back key for key, so the two apps share one list.

## Next

- [Managing notes](/docs/app/manage/) — what renaming does to your links.
- [Reading view](/docs/app/reading-view/)
