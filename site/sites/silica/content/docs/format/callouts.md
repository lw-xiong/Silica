---
title: Callouts
description: Obsidian callouts in Silica — the syntax, the built-in types and their aliases, folding, titles, and the parsing details that decide whether you get a callout or an ordinary quote.
---

# Callouts

A callout is a block quote whose first line declares a type. Silica renders them with
Obsidian's type colors and honors their fold state.

```markdown
> [!note]
> An ordinary note.

> [!warning] A title of its own
> The text after the first line is the body.

> [!tip]- Folded by default
> This content starts collapsed.

> [!info]+ Expanded by default
> This content starts open, and can be collapsed.
```

## The syntax, precisely

The first quoted line must be `[!type]`, optionally followed by `-` or `+` and then
optionally a title:

| | |
|---|---|
| `> [!note]` | Not foldable |
| `> [!note]-` | Foldable, starts **collapsed** |
| `> [!note]+` | Foldable, starts **expanded** |
| `> [!note] Title` | The rest of the line is the title |

With no title, the type's own name is used as the title.

Three details decide whether you actually get a callout, and they are Obsidian's:

- **The type is trimmed**, so `> [! bad]` is a callout of type `bad`.
- **`> [!tip]Title`, with no space**, is *not* a callout — it is an ordinary block quote.
- An id can be attached to a callout by putting ` ^id` at the end of its header line.

## The built-in types

Each has a color and an icon slot in Obsidian, and several have aliases that canonicalise to
the same type:

| Type | Aliases |
|---|---|
| `note` | |
| `abstract` | `summary`, `tldr` |
| `info` | |
| `todo` | |
| `tip` | `hint`, `important` |
| `success` | `check`, `done` |
| `question` | `help`, `faq` |
| `warning` | `caution`, `attention` |
| `failure` | `fail`, `missing` |
| `danger` | `error` |
| `bug` | |
| `example` | |
| `quote` | `cite` |

A type Silica does not recognize still renders as a callout, with the default appearance and
the type name as its title — the same as Obsidian, and the reason a custom callout type from
a theme or snippet degrades gracefully rather than vanishing.

## While editing

In Live Preview, a callout's `[!type]` marker is folded into its title, so the callout reads
as a callout while you write it. Put the cursor in the header line and the source returns.

## Next

- [Properties and tags](/docs/format/properties/)
