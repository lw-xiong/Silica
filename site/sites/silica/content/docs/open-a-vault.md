---
title: Open a vault
description: What a vault is to Silica, what happens the first time you open one, which configuration files are read, and what Silica does and does not write.
---

# Open a vault

A vault is a folder. That is the whole definition: Silica takes a path, scans the folder
below it, and opens. There is no vault file to create, no registration step and no vault
picker — the path is the argument.

```shell
silica ~/notes
```

If the folder has an `.obsidian` directory, Silica reads your settings out of it. If it does
not, Silica uses Obsidian's defaults and still opens the folder fine. Either way, **opening
a vault writes nothing**.

## What happens on open

1. **The configuration is read first.** `appearance.json` and `hotkeys.json` are read before
   the window is created, so the first frame already has the right colors and the right key
   bindings rather than flashing a default theme at you.
2. **The folder is scanned** on a background thread: every file below the root, skipping
   dot-folders. Each file is classified by extension into a kind — Markdown note, image,
   audio, video, PDF, Canvas, Base, or unknown.
3. **Every note is read and indexed.** The Markdown is parsed once, and from that one parse
   come the note's links, embeds, tags, headings, sections, list items, blocks and
   frontmatter. Then every link in the vault is resolved to a file, which is what makes
   backlinks, the graph and unresolved-link detection possible.
4. **The window opens** on the three-pane shell, with the Files tree on the left.

A 20,000-file vault indexes cold in under two seconds on the reference machine — that is one
of the project's gates, not an aspiration. See [performance](/docs/reference/performance/).

## What gets read from `.obsidian`

Every file is optional, and so is every key in it. A key Silica does not recognize is kept
and carried through rather than dropped.

| File | What Silica takes from it |
|---|---|
| `app.json` | `defaultViewMode`, `livePreview`, `strictLineBreaks`, `newLinkFormat`, `useMarkdownLinks`, `newFileLocation`, `newFileFolderPath`, `alwaysUpdateLinks`, `trashOption`, `promptDelete`, `showUnsupportedFiles`, `useTab`, `tabSize`, `smartIndentList`, `attachmentFolderPath`, `userIgnoreFilters`, and the rest of the editor and files settings |
| `appearance.json` | Base theme, accent color, text and interface fonts, font size |
| `hotkeys.json` | Your key bindings, replacing the defaults command by command |
| `core-plugins.json` | Which core plugins are on (both the modern map and the legacy array shape) |
| `community-plugins.json` | The list, and each plugin's manifest — shown in Settings, not run |
| `graph.json` | Graph filters, color groups, display settings and forces |
| `switcher.json` | Quick switcher options |
| `command-palette.json` | Pinned commands |
| `workspace.json` | The `main` tree of tab groups, the panes' own leaf state and `lastOpenFiles`, read at startup and written back in place on the way out |

## What Silica writes

No note at all while you are only reading. The whole list:

- **The note you edited**, and only that one. Autosave fires two seconds after your last
  keystroke, on `Ctrl + S`, when you open another note, when you switch to reading view, and
  when the window closes.
- **Notes whose links need rewriting**, when you rename or move a file and
  `alwaysUpdateLinks` is on. See [managing notes](/docs/app/manage/).
- **Files created or deleted** by you, through the Files menu or the commands.
- **A canvas you edited**, on its own autosave, in the bytes Obsidian writes.
- **`.obsidian/workspace.json`**, on the way out and only if the session changed the layout.
- **Files in `.obsidian`**, when you change something that lives in one: a Settings control
  writes its own key into `app.json`, `appearance.json`, `switcher.json`,
  `core-plugins.json` or `community-plugins.json`; a rebinding writes `hotkeys.json`; a
  property type writes `types.json`; a bookmark writes `bookmarks.json`; and the tab-group
  tree and the panes' own state write `workspace.json`. A config write merges into the
  file's own JSON, so only the key you changed moves and keys Silica does not recognise are
  left exactly where they were.

Nothing else. Silica writes no cache, index or sidecar file of its own, because it has
none.

On save, a file's byte-order mark and its line endings are restored as they were found. A
note that was CRLF with a BOM is written back CRLF with a BOM.

## Files changed underneath you

Silica watches the vault folder. A burst of writes — a git checkout, a sync client landing a
batch — is collected for 300 ms and applied as one update:

- Added, modified, removed and moved files re-index in place.
- When files appear or vanish, every link in the vault is re-resolved, because a link
  elsewhere may now resolve or stop resolving.
- The side panes, the search results and the graph refresh.
- An open note reloads, keeping its scroll position.

If you have unsaved edits in a note that changed on disk, your edits are left alone rather
than being overwritten by the file. And a reload is skipped entirely when the file on disk
is what Silica itself last wrote.

## Several vaults

One Silica process opens one vault. To work in two, run it twice. `app:open-vault` and
`app:open-another-vault` exist as command ids — your `hotkeys.json` bindings for them are
honored as bindings — but switching vaults from inside the window is not implemented; the
command is not offered in the palette.

## Next

- [Coming from Obsidian](/docs/from-obsidian/) — if the vault is one you use daily.
- [The window](/docs/app/window/) — what you are looking at.
