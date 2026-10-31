---
title: Tables, code and math
description: Tables and the pipe-escaping rule, fenced and inline code, math blocks, and %%comments%% — what Silica parses, what it renders, and what shows as source.
---

# Tables, code and math

## Tables

```markdown
| Option | Default | What it does      |
|--------|:-------:|------------------:|
| `a`    |   on    | something         |
| `b`    |   off   | something else    |
```

Alignment comes from the delimiter row: `:---` left, `:---:` centre, `---:` right.

**A `|` inside a code span does not split a cell.** This is the GitHub rule and Obsidian
follows it, so you can write `` `a|b` `` in a cell. To write a literal pipe outside code,
escape it: `\|`. That is also why a wikilink alias inside a table is written
`[[Note\|alias]]`.

Three structural rules decide whether you have a table at all:

- The header row must be a **one-line paragraph** followed immediately by the delimiter row.
- The header row **or** the delimiter row must contain a pipe.
- A table **cannot interrupt a longer paragraph**, and a row line with no pipe ends the
  table.

### How columns are sized

In reading view, columns follow HTML's automatic table layout, measured with the real font:
each column wants its widest cell, needs at least its longest unbreakable word, and when the
total does not fit, the slack is shared out in proportion. See [reading
view](/docs/app/reading-view/).

### Editing a table

In Live Preview a table stays a grid wherever the cursor is, the way Obsidian's table editor
does. Click a cell to open a field in it: `Tab` and `Shift + Tab` walk the cells, `Enter`
drops into the cell below, `Escape` abandons, and the column grows with what is typed.
Committing a cell replaces that cell's bytes and nothing else — ordinary typing re-pads
nothing, which is what Obsidian does too. A `|` typed into a cell is written `\|` and a
newline becomes a space, because either would otherwise restructure the table. In Source
mode you edit the pipes directly.

Obsidian's sixteen table commands are all here — add, move, duplicate and delete rows and
columns, set a column's alignment, insert a table — and, like Obsidian's, they are offered
only while the cursor is in a table. Each **rewrites the whole table**: every cell padded to
its column's width, outer pipes added, short rows filled out, the delimiter grown to match.
That is Obsidian's behavior, byte for byte, and one undo step takes it back.

> **Note** The table under the cursor comes from the one Markdown parser rather than from
> hunting for pipes, so a `|` inside a code span splits a cell here exactly where it splits
> one in reading view. One consequence: `` | `a|b` | **bold** | `` is a paragraph by GFM's
> rules, and Silica leaves it as prose where Obsidian's table editor would rewrite it into a
> three-column table.

## Code

Inline code is backticks: `` `like this` ``. A backtick run with no closer retries one
backtick shorter, so a stray triple backtick does not swallow the rest of the line.

Fenced blocks take a language label:

````markdown
```rust
fn main() {
    println!("hello");
}
```
````

The label is shown on the block in reading view. **Code never wraps** — a long line scrolls
sideways inside its own block, which is almost always what you want for code and is what
Obsidian does.

Nothing inside a code span or a fenced block is parsed as anything else: no wikilinks, no
tags, no math, and a rename never rewrites a link that appears in code.

## Math

```markdown
Inline: $e^{i\pi} = -1$

Block:

$$
\int_0^1 x^2 \, dx = \tfrac{1}{3}
$$
```

Math is typeset natively, with no browser and no JavaScript: a KaTeX-compatible engine
written in Rust lays the formula out, and Silica draws it in the text's colour and size with
KaTeX's own fonts. Inline math sits on the line's baseline inside the sentence; a `$$` block is
centred and, when it is wider than the note, scrolls sideways. In Live Preview the formula is
rendered until the cursor enters it, when its source and dollars come back, exactly as the
other inline markup behaves. `\textcolor` and `\color` keep their colours; everything else
takes the theme's text colour.

> **Note** The syntax is **KaTeX's**, not MathJax's. Nearly everything written in a note works
> identically — fractions, roots, sums and integrals with limits, matrices and the `aligned`,
> `cases` and `array` environments, Greek, `\mathbb` and friends, `\text`, stretchy
> delimiters, accents, `\ce` chemistry. What MathJax accepts and KaTeX does not — a few
> AMS extensions, `\newcommand` across formulas, `\require` — shows the formula's **source
> in red**, which is also what a formula with a syntax error looks like, in Obsidian too.

The delimiter rules are Obsidian's, and they are stricter than you might expect:

- `$...$` needs **no space** just inside the dollars, and the closing `$` must not be
  followed by a digit. That is what stops `$5 and $7` from becoming math.
- An unterminated `$$` block runs to the **end of the file**.
- `$$x$$` opened and closed on one line stays **inline** inside its paragraph.

## Comments

```markdown
Visible text %%hidden%% more visible text.

%%
A whole block that does not render.
%%
```

`%%…%%` hides text in both reading view and Live Preview.

- An **inline** `%%comment%%` does **not** cross a line break.
- An unterminated `%%` block runs to the end of the file, like `$$`.
- `%%x%%` closed on one line stays inline.

## Next

- [Settings](/docs/settings/)
