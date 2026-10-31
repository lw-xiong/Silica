---
title: Managing notes
description: Creating, renaming, moving, copying and deleting notes in Silica — and exactly how a rename rewrites the links in the rest of the vault, which links change and which are left byte for byte.
---

# Managing notes

Everything in this page writes to your vault. It is also the part of Silica whose rules were
hardest to get right, so the rules are spelled out rather than summarised.

| To | Use |
|---|---|
| New note | `Ctrl + N`, the Files menu, a click on a link nobody has created, or Enter in the switcher on a name nothing matches |
| New folder | `file-explorer:new-folder`, or the Files menu |
| Rename | `F2` (`workspace:edit-file-title`), or the Files menu |
| Move | `file-explorer:move-file` — a fuzzy folder picker |
| Make a copy | `file-explorer:duplicate-file`, or the Files menu |
| Delete | `app:delete-file`, or the Files menu |

Each opens a small prompt. The index is brought up to date before the file watcher's events
even arrive, so the Files tree, the panes and the graph are correct the moment the operation
finishes.

## Creating a note

A new note is named `Untitled`, then `Untitled 1`, `Untitled 2`, and so on. Where it lands
is your `newFileLocation` setting:

| `newFileLocation` | New notes go |
|---|---|
| `root` | At the top of the vault |
| `current` | In the folder of the note you have open |
| `folder` | In `newFileFolderPath` — falling back to the root if that folder does not exist |

A link with a folder in it creates the folders: clicking `[[reference/formats/DNG]]` where
none of that exists creates `reference/`, `reference/formats/` and the note.

Creating a note over a path that already exists fails with Obsidian's own message,
*"File already exists."*

## Renaming and moving: what happens to your links

This is the important part. When you rename or move a file, every link in the vault that
resolved to it is rewritten — and nothing else in those notes is touched.

**The new link is written in your `newLinkFormat`** — shortest path, relative path, or
absolute path — whatever form the old link had. All of `[[a]]`, `[[A.md]]`, `[[Sub/A]]`,
`[A](./A.md)` and `[A](/A.md)` become the same thing. Shortest format falls back to the full
path when a basename is ambiguous.

**A link whose written path already matches is left byte for byte.** This matters more than
it sounds: moving a note whose name is unique changes *nothing* in the notes that link to it,
because `[[Name]]` still says what it needs to say.

**The style of each link survives.** Silica rewrites the target, not the shape:

- A wikilink stays a wikilink; a Markdown link stays a Markdown link.
- An embed stays an embed.
- An angle-bracketed destination keeps its brackets.
- A Markdown destination keeps its `.md`, and encodes only spaces —
  `[x](Ä%20(b)%20100%.md)`.
- The subpath is kept: `[[Note#Heading]]` keeps `#Heading`.

**Aliases are kept verbatim**, with one exception that matches Obsidian: an alias that was
equal to the *old basename*, and is not the written path, follows the new basename. So
`[[Sub/S|S]]` becomes `[[S2|S2]]`, while `[[S|S]]` becomes `[[S2|S]]`. A Markdown link's text
follows the new basename when it *was* the old basename, or the new link path when it was the
old destination without its `.md`.

**What is never touched:**

- Code spans and fenced code blocks.
- Reference-style links and their definitions.
- `[[#Heading]]` self-links.

**A moved note's own links** are regenerated only when the move would break them — a
`[B](../B.md)` in a note that just went into a subfolder.

**Frontmatter links update.** Here Silica differs from Obsidian deliberately: Obsidian
re-serialises the whole YAML block when it rewrites a frontmatter link, which can turn
`aliases: [Bee]` into a block list. Silica replaces the link text in place and leaves the
rest of the bytes alone.

> **Note** With `alwaysUpdateLinks` off, Silica asks first — *"Update links?"* with a
> *Don't update* button — as Obsidian's dialog does. The default is on.

Renaming onto a path that exists fails with *"Destination file already exists!"*. A
case-only rename succeeds.

### How these rules were established

They were not inferred from documentation. A throwaway plugin in a scratch vault renamed
files through Obsidian's own `fileManager.renameFile` under every `newLinkFormat`, resolved
links through `getFirstLinkpathDest`, created and trashed files, and dumped the resulting
notes; what it showed is what the code implements and what its tests assert. A second suite
renames every note in a 1,009-note corpus, moves its folders under every format, and renames
its attachments, checking each time that every resolved link still resolves to the same file,
that no link appears or disappears, and that nothing outside the links changed.

The same probe corrected Silica's link-resolution order — see [internal
links](/docs/format/links/).

## Deleting

Where a deleted file goes is your `trashOption`:

| `trashOption` | Behavior |
|---|---|
| `system` (default) | The desktop trash. On Linux this is the freedesktop trash, with a proper `.trashinfo` record, so your file manager can restore it |
| `local` | The vault's own `.trash/` folder — files flat, folders whole, name collisions numbered ` 2`, ` 3` |
| `none` | Deleted outright |

With `promptDelete` on (the default) you are asked to confirm first.

> **Warning** The system trash is implemented for Linux. On Windows and macOS the vault's
> `.trash` folder is used as the fallback, so a "system trash" delete there puts the file in
> `.trash/` rather than in the Recycle Bin or the macOS Trash.

Deleting does **not** rewrite links. The links that pointed at the file simply become
unresolved, which is what Obsidian does and is almost certainly what you want — a broken
link is a reminder, a silently removed one is lost information.

## Not yet

- Drag and drop in the Files tree.
- Inline title editing (rename through `F2` or the menu).
- Pasting or dropping an attachment into the vault.
- The real system trash on Windows and macOS.

## Next

- [Internal links](/docs/format/links/) — how a link decides which file it means.
- [Settings](/docs/settings/) — where `newLinkFormat` and friends live.
