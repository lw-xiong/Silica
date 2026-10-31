---
title: Basic formatting
description: The Markdown Silica understands — headings, emphasis, lists, task items, quotes, horizontal rules and line breaks — including the places where Obsidian departs from CommonMark and Silica follows it.
---

# Basic formatting

Silica parses **Obsidian Flavored Markdown**: CommonMark 0.31.2, the GitHub extensions
Obsidian uses, and Obsidian's own syntax, all in one parser. All 652 CommonMark spec examples
render byte-identical HTML, so if you know CommonMark, you know the base.

This page is the ordinary syntax. The pages after it cover
[links](/docs/format/links/), [embeds](/docs/format/embeds/),
[callouts](/docs/format/callouts/), [properties](/docs/format/properties/),
[blocks and footnotes](/docs/format/blocks/) and
[tables, code and math](/docs/format/code-math/).

## Headings

```markdown
# Heading 1
## Heading 2
###### Heading 6
```

A heading is `#` through `######` followed by a space. Headings appear in the
[Outline](/docs/app/panes/) pane and are link targets: `[[Note#Heading 2]]`.

Setext headings work too — a line of text underlined with `===` or `---`:

```markdown
Heading 1
=========
```

> **Note** A setext underline only applies to a **one-line** paragraph. After a longer
> paragraph, a `---` is a horizontal rule, not an underline. This is Obsidian's behavior and
> Silica matches it.

## Emphasis

| Syntax | Result |
|---|---|
| `*italic*` or `_italic_` | *italic* |
| `**bold**` or `__bold__` | **bold** |
| `***both***` | strong wrapping emphasis |
| `~~strikethrough~~` | ~~strikethrough~~ |
| `==highlight==` | highlighted |
| `` `code` `` | `code` |

Several details here are Obsidian's rather than CommonMark's, and Silica reproduces them:

- **`~~` and `==` need exactly two delimiters.** `~~~x~~~` and `===x===` delimit with their
  *first two* characters; the third stays inside the text.
- **A single `_` cannot close before a letter or digit.** That is what keeps
  `snake_case_name` from becoming italics halfway through, while `a_b_ ` still emphasises
  the `b`.
- **A backtick run with no closer retries one backtick shorter**, so a stray triple backtick
  in a line does not swallow the rest of it.
- **Legacy HTML entities decode without a semicolon** — `&copy` is ©.

## Lists

```markdown
- a bullet
* also a bullet
+ and this

1. numbered
2. numbered
1) also numbered

- [ ] a task
- [x] a finished task
- [-] a task with any single character in the box
```

A task item takes **any single character** between the brackets, not just a space or an `x`
— Obsidian uses that for cancelled and deferred states, and the character is preserved.

> **Note** `- [ ]` with *nothing* after the bracket is plain text, not a task.

Two indentation details differ from CommonMark, in Obsidian's favor:

- **A list marker indented four or more columns inside a list is a sibling item.**
  CommonMark would make it a lazy continuation line of the previous item.
- **A `>` line directly after a list item's text stays inside that item.**

While editing, Enter continues a list for you and `Tab` reindents it, renumbering what needs
renumbering — see [smart lists](/docs/app/editing/).

## Quotes

```markdown
> a quote
> > nested one level deeper
```

Each level draws its own bar. A quote whose first line opens with `[!type]` is a
[callout](/docs/format/callouts/) instead.

## Horizontal rules

Three or more `---`, `***` or `___` on a line of their own.

## Line breaks

A **single newline inside a paragraph renders as a line break** — Obsidian's default, which
is CommonMark's "soft break" rendered hard. Turn `strictLineBreaks` on in `app.json` and a
single newline becomes a space instead, as CommonMark specifies; Silica honors the setting
either way.

A blank line starts a new paragraph, as everywhere.

## Bare URLs and email

`https://…`, `http://…`, `www.…` and email addresses are linked wherever they appear, even
in the middle of a word. The details of where such a link stops matter more than they
should, and match Obsidian:

- A trailing `"` stays **in** the URL.
- Trailing `?`, `!`, `.`, `,`, `:`, `*`, `_`, `~` do **not**.
- An unbalanced `)` does not.
- Angle-bracket autolinks work with one-letter schemes: `<a://b>`.

## HTML in notes

Raw HTML is parsed as Obsidian parses it, but **not rendered** — the tags show as literal
text in reading view. The one place it has an effect is frontmatter sanitising, where a
`<script>` is removed, unknown elements are unwrapped, and a raw `<a>` gains `target` and
`rel`. Rendering HTML is a later phase.

## Escaping

A backslash escapes a Markdown character: `\*not italic\*`. Inside a code span, a backslash
is literal — notably `\|`, which stays `\|` in code even though it escapes a cell boundary in
a [table](/docs/format/code-math/).

## Next

- [Internal links](/docs/format/links/)
