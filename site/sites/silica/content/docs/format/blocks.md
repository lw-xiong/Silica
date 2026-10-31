---
title: Block ids and footnotes
description: Block identifiers for linking to a single paragraph, list item, table or code block, and footnotes including the inline form — with the placement rules Silica takes from Obsidian.
---

# Block ids and footnotes

## Block ids

A block id gives a single block a name, so that another note can link to exactly that
paragraph rather than to the note or a heading.

```markdown
The measurement was repeated nine times. ^nine-times
```

Then, from anywhere:

```markdown
[[Measurements#^nine-times]]        a link to that paragraph
![[Measurements#^nine-times]]       an embed of it
```

### Where an id can go

Two placements, and the second one exists because the first cannot work everywhere:

1. **At the end of a paragraph or list item**, after a space: `text ^id`.
2. **Alone on a line after a block**, for things that have no "end of the text" to append to
   — a table, a list, or a fenced code block:

```markdown
| a | b |
|---|---|
| 1 | 2 |

^the-table
```

A paragraph that is *only* `^id` attaches the id to the previous block at the same level.
That is how Obsidian ids tables and code blocks, and Silica does the same.

A ` ^id` at the end of a **callout's header line** gives the id to the callout.

Block ids are **hidden** in reading view and in Live Preview — they are machinery, not
content.

## Footnotes

```markdown
Here is a claim.[^1]

[^1]: And here is the support for it.
```

A definition can be anywhere in the note; Silica collects them and renders them at the end,
**numbered by the order of first reference** rather than by the order of definition. So the
first footnote the reader meets is 1, whatever its label.

Labels can be words rather than numbers:

```markdown
As established earlier.[^burt-adelson]

[^burt-adelson]: Burt and Adelson, 1983.
```

### Inline footnotes

```markdown
The result held.^[Though only at room temperature.]
```

An inline footnote needs no separate definition. Inline footnotes are numbered **after** the
referenced definitions, in order of appearance.

### Undefined references

A `[^missing]` with no definition renders as the bare label. It is not an error and it does
not disappear — which is usually what you want while a note is a draft.

## Next

- [Tables, code and math](/docs/format/code-math/)
