---
title: Editing a note
description: Silica's editor — Live Preview and Source mode, what each hides and shows, smart lists, multiple cursors, undo grouping, and exactly when and what Silica saves.
---

# Editing a note

Silica has one editor with two faces. Both are the same buffer and the same parser; they
differ in how much of the Markdown they show you.

| Mode | Markers | Switch with |
|---|---|---|
| **Live Preview** | Hidden, unless the cursor or selection touches the construct | `editor:toggle-source` |
| **Source mode** | All visible, always | `editor:toggle-source` |

`Ctrl + E` (`markdown:toggle-preview`) switches between editing and [reading
view](/docs/app/reading-view/). Which editing mode you get is your vault's `livePreview`
setting; which of editing or reading a note opens in is `defaultViewMode`.

<figure class="shot">
  <img src="/assets/img/app/editing.gif" width="1000" height="667" loading="lazy" decoding="async"
       alt="Typing a task at the end of a note in Live Preview, two square brackets opening the completion popup which narrows as letters are typed, the accepted link written, find highlighting a word, then Split right and a second note opened beside the first.">
  <figcaption>
    Live Preview: a task typed at the end of a note, <code>[[</code> opening the completion
    popup and the accepted row writing the link, find highlighting every hit, then Split right
    and a second note beside the first.
  </figcaption>
</figure>

## Live Preview

The rule is Obsidian's: a construct renders unless your cursor is in it, and then it shows
its source so you can edit it.

- **Headings** are scaled by level; the `#` markers appear when you enter the line.
- **Wikilinks** show their alias, or their target when they have no alias. Markdown links
  show their text.
- **Tasks** render as checkboxes, and a click toggles one.
- **Quotes** show a bar per level; **callouts** fold the `[!type]` into the title.
- **Comments** (`%%…%%`) and **block ids** (`^id`) are hidden.
- **Inline math** is typeset in place, on the line's baseline; the source and its dollar
  signs return while the cursor is inside it (see [tables, code and
  math](/docs/format/code-math/)).
- **Emphasis, strong, strike and highlight** drop their delimiters.

### Block widgets

A top-level table, math block, or a paragraph that is nothing but an embed is rendered in
place of its source lines, through the same renderer reading view uses — so an embedded note
appears inline while you write around it. The widget shows its source again when your cursor
enters the block: with the arrow keys, or by clicking a table or math widget, which puts the
cursor at the row you clicked.

Links inside an embed widget are clickable, and follow.

### Clicking links while editing

In Live Preview, a click on a rendered link follows it. In Source mode, `Ctrl + click` does
— a plain click puts the cursor there, which is what you want when the markers are showing.

## Source mode

Every character of the file, with syntax highlighting and nothing hidden. This is the mode
for fixing a mangled table, seeing exactly where a stray `%%` is, or editing a note whose
formatting is itself the subject.

Decorations come from the parser, recomputed on a background thread after each edit — one
parse in flight and one queued — and swapped in when ready. Typing never waits for the
parser, which is why a very large note still types smoothly. The status bar shows the
current parse time and span count.

## Soft wrap and the readable line length

Lines wrap at the view width, capped at the readable line length and centred, the way
Obsidian does with `readableLineLength` on.

The scroll position is a line plus a pixel offset into that line, rather than an absolute
pixel count. That is what makes a 50,000-line note cheap: wrapped-row heights are only ever
measured near the viewport, so no full layout pass is needed to know where you are. Page Up,
Page Down, revealing the cursor and clamping at the end of the note all walk rows from that
anchor.

## Selection and movement

| | |
|---|---|
| Extend a selection | Shift with any movement key, or drag |
| Select a word | Double-click |
| Select a line | Triple-click |
| Select everything | `Ctrl + A` |
| Move or delete by word | `Ctrl + ←` / `→`, `Ctrl + Backspace` / `Delete` |
| Copy, cut, paste | `Ctrl + C`, `Ctrl + X`, `Ctrl + V` — cut and copy take the whole line when nothing is selected |

## Smart lists

With `smartIndentList` on (the default), pressing Enter continues what the line started:

- Its quote markers (`>`, `> >`).
- Its bullet, or the next number in an ordered list — and the siblings after it renumber.
- An empty checkbox, after a task item.

`Shift + Enter` is always a plain newline, whatever the line started. An empty item is
cleared rather than continued; an empty *nested* item outdents instead.
`Tab` and `Shift + Tab` indent and outdent list items, using a tab or `tabSize` spaces
depending on `useTab`, and any ordered items this unsettles renumber: an item follows its
previous sibling at its own depth, and one with no previous sibling restarts at 1.

## Auto-pairing

`autoPairBrackets` brings a closer along with `(`, `[`, `{`, `"` and `'`; `autoPairMarkdown`
does the same for the backtick, `*` and `_`. Typing the closer where one is already waiting
steps over it instead of doubling it, a `Backspace` between a fresh pair takes both away, and
with text selected the pair wraps the selection — `~`, `=`, `$` and `%` only ever wrap. Both
toggles default to on, and changing one in [Settings](/docs/settings/) takes effect on the
next keystroke without reopening the note.

Every rule here is one that was observed against Obsidian. The characters and positions the
probe left open are deliberately inert: pairing writes bytes you did not type, so a guessed
rule would leave a stray `)` in your prose and the vault would keep it.

## Autocompletion

`[[` and `![[` offer the vault's files, their aliases, their headings and their block
references; `#` offers its tags. The list narrows as you type and the rows carry the file's
folder, so two notes with the same name are told apart.

<figure class="shot">
  <img src="/assets/img/app/autocomplete.png" width="1600" height="1067" loading="lazy" decoding="async"
       alt="Typing two square brackets in Live Preview opens a completion popup listing matching notes.">
  <figcaption>
    The popup as <code>[[</code> opens it, listing the vault's notes, images and diagrams
    before a letter narrows it.
  </figcaption>
</figure>

Three keys accept a row, and the difference between them is the point:

| Key | What it writes |
|---|---|
| `Enter` | The whole link, cursor after it |
| `Tab` | The same link with the cursor left inside the brackets, so a `#heading` can follow |
| `Shift + Enter` | The query as you typed it, ignoring the highlighted row |

`↑` and `↓` — or `Ctrl + N` and `Ctrl + P` — move through the list, and `Escape` closes it.
The link itself is written in your vault's own style: `newLinkFormat` decides how much of the
path appears, and `useMarkdownLinks` decides whether you get `[[wiki]]` or `[markdown](…)`.

## Multiple cursors

| To add a cursor | |
|---|---|
| Above or below | `editor:add-cursor-above`, `editor:add-cursor-below` |
| At every match of a search | *Find all* — `Alt + Enter` in the [find panel](/docs/app/find/) |
| Anywhere | `Alt + click` (again on an existing cursor removes it) |

Typing, deleting, Enter, Tab, cut, copy and paste all apply at every cursor as a single undo
step. Copy joins the cursors' selections with newlines; paste gives one line to each cursor
when the counts match, and otherwise pastes the whole clipboard at each. Movement moves
every cursor, and selections that come to overlap merge. `Escape`, or a plain click, keeps
only the primary cursor.

## Undo

Undo and redo group the way CodeMirror's do: consecutive typing, or consecutive deleting,
within half a second of itself is one step. A *Replace all* is one step however many matches
it changed, and so is an edit made at twenty cursors at once.

## Saving

Silica saves the note you are editing, and only that note.

| Saves on | |
|---|---|
| Idle | Two seconds after your last keystroke |
| `Ctrl + S` | `editor:save-file` |
| Navigation | When another note opens |
| Mode change | When you switch to reading view (which then rebuilds from the saved text) |
| Close | When the window closes |

**What a save writes.** The buffer holds `\n` internally. The file's byte-order mark and its
line endings are remembered when it was loaded and restored when it is written, so a note
that was CRLF-with-a-BOM is written back CRLF-with-a-BOM, and a save changes nothing but the
bytes you actually edited.

**When the file changes underneath you.** If the note on disk is what Silica itself last
wrote, the watcher's reload is skipped. If the note changed underneath an unedited buffer,
it reloads and keeps your cursor. If you have unsaved edits, they are left alone — Silica
will not throw away your typing to take the disk's version.

## Not yet

- **Vim mode** (`vimMode` is read, not implemented).
- **File recovery.** Obsidian keeps its snapshots outside the vault, so this is deliberately
  out of the round-trip work rather than next in it.
- **`Ctrl + click` page previews**, heading folding, indent guides, line numbers.

## Next

- [Find and replace](/docs/app/find/)
- [Managing notes](/docs/app/manage/)
