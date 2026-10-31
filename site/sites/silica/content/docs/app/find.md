---
title: Find and replace
description: Searching inside the open note — the find panel, match case and regex, replacement captures, replace all as one undo step, and Find all for a cursor at every match.
---

# Find and replace

Find and replace work inside the note you are editing. To search the whole vault, use
[Search](/docs/app/search/) instead.

| | |
|---|---|
| `Ctrl + F` | Open the find panel (`editor:open-search`) |
| `Ctrl + H` | Open it with the replace row (`editor:open-search-replace`) |

The panel appears at the top right of the editor. Opening it from reading view switches to
editing first. If you have a selection on a single line when you open it, that text becomes
the query.

## Finding

Matching is **literal and case-insensitive** by default, with Obsidian's two toggles on the
panel, both off to start with:

- **`Aa`** — match case, making the query case-sensitive.
- **`.*`** — regex, treating the query as a regular expression.

A regex that does not parse yields no results rather than an error, so a half-typed pattern
just shows nothing until it is valid.

Every match in the note is tinted, and the selected one more strongly; the panel shows how
many there are. Typing selects the first match from the cursor onwards.

| | |
|---|---|
| Next match | `Enter`, or the down arrow on the panel |
| Previous match | The up arrow on the panel |
| Close | `Escape` |

Next and previous wrap around the end of the note. Matches refresh after every edit, so the
count stays true while you work.

> **Note** Matching is capped at 20,000 matches in one note. A query that hits more than that
> — a single space, say — stops counting there rather than stalling.

## Replacing

With the replace row open:

- **Replace** replaces the selected match and steps to the next one.
- **Replace all** replaces every match, as **one undo step**.
- `Enter` in the replace field does the same as the Replace button.

With **Regex** on, the replacement understands captures:

| In the replacement | Means |
|---|---|
| `$&` | The whole match |
| `$1` … `$9` | The *n*th capture group |
| `$$` | A literal `$` |

## Find all

`Alt + Enter` — or the *Find all* button — selects every match in the note at once and
closes the panel. You are left with one cursor per match, which is the fastest way to rename
a variable, reformat a list, or add a prefix to twenty lines.

From there everything in [multiple cursors](/docs/app/editing/) applies: typing replaces
every selection, movement moves every cursor, and the lot is one undo step. `Escape` drops
back to a single cursor.

## Next

- [Search](/docs/app/search/) — the whole vault, with a query grammar.
- [Editing a note](/docs/app/editing/)
