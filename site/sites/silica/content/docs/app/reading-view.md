---
title: Reading view
description: How Silica renders a finished note — every construct it draws, how tables lay out their columns, how embeds and footnotes resolve, and what is not drawn yet.
---

# Reading view

Reading view is the note as a finished document: no markers, no cursor, links you click.
`Ctrl + E` (`markdown:toggle-preview`) toggles between it and editing.

Whether a note opens here depends on your vault: `defaultViewMode` in `app.json` is either
`preview` (reading view) or editing, and Silica follows it.

## What is rendered

Everything in this list is drawn from the same parse tree the editor and the metadata cache
use, so what you read and what the cache believes cannot disagree.

| | |
|---|---|
| Headings | Levels 1–6, scaled |
| Paragraphs | With soft line breaks — a single newline is a `<br>` unless `strictLineBreaks` is on |
| Emphasis | `*italic*`, `**bold**`, `~~strike~~`, `==highlight==` |
| Inline code | With the monospace font |
| Links | `[[Wikilinks]]`, Markdown links, and bare URLs — all clickable |
| Tags | Clickable, and counted in the Tags pane |
| Images | Local files, with Obsidian's size hints: `![[pic.png\|200x100]]` |
| Embeds | Whole notes, `#Heading` sections and `#^block` references, nested up to four deep |
| Lists | Bulleted, numbered, and task items with checkboxes |
| Quotes | A bar per nesting level |
| Callouts | With Obsidian's type colors and their fold state |
| Tables | With column alignment, laid out as described below |
| Code blocks | With a language label; long lines scroll rather than wrap |
| Frontmatter | As a properties block at the top of the note |
| Footnotes | Numbered by order of first reference, collected at the end |

<figure class="shot">
  <img src="/assets/img/app/media.gif" width="1000" height="667" loading="lazy" decoding="async"
       alt="Notes with embedded photographs and SVG charts, an image file opened in its own tab, a canvas zoomed in from the whole diagram to its text cards, note embeds and image nodes joined by labelled edges, then the graph view settling.">
  <figcaption>
    Photographs and SVG charts embedded with <code>![[file|width]]</code>, an image opened as
    its own tab, a <code>.canvas</code> with text cards, note cards and image nodes joined by
    labelled edges, and the graph view laying the vault out.
  </figcaption>
</figure>

## Clicking things

- **A wikilink or Markdown link to a note** opens it.
- **`[[Note#Heading]]`** opens the note scrolled to that heading.
- **`[[Note#^block-id]]`** opens it scrolled to that block.
- **A tag** searches the vault for it.
- **A footnote reference** jumps to the definition.
- **A task checkbox** toggles it. The edit takes the same path a Live Preview click takes —
  one undo step, written back by the autosave — and the view rebuilds from that buffer.
- **A code block's copy button** copies the block and shows a check for a moment.
- **A link inside an embedded note** works, and opens the note it points at — not the note
  the embed is in.

## How tables lay out

Obsidian renders a table as an HTML table, which means the browser's automatic table layout
decides the columns. Silica reproduces that with the real font metrics:

1. Each column *wants* the width of its widest cell.
2. Each column *needs* at least the width of its longest unbreakable word.
3. If the sum of the wishes fits the note's width, every column gets its wish.
4. Otherwise the slack between each column's minimum and its wish is shared out in
   proportion.

The practical consequence, and the reason it was worth implementing rather than giving every
column an equal share: a table with one long identifier in it does not break that identifier
across lines, and a narrow column does not get the same width as a wide one.

Fenced code blocks never wrap. A long line scrolls sideways inside its own block.

## Long notes

The view lays out one item per top-level block, measured lazily, so only what is on screen is
ever laid out. A 50,000-line note opens and scrolls like a short one.

## Not yet

The gaps, honestly — none of these are in reading view today:

| Not drawn | What you see instead |
|---|---|
| Math (`$x$`, `$$x$$`) | The source text, unrendered |
| HTML in a note | The tags as literal text |
| Remote images (`https://…`) | Nothing; local images work |
| Bases (`.base`) | The format round-trips on disk, but there is no base view. A `.canvas` does open, in a canvas view of its own |
| Heading folding | Headings do not collapse |
| Editing a property | The properties table is read-only here; edit it in [Live Preview](/docs/app/editing/) |
| PDF export | — |

Frontmatter edge cases match Obsidian: a `<script>` in a note is removed, unknown HTML
elements are unwrapped, and a raw `<a>` gains `target` and `rel`.

## Next

- [Editing a note](/docs/app/editing/) — Live Preview and Source mode.
- [Basic formatting](/docs/format/basic/) — the syntax reference starts here.
