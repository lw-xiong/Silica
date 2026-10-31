---
title: Search
description: Vault-wide search in Silica — the query grammar, how results are grouped and read, the header's controls, keyboard navigation, explaining a query, and what is not implemented yet.
---

# Search

`Ctrl + Shift + F` (`global-search:open`) opens the Search tab in the left dock. It searches
every note in the vault, with the same query language Obsidian uses.

## A query

The simplest query is a word, and several words mean all of them:

```
focus stacking          both words, anywhere in the note
"focus stacking"        that exact phrase
focus OR stacking       either one
focus -stacking         "focus" but not "stacking"
tag:#project            notes tagged #project, or any child of it
path:archive/           notes under archive/
/colou?r/i              a regular expression
```

The full grammar — every operator, how precedence works, the `[property]` forms — is the
[search syntax reference](/docs/reference/search-syntax/). It is worth a read: the scoped
operators (`line:`, `block:`, `section:`, `task:`) are where the grammar earns its keep,
because they let you ask for two words *in the same line* rather than merely in the same
note.

<figure class="shot">
  <img src="/assets/img/app/search.gif" width="1000" height="667" loading="lazy" decoding="async"
       alt="The quick switcher matching a note as its name is typed, then the Search pane filling with grouped, highlighted results as a query is typed, a result opening at its match, then the Tags and Backlinks panes.">
  <figcaption>
    The quick switcher's fuzzy match, then Search grouping hits by note with every match
    highlighted and opening one at its line, the Tags pane with nested tags and counts, and a
    note's backlinks.
  </figcaption>
</figure>

## Reading the results

<figure class="shot">
  <img src="/assets/img/app/search.png" width="1600" height="1067" loading="lazy" decoding="async"
       alt="The Search pane with a one-word query, showing results grouped by note with the matches highlighted.">
  <figcaption>
    One query, grouped by note with a count per file and every match highlighted in its line.
  </figcaption>
</figure>

Results are grouped per file: a row with the file's name and its match count, and under it a
card of the matching lines with the matched text highlighted. Click a line and the note opens
at it; click the file row, or its chevron, to fold the card. A long line wraps. Under the
pointer a line shows two chevrons that add a line of context above or below it; **Show more
context** in the settings does that for every line at once.

The header has the same controls as Obsidian's:

- **`Aa`** inside the field — *Match case*. Matching is case-insensitive unless this is on or
  the query says `match-case:`.
- **The clear button** inside the field.
- **Search settings** (the sliders) — *Collapse results*, *Show more context* and *Explain
  search terms*.
- **The result count** — click it for *Copy search results* (a modal with Show path, Link
  style and List prefix, the same 24 forms Obsidian writes) and *Bookmark...*, which saves
  the query as a bookmark.
- **The sort order** at the right — file name A to Z or Z to A, modified time or created time
  either way. Changing it re-sorts the list in place.

Up and Down walk the rows from the field, Enter opens the focused one, Left and Right fold and
unfold its file, and Escape puts the keyboard back in the note. The query and the settings
are saved in `workspace.json` under the search leaf, with Obsidian's keys, so they come back
after a restart.

Searching runs on a background thread across the indexed note text, so a large vault stays
responsive while you type.

## Explaining a query

A query that returns the wrong thing is usually a precedence surprise — `OR` binds looser
than the implicit `AND`, so `a b OR c` is `(a AND b) OR c`. Turn on *Explain search terms* in
the search settings and the pane prints the parse tree above the results, with the same
labels Obsidian shows, so you can see how your query was actually read.

## Searching for a tag by clicking

Clicking a tag in a note, or in the [Tags pane](/docs/app/panes/), searches the vault for
it.

## Not yet

- **Search and replace across the vault.** Replace works inside one note — see [find and
  replace](/docs/app/find/).
- **A right-click menu on a result.**
- The sort order and the copy modal's selects are menus rather than native drop-downs, and
  the modal's preview is read-only where Obsidian's is a text area you can edit before
  copying.

## Next

- [Search syntax](/docs/reference/search-syntax/) — the whole grammar.
- [Switcher and palette](/docs/app/switcher/) — for finding a note by name instead.
