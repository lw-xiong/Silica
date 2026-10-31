---
title: What Silica is
description: Silica is a native Rust knowledge base that opens Obsidian vaults unchanged — what that means in practice, what it deliberately is not, and who it is for.
---

# What Silica is

Silica is a desktop application that opens a folder of Markdown files and lets you read,
search, link and edit them. It is written in Rust, its interface is [GPUI](https://gpui.rs),
and it is free software under the MIT license.

What makes it different from the many other Markdown editors is the compatibility target:
**Silica opens an Obsidian vault exactly as it stands.** The same folder, the same
`.obsidian` configuration, the same Markdown dialect, the same `.canvas` and `.base` files,
the same `hotkeys.json`. There is no import, no export, no conversion step and no second
copy of your notes in a database somewhere.

## The promise, stated precisely

Three rules shape the whole program, and they are worth stating because they are what you
are actually relying on:

1. **The vault folder is the only source of truth.** Silica has no database, no index file
   it depends on, no sidecar metadata. Everything it knows, it knows by reading your files.
   Delete Silica and the vault is untouched.

2. **It never writes a file it did not change.** Opening a vault writes nothing. Browsing
   writes nothing. Opening Settings writes nothing — a setting is written only when you
   change it, and then Silica writes the file the way Obsidian writes it, because Obsidian
   rewrites an entire settings file on every change and that was a thing to get right
   rather than quickly.

3. **When it does write, it changes only what you changed.** A file's byte-order mark and
   its line endings are remembered at load and restored on save. Unknown keys in a JSON or
   YAML file are carried through rather than dropped. A rename rewrites the links that point
   at the renamed file and leaves every other byte of those notes alone.

## What it is not

- **It is not Obsidian, and it is not affiliated with Obsidian.** It is an independent
  implementation that reads the same files. Obsidian is a trademark of Dynalist Inc.
- **It is not a fork or a repackaging.** No code, CSS or icons come from Obsidian's bundle.
  The behavior was written down as a specification first, from observing a real Obsidian and
  reading its public documentation, and the code implements that document.
- **It is not a plugin host for Obsidian's community plugins.** Those are JavaScript written
  against an Electron application. Silica is a native binary; they cannot run in it, and
  making them run is explicitly out of scope. A native plugin interface of Silica's own is
  planned instead.
- **It is not a sync service.** There is no Silica account and no Silica server. Use git,
  Syncthing, a file-sync service, or Obsidian Sync — they all move files, and files are all
  Silica needs.
- **It is not finished.** See [what differs from Obsidian](/docs/reference/differences/).

## Why it exists

Obsidian's vault format is a good format: plain files in a plain folder, readable in any
editor, with a well-understood Markdown dialect layered on it. That format deserves more
than one implementation — because an implementation can stop being maintained, change its
terms, or simply not be the program you want, and a format with one reader is only as
durable as that reader.

The practical reasons are more ordinary. A native application starts in a fraction of a
second, scrolls a 50,000-line note inside a frame budget, holds a 20,000-note vault in a
few hundred megabytes, and does not ship a browser engine to render a text file. If you
live in your notes, that difference is felt hourly.

## Who it is for, today

At version 0.3.2, honestly:

- **Good fit now:** reading and searching a vault you maintain in Obsidian; browsing a large
  vault where a native reader is simply nicer; anyone who wants a second implementation to
  exist and is willing to file bugs against one.
- **Workable with care:** editing notes, with a backup and preferably after a round-trip
  test on a scratch copy. The editor is real — Live Preview, Source mode, find and replace,
  multiple cursors, link-rewriting renames — but it is the newest code in the project.
- **Not yet:** anyone who needs Bases, rendered math, Mermaid diagrams, PDFs, community
  plugins or CSS themes.

## Where to go next

- [Install and run](/docs/install/) — get a binary and start it.
- [Open a vault](/docs/open-a-vault/) — what happens the first time.
- [Coming from Obsidian](/docs/from-obsidian/) — what carries over, and how to try it safely.
- [Obsidian compatibility](/docs/reference/compatibility/) — the four tiers and the numbers.
