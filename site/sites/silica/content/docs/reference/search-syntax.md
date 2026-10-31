---
title: Search syntax
description: The complete query grammar Silica's vault search implements — boolean logic and precedence, every field operator, the scoped operators, regular expressions, and property queries.
---

# Search syntax

Silica implements Obsidian's search grammar. This page is the whole of it.

Queries run from the [Search tab](/docs/app/search/) (`Ctrl + Shift + F`), and the same
grammar drives the [graph](/docs/app/graph/)'s `search` filter and its color groups.

## Terms and logic

| Query | Means |
|---|---|
| `focus` | Notes containing "focus" |
| `focus stacking` | Both terms — **AND** is implicit |
| `focus OR stacking` | Either term. `OR` must be **uppercase** |
| `-stacking` | **Not** containing "stacking" |
| `"focus stacking"` | That exact phrase |
| `(a OR b) c` | Grouping with parentheses |

**Precedence:** the implicit AND binds **tighter** than `OR`. So `a b OR c` is
`(a AND b) OR c`, not `a AND (b OR c)`. This is the single most common surprise in the
grammar, and the reason the *explain* output is worth looking at.

Inside a quoted phrase, `\"` is a literal quote.

Matching is **case-insensitive** by default, overridden per term with `match-case:` or
globally undone with `ignore-case:`.

## Field operators

Each operator applies to **one term or one parenthesised group**:

| Operator | Matches against |
|---|---|
| `file:` | The file's name |
| `path:` | The file's full vault path |
| `content:` | The note's body text only |
| `tag:` | A tag on the note |
| `match-case:` | The term, case-sensitively |
| `ignore-case:` | The term, case-insensitively |

```
file:Specification            notes whose name contains "Specification"
path:archive/                 notes under archive/
content:TODO                  "TODO" in the body, not in the file name
tag:#project                  tagged #project, or any child of it
match-case:API                "API" but not "api"
file:(spec OR draft)          an operator over a group
-path:archive/                everything outside archive/
```

> **Note** `tag:#project` matches `#project` **and** `#project/silica` — a parent tag matches
> its children. To match only the exact tag, there is no narrower form; search the child out
> with `-tag:#project/silica`.

## Scoped operators

These are where the grammar earns its keep. A plain `a b` means both terms are *somewhere in
the note*; a scoped operator means both are in the *same* line, block, section or task.

| Operator | Scope |
|---|---|
| `line:(a b)` | Both terms on one line |
| `block:(a b)` | Both in one block (paragraph, list item, table, code block) |
| `section:(a b)` | Both in one section — between two headings |
| `task:(a b)` | Both in one task item, of any status |
| `task-todo:(a b)` | Both in one **unfinished** task |
| `task-done:(a b)` | Both in one **finished** task |

```
line:(error timeout)             a line mentioning both
section:(install nixos)          one section covering both
task-todo:(review)               an open task about reviewing
task-done:(deploy)               a finished task about deploying
```

`task:` with no terms is not a thing — give it a term or a group.

## Regular expressions

```
/colou?r/            a regex
/^## /               lines starting with a heading marker
/\bERR-\d{4}\b/      a word-bounded code
/foo/i               with flags
```

A regex is delimited by `/`, with flags after the closing slash. It can be combined with
everything else: `path:docs/ /\bTODO\b/`.

## Property queries

Square brackets query frontmatter [properties](/docs/format/properties/):

| Form | Matches |
|---|---|
| `[status]` | Notes that **have** a `status` property |
| `[status:done]` | `status` equal to `done` |
| `[status:null]` | `status` present but empty |
| `[done:true]` | A boolean `true` |
| `[priority:>3]` | A numeric comparison |

```
[status:done] -tag:#archive        finished, not archived
[priority:>3] path:projects/       high priority projects
```

## Explaining a query

Silica can print the parsed query as a tree, with the same labels Obsidian's *Explain search
terms* panel uses. When a query returns something unexpected, this is the fastest way to see
why — usually that `OR` grabbed more than you meant.

## From the command line

`--search` opens Silica with the Search tab filled and the query run, which is useful for
scripting and for screenshots:

```shell
silica ~/notes --search 'tag:#project -path:archive/'
```

Quote it in your shell: `#` starts a comment in most of them.

## Not yet

- Sort options and copying results — see [search](/docs/app/search/).
- Saved searches.
- Searching inside PDFs or other attachments. Search covers note text.

## Next

- [Obsidian compatibility](/docs/reference/compatibility/)
