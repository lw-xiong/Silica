---
title: Vault files and formats
description: Every file kind Silica recognizes, what lives in .obsidian and which parts Silica reads, the JSON Canvas and Bases formats, forbidden file names, and what Silica writes.
---

# Vault files and formats

## File kinds

Silica classifies every file in the vault by extension:

| Kind | Extensions |
|---|---|
| Note | `.md` |
| Image | `.bmp`, `.png`, `.jpg`, `.jpeg`, `.gif`, `.svg`, `.webp`, `.avif` |
| Audio | `.mp3`, `.wav`, `.m4a`, `.3gp`, `.flac`, `.ogg`, `.oga`, `.opus` |
| Video | `.mp4`, `.webm`, `.ogv`, `.mov`, `.mkv` |
| PDF | `.pdf` |
| Canvas | `.canvas` |
| Base | `.base` |
| Unknown | everything else |

Unknown kinds are hidden in the [Files view](/docs/app/files/) unless
`showUnsupportedFiles` is on, and there is nothing to open them with either way.

Only notes are parsed, indexed and searched. The rest are tracked as files — so they appear
in Files, can be embedded, and are counted as link targets.

## File names

Obsidian forbids a set of characters in a file name, and Silica validates the same set:

```
*  "  \  /  <  >  :  |  ?
```

A name may also not begin with a dot. Vault-relative paths use `/` and keep their original
case; comparisons between them are case-insensitive.

## The `.obsidian` folder

Every file is optional, every key within it is optional, and **keys Silica does not
recognize are carried through untouched**.

| File | Silica |
|---|---|
| `app.json` | Reads and writes — the editor and files settings |
| `appearance.json` | Reads and writes — base theme, accent color, fonts, font size |
| `hotkeys.json` | Reads and writes — your key bindings, rebindable from the Hotkeys tab |
| `core-plugins.json` | Reads and writes — both the modern map and the legacy array shape |
| `community-plugins.json` | Reads and writes the list, and reads each plugin's `manifest.json` |
| `types.json` | Reads and writes — the property types |
| `bookmarks.json` | Reads and writes — the Bookmarks pane |
| `workspace.json` | Reads and writes — the tab-group tree, the panes' leaf state, `lastOpenFiles` |
| `graph.json` | Reads — filters, color groups, display settings, forces. Not written back |
| `switcher.json` | Reads and writes |
| `command-palette.json` | Reads — pinned commands |
| `themes/`, `snippets/` | Not applied (tier 3) |
| `plugins/` | Manifests read; plugin code not executed |

A write merges the one changed key into the file's own JSON rather than re-serialising it,
so unknown keys and the file's own order survive. The two exceptions are `hotkeys.json` and
`bookmarks.json`, written whole because *removing* an entry is what restores a default
there — and each entry Silica did not touch goes back key for key, unknown keys included.
See [settings](/docs/settings/).

Which `app.json` keys are read, and which of them actually change behavior, is the table in
[settings](/docs/settings/).

## `graph.json`

Read in full enough detail to reproduce your graph:

| Group | Keys |
|---|---|
| Filters | `search`, `showTags`, `showAttachments`, `hideUnresolved`, `showOrphans` |
| Color groups | `colorGroups`, each `{query, color: {a, rgb}}` |
| Display and force | Node size, link thickness, text fade, centre, repel, link force, link distance |
| View | `scale`, `close` |

`collapse-*` keys round-trip.

## JSON Canvas

`.canvas` files are [JSON Canvas 1.0](https://jsoncanvas.org). Silica implements the format:
its fields, z-order by array order, and tab-indented output when writing. A canvas
round-trips through Silica unchanged.

A `.canvas` **opens** in a canvas view of its own in the main area: it pans, zooms, selects,
moves, resizes, draws edges, colors, groups, copies, pastes and undoes, cards are typed
into, and it autosaves through the same writer — the bytes Obsidian writes on an edited
save. The menus, controls and commands are Obsidian's. An embed of a canvas inside a note is
still not drawn.

## Bases

`.base` files are Obsidian's Bases format, YAML. Silica implements the top-level keys and the
recursive filter structure, and round-trips a base file unchanged. There is no base view
yet: opening a `.base` says the kind is not supported.

## What Silica writes, completely

The whole list:

1. **A note you edited** — on autosave, `Ctrl + S`, navigation, mode change or close. Its
   byte-order mark and line endings are restored as found.
2. **Notes whose links need rewriting** after you rename or move a file, with
   `alwaysUpdateLinks` on. Only the link bytes change.
3. **Files you create, copy or delete.**
4. **A `.trashinfo` record and a moved file**, when deleting with `trashOption: system` on
   Linux, or a file moved into `.trash/` with `local`.
5. **A canvas you edited** — on its own autosave, through the JSON Canvas writer above.
6. **One key of one file under `.obsidian/`**, when you change something that lives there: a
   Settings control writes into `app.json`, `appearance.json`, `switcher.json`,
   `core-plugins.json` or `community-plugins.json`; a rebinding writes `hotkeys.json`; a
   property type writes `types.json`; a bookmark writes `bookmarks.json`. A config write
   merges into the file's own JSON, so unknown keys stay exactly where they are.
7. **`.obsidian/workspace.json`**, on the way out and only if the session changed the
   layout: the `main` tree of tab groups, each pane's own leaf state and `lastOpenFiles`,
   with every other key carried through.

Nothing else. No cache, no index, no sidecar, no lock file.

## Not a format: Sync and Publish

Obsidian Sync and Obsidian Publish are services, not file formats, and Silica has nothing to
do with either. A vault synced by Obsidian Sync is still a folder, so Silica opens it; the
syncing is Obsidian's doing and Silica neither helps nor hinders it.

## Next

- [How Silica works](/docs/reference/architecture/)
- [Glossary](/docs/reference/glossary/)
