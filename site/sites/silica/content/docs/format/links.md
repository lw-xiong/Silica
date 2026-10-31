---
title: Internal links
description: Wikilinks and Markdown links in Silica — the full link grammar including headings, block references and aliases, and the exact rule that decides which file an ambiguous link means.
---

# Internal links

A link between notes can be written two ways, and Silica reads both.

```markdown
[[Note]]                      a wikilink
[[Note|what to show]]         with an alias
[[Note#Heading]]              to a heading
[[Note#Heading#Subheading]]   to a nested heading
[[Note#^block-id]]            to a block
[[folder/Note]]               with a path
[Text](Note.md)               a Markdown link
[Text](<Note with spaces.md>) angle brackets for awkward paths
```

When Silica **rewrites** an existing link — because you renamed or moved its target — the
link keeps the form you wrote it in, and only its path changes; `newLinkFormat` decides what
shape that path takes. `useMarkdownLinks` is read and governs the form of links Silica
*creates*. See [managing notes](/docs/app/manage/).

## The wikilink grammar

The body of a wikilink is `path#heading#heading|alias`, or `path#^block-id`. Parsed in that
order: everything before the first `|` is the target, everything after it is the alias.

A couple of rules are worth knowing because they bite:

- **`[[` needs its `]]` on the same line.** A wikilink does not span lines.
- **Wikilinks and embeds are not parsed inside code spans.** `` `[[Note]]` `` is text.
- **`[[Note\|alias]]`** — with a backslash before the pipe, as you must write it inside a
  table — is the alias form: the target is the text before the first `|`, with a trailing
  backslash dropped. Inside a code span, `\|` stays literal.

## How a link finds its file

Most links are unambiguous and this never comes up. When two notes share a basename, the
order matters, and it is not what you would guess — this was wrong in Silica's own
specification until a probe against a running Obsidian corrected it.

**Extensions.** A link may name the `.md` or leave it off.

**Paths with a prefix are not searched for:**

| Written | Resolved |
|---|---|
| `/path/Note` | From the vault root **only** |
| `./Note`, `../Note` | Relative to the linking note **only** |
| `path/Note` | Root first, then relative to the linking note |

**A bare basename**, where more than one file could match, picks in this order:

1. A file at the **vault root**.
2. A file in the **linking note's own folder**.
3. The **shortest path**.
4. Failing all that, path order.

So `[[Zed]]` written in `Long/Way/Down/C.md` resolves to `Zed.md` at the root — *not* to the
sibling `Long/Way/Down/Zed.md`. But `[[X]]` in the same note resolves to the sibling
`Long/Way/Down/X.md` over `Mid/X.md`, because neither is at the root and the sibling rule
applies next.

Comparisons are **case-insensitive**, while the stored path keeps its original case.

## Links that go nowhere

A `[[Name]]` with no file behind it is an *unresolved link*. Silica tracks these rather than
discarding them:

- They appear in [Outgoing links](/docs/app/panes/) as unresolved.
- They appear in the [graph](/docs/app/graph/), unless `hideUnresolved` is on.
- They appear in the [quick switcher](/docs/app/switcher/), marked as not created.
- **Clicking one creates the note.** With a folder in the path, the folders are created too.

## Markdown links

`[Text](destination)` is a vault link when the destination has **no URL scheme**; with a
scheme it is an external link. The destination is percent-decoded before resolution, so
`[x](My%20Note.md)` finds `My Note.md`.

Markdown links count as links in the metadata cache, which means they produce backlinks and
graph edges exactly as wikilinks do.

## Links in frontmatter

A link written in a YAML property is a real link:

```yaml
---
source: "[[Paper]]"
related:
  - "[[Other note]]"
---
```

These appear in Outgoing links, produce backlinks in the target, and draw edges in the graph.
When the target is renamed, Silica rewrites the link text in place and leaves the rest of
the YAML bytes alone — Obsidian re-serialises the whole block, which is a difference in
Silica's favor.

## Display text

| Written | Shown in reading view |
|---|---|
| `[[Note]]` | Note |
| `[[Note\|alias]]` | alias |
| `[[Note#Heading]]` | Note > Heading |
| `[[Note#Heading\|alias]]` | alias, with `Note > Heading` as its accessible label |

## Clicking a link

In reading view, a click follows it. In Live Preview, a click on a rendered link follows it;
in Source mode, `Ctrl + click` does. A `#Heading` or `#^block-id` link opens the note
scrolled to that block.

## Completing a link as you type

`[[`, `![[` and `#` open a popup over the editor. It offers the vault's files, their
aliases, a note's headings after a `#`, the vault's tags, and a note's blocks after a `#^` —
`[[Note#^` lists that note's blocks and `[[#^` the ones in the note you are editing. An
empty `[[` lists files and aliases by modification time, newest first.

Three keys accept a row, and they do different things, as in Obsidian:

| Key | Writes |
|---|---|
| `Enter` | The finished link, cursor after the `]]` (a trailing space on a tag) |
| `Tab` | The same link with the cursor left *inside*, so a `#heading` or `\|alias` can follow |
| `Shift + Enter` | The text as you typed it, ignoring the selected row |

An accepted file is reformatted to your `newLinkFormat`, and display text is added exactly
when the path part carries a `/` and the target is a note — `[[Sub/Alpha|Alpha]]`, but
`[[../pic.png]]`. A heading or block pick keeps the path as you typed it and never takes
display text. Accepting a block that has no id yet writes a six-character id into the
*target* note, in the place Obsidian writes it, after checking that file is still the bytes
the insertion was planned against.

Ranking inside a query is Silica's own fuzzy matcher, so the order can differ from
Obsidian's; what each row *writes* is what matches.

## Not yet

- `Ctrl + hover` page previews.

## Next

- [Embedding files](/docs/format/embeds/)
