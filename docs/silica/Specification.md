# Functional specification

The full specification is maintained as a living document:

- Obsidian Functional Specification (Obsidian 1.12.7, derived from observing its behavior, file
  formats, and public documentation):
  https://claude.ai/code/artifact/6bc126ce-788f-484a-8213-c6eec1f99245

Sections: purpose and evidence basis; architecture; vault and filesystem model; Obsidian Flavored
Markdown; metadata cache and link resolution; editor; workspace shell; the 35 core plugins; Canvas
and Bases formats; search query language; settings, hotkeys and theming; main-process services, IPC
and URI protocol; community plugin API; out-of-scope services; feasibility and phased plan; appendices
with the command inventory and bundle evidence.

## Invariants this repository enforces today

| Invariant | Where |
| --- | --- |
| Known file kinds and extension tables | `silica-formats::FileKind` |
| Forbidden file-name characters `* " \ / < > : \| ?` and no leading dot | `silica-formats::validate_file_name` |
| One parser for editor, reading view, cache, and link rewriting; every node carries its byte span | `silica-markdown::parse` |
| CommonMark 0.31.2: all 652 spec examples render byte-identical HTML | `silica-markdown/tests/commonmark.rs` (ratchet) |
| Obsidian 1.13.7 reading view: 1007 of 1009 corpus notes render the same canonical HTML | `silica-markdown/tests/corpus.rs` (ratchet; vault and expected HTML under `tests/corpus`) |
| GFM tables (`\|` does not split a cell, even in code), strikethrough, task items with any single character, footnotes, bare URLs | `silica-markdown::Options` |
| Frontmatter only on line 1, closed by `---` alone (YAML's `...` does not close it), CRLF tolerated; a BOM before the fence is tolerated, which Obsidian 1.14.4 does not do | `silica-markdown::extract_frontmatter` |
| Wikilink body grammar `path#heading#heading\|alias`, `#^block` | `silica-markdown::parse_linktext` |
| Wikilinks and embeds are not parsed inside code spans; `[[` needs `]]` on the same line | `silica-markdown/tests/obsidian.rs` |
| Tags need a non-numeric character; `# Heading` is not a tag; `#` after an alphanumeric is not a tag | `silica-markdown/tests/obsidian.rs` |
| Block ids: ` ^id` at the end of a paragraph or list item, or `^id` alone on a line after a table, list, or code block | `silica-markdown/tests/obsidian.rs` |
| Callouts: `> [!type]-` or `+` on the first quoted line; title is the rest of the line, or derived from the type; built-in types and aliases | `silica-markdown::canonical_callout_type`, `silica-markdown::callout_default_title` |
| `==highlight==`, `~~strike~~` need exactly two delimiters; `$x$` needs no space inside and no digit after; `%%` comments hide inline and block text | `silica-markdown/tests/obsidian.rs` |
| Reading-view hooks: `internal-link` with `data-href`, `tag`, `internal-embed` with `src`/`alt`, `callout` with `data-callout`/`data-callout-fold`, `task-list-item` with `data-task`, `data-heading` | `silica-markdown::render_html` |
| Cache record: links, embeds, tags, headings, sections with Obsidian's type names, list items with Obsidian's `parent` encoding, blocks, footnotes, frontmatter links with key paths | `silica-cache::index_markdown` |
| Link resolution: extension handling; `/path` from the root only, `./` and `../` relative to the source only, other paths root then relative; an ambiguous basename picks a root file, then the source's folder, then the shortest path, then path order; case-insensitive | `silica-cache::LinkResolver` |
| Vault-wide cache: `resolvedLinks` and `unresolvedLinks` counts per source (links, embeds, frontmatter links), backlinks per occurrence, tags merged case-insensitively from inline text and the `tags` property (list or comma string, `#` optional), aliases from `aliases` | `silica-cache::MetadataCache` |
| Reading view from the same tree: embeds resolve to a whole note, a `#Heading` section (to the next heading of the same or higher level) or a `#^id` block, nested four deep; footnotes numbered by first reference; a single newline is a line break unless `strictLineBreaks` | `silica::reading::model` |
| Reading view tables follow HTML automatic layout: a column is as wide as its widest cell, never narrower than its longest word, and the slack is shared when the note is narrower; fenced code never wraps | `silica::reading::view` |
| Source editor: the buffer holds `\n`; a save restores the file's BOM and its dominant `\r\n` ending; a file whose endings disagree is written back verbatim until it is edited; autosave two seconds after the last edit, on Ctrl+S, on navigation, on leaving source mode and on close; only the open note is written | `silica-text::FileText`, `silica::workspace::Workspace::flush_save` |
| Writer round trips: opening a note and saving it changes no bytes, every editing command undoes back to the file's bytes, typing changes only the bytes at the cursor, a property write stays inside its block, and a config file read and written back is unchanged | `silica-text/tests/writer_roundtrip.rs` (ratchet) |
| Smart lists: Enter continues quote markers, bullets, numbering (following siblings renumber) and task checkboxes, clears an empty item and outdents an empty nested one; Tab and Shift+Tab indent and outdent list items with a tab or `tabSize` spaces per `useTab`, and the ordered items they unsettle renumber: an item follows its previous sibling at its depth, one with none restarts at 1 | `silica::lists` |
| Auto-pairing, as the rows under "Observed auto-pair behavior" record it: `(`, `[` and `{` close unless a word character follows; `"`, `'`, `` ` ``, `*` and `_` close only after a space or the start of the text at end of line; `<`, `~`, `=`, `$` and `%` never close; a second quote or marker skips while a second bracket nests; a typed closer skips only over one the editor planted; Backspace between an empty pair takes both; every pair character but `<` wraps a selection, and the opener and its closer are one undo step | `silica-text::autopair` |
| Live Preview: markers of a construct are hidden unless the cursor or selection touches it; headings scaled by level, a closed ATX heading's trailing `#` run hidden with the opening one and a setext underline row hidden and laid out at body size; wikilinks show their alias, or their target with each subpath separator rendered as ` > ` (`Note > Heading`, `Note > ^blockid`, a leading `#` dropped), markdown links their text; tasks render as checkboxes that toggle on click; quotes as a bar per level; callouts fold the type into the title; comments and block ids hidden; inline math loses its dollars; a click on a rendered link follows it, Ctrl+click in Source mode too | `silica::preview`, `silica::editor` |
| Live Preview widgets: a top-level table, math block, or paragraph that is one embed (note, section, block or image) renders in place of its lines through the reading renderer while no selection touches them; the cursor entering the block (arrows, or a click on a table or math widget, at the clicked row) shows its source; links inside an embed widget work | `silica::reading::model::build_widgets`, `silica::editor` |
| Find and replace: `editor:open-search` (Mod+F) and `editor:open-search-replace` (Mod+H) open a panel in the editor; literal and case-insensitive by default, with match case and regex toggles (`$&`, `$1`, `$$` in a regex replacement); all matches highlighted; next and previous wrap; replace steps on; replace all is one undo step; Find all (Alt+Enter) selects every match and closes the panel | `silica::find`, `silica::editor` |
| Multiple cursors: Find all, Alt+click (again on a cursor removes it), `editor:add-cursor-above` and `below`; typing, deleting, Enter, Tab, cut, copy (joined with newlines) and paste (one line per cursor when the counts match) apply at every cursor as one undo step; movement moves them all, overlapping selections merge; Escape or a plain click keeps only the primary | `silica::editor` |
| View modes: `defaultViewMode` picks reading view (`preview`) or editing for a note at startup, `livePreview` picks Live Preview or Source for editing; `markdown:toggle-preview` toggles reading and editing, `editor:toggle-source` toggles Live Preview and Source | `silica::workspace::Workspace::default_mode` |
| Files view: folders before files, names compared case-insensitively, `.md` hidden, unknown kinds hidden unless `showUnsupportedFiles` | `silica::explorer::FileTree` |
| Search grammar: implicit AND, uppercase `OR` binds looser, `-` negation, `"phrase"` with `\"`, `/regex/flags`, field operators apply to one term or group, `[prop]` forms; explain tree uses Obsidian's labels; `tag:` matches exactly or a parent of a nested tag | `silica-search::query`, `silica-search::matcher` |
| Fuzzy search: query characters in order, case-insensitive, word starts and consecutive runs score higher; simple search needs every token as a substring | `silica-search::fuzzy` |
| Filesystem changes: dot-folders ignored, a removed folder removes its files, a moved-in folder is walked; adding or removing a file re-resolves every link | `silica-vault::Vault::refresh_path`, `silica-cache::MetadataCache::reresolve` |
| File operations: new notes are `Untitled`, `Untitled 1`, ... in the `newFileLocation` folder (`root`, `current`, or `newFileFolderPath` when it exists); a `folder/Name` link creates its folders; creating over a path fails ("File already exists."), renaming onto one fails ("Destination file already exists!") but a case-only rename succeeds; `trashOption` `system` (freedesktop trash), `local` (flat in `.trash/`, folders whole, collisions ` 2`, ` 3`) or `none`; defaults `system`, `promptDelete` on, `alwaysUpdateLinks` on | `silica-vault::ops` |
| Link rewriting on rename or move: every link resolving to the file is regenerated in `newLinkFormat` (shortest falls back to the full path when ambiguous), keeping its style (wikilink, embed, markdown, angle brackets), `.md` and `%20` in markdown destinations, the subpath, and the alias verbatim unless it equals the old basename and not the written path; markdown link text follows the new basename or link path when it was the old one; code, reference links and `[[#Heading]]` untouched; a moved note's own links change only when they would break; frontmatter links replaced in place | `silica-cache::links` |
| Properties: the block parses into ordered entries with byte spans; a type comes from `types.json`, then the reserved keys `tags` and `aliases`, then the value (string `text`, bare `YYYY-MM-DD` `date`, number `number`, boolean `checkbox`, sequence `multitext`; `cssclasses` is a plain list and an ISO date-time is `text`) | `silica-formats::properties` |
| Property serialization as Obsidian 1.14.4 writes it: a scalar plain where YAML allows it else double quoted (`"a: b"`, `"  padded  "`, `"42"`, `"[[A]]"`, `""`, `"#a"`), a date bare, a newline as a `\|-` block scalar, a list as a two-space block sequence (the empty list `[]`), null as a bare key, a new property after the last entry, and removing the last property removes the fences | `silica-formats::properties::render_entry`, `plan` |
| Property edits are surgical: only the entries whose value actually changes are rewritten, so comments, flow sequences, quoting, blank lines and the file's line endings survive — Obsidian re-serializes the whole block and loses them | `silica-formats::properties::plan`, `silica-cache/tests/properties_corpus.rs` (ratchet) |
| Properties UI: a foldable block above the note with a type glyph, key and value per row, links and tags inside a value clickable, lists and tags as pills, a value that does not fit its type marked and kept as written; `propertiesInDocument` shows the table (`visible`, the default), nothing (`hidden`) or the raw YAML (`source`) | `silica::properties`, `silica::reading::view` |
| Property editing in Live Preview: a click opens a field whose keys the editor routes (Enter commits, Tab commits a key and moves to its value, Escape abandons), a commit is one undo step, a key typed empty removes the property, a rename keeps the entry in place (Obsidian moves it to the end), a new row writes nothing until it is named, and a type chosen from the row menu is recorded in `types.json` | `silica::properties`, `silica::editor` |
| Unlinked mentions: whole-word, case-insensitive, names and aliases, longest name wins, not inside `[[ ]]`, `[ ]( )`, inline or fenced code | `silica::mentions` |
| `graph.json`: filters (`search`, `showTags`, `showAttachments`, `hideUnresolved`, `showOrphans`), `colorGroups` with `{query, color: {a, rgb}}`, display and force keys, `scale` and `close`; `collapse-*` keys round-trip | `silica-formats::config::GraphConfig` |
| Config files are sparse and optional; unknown keys round-trip | `silica-formats::config`, `silica-vault::Vault::read_config` |
| Writing a config file changes only what the value changed: the new value is merged into the file's own JSON, so key order, number spelling (`1` not `1.0`, and more digits than an `f32` field holds), and keys the type does not serialize all survive | `silica-vault::Vault::write_config` |
| `core-plugins.json` map and legacy array shapes | `silica-formats::config::CorePlugins` |
| JSON Canvas 1.0 fields, z-order by array order, unknown keys at every level; written as Obsidian 1.14.4 writes an edited save: the file's own key order kept with the model's keys appended in its fixed order, groups first, default edge ends dropped, one object per line unless it nests one, tabs, no trailing newline, see [[#Observed canvas write behavior, Obsidian 1.14.4 (installer build 1.13.7)]] | `silica-formats::canvas`, `silica-formats/tests/canvas_obsidian.rs` (ratchet), `silica-text/tests/writer_roundtrip.rs` |
| Canvas view: the dot grid, cards, groups, edges, labels and controls drawn as Obsidian 1.14.4 draws them; pan, zoom, selection, moving, resizing, drawing edges, typing into a card, the menus and the keyboard, saving through the canvas writer; the leaf's `viewState`, see [[#Observed canvas view behavior, Obsidian 1.14.4 (installer build 1.13.7)]] | `silica::canvas`, `silica-canvas` |
| Bases top-level keys and recursive filters | `silica-formats::base` |
| Command ids, palette names and default hotkeys as Obsidian 1.14.4 registers them; `hotkeys.json` entries replace the defaults and an empty list unbinds; labels read `Ctrl + Shift + F` | `silica-commands` |
| Command palette: fuzzy over names, pinned ids from `command-palette.json` first, then this session's recent commands, then registration order; commands that cannot run are not listed | `silica::palette` |
| Settings tabs, rows and defaults as Obsidian 1.14.4 shows them; `switcher.json`, `command-palette.json` and plugin manifests round-trip | `silica::settings`, `silica-formats::config` |
| Settings toggles write: a click sets the key Obsidian writes in `app.json`, `appearance.json`, `switcher.json`, `core-plugins.json` or `community-plugins.json`, changing nothing else in the file, and the running app follows what it can (readable line length, smart lists, indentation, properties in document, strict line breaks, show all file types); a vault with no `core-plugins.json` gets the whole default map rather than a one-key file | `silica::settings`, `silica::workspace::Workspace::change_setting` |
| Each settings row says how far it gets in Silica: untagged means the app follows it at once, "on restart" the theme and fonts, "not in Silica yet" a value written only for Obsidian to read. A Silica divergence: Obsidian has no such tags, and shows every setting as live | `silica::settings::Honored` |

## Parser notes

The parser follows the CommonMark two-phase design: a line-by-line block pass
with a stack of open containers (lazy continuation, tab stops, list item
indentation), then an inline pass per paragraph, heading, callout title, and
table cell using the delimiter stack for emphasis and the bracket stack for
links. Obsidian's constructs are extra block starts and inline scanners in the
same two passes, each behind a flag in `Options` so the CommonMark suite runs
with them off.

Decisions taken from observation that the spec text does not spell out:

- A table's header row must be a one-line paragraph followed by the delimiter
  row, and the header or delimiter row must contain a pipe. A table cannot
  interrupt a longer paragraph, and a row line without a pipe ends the table.
- A paragraph that is only `^id` attaches the id to the previous block at the
  same level, which is how Obsidian ids tables and code blocks. A ` ^id` at
  the end of a callout header ids the callout.
- An unterminated `$$` or `%%` block runs to the end of the file. `$$x$$` or
  `%%x%%` closed on one line stays inline inside a paragraph.
- Footnote references to undefined labels render as the bare label; inline
  footnotes `^[text]` are numbered in order of appearance after referenced
  definitions.
- Markdown links count as vault links in the cache when the destination has
  no URL scheme; the destination is percent-decoded.

## Observed Obsidian behavior

Every item below was seen in Obsidian 1.13.7's output for the corpus under
`crates/silica-markdown/tests/corpus` and is reproduced by the parser, most
behind `Options::quirks` so the CommonMark suite still runs clean.

Blocks:

- A single newline inside a paragraph renders as `<br>` (strict line breaks off).
- A setext underline only applies to a one-line paragraph; after a longer
  paragraph `---` is a thematic break.
- `- [ ]` with nothing after the bracket is plain text, not a task.
- A list marker indented four or more columns inside a list is a sibling
  item; CommonMark would make it a lazy continuation line.
- A `>` line directly after a list item's text stays inside that item, and so
  do the quoted lines after it: `- item` / `> quote` / `> more` puts both
  quoted lines in one blockquote inside the item, not a second blockquote after
  the list. This is also what keeps the body of a callout nested in a list item
  of another callout.
- A callout header needs *something* between its brackets, which is then
  trimmed: `[! bad]` is a `bad` callout, `[! ]` and `[!   ]` are callouts with
  an empty type (an empty `data-callout`, and no title to derive), and a bare
  `[!]` is not a callout at all. `[!tip]Title` without a space is an ordinary
  quote.
- A callout header with no title of its own shows the type with each `-` turned
  into a space, the whole lowercased and the first character upper-cased:
  `[!note]` shows "Note", `[!bad-type-two]` shows "Bad type two", `[!NOTE-X]`
  shows "Note x". `_` is not touched, so `[!bad_type]` shows "Bad_type".
- A callout's title is block-parsed, not inline-parsed: `[!note] # h` puts an
  `<h1>` inside `.callout-title-inner`, `[!note] 1. item` an `<ol>`, and
  `[!note] > quote` a nested blockquote. This is also why `[!note] -` is not a
  fold marker — the space ends the header, and the lone `-` becomes a list with
  one empty item. The ordinary one-paragraph title renders bare, with no `<p>`.
- A callout's first body line takes the ordinary block rules. A table's
  delimiter row or a setext underline on the line after the header is read as a
  table or a heading, exactly as it would be anywhere else — the header is not
  one of the body paragraph's lines.
- A link ends at the first child that is a link of its own, because an `<a>`
  never nests: `[md [[Links]] link](url)` renders the anchor around `md `
  (trailing space included), the wikilink as a sibling anchor, and ` link` as
  bare text outside both. Once closed it stays closed, so with two wikilinks
  inside one link text the run between them is bare as well. A tag ends a link
  too, being an anchor; an embed or an image does not, being a `<span>`.
- Frontmatter renders as a hidden `pre.frontmatter`; `<script>` is removed;
  unknown elements are unwrapped; raw `<a>` gains `target` and `rel`.

**Observed, not yet reproduced** — callouts, probed 2026-10-29 against Obsidian
1.14.4 (installer build 1.13.7) with `tools/probe-gen`. Unlike everything else
in this section, the parser does not match these yet:

- A `^blockid` as a callout's only body line is consumed as the block id,
  leaving a `.callout-content` that is present but empty. The parser renders it
  as a paragraph of literal text. Matching it needs the renderer to know the
  body had a non-blank line even though it produced no block, and a block id
  with nothing to identify has no effect on a real vault, so this is left
  alone deliberately.
- A `^id` on its own line at the end of a callout body is consumed, but the
  line break before it is kept, so `> [!note]` / `> body` / `> ^id` renders
  `<p>body<br></p>`. The parser drops the break with the id. Also left alone:
  the stray `<br>` is an artifact of Obsidian's own stripping.
**Observed, not yet reproduced** — asterisk and `=` runs of four or more,
probed 2026-10-29 the same way. These are the whole of what keeps
`edge/Probe Inlines.md` from passing, and they look like artifacts of
Obsidian's own delimiter scanner rather than a rule:

| source | Obsidian |
| --- | --- |
| `****a****` | `*<strong>*a**</strong>` |
| `*****a*****` | `<strong>*</strong>a<strong>*</strong>` |
| `***a**b*` | `<strong>*a</strong>b*` |
| `*a **b* c**` | `<em>a **b</em> c**` |
| `==== four` | all literal; but `==== four ==x===y==` gives `=<mark>= four ==x</mark>=y==` |

A run of four leaves its first character literal and opens at the second, and
`==` behaves the same way, so the two share whatever the mechanism is. A run of
five instead resolves inside itself. The parser follows CommonMark's
rule-of-three here, which nests where Obsidian splits. Pinning the real rule
wants a systematic sweep of run lengths three to eight, with and without a
partner run.

**Observed, not yet reproduced** — link nesting, probed 2026-10-29 the same
way:

- A link whose text holds a *markdown* link keeps the outer link and leaves the
  inner one as literal text: `[a [b](f) c](e)` is one anchor reading
  `a [b](f) c`. The parser nests them instead, which is why the rule above
  counts only wikilinks and tags.
- A wikilink nested deeper than the link's own children still ends the anchor,
  and the formatting around it reopens outside: `[md **[[Links]]** link](url)`
  gives `<a>md <strong></strong></a><strong><a>Links</a></strong> link`, the
  way an HTML parser re-opens formatting elements across a closed `<a>`. The
  parser only splits on a direct child.
- `[[[Links]] md](url)` resolves as a wikilink whose body is `[Links`, with the
  rest of the line literal; the parser reads it as a markdown link.

**Observed, not yet reproduced** — callouts, continued:

- Not an Obsidian behavior but a deliberate divergence in Silica: the reading
  view paints a callout title as one run of rich text, so a title that
  block-parses to a heading or a list shows its *text* without that block's
  styling. `render_html` matches Obsidian; the painted view does not yet.

Inlines:

- `#` starts a tag after whitespace or right after another inline construct
  (`[[link]]#tag`, `` `code`#tag ``, `**b**#tag`, `#a#b`), not after a letter
  or `(`. Tags may contain emoji and any non-ASCII letters.
- `~~~x~~~` and `===x===` delimit with their first two characters; the third
  stays inside. `***x***` nests strong outside em.
- Underscore emphasis is looser than CommonMark's. A run may *open* inside a
  word, which CommonMark forbids, and a run of two or more may close there as
  well: `a_b_` is `a<em>b</em>`, `x__y__z` is `x<strong>y</strong>z`. A single
  `_` still cannot close before a letter or digit, which is what keeps
  `snake_case_name`, `CONST_NAME_HERE` and `a_b_c` plain. Behind
  `Options::quirks`; the CommonMark suite keeps the spec's own rule.
- A backtick run without a closer retries one backtick shorter.
- Legacy entities decode without a semicolon (`&copy` is ©).
- One-letter URL schemes autolink (`<a://b>`); bare `https://`, `http://`,
  `www.` and email addresses link wherever they appear, even mid-word; a
  trailing `"` stays in the URL while `?!.,:*_~` and unbalanced `)` do not.
- `[[Note\|alias]]` is the alias form: the target is the text before the first
  `|` with a trailing backslash dropped. Inside code spans `\|` stays literal.
- An inline `%%comment%%` does not cross a line break.
- `$...$` closes at a `$` preceded by a non-space and not followed by a digit.
- `![alt](path)` without a scheme is an internal embed whose `alt` is the raw
  alt text up to a `|`; `![alt][ref]` is always an `<img>`. For `![[x|a|200]]`
  the alt is the first alias part that is not a size hint, else the display
  text (`Note > Heading`).
- `[[Note#H]]` displays as `Note > H`; an alias adds `aria-label`.

Line endings and the byte order mark, from the corpus and a 1700-note real
vault rather than from Obsidian's source:

- A file's endings are not always uniform. One note in the wild carried a
  single `\r\n` among 4430 `\n` lines, so "the file used CRLF" has to mean
  the dominant ending, not the presence of one.
- Silica therefore writes the dominant ending on every line once a file is
  edited, and writes a mixed file back verbatim while nothing has changed.

What Obsidian itself does was the open question here, and it is now
observed (see [[Development#Line endings and the byte order mark, 2026-10-18]]).
**Its editor normalizes on save**: every ending becomes `\n` and a UTF-8
byte order mark is dropped.

| Before | After one character is typed |
| --- | --- |
| `# Mixed\nline one\nline two\r\nline three\n` | every line `\n` |
| `# Mixed\r\nline one\r\nline two\nline three\r\n` | every line `\n` |
| `a\r\nb\nc\r\nd\n` | `aX\nb\nc\nd\n` |
| `# All\r\nline one\r\nline two\r\n` — uniform CRLF | every line `\n` |
| `\u{feff}# Bom\nline one\n` | `# Bom\nlineX one\n` — the mark is gone |

A note that is only **opened** is not rewritten: the same mixed file read
back after being opened and left alone is byte-identical. The normalization
belongs to the save, not the load.

This is the one place Silica deliberately keeps *more* of the file than
Obsidian does. Silica writes the file's own dominant ending and puts a byte
order mark back, so it never introduces a difference; Obsidian's editor
converts the file the first time it is edited there. A vault used in both
therefore drifts towards LF with no mark, driven by Obsidian, and Silica
never drives it the other way.

Note that this is the *editor's* save. The frontmatter API is not the same
path and does not normalize: writing a property into a CRLF note leaves
`---\r\ntitle: Hi\nadded: v\n---\r\n`, recorded under the properties
section above.

Not reproduced (two corpus notes): nested runs such as `****a****` and
`==== four ==x===y==`, and a wikilink inside markdown link text, which splits
the link in Obsidian.

## Observed properties behavior, Obsidian 1.14.4

Observed with a throwaway plugin in a scratch vault (see
[[Development#Properties, 2026-10-18]]), driving
`app.fileManager.processFrontMatter`, `app.metadataTypeManager` and the three
property commands, then reading the files back byte for byte.

The serializer:

- A `processFrontMatter` call that mutates **nothing** still rewrites the block:
  `# a comment` is deleted, `tags: [one, two]` becomes a two-space block
  sequence, and a blank line inside the block is dropped. A block already in
  that canonical form round-trips byte-identical, so the canonical form is
  what Silica writes for a value it changes — and only for that value.
- Scalars are plain where YAML allows it and double quoted otherwise:
  `k: "a: b"`, `k: "a #b"`, `k: "  padded  "`, `k: ""`, `k: "42"`,
  `k: "[[Target]]"`, `k: "#a"`. `say "hi"`, `Ä ö — ✓`, `https://example.com`
  and a 200-character run all stay plain and unwrapped. A date is written
  bare (`d: 2026-10-18`).
- A value containing a newline becomes `k: |-` with the lines indented two
  spaces. A nested mapping is written with two-space indentation.
- Lists are block sequences (`k:\n  - a\n  - b`), one `- ` per item, each item
  quoted by the scalar rules. The empty list is the one flow form, `k: []`.
- Null is the bare key (`k:`); a JavaScript `undefined` writes no key at all.
- A new property goes after the last entry. Deleting the last property deletes
  the fences too: `---\na: 1\n---\nbody\n` becomes `body\n`.
- Renaming through `processFrontMatter` (delete then set) moves the key to the
  end of the block. Silica renames in place instead, changing fewer bytes.
- Duplicate keys make the whole block fail to parse ("Map keys must be
  unique"), so such a note has no properties in Obsidian's cache at all.

Types, from `registeredTypeWidgets`:

- The registry is `text`, `multitext` (shown as "List"), `number`,
  `checkbox`, `date`, `datetime`, `tags`, `aliases`, plus `file`, `folder`
  and `property` for Bases views. There is **no** `cssclasses` type.
- `tags` and `aliases` are `reservedKeys` of their widgets, so those keys keep
  their type whatever the value is.
- Inference from the value: a string is `text` except a bare `YYYY-MM-DD`,
  which is `date`; a YAML number is `number`, a boolean `checkbox`, a sequence
  `multitext`; null, a nested mapping and a block scalar are all `text`.
  An ISO *date-time* string (`2026-10-18T09:30:00`) infers as `text`, not
  `datetime` — `datetime` is only ever assigned.
- `types.json` is written in insertion order, seeded with `aliases: aliases`,
  `cssclasses: multitext` and `tags: tags` before any assigned key, with
  two-space indentation and no trailing newline.

Commands and settings:

- `markdown:add-metadata-property` ("Add file property") writes nothing to the
  file; it opens an empty row in the UI, and the property appears only once it
  is named.
- `markdown:clear-metadata-properties` ("Clear file properties") removes the
  whole block, leaving the body alone.
- `propertiesInDocument` defaults to `visible` with the key absent from
  `app.json` (confirmed through `vault.getConfig`).
- The properties block is the same editable widget in reading view as in Live
  Preview; the heading reads "Properties" and folds.

Where Silica deliberately differs, because Obsidian loses bytes:

- **CRLF.** Writing a property into a CRLF block gives mixed endings
  (`---\r\ntitle: Hi\nadded: v\n---\r\n`). Silica writes the file's own
  ending throughout.
- **Byte order mark.** Obsidian does not recognise frontmatter after a BOM: it
  prepends a *second* block, leaving
  `---\nadded: v\n---\n\u{feff}---\ntitle: Hi\n---\n`. Silica reads the block
  through the BOM and edits it in place.
- **`...`** does not close a properties block in Obsidian, and no longer does
  in Silica either; adding a property to `---\na: 1\n...\n` prepends a second
  block there too.

## Observed autocompletion behavior, Obsidian 1.14.4

Observed with a throwaway plugin in a scratch vault (see
[[Development#Autocompletion, 2026-10-18]]), driving the four
`app.workspace.editorSuggest.suggests` instances directly: `onTrigger` for
every shape of line, `getSuggestions` for what each offers, and the popup's
own `scope` key handlers for what accepting one writes.

The popup binds `ArrowUp`, `ArrowDown`, `PageUp`, `PageDown`, `Home`, `End`,
`Ctrl+P`, `Ctrl+N`, `Enter` and `Escape`; the link suggester also binds
`Tab`, `Shift+Enter`, `#` and `^`. All four have a limit of 100 items.

What triggers, scanning back from the cursor on the cursor's line only:

- **Links.** The last `[[` before the cursor with no `]` after it. The query is
  everything from there to the cursor and keeps its spaces, `#`, `^`, `|` and
  `/`: `[[Alpha#first` is one query, `Alpha#first`, which the suggester
  splits itself. `[[[al` takes the last `[[`. A cursor inside `[[al]]` still
  triggers; one after the `]]` does not. `![[` is the same suggester, the
  query starting after the third bracket.
- **Tags.** A `#` at the start of the line or after whitespace, through the
  cursor; `word#inl` does not trigger. The query excludes the `#` but the
  range to replace includes it.
- Nothing suppresses either one: both fire inside a code span, inside a fenced
  code block, inside `%%comments%%` and inside frontmatter. The link
  suggester wins when both would fire, being first in the list.
- `[text](` offers nothing. There is no markdown-link path completion.

What each offers:

- An **empty** `[[` query lists every file in the vault plus every alias,
  ordered by the file's modification time, newest first — not by recency of
  opening, which is what the quick switcher uses. A non-empty query is fuzzy
  matched and an alias scores just below its file.
- The list is `metadataCache.getLinkSuggestions()`: files and aliases, and
  **no unresolved link targets**, which is the other way the link suggester
  differs from the quick switcher. A query nothing matches offers one item
  with no file, which links the text as typed.
- `[[Note#` lists that note's headings in document order, the H1 included,
  unfiltered while the query after the `#` is empty. `[[#` lists the
  *current* note's headings.
- `[[Note#^` lists every block of the note — headings as `## First Heading`
  with their markers, paragraphs as their text with any `^id` stripped.
- `[[Note|` lists that note's aliases; with text after the `|` it offers that
  text as the display text.
- `#` lists every tag in the vault alphabetically, parent segments
  (`project` for `project/alpha`) as their own items, without the `#`.

What accepting one writes, with `newLinkFormat: shortest` and wikilinks:

| Typed | Accepted | Written |
| --- | --- | --- |
| `[[beta no` | the file | `[[Beta Note]]` |
| `[[al` | `Sub/Alpha` | `[[Sub/Alpha\|Alpha]]` |
| `[[alpha ali` | the alias | `[[Alpha\|Alpha Alias]]` |
| `[[Alpha\|disp` | the typed text | `[[Alpha\|disp]]` |
| `[[Alpha#first` | the heading | `[[Alpha#First Heading]]` |
| `[[Alpha#^` | a block with no id | `[[Alpha#^07fe38]]` |
| `[[pi` | `pic.png` | `[[pic.png]]` |
| `![[pi` | `pic.png` | `![[pic.png]]` |
| `[[No Such Note` | the typed text | `[[No Such Note]]`, and no file is created |
| `#inl` | the tag | `#inline-tag ` |

- The display text after `|` is added exactly when the path part contains a
  `/`, and only for markdown targets: `[[Sub/Alpha|Alpha]]` and
  `[[../Beta Note|Beta Note]]`, but `[[../pic.png]]` with none.
- The path part is `fileToLinktext` in the vault's `newLinkFormat`, so the
  same note is `[[Gamma]]` from inside `Sub`, `[[Sub/Gamma|Gamma]]` from the
  root under `absolute`, and `[[../Beta Note|Beta Note]]` under `relative`.
- A **heading** pick is the exception twice over: the path is left exactly as
  it was typed rather than reformatted, and no display text is ever added.
  `[[Sub/Gamma#` becomes `[[Sub/Gamma#Gamma Heading]]` even though
  `shortest` would have written the path as `Gamma`.
- A closing `]]` already after the cursor is consumed rather than doubled, so
  accepting inside `[[be|]]` leaves `[[Beta Note]]` and not four brackets.
- Accepting a **block** with no id of its own writes the id into the target
  note. The id is six lowercase hex characters (twelve of twelve samples),
  appended to a paragraph inline (`para one ^3a3815`) and written on a line
  of its own after a heading (`## First Heading\n\n^07fe38\n`). An id
  already there is reused and the target is not touched.

With `useMarkdownLinks` the same picks write markdown links: the display text
is the basename, the alias, or the heading text, the destination is the link
path with `.md` kept for notes, and only spaces are percent-encoded —
`[Beta Note](Beta%20Note.md)`, `[Alpha Alias](Alpha.md)`,
`[First Heading](Alpha.md#First%20Heading)`, `[pic](pic.png)`,
`![Beta Note](Beta%20Note.md)`. A pick with no file behind it writes an
**empty** display text: `[](No%20Such%20Note%20At%20All)`, and a block
reference writes `[](Alpha.md#^07fe38)`; the cursor then lands between the
`[` and the `]`, whichever key accepted it, since that is the part left to
fill in.

The three accept keys differ, and the difference is the point:

- **Enter** writes the whole link and leaves the cursor after the `]]`; on a
  tag it writes a trailing space (`#inline-tag `) and sits after it.
- **Tab** writes the same link but leaves the cursor *inside*, before the
  `]]`, so a `#heading` or `|alias` can follow; on a tag it writes no
  trailing space. With `useMarkdownLinks` there is nowhere to stand, and Tab
  lands where Enter does.
- **Shift+Enter** ignores the selected item and links the text as typed:
  `[[beta no` becomes `[[beta no]]`. The tag suggester does not bind it.

**Unprobed:** whether the `#` and `^` keys the link popup binds do anything
beyond typing the character, and whether typing `[[` auto-pairs the closing
brackets. Silica does auto-pair `[` per the rows below, so `[[` there gives
`[[]]` with the suggester opening inside; what Obsidian does is still unread.

## Observed table editing behavior, Obsidian 1.14.4

Observed with a throwaway plugin in a scratch vault (see
[[Development#Table editing, 2026-10-18]]), running each of the sixteen
table commands against eight shapes of table with the cursor in two
different cells, then reading the note back. The commands' own
`editorCheckCallback` only passes in Live Preview with the editor focused —
in raw source mode it never does, so **Obsidian's table commands are a Live
Preview feature**; the 186 runs where the check passed are what is recorded
here.

Any table command **rewrites the whole table**, not the row or column it
was asked about. The written form is the same every time:

- Every row gets outer pipes, even when the table was written without them,
  and one space either side of each cell's text.
- A column is as wide as its widest cell, and never narrower than three,
  which is what the delimiter needs. Cells are padded to that width.
- The delimiter fills the column width: `---`, and with an alignment
  `:--`, `:-:` and `--:`, growing to `:-----`, `:----:` and `-----:`.
- Padding follows the column's alignment: left and none pad on the right,
  right pads on the left, and centre splits the difference with the odd
  space on the right (`B` in a six-wide centred column is `  B   `).
- A row with too few cells is filled out with empty ones.
- Cell text is kept as written, escapes included: `x \| y` stays one cell.
  A `|` inside a code span is **not** protected — `` `a|b` `` is two cells,
  `` `a `` and `` b` `` — which is what the parser already does.

What each command does, with the cursor in some cell:

| Command | Effect |
| --- | --- |
| `table-row-before`, `table-row-after` | an empty row above or below |
| `table-row-up`, `table-row-down` | swap with the row the other side; the first body row swaps with the **header**, delimiter staying put |
| `table-row-copy` | the row again, directly below |
| `table-row-delete` | the row; deleting the header promotes the next row into it |
| `table-col-before`, `table-col-after` | an empty column either side, alignment `None` |
| `table-col-left`, `table-col-right` | swap with the column the other side, alignment travelling with it |
| `table-col-copy` | the column again, directly to the right, alignment included |
| `table-col-delete` | the column and its alignment |
| `table-col-align-left/center/right` | the column's alignment, and every cell in it re-padded |

`editor:insert-table` is the one command that is not about an existing
table: on an empty line it writes a blank line and then a two-by-two table
of empty three-wide cells, leaving the cursor in the first header cell. On a
non-blank line it keeps the line, puts a blank line after it and the table
below that, without splitting the line even when the caret is mid-line — see
[[#Observed editing command behavior, Obsidian 1.14.5 (installer build 1.13.7)]].

```
|     |     |
| --- | --- |
|     |     |
```

Editing a cell is the opposite: **ordinary typing inside a table re-pads
nothing**. A change to one cell's text leaves every other byte of the table
as it was, and it stays that way when the cursor moves to another cell and
when it leaves the table altogether — `| 1 | 2 |` edited to `| WIDE | 2 |`
stays ragged until a table command runs. So the padded form is what the
commands write, not an invariant Obsidian maintains.

The widget itself is a real `table.table-editor` inside a `.table-wrapper`,
each cell holding a `.table-cell-wrapper`; the cells are not
`contenteditable` themselves, so the table stays drawn as a grid while the
cursor is inside it rather than unfolding to its source the way other
widgets do.

**Unprobed:** where Obsidian leaves the cursor after each command (the
columns it reported came from an unfocused editor and disagreed with
themselves), and whether a column's width counts characters or display
columns for CJK and emoji.

## Observed block reference behavior, Obsidian 1.14.4

Observed with a throwaway plugin in a scratch vault (see
[[Development#The probe batch, 2026-10-18]]), driving the link suggester's
`onTrigger`/`getSuggestions` and the popup's own Enter handler over a note
holding one of every block kind, then reading the target note's bytes back
as an escaped string and a hex dump.

`[[Note#^` offers one row per block, in document order. The kinds, with the
label each row reads for the probe note:

| Node | Row reads |
| --- | --- |
| heading | `# Probe Heading` — the markers are kept |
| paragraph | `A plain paragraph with no id.` |
| table | `A B \n1 2` — cells joined by spaces, rows by a newline |
| code (fenced) | `let fenced = 1;` — the fence and its language are dropped |
| code (indented) | `indented code line` |
| callout | `A callout titlecallout body line` — title and body run together |
| blockquote | `a blockquote\nsecond quote line` |
| math | `x = y + 1` — the `$$` fences are dropped |
| listItem | one row per item, nested items getting their own row as well |

A thematic break is **not** offered, and neither is the frontmatter block.
A list item inside a blockquote is offered as the *blockquote*, and one
inside a callout as the *callout*: the quote is the block, not the item.
`[[#^` lists the blocks of the note being edited. A path that resolves to
nothing (`[[NoSuchNote#^`) and one that resolves to an attachment
(`[[pic.png#^`) each offer a single row of type `none` rather than an empty
list.

Accepting a row whose block has no id of its own writes one into the target
note. The id is six lowercase hex characters — 180 generated in a row were
180 distinct values — and where it goes depends on the kind:

- **A paragraph** takes ` ^id` appended to its last line, *verbatim*, after
  whatever is already there. A line ending in a hard break keeps it and the
  id goes after: `Ends in two spaces.  ` becomes
  `Ends in two spaces.   ^id`, and `Ends in a backslash\` becomes
  `Ends in a backslash\ ^id`.
- **A heading** takes the id on a line of its own, with a blank line either
  side, and the bytes after it are normalized: the heading keeps its text,
  then `\n\n^id\n`, then what followed with its leading run of newlines
  collapsed to one. Every shape agrees:

| Before | After |
| --- | --- |
| `## H` (no trailing newline) | `## H\n\n^id\n` |
| `## H\n` | `## H\n\n^id\n\n` |
| `## H\n\nbody\n` | `## H\n\n^id\n\nbody\n` |
| `## H\nbody\n` | `## H\n\n^id\n\nbody\n` |
| `## H\n### H2\nbody\n` | `## H\n\n^id\n\n### H2\nbody\n` |

  The last two are the ones worth noticing: Obsidian inserts a blank line
  that was not there, so the id is a paragraph of its own rather than a
  second line of the following block.

A block that already carries an id is linked to by that id and the target
note is not touched at all.

Where Silica deliberately differs, because Obsidian loses bytes:

- **CRLF.** Writing an id into a CRLF note gives the inserted lines LF
  endings and leaves the rest CRLF: `# Crlf\r\n\r\npara` becomes
  `# Crlf\n\n^id\n\r\n\r\npara`, a mixed file. Silica writes the file's own
  dominant ending throughout, as it already does for properties.

Where the id goes depends on the kind, and there are only two shapes. A
block whose text ends a line takes the id **inline**, and a block that
spans lines takes it on a **line of its own** with a blank line either side:

| Kind | Written |
| --- | --- |
| paragraph | `last line ^id` |
| list item | `- alpha item ^id`, and on the parent's own line when it has a nested child |
| heading | `## H\n\n^id\n` |
| table | `\| 1 \| 2 \|\n\n^id\n` |
| fenced code | ` ```\n\n^id\n ` |
| indented code | `    second line\n\n^id\n` |
| callout | `> body line\n\n^id\n` |
| blockquote | `> second line\n\n^id\n` |
| display math | `$$\n\n^id\n` |

A list item inside a blockquote or a callout is not addressed as an item at
all: the row is the quote, and the id goes on its own line after the whole
quote. At the end of a file with no trailing newline, an inline id is
appended and the file still ends without one
(`last paragraph ^id`), while an own-line id brings a trailing newline with
it (`\| 1 \| 2 \|\n\n^id\n`).

A block that already carries an id is linked by it and the file is not
written at all — its bytes and its modification time are both untouched.

**Unprobed:** an ordered list item and a list item at the end of a file,
whose accept steps did not fire; the inline rule is taken from the two
unordered samples that did.

## Observed settings write behavior, Obsidian 1.14.4

Observed with a throwaway plugin in a scratch vault (see
[[Development#The probe batch, 2026-10-18]]), setting each key through
`vault.setConfig` and reading `.obsidian/app.json` and
`.obsidian/appearance.json` back as raw bytes before and after every
change.

**A setting returned to its default is written, not removed.** Every one of
the thirty-eight changes the run made left its key in the file, the
default-valued ones included: `newLinkFormat` back to `shortest`, `theme`
back to `system`, `tabSize` back to `4`, `baseFontSize` back to `16`,
`livePreview` back to `true`, `readableLineLength` back to `true` and
`accentColor` back to `""` all kept their key with the explicit value. A
key absent from the file stays absent until something sets it, and from
then on it is always written. That is what Silica already does, so no way
of removing a config key is needed.

The values each enum key takes, as written:

| Key | File | Values |
| --- | --- | --- |
| `defaultViewMode` | `app.json` | `preview`, `source` |
| `propertiesInDocument` | `app.json` | `visible`, `hidden`, `source` |
| `newFileLocation` | `app.json` | `root`, `current`, `folder` |
| `newLinkFormat` | `app.json` | `shortest`, `relative`, `absolute` |
| `trashOption` | `app.json` | `system`, `local`, `none` |
| `fileSortOrder` | `app.json` | `alphabetical`, `byModifiedTime` |
| `theme` | `appearance.json` | `moonstone`, `obsidian`, `system` |

`livePreview` is a boolean rather than an enum, and `tabSize` and
`baseFontSize` are written as plain integers (`2`, `4`, `8`, `14`, `16`,
`20`) with no decimal point. A text field writes its string verbatim,
including a font family with a space in it (`"Noto Sans Mono"`), and an
emptied field writes `""` rather than dropping the key.

**Unprobed:** the option *labels* each dropdown shows, and the min, max and
step of the two sliders — the run drove the keys rather than the controls,
so it settled what is written and not what the tab reads.

## Observed canvas write behavior, Obsidian 1.14.4 (installer build 1.13.7)

First seen on 2026-10-18 in the gate vault: a canvas authored outside
Obsidian (2-space pretty-printed, every field on its own line, key order
`id, x, y, width, height, type, text, color`) was opened once and came back
rewritten whole, 6215 bytes down to 4731, every node on one line in the
order `id, type, text, x, y, width, height, color`. Then pinned down on
2026-10-19 over CDP (`tools/obsidian-probe/canvas.mjs`, four runs, raw
results in `tools/obsidian-probe/results/2026-10-19-canvas.json`) with
scratch canvases carrying every node and edge type, every optional key in a
scrambled order, and unknown keys at every level. `silica-formats::canvas`
reproduces all of it, and `silica-formats/tests/canvas_obsidian.rs` holds
the files the probe wrote beside what Obsidian wrote back.

**When Obsidian writes.** Opening a canvas writes nothing. Closing it
rewrites the file even when nobody edited it, through the formatter below
but from the data as loaded: keys stay in file order and nothing is added
or dropped. An edit (a node moved, an edge drawn) saves within about two
seconds and rebuilds every object from the model, which is the form below.
`canvas:new-file` creates `Untitled.canvas` as an **empty file** (zero
bytes) and writes `{"nodes":[...],"edges":[]}` once something is in it.

**The formatter.** Tab indentation, no space after `:` or `,`, no trailing
newline. A container is written on one line unless one of its values is
`null`, an object or an array (`typeof v === 'object'`); then every value
goes on its own line one tab deeper and the closing bracket on its own line
at the container's indent. So a plain node or edge is one line, a node that
carries a plugin's nested object is spread over several, and `"edges":[]`
and `{}` stay as written. Numbers and strings are `JSON.stringify`'s: `1e3`
comes back `1000`, `-0` as `0`, `0.5` as `0.5`; `\n`, `\"`, `\\` and `\t`
are escaped, `é` and U+2028 are not.

**Key order on an edited save** — the model's `getData` writes each object
as the keys of the object it was read from, in file order, minus the keys
the model owns, and then the model's keys in a fixed order, each
overwriting in place when the file already had it and appended otherwise:

| Object | Keys the model owns (removed from the file order) | Appended, in this order |
| --- | --- | --- |
| every node | `x`, `y`, `width`, `height`, `color` | `id`, `x`, `y`, `width`, `height`, `color` (when set), `type`, then the type's own keys |
| text node | — | `text` |
| file node | — (`file` and `subpath` keep their place) | `file`, `subpath` |
| link node | — (`url` keeps its place) | `url` |
| group node | `label`, `background` as well (`backgroundStyle` keeps its place) | `label`, `background`, `backgroundStyle` |
| edge | everything but `id` and unknown keys | `id`, `fromNode`, `fromSide`, `toNode`, `toSide`, `fromEnd`, `toEnd`, `color`, `label` |
| the file | — (`nodes` and `edges` keep their place) | `nodes`, `edges` |

Consequences, each observed:

- A node Obsidian made itself has no file order, so it is written `id, x,
  y, width, height, color, type, text` (`file, subpath`; `url`; `label`).
  Read back and saved after an edit, the same node comes out `id, type,
  text, x, y, width, height, color` — Obsidian's own output is not a fixed
  point, which is exactly what the gate vault's canvas showed. The output
  for a file it has read *is* one.
- Unknown keys survive at every level (`pluginTop`, `pluginNode`,
  `pluginEdge` in the probe), in place.
- An edge's `fromEnd: none` and `toEnd: arrow` are defaults and are dropped
  even when the file spelled them out; `fromEnd: arrow` and `toEnd: none`
  are written, after `toSide` and before `color`.
- Reading a file puts the **groups first**, then the rest in file order,
  and the nodes are written in that order; a node created afterwards goes
  on the end in creation order (the probe created text, file, link, group,
  group, text and got them back in that order). A new group gets the label
  `Untitled group`.
- A file with no `edges` key gets one appended after `nodes` on its first
  edited save.

**Unprobed:** a canvas whose node geometry is fractional after a drag at an
odd zoom (the writer formats `0.5` as JavaScript would, but exponent forms
from 1e21 and below 1e-6 are not reproduced), and whether `backgroundStyle`
is written for a group that has no `background`.

## Observed canvas view behavior, Obsidian 1.14.4 (installer build 1.13.7)

Observed on 2026-10-21 over CDP on a scratch vault
(`tools/obsidian-probe/canvas-view.mjs` for the API, command and CSS
dumps, then `canvas-view-probe-a.mjs`, `-b.mjs` and `-c.mjs` for the
measurements and the input events; raw results in
`tools/obsidian-probe/results/2026-10-21-canvas-view.json`). The fixture
held every node type, the six preset colours and a hex colour, three
groups (one coloured, one with a `cover` background) and seven edges with
labels, both arrow directions and no arrows. `silica-canvas` carries the
numbers as constants and `silica::canvas` draws from them.

**The viewport and the leaf.** The canvas is one CSS transform,
`translate(w/2 - x·s, h/2 - y·s) scale(s)` with `s = 2^zoom`: `x` and `y`
are the canvas point at the view's centre and `zoom` is the base-2
logarithm of the scale. Those three are what the leaf records,
`{"type":"leaf","state":{"type":"canvas","state":{"file":"Fixture.canvas","viewState":{"x":123.5,"y":-45.25,"zoom":-0.75}},"icon":"lucide-layout-dashboard","title":"Fixture"}}`,
written on the layout's save. The zoom runs from -4 (scale 1/16) to 1
(scale 2). A plain wheel pans (a 100-unit wheel step moved the view 93
units at scale 1), Shift+wheel pans sideways, Ctrl+wheel zooms by a third
of a doubling per 100 units around the pointer; the middle button drags
the view and so does Space with the left button. Zoom to fit is Shift+1,
zoom to selection Shift+2.

**Chrome.** Top right, 8 px in, four bordered groups of 32 px buttons
(34 px with the 1 px border, radius 4): *Canvas settings*; *Zoom in*,
*Reset zoom*, *Zoom to fit (Shift + 1)*, *Zoom out*; *Undo*, *Redo*;
*Canvas help*. Bottom centre, 16 px up, the card menu: three 48 px
buttons in a 144 by 48 box of radius 4, *Drag to add card*, *Drag to add
note from vault*, *Drag to add media from vault*. Dark theme: group
`#333333` on `#333333`, buttons `#1c1c1c` with `#b3b3b3` glyphs, the card
menu `#282828`; light: white on `#e4e4e4`, glyphs `#5c5c5c`.

**The dot grid.** An SVG pattern of `0.7` px circles in
`--canvas-dot-pattern` (`#333333` dark, `#e4e4e4` light) on
`--canvas-background` (`#1c1c1c`, `#ffffff`), anchored to canvas space.
The pattern is 20 canvas units; it is doubled while it would be smaller
than 20/√2 ≈ 14.14 px on screen (20 px at scale 1 and at 0.5, 14.14 px at
scale 0.177).

**Cards.** Border 1 px plus a 1 px inset ring of the same colour, radius
8, all in canvas units, so they scale with the zoom. A coloured card's
border is its colour at alpha 0.7 and its body is tinted with it at alpha
0.07 over the canvas background; an uncoloured card has a solid
`--canvas-color` border (`#7e7e7e` dark, `#c0c0c0` light) and no tint.
Presets, dark: 1 `#fb464c`, 2 `#e9973f`, 3 `#e0de71`, 4 `#44cf6e`, 5
`#53dfdd`, 6 `#a882ff`; light: `#e93147`, `#ec7500`, `#e0ac00`,
`#08b94e`, `#00bfbc`, `#7852ee`. A hex colour is used as is. Content is
padded 16 units. Selected (`is-focused`): the border solid and a 2 unit
ring of the colour outside it. The resize handles are invisible: 20 unit
strips along each side and 20 unit squares at the corners, which appear
on hover with the four 20 unit connection points at the side midpoints
(no visible dot). Smallest card 50 by 50. A double click on the
background makes a 250 by 60 text card centred on the pointer, unsnapped,
already in edit mode; a file, link or group card made from the menus is
400 by 400; a new group is labelled `Untitled group`.

**Labels.** Everything above a card is drawn in canvas units and
counter-scaled by 1/√scale, so it shrinks slower than the cards. A file
card's name (the page title for a link card) sits at the frame's top
left, 15 units of `--canvas-card-label-color` (`#666666`, `#ababab`),
its bottom 4 units above the frame, at most the card's width, ellipsised;
a text card has none. A group's label is a pill at the same place: 22.5
units of text, padded 4 by 8, radius 4; solid in the group's colour with
black text (white in the light theme) for a coloured group, the default
colour at alpha 0.1 under `--text-muted` for a plain one. An edge's label
is a pill at the curve's midpoint: 20 units of text in 26 of line, padded
6, radius 4, the canvas background behind it, drawn above the cards.

**Edges.** The curve starts 7 units out from the middle of the side and
ends 7 units short of the other; an end without an arrow gets a straight
7 unit stub to the card, an arrow (tip on the card's edge, 10.4 long,
13 wide, pointing in) covers it. The control points sit along the sides'
normals at half the distance between the curve's ends, clamped to
[70, 150] (observed at spans from 46 to 1826 units). Stroke 2/√scale
units (2√scale px on screen), colour the edge's or `--canvas-color`; the
hit area is 24/√scale units wide. A selected edge keeps its stroke.
Dropping a drawn edge on a card records the side nearest the drop point;
dropping it on empty space makes a 250 by 60 text card there and opens
the creation menu (*Add card*, *Add note from vault*, …), and Escape
takes the card and the edge away again.

**Groups.** The body of a group is background: a click there selects
nothing and a drag there is a marquee; the frame and the label select
and drag it. Dragging a group moves the cards wholly inside it (one
straddling the frame stays). Deleting a group deletes only the group.
Creating one around the selection (Ctrl+G, or *Create group* on the
floating menu) appends it to the nodes.

**Selection and keys.** A marquee selects what it crosses (a card half
covered is selected). The view's own scope binds Ctrl+Z, Ctrl+Shift+Z,
Ctrl+Y, Ctrl+A, Shift+1, Shift+2 and the arrows: an arrow nudges the
selection by 20 units, with Shift by 100. Delete and Backspace remove
the selection, Escape ends editing and leaves the card selected, Enter
on a selected card edits it, Ctrl+C puts the selection on the clipboard
as JSON Canvas text and Ctrl+V pastes it (plain text pastes as a new
card), Ctrl+D duplicates. Snapping (grid and objects, both on by default
in the Canvas settings tab) is off while Alt is held. A card emptied
during editing is removed when editing ends; a click outside ends it.

**Menus.** The floating menu above a selection: *Remove*, *Set color*,
*Zoom to selection (Shift + 2)*, then *Edit* for a text or note card;
*Align*, *Edit label*, *Set background* for a group; *Line direction*
(*Nondirectional*, *Unidirectional*, *Bidirectional*) and *Remove label*
or *Edit label* for an edge; *Create group* and *Align* for several
cards. *Set color* opens the default plus the six presets plus a custom
swatch. Right-click on a card: *Zoom to selection*, *Edit*, *Convert to
file...*, separator, *Remove*; on several: *Zoom to selection*, *Create
group*, separator, *Cut*, *Copy*, *Paste*, separator, *Delete*; on an
edge: *Edit label*, *Follow connection*, separator, *Remove*; on the
background: *Add card*, *Add note from vault*, *Add media from vault*,
*Add web page*, *Create group*, separator, *Undo*, separator, *Paste*,
separator, *Snap to grid*, *Snap to objects*, *Read-only*. Ctrl+drag on
the background draws a rectangle and offers *Add card*, *Add note from
vault*, *Add media from vault*, *Add web page*, *Create group* for it.
The item-level lists here were re-observed on 1.14.5 and
corrected: see [[Specification#Observed canvas menu behavior, Obsidian 1.14.5 (installer build 1.13.7)]].
The commands are `canvas:new-file` (*Create new canvas*),
`canvas:jump-to-group`, `canvas:convert-to-file` (*Convert to file...*)
and `canvas:export-as-image`, none with a default hotkey; the last three
are shown only while a canvas is active.

**Zoomed out.** At zoom -1.7 and below (scale 0.3078; the *Zoom
threshold for hiding card content* setting) the wrapper gets
`mod-zoomed-out`: a text card shows a centred `align-left` icon, a note
card its name at 32 px semibold, an image card keeps its image and a
link card its web view.

**Saving.** An edit is written about 2 s later (2011 ms measured); a
viewport change alone writes nothing to the canvas file.

**Silica's divergences.** The resize handles and connection points are
drawn (small dots in the card's colour) rather than invisible; a
selected edge gets a translucent halo; an uncoloured selected card uses
the accent for its ring (not observed); a link card shows its address
rather than a web view and its label is the address rather than the
page title; *Align*, *Set background*, *Edit label*, *Add web page*,
*Read-only*, the snapping toggles, the settings and help buttons and
`canvas:export-as-image` are not there; the card menu's buttons are
clicked, not dragged; an edge dropped on empty space makes its card at
once rather than through the creation menu; the editor in a card draws
at the base font size rather than scaled with the zoom; *Convert to
file* names the note after the card's first line rather than asking.

**Unprobed:** the exact fit padding of Shift+1, the zoom step of the
buttons, snap-to-objects' rules, the creation rectangle of a Ctrl+drag,
and the colour of an uncoloured card's selection ring.

## Observed canvas menu behavior, Obsidian 1.14.5 (installer build 1.13.7)

Observed on 2026-10-25 over CDP — `tools/obsidian-probe/canvas-menus.mjs`, raw
results in `tools/obsidian-probe/results/2026-10-25-canvas-menus.json` — the
re-run the menus section above was waiting on, because the 2026-10-21 capture
read its lists wrong. **1.14.5 nests a menu:** `.menu` holds a `.menu-grabber`
and a `.menu-scroll`, and the rows sit in `.menu-group` children of that, each
tagged with a `data-section` (`canvas`, `open`, `action`, `system`, `danger`,
`create`, `history`, `clipboard`, `selection`). Walking `.menu > *` sees the
two wrappers and nothing else, and reading `.menu-item-title` alone drops
every row without one — which is where 2026-10-21's "two adjacent separators"
came from. The separators are real but paired: of the two DOM nodes at a group
boundary **one is painted (1 px) and one is `display: none`**, so a parity
list carries one separator there. Each target is panned to the middle of the
view and right-clicked on its own DOM rect, with `elementFromPoint` recorded
first, so a click that lands wrong says so.

**The floating row above the selection**, by what is selected (the tooltip is
two lines, `Zoom to selection\n(Shift + 2)`):

| selection | items after *Remove*, *Set color*, *Zoom to selection* |
| --- | --- |
| text card, coloured or not | *Edit* |
| Markdown file card | *Edit* |
| image file card, URL card | — |
| group, coloured or not | *Align*, *Edit label*, *Set background* |
| edge with a label | *Line direction*, *Remove label*, *Edit label* |
| edge with no label | *Line direction*, *Edit label* |
| several nodes, any mix of cards and groups | *Create group*, *Align* |
| several edges | *Line direction* |
| a node and an edge together | — |

*Set color* opens eight swatches: the active default (`#7e7e7e`), the six
presets, and `canvas-color-picker-custom`. *Line direction* opens
*Nondirectional*, *Unidirectional*, *Bidirectional* with `mod-checked` on the
current one. *Align* opens thirteen rows in four painted groups —
*Align left*, *Align center*, *Align right* | *Align top*, *Align middle*,
*Align bottom* | *Arrange in a row*, *Arrange in a column*, *Arrange in a
grid* | *Distribute horizontal spacing*, *Distribute vertical spacing* |
*Justify horizontally*, *Justify vertically* — and the same thirteen for a
single group as for several cards. *Set background* opens no menu at all: it
opens the vault's file suggester ("Type to search...", "No recent files
found."). *Edit label* on an edge opens no menu either; it focuses the edge's
own `.canvas-path-label`, which gains `is-editing` and is `contenteditable`.

**The right-click lists.** A card: *Zoom to selection*, *Edit*, *Convert to
file...*, separator, *Remove* (`danger`). A **Markdown file card**, never
captured before: *Zoom to selection*, *Swap file...*, *Edit*, *Narrow to
heading...*, *Narrow to block...* | *Open in new tab*, *Open in new window* |
*Rename...*, *Move file to...*, *Bookmark...*, *Merge entire file with...* |
*Open in default app*, *Show in system explorer*, *Reveal file in navigation*
| *Remove* | *Copy path* (a submenu). An **image file card** is that list
without *Edit*, *Narrow to…*, *Open in new tab* and *Merge entire file
with...*. A **URL card**: *Zoom to selection*, *Change URL...*, *Open external
link*, *Copy URL*, *Reload page*, separator, *Remove*. A **group's label**:
*Zoom to selection*, *Edit label*, separator, *Remove*. An **edge**: *Edit
label*, *Follow connection*, separator, *Remove*. The **background**: *Add
card*, *Add note from vault*, *Add media from vault*, *Add web page*, *Create
group* | *Undo* | *Paste* | *Snap to grid*, *Snap to objects*, *Read-only*,
the two snap rows checked. *Undo* is conditional: the row is absent in a
session where nothing has been edited yet. **Several objects selected**: *Zoom
to selection*, *Create group*, separator, *Cut*, *Copy*, *Paste*, separator,
*Delete*.

**What a right-click does to the selection.** On an unselected card it moves
the selection to that card and shows the card's own list. Inside a
multi-selection — including a mixed one of a card and an edge — it keeps the
selection and shows the several-objects list. On the background it clears the
selection. Two targets answer with no menu of their own: a **group's body** is
not a hit target at all (`elementFromPoint` is the background there), so the
background list opens with nothing selected; and a right-click **on an edge's
label** opens nothing and leaves the selection empty, while the same edge
right-clicked anywhere else along its path opens the edge list.

**Escape clears the canvas selection.** Worth stating because it invalidates
evidence: a harness that presses Escape to dismiss one menu before opening the
next has deselected everything, and every "with several selected" case it then
records is really a single-card case. The 2026-10-21 captures and three steps
of this run's first pass went that way; `context-multi-escape` holds both
orders side by side.

**What Align and Arrange do**, applied to three cards at `(2600,0,160×80)`,
`(2820,160,240×60)` and `(2660,320,120×140)` — bounding box `x 2600..3060`,
`y 0..460`. Each entry is one undo step, and undo restored the ragged layout
exactly every time. *Align left/right* move every `x` to the box's left or to
`right − width`; *align top/bottom* the same vertically; *align
center/middle* centre every card on the box's centre (`2830`, `230`).
*Arrange in a row* puts every card at the box's top and lays them left to
right with a **20 unit gap**, in the canvas file's node order rather than by
position; *arrange in a column* is the same down the box's left edge; *arrange
in a grid* with three cards is identical to the column, so its shape rule
needs more cards to pin down. *Distribute horizontal spacing* keeps the
extreme cards and equalises the gaps between the rest, and the gap goes
negative when the cards are wider than their span (here `(460 − 520)/2 =
−30`, so the cards overlap); *distribute vertical spacing* likewise (`(460 −
280)/2 = 90`). *Justify horizontally* sets every card's `x` to the box's left
and its `width` to the box's width (460); *justify vertically* does it with
`y` and `height`.

**The quick settings** (the gear in the canvas controls) are *Snap to grid*,
*Snap to objects* — both checked out of the box — and *Read-only*. The two
snap rows are app-wide and persist: turning *Snap to grid* off created
`.obsidian/canvas.json` holding `{\n  "snapToObjects": true,\n  "snapToGrid":
false\n}`, two-space indented with no trailing newline, in a vault that had no
such file. *Read-only* is not written there: it is per view. With it on the
wrapper gains `mod-readonly`, the floating row collapses to *Zoom to
selection* alone and a card's right-click list is *Zoom to selection* and
nothing else, while the card menu stays in the DOM.

**Convert to file...** opens a modal titled *File name* holding a
`textarea.rename-textarea` prefilled `Untitled`, with *Save* and *Cancel*.
**Enter does not submit it** — the modal stays open and nothing is written;
*Save* does the work. It writes `Untitled.md` whose bytes are the card's text
with **no trailing newline** (`Text card`, 9 bytes), and it **replaces** the
text node rather than editing it: the node id changes to a fresh 16 hex
characters, `type` becomes `file` with `file: "Untitled.md"`, the geometry is
kept, and every edge that named the old node is rewritten to the new id with
its own id and label intact. The canvas file is written on the usual ~2 s
debounce.

**No menu offers card order.** *Bring to front*, *send to back* and anything
matching `front|back|raise|lower|order|layer` are absent from the card, group
and several-objects lists, so `Canvas::raise_selection` has no menu home in
1.14.5.

**Unprobed:** the *Copy path* submenu of a file card, what *Swap file...*,
*Narrow to heading...*, *Narrow to block...*, *Merge entire file with...* and
*Change URL...* write, *Set background*'s suggester beyond its empty state,
whether *Edit label* on a group focuses the label the way an edge's does (the
click left focus on the button), and *Arrange in a grid* with more than three
cards.

## Observed editing command behavior, Obsidian 1.14.5 (installer build 1.13.7)

Observed on 2026-10-25 over CDP — `tools/obsidian-probe/editing-commands.mjs`
and `footnote-insert.mjs`, raw results in
`tools/obsidian-probe/results/2026-10-25-editing-commands.json` — closing the
editing half of the backlog's "Declined until Obsidian is probed" list. Each
case seeds the note **through the vault** (`app.vault.modify`), places the
caret, runs the command the way the palette does
(`app.commands.executeCommandById`) and reads the document, the caret and the
selection back. Seeding only the editor is not enough: a command that touches
the file reloads the editor from disk, and the reload puts the previous case's
bytes back underneath the command's own edit. One control proves the editor's
text is the file's bytes — `control line` typed into and saved from the editor
read back from disk as the expected 22 bytes.

Two traps are worth keeping. A second markdown leaf makes
`executeCommandById` act on a view the probe is neither seeding nor reading,
and every case then reads back as an untouched no-op while still reporting
that it ran; and a file-change event landing after `setCursor` moves the caret
under the command, which is how one attempt at the footnote rows produced a
reference inside a definition line and tab-indented definitions.

**`editor:set-heading` does nothing.** "Toggle heading" is a real palette
entry whose check passes, and it changes nothing. Twelve shapes — a plain
line, `#` through `######`, a line in a blockquote, `> # quoted h1`, a bullet,
a task, an empty line, a word selection, a two-line selection — came back
byte-identical with the caret where it started, in Live Preview and in source
mode, and ten presses in a row on one line never moved it. Driven three ways:
the palette, a hotkey bound for the probe (Alt+H), and the command's own
`editorCallback` called with an explicit editor and view. In the same runs
`editor:set-heading-0` turned `# one` into `one` and `editor:set-heading-3`
turned `plain text` into `### plain text`, so the dispatch was live, and
running `set-heading-2` first did not give the bare toggle a level to toggle.
Registering the id and doing nothing is therefore parity.

**`editor:cycle-list-checklist` is a three-state cycle, and it discards custom
statuses.** On one line, each press advances bullet → `- [ ] ` → `- [x] ` →
bullet, and a line that is not a list item becomes a bullet first:

| Start | 1st | 2nd | 3rd |
| --- | --- | --- | --- |
| `plain text` | `- plain text` | `- [ ] plain text` | `- [x] plain text` |
| `- bullet` | `- [ ] bullet` | `- [x] bullet` | `- bullet` |
| `- [ ] open` | `- [x] open` | `- open` | `- [ ] open` |
| `- [x] done` | `- done` | `- [ ] done` | `- [x] done` |
| `- [?] q` | `- q` | `- [ ] q` | `- [x] q` |
| `> quoted` | `> - quoted` | `> - [ ] quoted` | `> - [x] quoted` |
| *(empty line)* | `- ` | `- [ ] ` | `- [x] ` |

So it is **not** the path to `[?]`, `[-]` and `[>]` that
[[#Observed task checkbox behavior, Obsidian 1.14.4 (installer build 1.13.7)]]
suspected: `- [?] q` loses its status on the first press exactly as a rendered
click does. The list marker is kept as written — `* star` becomes `* [ ] star`,
`+ plus` becomes `+ [ ] plus`, `1. ordered` becomes `1. [ ] ordered` — so the
cycle differs inside a numbered list only in that the `1. ` stays. Indentation
is kept (`\t- two` cycles in place), and the other lines of a list are never
touched.

Across a multi-line selection the lines move together and the first press
normalises them. Where every selected line is in the same state, all advance
one step; where the selection holds a line that is not a list item, that line
becomes a bullet and the list lines are pulled back to bullet with it:

| Selection | 1st | 2nd | 3rd |
| --- | --- | --- | --- |
| `a` / `b` | `- a` / `- b` | `- [ ] a` / `- [ ] b` | `- [x] a` / `- [x] b` |
| `- a` / `- b` | `- [ ] a` / `- [ ] b` | `- [x] a` / `- [x] b` | `- a` / `- b` |
| `a` / `- b` | `- a` / `- b` | `- [ ] a` / `- [ ] b` | `- [x] a` / `- [x] b` |
| `a` / `- [ ] b` | `- a` / `- b` | `- [ ] a` / `- [ ] b` | `- [x] a` / `- [x] b` |
| `a` / `- b` / `- [ ] c` | all `- ` | all `- [ ] ` | all `- [x] ` |

Two selections do not converge. `- a` / `- [ ] b` oscillates between
`- [ ] a` / `- b` and `- a` / `- [ ] b` for ever, and `- a` / *(blank)* /
`- [x] b` oscillates the same way — a selection of list lines that differ only
in whether they carry a checkbox inverts each line's checkbox rather than
advancing it, so pressing again swaps them back. Both rows look like Obsidian
applying the later line's edit at an offset the earlier line's edit has
already shifted; the later line always loses exactly the four characters
`[ ] `. Silica implements the converging rule above and does not copy the
oscillation.

**`editor:toggle-checklist-status` (Mod+L) inserts a checkbox on a line that
has none.** This is the row the backlog had the wrong way round: the command
is not confined to flipping an existing box. On one line it adds `- [ ] `
after any existing list marker, after a blockquote's `> `, and after leading
whitespace, and leaves everything else on the line alone:

| Start | After Mod+L |
| --- | --- |
| `plain text` | `- [ ] plain text` |
| `- bullet` | `- [ ] bullet` |
| `1. ordered` | `1. [ ] ordered` |
| `> quoted` | `> - [ ] quoted` |
| `# heading` | `- [ ] # heading` |
| `    indented` | `    - [ ] indented` |
| *(empty line)* | `- [ ] ` |
| `- [ ] open` | `- [x] open` |
| `- [x] done` | `- [ ] done` |
| `- [?] q`, `- [-] dash` | `- [ ] q`, `- [ ] dash` |

The caret moves with the inserted text (`plain text` at ch 3 ends at ch 9),
and a second press flips the new box, so `plain text` round trips
`- [ ] ` ↔ `- [x] `. The real keystroke agrees with the palette: Mod+L is the
command's only default hotkey.

Across a selection the decision is taken for the whole selection, and blank
lines are ignored:

| Selection | Result |
| --- | --- |
| every line has a checkbox | flip the status: check all unless all are checked, then uncheck all (`- [ ] a` / `- [x] b` → both `- [x] `) |
| no line has a checkbox | add `- [ ] ` to all (`- a` / `- b`, and `a` / `b`) |
| some lines have one | invert each line's checkbox: the ones that have it lose it, the ones that do not gain `- [ ] ` |

The third row oscillates with period two the same way the cycle command does,
and for the same reason.

**The insert-block commands.** All six write their markup at or around the
caret, and only `insert-callout` leaves a selection behind:

| Command | On a blank line | On a non-blank line |
| --- | --- | --- |
| `insert-callout` | a blank line, `> [!NOTE] Title`, `> Contents`, a blank line; caret at ch 8 with `NOTE` **selected** | wraps the line: `> [!NOTE]` then `> ` and the line's text, no Title/Contents, `NOTE` selected |
| `insert-codeblock` | ```` ``` ````, an empty line, ```` ``` ````; caret on the empty line | wraps the line in a fence, caret keeping its column on the body line |
| `insert-mathblock` | the same shape with `$$` | the same wrap with `$$` |
| `insert-horizontal-rule` | a blank line, `---`, a blank line; caret on the line after `---` | pushes the rule below the line (`hello world` → `hello world\n\n---\n`), and **splits the line** when the caret is mid-line (`hello| world` → `hello\n\n---\n world`) |
| `insert-tag` | a bare `#`, caret after it | a bare `#` at the caret, caret after it; no suggester opened |
| `insert-footnote` | `[^n]` at the caret and `[^n]: ` appended at the end of the document after a blank line | the same |

On an empty document `insert-callout`, `insert-horizontal-rule` and
`insert-table` each write a leading blank line; `insert-codeblock` and
`insert-mathblock` do not. A selection is ignored by all six — the caret's
line is what they act on, and `alpha` selected in `alpha beta` gives
`> [!NOTE]\n> alpha beta`, not a callout around `alpha`.

`insert-footnote`'s number is **the highest existing numeric label plus one**,
and the definition always lands at the end of the document. Read from disk,
one case per Obsidian launch because the command only lands once per launch
when the probe reaches the file with `openFile`:

| File | Result |
| --- | --- |
| `body text` | `body text[^1]` + `\n\n[^1]: \n` |
| `body[^1] text` with `[^1]: one` | `body[^1] text[^2]` + `\n\n[^2]: \n` |
| `a[^1] b[^2] c` with both definitions | `[^3]` |
| `a[^2] b` with `[^2]: two` | `[^3]` — max plus one, not count plus one |
| `a[^note] b` with `[^note]: named` | `[^1]` — a named label does not take a number |
| `# Head` / `para one` / `para two`, caret in `para one` | `para one[^1]`, definition still at the very end |
| `- item one` | `- item one[^1]`, definition at the end, not indented |

The caret ends immediately after the inserted reference, not in the
definition.

**The six highlight colours write a colour emoji inside the `==` pair.**
`editor:toggle-highlight-<colour>` wraps the selection in `==` and puts one
emoji directly after the opening pair:

| Command | Writes | Emoji |
| --- | --- | --- |
| `editor:toggle-highlight` | `==word==` | — |
| `editor:toggle-highlight-red` | `==🔴word==` | U+1F534 |
| `editor:toggle-highlight-orange` | `==🟠word==` | U+1F7E0 |
| `editor:toggle-highlight-yellow` | `==🟡word==` | U+1F7E1 |
| `editor:toggle-highlight-green` | `==🟢word==` | U+1F7E2 |
| `editor:toggle-highlight-blue` | `==🔵word==` | U+1F535 |
| `editor:toggle-highlight-purple` | `==🟣word==` | U+1F7E3 |

The reader consumes the emoji as the colour and does not show it: the
rendered element is `<mark data-highlight="red">word</mark>`, computed
background `oklch(0.659737 0.216875 23.8105 / 0.3)` against the probe's dark
theme. Running the same colour again removes the whole highlight, a different
colour replaces the emoji in place (🔴 → 🔵), and a colour over an existing
plain `==word==` inserts the emoji whether the selection is the inner word or
spans the marks. With no selection the command highlights the word under the
caret. A multi-line selection is wrapped **per line**:
`==🟡one==` / `==🟡two==` / `==🟡three==`.

**`editor:clear-formatting` strips every marker in the selection, line
prefixes included.** It needs a selection: with a bare caret, inside a bold
span or on a heading line, it runs and changes nothing.

| Selection | Result |
| --- | --- |
| `**bold _italic_ text**` | `bold italic text` |
| `_italic_ ` inside that line only | `**bold italic text**` |
| `**==both==** tail` | `both tail` |
| `~~gone~~ and ` + "`code`" | `gone and code` |
| `## Heading text` | `Heading text` |
| `> quoted text` | `quoted text` |
| `- **bold** item` | `bold item` |
| `$x^2$ #tag` | `x^2 tag` |
| `**one**` / `_two_` / `==three==` | `one` / `two` / `three` |
| `see [[Other\|alias]] end` | unchanged — a wikilink is not formatting |
| `nothing to clear` | unchanged |

So it removes emphasis, highlight, strikethrough, inline code, inline math and
the `#`/`>`/`-` of a line, and leaves links alone.

**Move line up and down have no default hotkey.** `app.hotkeyManager`'s
default table holds 41 ids and neither `editor:swap-line-up` nor
`editor:swap-line-down` is among them; the Hotkeys tab filtered to "Move line"
shows both rows with the chord column reading `Blank`. Alt+Up and Alt+Down
pressed in the editor change nothing. The commands themselves work — `one` /
`two` / `three` with the caret on line 1 gives `two` / `one` / `three` up and
`one` / `three` / `two` down, a selection moves every line it covers
(`two` / `three` / `one` for up over lines 1-2), and the command is a no-op at
the first and last line. Leaving the pair unbound is parity.

**The formatting toggles wrap each line of a multi-line selection
separately**, except the comment toggle:

| Command | `one` / `two` / `three` selected whole |
| --- | --- |
| `toggle-code` | `` `one` `` / `` `two` `` / `` `three` `` |
| `toggle-inline-math` | `$one$` / `$two$` / `$three$` |
| `toggle-bold` | `**one**` / `**two**` / `**three**` |
| `toggle-italics` | `*one*` / `*two*` / `*three*` |
| `toggle-strikethrough` | `~~one~~` / `~~two~~` / `~~three~~` |
| `toggle-comments` | `%%one` / `two` / `three%%` — **one** pair around the whole selection |
| `toggle-bullet-list` | `- one` / `- two` / `- three` |
| `toggle-numbered-list` | `1. one` / `2. two` / `3. three` |
| `toggle-blockquote` | `> one` / `> two` / `> three` |

So Silica's single backtick for `toggle-code` and single `$` for
`toggle-inline-math` across lines are wrong: both wrap per line. A selection
that starts and ends mid-line still takes the inline markers at the selection
edges (`o` + `` `ne` `` / `` `two` `` / `` `th` `` + `ree`), while the line
toggles ignore the edges and prefix whole lines. Running a toggle twice
restores the original bytes in every row. With no selection the inline
toggles take the word under the caret (`one two` at ch 4 gives
`one **two**`) and the line toggles take the line.

A blank line inside the selection is **skipped** by the bullet and numbered
toggles (`one` / *(blank)* / `three` gives `- one` / *(blank)* / `- three`,
and `1. one` / *(blank)* / `2. three` — the numbering counts only the lines it
writes), but the blockquote toggle prefixes it too (`> one` / `> ` /
`> three`), and the inline toggles wrap the non-blank lines only.

The mixed-selection rule for the line toggles is **remove only when every
line already has the prefix**, which is what Silica does:

| Selection | `toggle-bullet-list` / `-numbered-list` / `-blockquote` |
| --- | --- |
| `- one` / `two` / `- three` | `- one` / `- two` / `- three` — the bare line gains one |
| `- one` / `- two` / `- three` | all three lose it |
| `> one` / `two` / `> three` | all three quoted |
| `> one` / `> two` | both unquoted |
| `1. one` / `2. two` | both lose the number |
| `a` / `b` / `c` / `d` | `1.` `2.` `3.` `4.` — numbering always starts at 1 |
| `1. one` / `a` / `b` with only the last two selected | `1. one` / `1. a` / `2. b` — it does not continue the list above |

Indentation survives: `- one` / `\t- two` / `three` toggled gives
`one` / `\ttwo` / `- three`, i.e. each line is treated on its own.
`toggle-comments` over `%%one%%` removes the pair, and over the inner lines of
a `%%` block leaves the block alone and adds a second pair.

**`editor:insert-table` on a non-blank line puts the table below it.** The
blank-line case the spec already recorded is unchanged; every non-blank
position gives the same answer — the line is kept, a blank line follows it,
then the two-by-two table, then a trailing newline — and the caret lands in
the first header cell (ch 1 of the first table line):

| Caret | Result |
| --- | --- |
| end of `hello world` | `hello world` / *(blank)* / the table |
| middle of `hello world` | the same; the line is **not** split |
| start of `hello world` | the same |
| end of the last paragraph of a note | the table after it |
| `- item`, `> quoted` | the table after the line, not inside the list or quote |
| a selection | the selection is ignored, the table goes after the line |
| empty document | a leading blank line, then the table |

Source mode gives the same bytes as Live Preview. So Silica's "blank line
before, exactly one newline after" is right, and the only correction is that
a mid-line caret must not split the line.

**Shift+Tab removes exactly one indent unit of display columns, clamped at
zero** — it does not dedent to the previous multiple, which is what the
backlog assumed. Observed on 2026-10-25 with real keystrokes
(`tools/obsidian-probe/editing-typing.mjs` step `s08`), reopening the note
after each config change because `setConfig` alone does not rebuild the
editor's indent extension, and calibrating what one Tab inserts before each
block of rows.

A tab character counts as `tabSize` columns, and the dedented indentation is
re-emitted as tabs then spaces when `useTab` is on:

| Line | `tabSize: 2`, `useTab: true` | `tabSize: 4`, `useTab: true` |
| --- | --- | --- |
| `   - b` (3 spaces) | ` - b` | `- b` |
| `  - b` (2) | `- b` | `- b` |
| ` - b` (1) | `- b` | `- b` |
| `     - b` (5) | `\t - b` (3 columns) | ` - b` |
| `\t- b` | `- b` | `- b` |
| `\t - b` | ` - b` | ` - b` |
| `   plain` (not a list) | ` plain` | `plain` |
| `- a` (top level) | unchanged | unchanged |

So Silica's `min(tabSize, leading spaces)` is already the right arithmetic for
a space-indented line; what it has to add is that a tab counts `tabSize`
columns and that the result is written back as tabs plus spaces.

With **`useTab: false` the unit is four spaces whatever `tabSize` says**. The
calibration row is the evidence: with `tabSize: 2` and `useTab: false` one Tab
inserts four spaces, not two, and Shift+Tab removes four columns — `   - b`
goes to `- b` and `     - b` to ` - b`, the same as `tabSize: 4`. The same
session's `useTab: true` rows did honour `tabSize: 2`, so the config had
reached the editor.

Tab is the mirror image and always prepends one unit to the existing
indentation rather than normalising it: `   - b` with `useTab: true` becomes
`\t   - b`. `editor:unindent-list` run from the palette gives the same bytes
as Shift+Tab on every shape above, so the command and the keystroke are one
behavior.

## Observed task checkbox behavior, Obsidian 1.14.4 (installer build 1.13.7)

Observed on 2026-10-18 with a throwaway plugin in a scratch vault (see
[[Development#The probe batch, 2026-10-18]]), clicking each
`input.task-list-item-checkbox` in the **reading view** with `el.click()` and
reading the note back as raw bytes after every click.

**Any status but a space becomes a space.** The note started as
`- [ ] t0`, `- [x] t1`, `- [X] t2`, `- [?] t3`, `- [-] t4`, `- [>] t5`, and
six clicks, one per line, each changed exactly that line and left the other
five byte-identical:

| Clicked | Line before | Line after |
| --- | --- | --- |
| 0 | `- [ ] t0` | `- [x] t0` |
| 1 | `- [x] t1` | `- [ ] t1` |
| 2 | `- [X] t2` | `- [ ] t2` |
| 3 | `- [?] t3` | `- [ ] t3` |
| 4 | `- [-] t4` | `- [ ] t4` |
| 5 | `- [>] t5` | `- [ ] t5` |

So the click is a two-state toggle writing the literal three bytes `[x]` or
`[ ]`, not a cycle through the statuses, and a custom status is **not**
preserved: `[X]` does not come back capitalised, and `[?]`, `[-]` and `[>]`
are all lost. Silica does exactly this already (`Editor::click_rendered`),
so the apparent data loss is parity, not a defect.

The rendered markup carries the status character even though the click
discards it:

| Source | `li` class | `li` `data-task` | input |
| --- | --- | --- | --- |
| `- [ ] t0` | `task-list-item` | `""` | `<input type="checkbox" class="task-list-item-checkbox">` |
| `- [x] t1` | `task-list-item is-checked` | `x` | same, with `checked` |
| `- [X] t2` | `task-list-item is-checked` | `X` | same, with `checked` |
| `- [?] t3` | `task-list-item is-checked` | `?` | same, with `checked` |
| `- [-] t4` | `task-list-item is-checked` | `-` | same, with `checked` |
| `- [>] t5` | `task-list-item is-checked` | `>` | same, with `checked` |

Every non-space status is `is-checked` and carries the `checked` attribute,
and the `ul` is `contains-task-list`. The two shapes the block parser rejects
rendered as plain text with no input at all, confirming the parser's rule
from the rendered side: `- [ ]Not a task` (no space after the bracket) and
`- [ ]` alone at end of line.

No configuration key governs any of this. `taskStatus`, `taskStatuses`,
`customTaskStatus`, `checkbox`, `checkboxes`, `cycleTaskStatus`,
`alternateCheckbox`, `completedTaskCharacter` and `taskCompletionText` all
read undefined.

Two commands exist for statuses beyond the click, both already in
`silica-commands/data/commands.txt` with their observed labels:
`editor:toggle-checklist-status` ("Toggle checkbox status") and
`editor:cycle-list-checklist` ("Cycle bullet/checkbox"). Both were run on
2026-10-25 and are recorded in
[[#Observed editing command behavior, Obsidian 1.14.5 (installer build 1.13.7)]].
Neither reaches `[?]`, `[-]` or `[>]`: the cycle command discards a custom
status on its first press exactly as the rendered click does, so **no path in
Obsidian writes those three** — they can only be typed.

**Unprobed:** Live Preview: the matrix step returned `null` for all
six rows and read no bytes, so clicking a checkbox in Live Preview is
unmeasured here (Silica's own Live Preview toggle is covered by
[[Development#The checkbox the padding hid, 2026-10-18]]). Repeated toggling
of one checkbox: the step that tried it reported the DOM reacting with the
file never changing, contradicting the matrix above, so round-tripping a
checkbox is unsettled. Also unprobed: whether a theme or CSS can make a
custom status survive, whether a checkbox inside an embedded note writes to
the embedded file, and what a click does when the note is open and dirty in
another pane.

## Observed heading fold behavior, Obsidian 1.14.4

Observed on 2026-10-18 with a throwaway plugin in a scratch vault, Obsidian
1.14.4 (installer build 1.13.7), folding one 33-line note (`Fold Probe.md`,
450 bytes, frontmatter plus an `#`/`##`/`###` tree, a list and a footnote
definition) and reading the fold record, the DOM and the vault back after
every step. All fourteen steps of the run completed; nothing below rests on a
failed step. Every editor observation was made in **Live Preview** — the view
reported `{"mode":"source","source":false}` and the container class read
`is-live-preview is-folding`. Raw Source mode was never exercised.

A fold is recorded as line ranges, zero-based and inclusive at both ends:
`{"folds":[{"from":11,"to":18}],"lines":33}`. `lines` is the document's line
count including the empty line after the final newline (the 450-byte note
splits into 33 lines).

What `editor:toggle-fold` folded, against the source line numbering:

| Line folded | Range recorded |
| --- | --- |
| `## Section A` (11), next heading of the same level `## Section B` on 19 | `11`-`18` — the trailing blank line 18 included |
| `### Sub A1` (15), the last thing in Section A | `15`-`18` |
| `## Section B` (19), a list in its body | `19`-`26` |
| `## Section C` (27), the last heading | `27`-`32` — to the final line, the `[^1]:` definition on 31 and the empty line 32 included |
| `# H1 Top` (7) | `7`-`32` — the nested `##`/`###` headings inside it included |
| `- list parent` (23) with two indented children | `23`-`25` — the blank line 26 is *not* part of a list fold, unlike a heading fold's trailing blank |
| `PREAMBLEBODY ...` (5), a paragraph before any heading | command returns `true`, nothing folds |
| `---` (0), the frontmatter's opening fence | command returns `true`, nothing folds |

So the extent is the heading line through the line before the next heading of
the same or a higher level, trailing blanks included, and the last heading
folds to the end of the file.

In the editor the folded heading line is still rendered with `…` appended
(element class `cm-foldPlaceholder`); the folded lines are gone from the
rendered line list. The placeholder line still follows Live Preview's
cursor-reveal rule: with the cursor on it the line reads `## Section A…`, and
after a switch away and back it reads `Section A…`. Each foldable line also
carries a zero-width `div.cm-fold-indicator` (inline-block, `position:
relative`) paired with a sibling `div.collapse-indicator.collapse-icon` at
`opacity: 0`, `position: absolute`, `left: -15px` on the heading lines
captured and `-10px` on the list line; the folded line's indicator gains
`is-collapsed`. The capture returned five such pairs for a note with six
foldable lines, plus one unpaired `collapse-indicator` at `left: -22px` above
the first heading, so it is not established to be a complete listing.

**Nothing is written to the note.** The 450 bytes are byte-identical after
the fold and after the whole run, and no file in the vault's listing was
added, changed or removed. The record lives in `localStorage`, under the key
`5c0r1ac0rpu50001-note-fold-Fold Probe.md` — the app id, `-note-fold-`, then
the vault-relative path. Only this one root-level note was observed, so how a
path in a subfolder is spelled in the key is not known. The vault diff the
run took does not cover `.obsidian/` (its file listing holds no dotfiles),
and the probe's own `setConfig` call did rewrite `.obsidian/app.json` during
the run, so "a fold touches no vault byte" is established for the note and
the vault's non-dot files only.

The fold survives leaving the note and coming back in the same session: the
record is written on leaving and the section is still folded on return.

In reading view:

| Trait | Observed |
| --- | --- |
| Sharing | the editor's fold collapses the same section in reading view, and returning to the editor shows it still folded |
| Record shape | merely entering reading view rewrote `{11,18}` to `{11,12}`, before any click; all four reading-view folds took the `{h, h+1}` form — `{7,8}`, `{11,12}`, `{19,20}`, `{27,28}` |
| Back in the editor | the same fold reads at its full extent again (`{11,18}`), so a stored `to` is not authoritative for the editor |
| Collapsed content | absent from the preview DOM, not hidden: the wrapper list runs `el-p` (TOPBODY) → `el-h2 is-collapsed` (Section A) → `el-h2` (Section B), with no wrapper for the section's paragraph, its `### Sub A1` heading or that heading's body |
| Extent | collapsing `# H1 Top` hid everything below it and left the paragraph *above* it visible; `## Section B` hid its body and the list; `## Section C` hid its body and the footnote definition |
| Indicator | `span.heading-collapse-indicator.collapse-indicator.collapse-icon`, the heading element's first child, `position: absolute`, `left: 31.9935px`, `opacity: 0` expanded and `opacity: 1` plus `is-collapsed` collapsed; the glyph is an inline `svg.svg-icon.right-triangle` with path `M3 8L12 17L21 8` |
| Classes | the `<h1>`/`<h2>` itself keeps an empty class; `is-collapsed` lands on the indicator span and on the wrapper div (`el-h2 is-collapsed`) |
| Clicking the indicator | toggles, both ways — clicking an already-collapsed one expanded it |

One click on heading *text* (an `H2`; which heading is not recorded) changed
nothing. That is one observation, not a rule.

The six fold command ids, with the names the registry reported, all queried
by id rather than enumerated:

| Id | Name |
| --- | --- |
| `editor:toggle-fold` | Toggle fold on the current line |
| `editor:fold-all` | Fold all headings and lists |
| `editor:unfold-all` | Unfold all headings and lists |
| `editor:fold-more` | Fold more |
| `editor:fold-less` | Fold less |
| `editor:toggle-fold-properties` | Toggle fold properties in current file |

Each reports `hotkeys: []` for itself. The five editor ones all pass their
check callback with a markdown view focused in Live Preview.

`editor:fold-all` folded everything in one pass, leaving three rendered lines
and the set `{0,0}`, `{2,3}`, `{7,32}`, `{11,18}`, `{15,18}`, `{19,26}`,
`{23,25}`, `{27,32}` — one range per heading and per list parent, plus
`{2,3}`, which covers the indented YAML list under `tags:` on source lines 2
and 3, and `{0,0}`. No range covers the frontmatter block (lines 0-4), so the
frontmatter is not shown to fold. `editor:unfold-all` cleared every heading
and list fold; in three invocations the set ended at exactly `[{0,0}]` each
time. The `{0,0}` entry is unexplained — it is never correlated with hidden
content, and it reaches storage: the run's final record read
`{"folds":[{"from":0,"to":0},{"from":19,"to":20}],"lines":33}`.

With `.obsidian/app.json` holding literally `{}`, `foldHeading` and
`foldIndent` both read `true` and folding worked, so both default to on.
With `foldHeading` set to `false` through the API: the editor DOM query for
fold indicators came back empty (it returned nothing for the list indicator
either, which `foldIndent: true` should still produce), and
`editor:toggle-fold` returned `true` and folded nothing — the step records no
target line, so the toggle is not shown to have been aimed at a heading. In
reading view the three headings captured still carried the indicator span,
computed `display: none` with a zero rect, and a scripted click on that
hidden indicator still collapsed the section: the machinery stays live, the
affordance is hidden.

One incidental: writing `foldHeading` through the API did not reach
`.obsidian/app.json` at once. When the app read `false`, the file still read
`{}`; after restoring the in-app value to `true`, the file read
`{"foldHeading": false}` — the file trailing the in-memory value by one
change. One observation, in a run whose cleanup did not restore the config.

**Unprobed as of 2026-10-18** — the 2026-10-25 pass below answers most of
this paragraph: what the editor does on a cold load with a record the reading
view wrote (`{h, h+1}`) — the app was never restarted, and merely rendering
the note in preview rewrites the stored record, so a cold editor load after a
reading-view session is the likely case and is unobserved; whether fold state
survives a restart at all, and whether anything outside `localStorage` holds
it — `foldManager` exposes `loadPath`, `savePath`, `load` and `save`, none of
which was traced to a file, and the step's `.obsidian` listing came back
empty from a directory we know holds `app.json`, so it is not evidence of
absence; whether `.obsidian/` is untouched by a fold; what `{0,0}` is;
`editor:fold-more` and `editor:fold-less`, invoked four times between them,
each returning `true` with the fold set and the rendered lines unchanged and
the cursor position unrecorded — a no-op observation, not their semantics;
`editor:toggle-fold-properties`, never invoked; keyboard folding, since
`hotkeys: []` is each command's own report and no key was pressed; folding
with raw Source mode on; `foldIndent: false`; the `localStorage` key for a
note in a subfolder; whether a click on heading text ever toggles a fold in
reading view, and whether any user-visible affordance remains there when
`foldHeading` is `false` (only a synthetic click on the `display: none`
indicator was tried); what navigation the jump step performed — it recorded
only that the cursor ended at line 19 column 0 with the section unfolded in
the live fold set while the saved record still carried `{19,26}`, so "opening
a link into a folded section unfolds it" is not established; whether folding
a heading while another fold is already open removes only its own record (the
one expand observed had no other entry left to leave alone); and folding for
headings inside callouts or blockquotes, setext headings, a heading on the
first line of a file, a file with no trailing newline, and an empty last
section.

### The 2026-10-25 pass: the cold restart and the rest of the block

`tools/obsidian-probe/heading-fold.mjs` against **Obsidian 1.14.5**, results in
`tools/obsidian-probe/results/2026-10-25-heading-fold.json`. The `run` phase
ends by quitting the app so `run-block.sh` relaunches it and an
`after-restart` phase can read what survived. The fixture is the same shape of
note, one line shorter (32 lines, no `lines: 33`), so the line numbers below
are its own.

**The subfolder key has the same shape as a root note's.** Folding in
`Sub Folder/Fold Probe.md` wrote
`<appId>-note-fold-Sub Folder/Fold Probe.md` beside the root note's
`<appId>-note-fold-Fold Probe.md`. So the key is the app id, `-note-fold-`,
then the vault-relative path with `/` separators, original case and the `.md`
extension — no escaping of the space. (localStorage is shared across every
vault the app has opened, so a dump holds other vaults' keys too; the saved
evidence is filtered to this probe's id.)

**`{0,0}` is the properties fold.** `editor:toggle-fold-properties` is what
owns it: three runs in a row gave `folds: []`, `[{0,0}]`, `[]`. Nothing else
in the fold set moved, and the rendered first lines were unchanged in both
states because Live Preview draws the properties widget either way. So the
entry that "is never correlated with hidden content" is the frontmatter's own
record, and it is present by default.

**`fold-more` and `fold-less` walk outward and back.** With the caret in
`## Section A`, each `fold-more` adds the next enclosing level and each
`fold-less` removes the outermost:

| Press | Fold set | Rendered lines |
| --- | --- | --- |
| start | `{0,0}` | 27 |
| fold-more ×1 | `{0,0}`, `{12,19}` — the caret's own section | 20 |
| fold-more ×2 | `+ {8,31}` — the enclosing `# H1 Top` | 4 |
| fold-more ×3, ×4 | unchanged | 4 |
| fold-less ×1 | `{0,0}`, `{12,19}` | 20 |
| fold-less ×2 | `{0,0}` | 27 |
| fold-less ×3, ×4 | unchanged | 27 |

**A fold still writes nothing to the vault, `.obsidian/` included.** The note
was byte-identical afterwards and `.obsidian/` held only the four files the
fixture and the app had already written (`app.json`, `appearance.json`,
`core-plugins.json`, `workspace.json`). The record is localStorage only.

**Raw source mode folds exactly like Live Preview.** With `source: true` the
same `editor:toggle-fold` on `## Section A` recorded `{12,19}`, left 25
rendered lines, and the DOM carried 7 `.cm-fold-indicator`s and 8
`.collapse-indicator`s. So folding is not a Live Preview feature.

**`foldIndent: false` turns off list folding only.** With it off,
`editor:toggle-fold` on the list parent returned `true` and folded nothing,
while the same command on `## Section A` still recorded `{12,19}`.

**In reading view only the indicator is clickable.** Real clicks: on the
heading's **text** — nothing; on the **empty space** to its right — nothing;
on the `.heading-collapse-indicator` — toggles, and toggles back. The record
it writes is the `{h, h+1}` form (`{12,13}`), as the earlier pass found.
Hovered, the indicator computes `opacity: 1`, `display: flex`,
`position: absolute`, `left: 31.9935px`.

**The edge headings.** Caret on the line, `editor:toggle-fold`, in Live
Preview:

| Line | Folds? |
| --- | --- |
| `# Line zero heading` on line 0 | yes, `{0,3}` |
| a setext heading's **text** line | yes, `{4,16}` |
| a setext heading's `======` line | no |
| `> ## Heading in a callout` | **no** |
| `> ## Heading in a blockquote` | **no** |
| `## Empty Last Section`, last line, file with no trailing newline | **no** |

The command returns `true` in every one of those cases, folding or not, and
the file stayed byte-identical (195 bytes, no trailing newline).

**There is no keyboard path to folding.** All six fold commands report no
default hotkey, and `Ctrl+Shift+[`, `Ctrl+Shift+]` and `Ctrl+Alt+[` pressed in
the editor with the caret on a heading changed neither the fold set nor the
rendered line count.

**A link into a folded section unfolds it.** With `{20,25}` folded,
`openLinkText('Fold Probe#Section B')` left the fold set at `{0,0}` — the
section's own entry is gone — with the caret at line 20 column 0 and all 27
lines rendered. So the jump expands rather than scrolling to a placeholder.

**The fold survives a restart, and the editor re-derives the extent.** Folding
`## Section B` in the editor, switching to reading view (which rewrote the
record to `{20,21}`) and quitting left `{0,0},{20,25}` in localStorage — the
editor's full extent, not the reading view's pair. After the relaunch the note
reopened with the fold restored: 22 rendered lines against 27 unfolded, one
`cm-foldPlaceholder` reading `…`. Switching to reading view rewrote `{20,21}`
again and switching back read `{20,25}`, so the editor re-derives the extent
from the heading every time and a stored `to` is advisory.

**Unprobed after this pass:** whether folding a heading while another fold is
open leaves the other record alone (still only one fold at a time was
expanded); the `foldHeading: false` affordance question, which the earlier
pass left on a synthetic click; and whether `foldManager`'s `save`/`load` ever
reach a file rather than localStorage.

## Observed code-block copy behavior, Obsidian 1.14.4 (installer build 1.13.7)

Observed on 2026-10-18 with a throwaway plugin in a scratch vault: notes
written byte-exactly through the vault API, then every `copy-code-button` in
the rendered container clicked while `navigator.clipboard`, Electron's
`clipboard` and the `copy` event were tapped, and the clipboard read back.
The version numbers are `apiVersion` and `installedVersion`; `appVersion` was
null in this build, so the app version was never captured.

Two counting caveats that the rest of this section already accounts for. The
scanned container held **both** views — it opens with
`div.markdown-source-view … is-live-preview style="display: none"` — so of the
ten `<pre>` elements found in the probe note, nine are reading view and the
tenth is the editor's own live-preview callout widget (a byte-identical
duplicate of the reading-view callout block, with an all-zero rect). Ten
clicks therefore produced nine distinct payloads, not ten. And every click was
dispatched programmatically on a button whose computed `display` was `none`;
the hover → reveal → click path a person takes was never exercised.

The markup:

- Every `<pre>` the Markdown renderer produces carries exactly one
  `<button class="copy-code-button">`, appended as the pre's last child,
  after the `<code>`. Ten of ten pres in the probe note had one.
- `class` is the button's only attribute: no `aria-label`, no `title`, no
  text, no `tabindex` (`tabIndex` reads 0, which is the browser default for
  `<button>`, not an Obsidian choice). Its sole child is an
  `svg.svg-icon.lucide-copy`.
- The renderer adds it, not the reading-view leaf: a detached
  `MarkdownRenderer.render` that was never attached to a view already had a
  button on all three of its pres, the callout block and a bare
  `<pre><code>indented block\n</code>` included.
- Reading view shows no language name anywhere on the block: `::before` and
  `::after` on both `pre` and `code` compute to `content: none`, and the
  flair-element scan came back empty on every pre in both scanned notes.

The four `app.css` rules that matched, plus the two mobile overrides:

| Selector | Declarations |
| --- | --- |
| `.markdown-rendered pre:not(:hover) > button.copy-code-button` | `display: none` |
| `.markdown-rendered button.copy-code-button` | `margin: 6px; padding: 6px 8px; height: auto; background-color: transparent; box-shadow: none; color: var(--text-muted); font-size: var(--font-ui-smaller); font-family: var(--font-interface); position: absolute; top: 0; inset-inline-end: 0` |
| `.markdown-rendered button.copy-code-button:hover` | `background-color: var(--background-modifier-hover)` |
| `.is-mobile .markdown-rendered pre:not(:hover) > button.copy-code-button` | `display: block` |
| `.is-mobile .markdown-rendered button.copy-code-button` | `width: auto` |

So the button is hover-only on desktop and always visible on mobile. The
computed style of an unhovered button is `display: none`, `border-radius:
5px`, `color: rgb(179, 179, 179)`; the containing pre is `position: relative`.

What a click copies. The one intercepted call is
`navigator.clipboard.writeText`, once per click; the Electron tap and the
`copy` listener were installed and their intercept arrays stayed empty (an
absence, not a recorded negative). The payload is the `code` element's plain
text — Prism's `<span class="token …">` markup does not survive — with
**exactly one** trailing newline removed, not a trim:

| Case | DOM text | Copied |
| --- | --- | --- |
| Fence body ending in three newlines | `…tab indented\n\n\n` (79 chars) | `…tab indented\n\n` (78) |
| Ordinary one-line fence | `const CASE_LANG = 1;\n` | `const CASE_LANG = 1;` |
| 4-space indented block | `…four spaces\n…second line\n` (52) | same without the final `\n` (51) |
| Frontmatter `pre` (no trailing newline) | `title: Code Copy Probe\ntags:\n  - probe` (38) | identical (38) |
| Empty fence | `""` | `""` |

Trailing spaces and a leading tab inside the body survive untouched. The
byte claims rest on a deliberate input/output diff: the probe wrote the note
and read it back byte-identical before rendering it.

A CRLF file's code block is LF-only in both the DOM and the clipboard —
`CRLF line one\r\nCRLF line two\r\n` on disk (6 CRs, round-tripped) renders as
`CRLF line one\nCRLF line two\n` and copies as `CRLF line one\nCRLF line two`.
Obsidian normalizes the endings while leaving the file's CRs alone.

One sample of each block shape got a button and copied its own body, all from
a single hand-built note: fence with a language, fence with none, 4-space
indented block, `~~~` fence, an unregistered info string, inside a callout,
inside a blockquote, an empty fence, and the frontmatter block. The
frontmatter case is the odd one: `pre.frontmatter.language-yaml` computes to
`display: none` (the properties widget is shown instead) and still carries a
button, and a synthetic click on it copies the raw YAML. The one inline
`<code>` in the note had no sibling button.

Prism classes on the rendered block. The language is the info string's first
word, **lowercased**; a word holding anything but `[A-Za-z0-9_-]` does not
resolve, and then the `pre` carries `language-none` while the `code` keeps the
raw word beside it. Unregistered is not the same as unresolvable:
`silica-made-up` is a name Prism has not got but could have, and keeps its own
class, while `c#` and `C++` -- real languages, and common fence words -- do
not. Probed 2026-10-29.

| Fence | `pre` class | `code` class |
| --- | --- | --- |
| ```` ```js ```` | `language-js` | `language-js is-loaded` |
| `~~~python` | `language-python` | `language-python is-loaded` |
| ```` ```silica-made-up ```` (unregistered) | `language-silica-made-up` | `language-silica-made-up is-loaded` |
| ```` ```JS ```` (uppercase) | `language-js` | `language-js is-loaded` |
| ```` ```foo bar ```` | `language-foo` | `language-foo is-loaded` |
| ```` ```c# ````, `f#`, `C++`, `.net`, `{python}` | `language-none` | `language-c# language-none is-loaded` |
| ```` ``` ```` with no info string | *(none)* | *(none)* |
| 4-space indented block | *(none)* | *(none)* |
| Frontmatter | `frontmatter language-yaml` | — |

Click feedback is icon-only. The SVG is `lucide-check` at 0, 150 and 400 ms
after the click and back to `lucide-copy` at 900, 1500 and 2200 ms. Across
all six samples `className` stays `copy-code-button` and `textContent` stays
empty, and no Notice appeared on any of the twelve clicks the run made.

In the editor there are **two** affordances, not one:

- A plain fence line gets `span.code-block-flair`, `aria-label="Copy"`,
  `contenteditable="false"`, on the `HyperMD-codeblock-begin-bg` line, with
  the text `JavaScript` for a ```` ```js ```` fence. Its rule is
  `.markdown-source-view.mod-cm6 .code-block-flair { position: absolute;
  right: 6px; top: 6px; z-index: 1; display: inline-block; padding:
  var(--size-4-1) var(--size-4-2); border-radius: var(--code-radius);
  font-family: var(--font-interface); font-size: var(--font-ui-smaller);
  color: var(--text-muted); cursor: var(--cursor); }`, plus a `:hover`
  background.
- A code block rendered as an embed widget — the callout — gets a real
  `button.copy-code-button` inside the CodeMirror DOM: one `pre` inside the
  editor, one `cm-embed-block`, and the focusable-element walk lists
  `BUTTON.copy-code-button tabindex=0` immediately after
  `DIV.cm-embed-block cm-callout`.

No command in the registry copies a code block. Filtering the registry, the
ids containing "copy" are `workspace:copy-path`, `workspace:copy-full-path`,
`workspace:copy-url`, `editor:table-row-copy`, `editor:table-col-copy` and
`bases:copy-table`; the ones containing "code" are `editor:toggle-code` and
`editor:insert-codeblock`.

Every reading-view `pre` sampled computed to `white-space: pre-wrap` with
`overflow-x: auto`. This contradicts the "fenced code never wraps" row above
and is a computed value only — the matched-rule dump covered the copy-button
selectors, and the page carried seven stylesheets besides `app.css`.

**Unprobed as of 2026-10-18** (the rows the 2026-10-25 pass below answers are
struck from this paragraph): Whether any setting turns the button off: the config scan filtered a
probe-chosen list of eight keys, not an enumeration of Obsidian's settings.
Mobile behavior beyond the two `.is-mobile` rules, nothing having run
with `isMobile` true. Whether the copy text can diverge from the file's bytes
in ways other than line endings — HTML entities, unicode normalization,
zero-width characters. Whether the frontmatter pre's button is reachable by a
person at all, its pre being `display: none`. And which rule sets the
reading-view `pre` to `pre-wrap`.

### The 2026-10-25 pass: a real mouse, and the clipboard read back

`tools/obsidian-probe/code-copy.mjs`, against **Obsidian 1.14.5**, results in
`tools/obsidian-probe/results/2026-10-25-code-copy.json`. This run does what
the 2026-10-18 one could not: it moves a real pointer onto the block so the
hover rule reveals the button, clicks the painted button, and reads the
clipboard through Electron **without replacing any clipboard API** — so the
read-back is Obsidian's own write, over a sentinel the probe had put there
first.

**The hovered button is painted and the click copies.** Eight blocks of eight
different shapes in one note, each hovered and clicked: the button computes
`display: flex` with a rect of **31.98 × 27.97 px** at the pre's top-right
(the unhovered `display: none` and zero rect are confirmed too), its svg is
`lucide-copy`, and the clipboard afterwards holds the block's body with
**exactly one** trailing newline removed. `className` stays
`copy-code-button`, `textContent` stays empty, and no Notice appears.

| Block | Copied |
| --- | --- |
| ```` ```js ```` | `const CASE_LANG = 1;` |
| ```` ``` ```` no info string | `no info string` |
| `~~~python` | `print("tilde fence")` |
| unregistered info string | `unregistered info` |
| 4-space indented | `four space indented\nsecond line` |
| inside a callout | `const IN_CALLOUT = 2;` |
| inside a blockquote | `in a blockquote` |
| **empty fence** | nothing — see below |

**The empty fence's button writes nothing.** Its click left the clipboard
holding neither the sentinel nor `""` but content from outside the app, so all
that is certain is that Obsidian did not write: an empty body is not copied as
an empty string, it is not copied at all. That matches the 2026-10-18 run's
unexplained row.

**Trailing whitespace, unicode and entities survive; CRLF does not.**

| Fence body on disk | DOM text | Copied |
| --- | --- | --- |
| `tab\tindented  \n\n\n` | identical (17 chars) | `tab\tindented  \n\n` — one newline off, the tab and the two trailing spaces kept |
| `café — naïve <U+200B> zero width ❤️\n&amp; &lt;b&gt; entity text\n` | identical | identical minus the final newline — the zero-width space survives and `&amp;` is **not** decoded |
| `CRLF line one\r\nCRLF line two\r\n` (CRs on disk) | `…\n…\n`, LF only | `CRLF line one\nCRLF line two` |

**The check icon reverts after 1000 ms.** Sampled every 100 ms from the click:
`lucide-check` at 0 ms, still check at 900 ms, back to `lucide-copy` at
1000 ms. The old bound was "between 400 and 900".

**A real Tab does not reach the button.** Clicking the note body and pressing
Tab eight times walks only the view header's `clickable-icon` buttons; the
copy button never takes focus, which follows from its being `display: none`
until hovered. So there is no keyboard path to it.

**The frontmatter block's button is unreachable.** `pre.frontmatter.language-yaml`
computes `display: none` with a zero rect, and hovering does not paint its
button — so the synthetic click the earlier run made on it exercised something
a person cannot do. The ordinary fence after the frontmatter copies normally.

**In the editor, `span.code-block-flair` IS the copy button, and it shows a
Notice.** Clicked with a real mouse it copied the fence body (`js fence`) and
raised the notice **"Copied to your clipboard"** — which the reading view's
own button never does. The caret does not move. Hovered it computes
`background-color: oklch(0.999994 0.0000497986 none / 0.067)`,
`color: rgb(179, 179, 179)`, `cursor: default`.

**The flair's text is a language display-name table with a verbatim
fallback**, and the affordance is Live Preview only:

| Fence | Flair text |
| --- | --- |
| ```` ```js ```` | `JavaScript` |
| ```` ```python ```` | `Python` |
| ```` ```rust ```` | `Rust` |
| ```` ```ts ```` | `TypeScript` |
| ```` ```c ```` | `C` |
| ```` ```yaml ```` | `YAML` |
| ```` ```silica-made-up ```` | `silica-made-up` — verbatim |
| ```` ``` ```` no info string | `""` — the span is still there, still `aria-label="Copy"` |

Counted in the same leaf: Live Preview has 7 flairs and **no**
`button.copy-code-button` in the editor DOM; source mode has **0 flairs** and
no button. So a plain fence's editor affordance exists in Live Preview alone,
and the `copy-code-button` the earlier run found inside CodeMirror belongs to
an embed widget (the callout), not to a plain fence.

**A transcluded note's code block does get a button** — `![[Embedded]]`
rendered a `pre.language-js` holding the embedded file's code with
`hasButton: true` — but the hover did not paint it at the position the probe
aimed at, so whether a person can click it is still open.

**Unprobed after this pass:** whether the embed's button can actually be
hovered and clicked; whether any setting turns the button off (the config scan
was a chosen list, not an enumeration); mobile behavior; and which rule sets
the reading-view `pre` to `pre-wrap`.

## Observed raw HTML behavior, Obsidian 1.14.4 (installer build 1.13.7)

Observed on 2026-10-18 with a throwaway plugin in a scratch vault: a note of raw-HTML constructs opened in reading view and read back, a second note in a subfolder for source-relative `src` values, and 26 single-construct cases from a second probe file. Three limits bound everything below. The run executed its step set twice and the stored values are the second pass, which created nothing — `setup.made` is empty, every probe file already existed. The reading view it measured stopped short: `sectionChildTags` holds 18 entries and ends on the `DIV.el-h2` for "Sup and sub" with no body element after it, so the note's tail never rendered. And the 26 cases come from a different session — its `app://` host hash and its `pic.png` query both differ from the main run's — in a file that records no step name, so the surface those cases rendered into is not stated.

### Images

A raw `<img>` whose `src` resolves to a vault file has the attribute itself rewritten to `app://<36 hex>/<absolute filesystem path>?<13 digits>`. Resolution is Obsidian's link resolution, not path arithmetic:

| Authored in | `src` | Resolves to |
| --- | --- | --- |
| root note | `pic.png`, `./pic.png`, `/pic.png` | `pic.png` |
| root note | `assets/deep.png`, `deep.png` | `assets/deep.png` |
| root note | `pic two.png`, `pic%20two.png` | `pic two.png`, written `pic%20two.png` |
| `html/Nested HTML.md` | `sibling.png`, `./sibling.png`, `html/sibling.png` | `html/sibling.png` |
| `html/Nested HTML.md` | `../pic.png`, `pic.png` | root `pic.png` |
| `html/Nested HTML.md` | `assets/deep.png`, `deep.png` | `assets/deep.png` |

A bare name is resolved like a wikilink target rather than joined to the note's folder: from `html/`, `sibling.png` finds the sibling and `deep.png` finds `assets/deep.png`. Each of those basenames was unique in the vault, so the preference order between a root file and the source's own folder was not exercised.

- The host is 36 hex characters and differs between the two probe sessions (`e53bdfa6…cfc9f`, `4440591f…6345b`) — per session and opaque.
- The `?<13 digits>` query is constant per file within one run (`pic.png` `?1792374870859`, `assets/deep.png` `?1792374954193`, `html/sibling.png` `?1792374954195`), the same value in both notes and both DOMs. `pic.png` carries `?1792375399934` in the other session's file, so as observed it is not a stable property of the file.
- A `src` that resolves to nothing is left verbatim (`missing.png`); the browser then resolves it against `app://obsidian.md/` and the image is broken (`naturalWidth` 0).
- An `http://` `src` is left alone and really fetched (`naturalWidth` 1). The request carries no `Referer` header, which is what the `referrerpolicy` below asks for.
- The reading view adds `referrerpolicy="no-referrer"` to every raw `<img>`, the unresolved and the remote one included. It belongs to raw-HTML imgs specifically: the `<img>` inside a reading-view markdown embed (`span.internal-embed media-embed image-embed is-loaded`) carries only `alt` and `src`.
- Nine consecutive raw `<img>` lines with no blank line between them are one HTML block and render as a single `div.el-img` with the images as direct children — no `<p>`. Raw imgs written inside a paragraph stay in that paragraph's `<p>`.
- Markdown embeds have a different shape, for contrast: reading view wraps the img in `<span alt="md-sub" src="assets/deep.png" class="internal-embed media-embed image-embed is-loaded">` — the unresolved vault path stays on the wrapper and the resolved `app://` URL goes on the inner img — while the editor-side form is `div.internal-embed … tabindex="-1" contenteditable="false"` with `div.image-wrapper`, `div.image-resize-corner` and `div.embed-actions` children.

The editor DOM retained inside the same leaf renders raw HTML through a `span.cm-html-embed` widget whose imgs carry the same resolved `app://` srcs but no `referrerpolicy`. Those rows were read while the leaf was displaying the reading view and are not laid out (zero rects), so they are widget output rather than a measured Live Preview session.

### Anchors

- A raw `<a>` keeps its href exactly as authored — no resolution, no extension handling, no `app://` — and gains `target="_blank" rel="noopener nofollow"`. It gets neither `class="internal-link"` nor `data-href`, even when the href names a note that exists.
- A wikilink in the same paragraph renders `<a data-href="Target Note" href="Target Note" class="internal-link" target="_blank" rel="noopener nofollow">`, so `target` and `rel` are on internal links too in 1.14.4, and `data-href` plus the class are what separate them from a raw anchor.
- Clicking each of five raw anchors (`Target Note`, `Target Note.md`, `assets/deep.png`, `Missing Target`, `#Images block`): no leaf opened (one before and after), the active file stayed `Raw HTML.md`, `window.open` and electron `shell.openExternal` were never called, `location.href` was unchanged, and no vault file was created or removed — not even for the missing target. `defaultPreventedByObsidian` was false on all five, but the probe's own navigation guard also fired and cancelled the default on all five, and nothing records the handler order, so this says what did not happen, not that Obsidian declined the click.

### `<pre>`

- A raw `<pre>` renders as a real `<pre>` inside `div.el-pre`, with an empty `className` and no attributes of Obsidian's own. Its content is literal — `**not bold**` and `[[Target Note]]` stay text, `&` serializes as `&amp;`, two leading spaces survive — but a tag inside it is live markup: `<b>tag inside pre</b>` is an element, and the only `<b>` in the note.
- The copy button belongs to `pre > code`: a raw `<pre><code>` gets `button.copy-code-button` (a lucide-copy svg), a raw `<pre>` with no `<code>` child does not, a fenced block does. Two copy buttons in the note, and two buttons of any kind.
- A fenced block is `pre.language-text > code.language-text.is-loaded[data-line="0"]`; a raw one has neither the class nor `data-line`.
- Wrapping, measured once in a 304 px pane: the three reading-view `<pre>` elements all computed `white-space: pre-wrap; overflow-wrap: break-word; overflow-x: auto; tab-size: 4`, and a 295-character line whose longest unbroken token is 240 characters did not scroll sideways (`scrollWidth == clientWidth == 304`; the block grew to 408 px tall). The same note's two editor-side `<pre>` elements, read from the same DOM, computed `white-space: pre; overflow-x: visible` with zero rects. This disagrees with the "fenced code never wraps" invariant recorded above, but one pane width, an uncaptured theme and an unlaid-out counter-example are not enough to change it.

### Sanitizer, 26 single-construct cases

| Authored | Rendered |
| --- | --- |
| `<span onclick="…">x</span>` | `<span>x</span>` — the handler attribute is dropped, the element stays; `onclickFired` false |
| `<script>window.__probeScript=1</script>after` | `after` — element and script text gone, following text survives; `scriptExecuted` false |
| `<a href="javascript:…">x</a>` | `<a target="_blank" rel="noopener nofollow">x</a>` — no `href` at all; `javascriptHrefFired` false |
| `<a href="https://example.invalid/page">`, `<a href="obsidian://open?vault=x&file=y">` | href kept verbatim (`&` re-escaped as `&amp;` in serialization), `target` and `rel` added |
| `<span class id data-foo title>` | all four attributes kept |
| `<img src="pic.png" width="100" height="50" alt="A">` | `<img alt="A" height="50" width="100" src="app://…" referrerpolicy="no-referrer">` |
| `<iframe src="http://…" sandbox="">` | both kept as written — an author's empty `sandbox` is not replaced |
| `<table><tr><td>c</td></tr></table>` | `<table><tbody><tr><td dir="ltr">c</td></tr></tbody></table>` |
| `start <b>bold never closed` | `<p dir="auto">start <b>bold never closed</b></p>` — closed at the end of its paragraph, no leak into later blocks |
| ins, del, kbd, samp, var, small, big, center, font[color], strike, tt, details, summary, sup, sub | all kept with their content |

- The `style` attribute survives verbatim in all nine style cases, and the computed style shows it taking effect for `color`, `background-color`, `font-size`, `font-weight`, `display:none`, `position:absolute` and `display:inline-block`. Two cases show survival only: `background-image:url(http://…)` left no `backgroundImage` in the computed set and no request reached the probe's server, and `<div style="text-align:center">` was recorded with its `attrs` and `computed` both null.
- Attribute order is rebuilt, not passed through: `class id data-foo title` comes back `title data-foo id class`, and `src width height alt` comes back `alt height width src referrerpolicy`.
- Obsidian stamps direction attributes a matching renderer has to emit too: `<p dir="auto">` on paragraphs, `dir="auto"` on headings (`<h2 data-heading="Anchors" dir="auto">`), `dir="ltr"` on a `<td>` in a raw table.
- Inline HTML is wrapped in `<p dir="auto">`; HTML that opens a CommonMark HTML block — `div`, `center`, `details`, `table`, `iframe`, a lone `<img>` on its own line — gets no paragraph wrapper.
- `<sup>` and `<kbd>` are themed rather than merely structural: sup computes `vertical-align: super` at 12.5 px against a 15 px body, kbd gets `background rgb(35, 35, 35)` at 13.125 px.

The recorded pass left the vault's 74 paths identical before and after, and cleanup detached one markdown leaf. It also attempted no write, so that is not evidence that rendering and clicking are read-only — only that nothing was written.

**Unprobed as of 2026-10-18** — the 2026-10-25 pass below answers much of this paragraph: where a raw anchor click ends up, and whether `href="#Images block"` scrolls, since the probe's guard cancelled every navigation; `<details>` in a real note and in-note `<sup>`/`<sub>` — `details.count` is 0 and the sup/sub paragraph was "not found", both consistent with the preview stopping at the last heading rather than with Obsidian omitting them, and the second file's own `supsub` value is null, so sup/sub rests on the one detached case; whether an iframe or a CSS `background-image` in a note actually fetches (no `/san.png` request appears, but that HTTP log is shared across scenarios and those cases ran in the other session); whether the `?<digits>` query is the file mtime, as no file timestamps were captured; whether the pre-wrap measurement is the stock default, since only six config keys were recorded (`attachmentFolderPath`, `newLinkFormat` `shortest`, `useMarkdownLinks` false, `showInlineTitle`, the probe's HTTP base, the version) and none covers theme, snippets, appearance or readable line length — and every link-resolution observation above was made under `useMarkdownLinks: false`, `newLinkFormat: shortest`; an ambiguous bare basename, root file against source folder, since each probe basename was unique; which elements get unwrapped, since all 26 cases used tags that survived and none covered object, embed, form, svg, style, link or meta; raw HTML inside a callout, blockquote, table cell or list item, and a raw HTML block interrupting a paragraph; whether the notes' bytes changed, since only byte counts and a truncated head were read back; and the markup behind the Embeds, Anchors, Pre and Sup-and-sub sections, since the 2367-byte source of `Raw HTML.md` is truncated after 700 bytes in the evidence.

### The 2026-10-25 pass: a real anchor click, and which tags survive

`tools/obsidian-probe/raw-html-remote.mjs` against **Obsidian 1.14.5**, results
in `tools/obsidian-probe/results/2026-10-25-raw-html-remote.json`. The probe
runs its own HTTP server, so the request log is read from the host rather than
from the renderer, whose `fetch` the CSP blocks. The anchor clicks are guarded:
a bubble-phase listener records `defaultPrevented` as Obsidian left it and then
cancels the default itself, because an uncancelled raw-anchor click navigates
the Electron frame and kills the app.

**A raw `<a>` is left as an external link, and Obsidian does not intercept its
click.** Every one of six anchors came out of the renderer with
`target="_blank"` added, **no class** and **no `data-href`** — so `href="Other"`
is not turned into an `internal-link`, and nor is `href="Other.md"`:

| Written | Rendered |
| --- | --- |
| `<a href="Other">` | `href="Other"`, `target="_blank"`, class `""`, no `data-href` |
| `<a href="Other.md">` | the same |
| `<a href="pic.png">` | the same |
| `<a href="No Such Note">` | the same |
| `<a href="#Images block">` | the same |
| `<a href="http://…">` | the same |

Clicked with a real mouse, **`event.defaultPrevented` was `false` in every
case** — Obsidian had not handled the click. No leaf changed, no file opened
and no new window appeared, because the guard cancelled the default. So "where
a raw anchor click ends up" is: not anywhere inside Obsidian. It is left to the
default action for `target="_blank"`, which is Electron's own external-link
handling. The in-note `#Images block` case cannot be settled this way — the
guard necessarily blocks the browser's own fragment navigation too.

**Which tags survive, and which are dropped.** Rendered counts inside the
reading view, from one note carrying one of each:

| Written | In the DOM |
| --- | --- |
| `<object>` | **dropped** |
| `<embed>` | **dropped** |
| `<style>` | **dropped** — and a `.silica-probe` div computed the default colour, so the rule never applied |
| `<link rel="stylesheet">` | **dropped** |
| `<meta>` | **dropped** |
| `<script>` | **dropped**, and it did not run (`window.__silicaRawScriptRan` stayed undefined) |
| `<form>` with `<input>` | **kept**, both of them, laid out and interactive |
| `<iframe>` | **kept**, and rewritten with a `sandbox` attribute (`allow-forms allow-presentation allow-same-origin allow-scripts allow-mod…`) |
| `<details>` / `<summary>` | **kept** |
| `<sup>` / `<sub>` | **kept**, 7.63 × 18.62 px inline |
| `<b>` | **kept** |

A raw `<svg>`'s fate cannot be read off this capture: the reading view already
holds eleven SVGs of Obsidian's own icons and the probe did not distinguish
them.

**An iframe and a CSS `background-image` both fetch.** The host's request log
shows `GET /frame.html` and `GET /bg.png` 643 ms after the note rendered, so
raw HTML in a note does reach the network through both.

**Raw HTML inside every container renders as HTML**, which the earlier pass had
not tried:

| Container | Rendered |
| --- | --- |
| callout | `<b>` and `<img>` render inside `.callout` |
| blockquote | `<blockquote dir="auto"><p><b>bold in a blockquote</b></p></blockquote>` |
| table cell | `<td dir="ltr"><b>bold in a cell</b></td>` and a second cell holding a real `<img>` |
| list item | `<li data-line="0" dir="auto"><span class="list-bullet"></span><b>bold in a list item</b></li>` |
| inline, mid-paragraph | `<p dir="auto">A paragraph that is <b>interrupted</b> by inline HTML.</p>` |
| a raw block interrupting a paragraph | its own `div.el-div` section between the two text paragraphs |

**The note's bytes never change.** Both fixture notes were byte-identical after
the whole run, rendering, clicks and all.

## Observed remote image behavior, Obsidian 1.14.4 (installer build 1.13.7)

Observed on 2026-10-18 with a throwaway plugin in a scratch vault: a table of
image syntaxes rendered through `MarkdownRenderer` into a detached container,
one note opened in the reading view, and a local HTTP server logging every
request with its headers. Ten of the sixteen requests came from the detached
container — that is the renderer API, not a note anyone opened; the other six
(`rv-inline.png`, `slow`, `404`, `rv-sized.png`, `cache-probe.png`,
`rv-rawhtml.png`) came from the opened note.

Fetching:

- Every `http:` destination in rendered markdown is requested, with no prompt,
  no allowlist and no gate: sixteen `GET`s in a 9.3 s run, one per distinct URL.
- The six images of the opened note were all requested inside 2 ms of each
  other, and no sampled `<img>` carries a `loading` or `decoding` attribute.
- Mounting one URL repeatedly produces no further request: the DOM held four
  elements on that URL (the probe accounts for three of them — two extra mounts
  plus the reading-view one) and the server logged exactly one `GET`, with a
  single `PerformanceResourceTiming` entry. Whether that cache is Obsidian's or
  Chromium's ordinary one cannot be told apart here.
- An image request carries no `Referer`, no `Origin` and no `Cookie`. The
  plugin's own `XMLHttpRequest` in the same run does send
  `Origin: app://obsidian.md`, so the absence is specific to images.
- What it does send: `Sec-Fetch-Dest: image`, `Sec-Fetch-Mode: no-cors`,
  `Sec-Fetch-Site: cross-site`, `Sec-Fetch-Storage-Access: active`,
  `Accept: image/avif,image/webp,image/apng,image/svg+xml,image/*,*/*;q=0.8`,
  `Accept-Encoding: gzip, deflate, br, zstd`, `Accept-Language: en-US`,
  `Connection: keep-alive`, the `sec-ch-ua` trio, and a Chrome user agent
  ending `obsidian/1.13.7 Chrome/150.0.7871.47 Electron/43.1.0`.

What each syntax renders to, and whether it is fetched:

| Source | Emitted | Fetched |
| --- | --- | --- |
| `![alt](http://h/x.png)` | `<img alt="alt" src="http://h/x.png" referrerpolicy="no-referrer">` | yes |
| `![alt\|200x100](http://h/x.png)` | adds `height="100" width="200"`, `alt="alt"` | yes |
| `![alt\|200](http://h/x.png)` | adds `width="200"`, no `height` attribute | yes |
| `![200x100](http://h/x.png)` | `alt="200x100"` *and* `height="100" width="200"` | yes |
| `![alt\|200x100](http://h/x.png "the title")` | adds `title="the title"` | yes |
| `![alt](<http://h/x.png>)` | same as the plain form | yes |
| `![ref alt][rr]`, `[rr]: http://h/x.png "Ref title"` | `<img title="Ref title" alt="ref alt" src=…>` | yes |
| `<img src="http://h/x.png">` (raw HTML) | `<img src=… referrerpolicy="no-referrer">` | yes |
| `![alt](http://h/noext?format=png&x=1)` | loads (`naturalWidth` 1): no extension sniffing | yes |
| `![alt](data:image/png;base64,…)` | inline `<img>`, `referrerpolicy` still added | n/a |
| `![alt\|120x60](data:image/svg+xml;base64,…)` | `height="60" width="120"` honoured | n/a |
| `![[http://h/x.png]]` | `<span class="internal-embed is-loaded file-embed mod-empty-attachment">“http://h/x.png” could not be found.</span>`, no `<img>` | no |
| `![[http://h/x.png\|200]]` | the same span, plus `width="200"` on it | no |
| `![alt](//h/x.png)` | `<img src="//h/x.png">`, resolving to `app://h/x.png` — scheme *and* port dropped — `naturalWidth` 0 | no |
| `![alt](file:///abs/path.svg)` | rewritten to `app://<36 hex>/abs/path.svg`, loads | n/a |
| `![alt](app://local/abs/path.svg)` | left as written, `naturalWidth` 0 | no |
| `![[probe-pic.svg]]` (internal control) | `span.internal-embed.media-embed.image-embed` wrapping `<img src="app://<36 hex>/abs/path.svg?1792374924921">` | n/a |

A URL inside a wikilink embed is therefore not remote content at all: it is an
unresolved file embed, and nothing is requested. The internal embed's query
string is the file's mtime in milliseconds, matching the vault listing exactly
— a cache-buster keyed on modification time.

`referrerpolicy="no-referrer"` is on every markdown-rendered `<img>`, raw-HTML
and `data:` ones included. The internal embed's `<img>` has none
(`referrerPolicy` is `""`).

Attribute order is not canonical, so nothing should depend on it. The
markdown-rendered sized image serialises as `height, width, title, alt, src,
referrerpolicy`; a plain one as `alt, src, referrerpolicy`; the raw-HTML one
written `src, width, height, alt` comes back as `alt, height, width, src,
referrerpolicy`; the internal sized embed as `alt, width, src`. The raw-HTML
case shows the sanitizer rebuilds the element rather than passing it through.

A size hint is stripped from the alt text and applied as attributes. `|200`
sets `width` only and no `height` attribute; the rendered height then follows
the intrinsic ratio — demonstrated by the internal 300x150 embed
(`width="200"` → 200x100), not by the external cases, whose probe image is 1x1.

The opened-note run captured nodes from more than one surface, and they differ:

| | Markup | Computed |
| --- | --- | --- |
| Wrapper surface | `<div class="image-wrapper"><img src=… alt=…><div class="image-resize-corner"></div></div>`, **no** `referrerpolicy` | `display: block`, `width: 100%`, `max-width: 100%` |
| Raw HTML there | `<img>` inside `span.cm-html-embed`, no `referrerpolicy` | `display: inline`, `width: auto`, `max-width: 100%` |
| Reading view | `<div class="el-p"><p dir="auto">Text before <img alt="inline ext" src=… referrerpolicy="no-referrer"> and text after…</p></div>` | not captured |

The wrapper and its resize corner are present around a 404 and around a slow
image that has not loaded, so they do not depend on load state. Which leaf the
wrapper nodes belong to was not established: the view mode was `preview` and
the sampling was document-wide. Computed sizes from the detached render
container are not evidence of layout — it has none of the reading view's CSS,
and every `<img>` in it computes `display: inline`.

Load states, sampled on the wrapper surface:

- A 404 stays a plain `<img>`: `complete` true, `naturalWidth` 0, no class, no
  placeholder element, no alt substitution in the DOM.
- A five-second image is `complete` false with `currentSrc` `""` at 250 ms and
  at 1451 ms, and `complete` true with `naturalWidth` 1 at 6652 ms; its
  resource timing reads 5004.9 ms.
- `https://` to the plain-HTTP probe port never reached the server, but a
  resource-timing entry exists for it (2.7 ms), so the attempt was made and
  failed in transport. This says nothing about how Obsidian treats `https:`.

No remote-content setting surfaced. `vault.getConfig` returned undefined for
ten guessed keys (`showRemoteContent`, `allowRemoteContent`, `remoteContent`,
`remoteImages`, `externalImages`, `loadRemoteImages`, `trustedExternalContent`,
`privacyMode`, `offlineMode`, `imageCache`) in the same call that returned
values for `attachmentFolderPath`, `showInlineTitle` and `strictLineBreaks`,
and `app.json` held only `foldHeading` and `propertiesInDocument`. Of the nine
setting tabs, three were rendered — Interface (8 items), Editor (25), Files and
links (22) — and none of their items concerns remote content. The window's own
CSP meta constrains `style-src` only, so it is not what governs image loading;
`document.cookie`, `document.referrer` and the meta referrer are all empty.

**Unprobed as of 2026-10-18** — the 2026-10-25 pass below answers part of
this paragraph: which leaf the `image-wrapper` nodes come from, since sampling was
document-wide while the view mode was `preview`; the reading view's own markup
for the slow, 404, sized and raw-HTML images, because the captured
`previewHTML` is truncated (`…[+770 chars]`) before them; layout of any kind —
every sampled rect is 0x0 and the layout capture is null, so whether space is
reserved before an image arrives, and how it flows with surrounding text, is
unmeasured; whether `div.image-resize-corner` resizes a remote image and
whether a drag writes a `|WxH` hint back into the note; lazy loading, since
nothing was scrolled and no image was shown to be offscreen; how `https:` URLs
behave, since the probe server spoke plain HTTP; why `app://local/<abs path>`
failed while `app://<36 hex>/<abs path>` loaded; caching across restarts or to
disk, revalidation, and `Cache-Control`/`ETag` handling — no response headers
are in the capture and no conditional request appears in the log; redirects,
a non-image `Content-Type`, authentication, and very large images, none of
which were requested; what `CachedMetadata` records for an external image, the
run having measured DOM mounts rather than the cache; how a broken image looks
to the eye, no screenshot being taken; and whether a user-facing switch exists,
six setting tabs (General, Appearance, Hotkeys, Keychain, Core plugins,
Community plugins) having been skipped unrendered.

### The 2026-10-25 pass: remote-image layout, and the `?<digits>` query

Measured in the same run, on a note with four remote embeds.

**The `?<digits>` query on a vault-relative image is the file's mtime in
milliseconds.** `pic.png` rendered as
`app://<36 hex>/<abs path>/pic.png?1792942993635`, and `fs.stat` on the file
reports `mtimeMs: 1792942993635.2686` — the query is that value truncated to a
whole millisecond.

**A remote embed is a bare `<img>` with `referrerpolicy="no-referrer"`,** no
`loading` attribute, inside the paragraph's own `div.el-p` section:
`<p dir="auto"><img alt="missing" src="http://…" referrerpolicy="no-referrer"></p>`.
Computed `display: inline`, `object-fit: fill`.

**Layout, which the earlier pass could not measure at all** (every rect was
0×0 there):

| Case | `img` rect | its `el-p` section |
| --- | --- | --- |
| loaded 120 × 60 PNG | 120 × 60 | 604.43 × 66.07 |
| `![sized\|60x30](…)` of the same PNG | **60 × 30** — the size hint applies to a remote image | 604.43 × 36.07 |
| a 404 | **74.23 × 24** — the alt text's own box at the line height, `min-height: 0` | 604.43 × 24, the height of a text paragraph |
| a text paragraph, for scale | — | 604.43 × 24 |

So **no space is reserved**: a broken image collapses to its alt text and the
paragraph is as tall as a line of text. There is **no
`.image-resize-corner`** on any remote image, so the drag-resize affordance a
vault image has is absent and no `|WxH` hint can be written back by dragging.

**A 200 is cached for the session; a 404 is re-requested every render.** The
second note's `remote.png` and `slow.png` produced **no new HTTP request** —
they had been fetched by the first note — while `missing.png` was requested
again on every render, four times across the run (at 120, 648, 13463 and
18691 ms). The delayed image could therefore not be used to measure
pre-arrival layout: by the time it was read it was already cached and
`complete`.

## Observed UI rules, Obsidian 1.14.5 (installer build 1.13.7)

Observed on 2026-10-25 over CDP — `tools/obsidian-probe/ui-rules.mjs`,
`attachment-dropdown.mjs` and `search-collation.mjs`, raw results in
`tools/obsidian-probe/results/2026-10-25-ui-rules.json` — closing the UI half
of the backlog's "Declined until Obsidian is probed" list. The window title of
the probe's own launch reads `Obsidian 1.14.5`, so the app had updated itself
since the earlier passes; the user agent still says `obsidian/1.13.7`, which
is the installer build.

Two traps in the method are worth keeping. The reading view only builds the
sections near the viewport, so the first attempt at the callout table saw 13
of 28 and the rest had to be collected while scrolling; and a selector like
`document.querySelector('.view-header-title')` matches a *sidebar* pane's
header first, so every view-header read has to be scoped to
`.workspace-leaf.mod-active`.

### The lucide icon and colour of every built-in callout type

Read from the rendered `.callout-icon svg`'s own class, which is the lucide
name Obsidian's `setIcon` leaves behind — the corpus's empty `<svg>` was a
serialisation artefact, not what the DOM holds. The same icon appears in Live
Preview and in the reading view. `--callout-color` is the computed value on
the `.callout` in the probe's default dark theme.

| `data-callout` | Icon | `--callout-color` |
| --- | --- | --- |
| `note` | `lucide-pencil` | `#027aff` |
| `abstract`, `summary`, `tldr` | `lucide-clipboard-list` | `#53dfdd` |
| `info` | `lucide-info` | `#027aff` |
| `todo` | `lucide-check-circle-2` | `#027aff` |
| `tip`, `hint`, `important` | `lucide-flame` | `#53dfdd` |
| `success`, `check`, `done` | `lucide-check` | `#44cf6e` |
| `question`, `help`, `faq` | `lucide-help-circle` | `#e9973f` |
| `warning`, `caution`, `attention` | `lucide-alert-triangle` | `#e9973f` |
| `failure`, `fail`, `missing` | `lucide-x` | `#fb464c` |
| `danger`, `error` | `lucide-zap` | `#fb464c` |
| `bug` | `lucide-bug` | `#fb464c` |
| `example` | `lucide-list` | `#a882ff` |
| `quote`, `cite` | `lucide-quote` | `#9e9e9e` |
| anything else | `lucide-pencil`, `#027aff` — the `note` pair | |

An unknown type keeps its own `data-callout` (`[!nosuchtype]` renders
`data-callout="nosuchtype"`) and falls back to note's icon and colour.

Every value above is a `--color-<name>` token — the same six the canvas presets
and the highlight washes draw from, plus `--color-blue` (`#027aff` dark /
`#086ddd` light) for the `note` group, which is not a canvas preset. Silica
picks the hue here and takes the value from the theme, so the three lookups
cannot drift apart; until 2026-10-28 `Theme::callout_color` ignored the theme
and answered the light values in both, which made every callout but `quote` the
wrong colour on the dark theme Obsidian ships. OBSERVE: the light theme's
computed `--callout-color` per type — only the dark column above was read, and
the light column Silica draws is that token's light value rather than an
observation.

### The callout fold chevron is ephemeral

Folding in Live Preview **does not touch the file**. `> [!tip] plain`,
`> [!tip]- minus` and `> [!tip]+ plus` in one note, each fold affordance
clicked with a real mouse and the view saved after every click, left the
note byte-identical throughout — collapsing the `+` callout does not rewrite
it to `-`, and expanding the `-` one does not rewrite it to `+`. Silica's
ephemeral fold is parity.

What the markup says, in both Live Preview and the reading view:

| Source | `data-callout-fold` | classes | fold element |
| --- | --- | --- | --- |
| `> [!tip]` | `""` | `callout` | **none** |
| `> [!tip]-` | `-` | `callout is-collapsible is-collapsed` | `.callout-fold.is-collapsed` |
| `> [!tip]+` | `+` | `callout is-collapsible` | `.callout-fold` |

So a callout with no `-` or `+` has no fold affordance at all: the chevron is
drawn only when the source asks for one. The chevron is
`lucide-chevron-down` in both states, and `.callout-fold`'s own computed
`transform` reads `none` collapsed and expanded alike, so whatever rotates it
is not a transform on that element.

### The setext underline row stays, at body size

In Live Preview the `=====` or `-----` row is **not hidden and not
heading-sized**: it keeps its own `.cm-line`, 16 px at weight 400 and 24 px
tall, the same as a body line, while the heading line above it is 25.888 px /
700 for an H1 and 23.392 px / 680 for an H2 — exactly the sizes the reading
view gives `h1` and `h2`.

| Line | class | font-size | height |
| --- | --- | --- | --- |
| `Setext One` | `cm-line HyperMD-header HyperMD-header-1` | 25.888px | 49.1px |
| `==========` | `cm-line HyperMD-header-line HyperMD-header-line-1` | 16px | 24.0px |
| `Setext Two` | `cm-line HyperMD-header HyperMD-header-2` | 23.392px | 46.1px |
| `----------` | `cm-line HyperMD-header-line HyperMD-header-line-2` | 16px | 24.0px |
| body text | `cm-line` | 16px | 24.0px |

The underline row's text sits in a
`cm-formatting cm-formatting-header cm-formatting-header-<n> cm-header cm-header-<n>`
span. Source mode is the same but for the heading line's height, which loses
Live Preview's fold-indicator and widget padding (33.1 px against 49.1 px).

### A checked task is struck through and muted

The reading view's `li.task-list-item.is-checked` computes
`text-decoration-line: line-through` and `color: rgb(179, 179, 179)` against
an unchecked item's `none` and `rgb(218, 218, 218)`. `[X]` behaves exactly as
`[x]`. Live Preview puts the same pair on the `.cm-line`
(`HyperMD-task-line`), and the inner `cm-list-1` span inherits the colour
without a decoration of its own. So the strikethrough Silica draws is
observed behavior, and the muted colour is the part it still owes.

### The empty tab

It is a leaf of type `empty`, and both its display text and its view-header
title read **"New tab"** — not "No file is open". Its body holds four
`div.text-icon-button.tappable.mod-pill.empty-state-action` pills, in order:

| Pill text |
| --- |
| `New note Ctrl + N` |
| `New base` |
| `New canvas` |
| `Go to file Ctrl + O` |

The header carries the two navigation buttons (`lucide-arrow-left`,
`lucide-arrow-right`, both `aria-disabled="true"`) plus Bookmark and More
options, and **no mode switch**. The status bar is *not* hidden: with the
empty tab active it reads `0 properties`, `0 words`, `0 characters`. The
window title is `New tab - <vault> - Obsidian 1.14.5`. Reaching the empty tab
by closing every tab gives the same view as the one a fresh vault opens with.

### The view-header title is an inline rename field

`.view-header-title` is `contentEditable="true"` with `tabindex="-1"` in its
resting state — nothing has to happen to make it editable. A **single click**
focuses it and puts the caret at the clicked offset; typing edits the title in
place (`Props` clicked at offset 3 and typed `X` reads `ProXps` while the file
is still `Props.md`); **Enter commits and renames the file** — the view's file
became `ProXps.md` and the vault listing agrees. Escape abandons an
uncommitted edit without renaming. A double click puts the caret in the word
rather than opening anything, and no modal or menu appears at any point.

The inline title in the note body is a separate `contentEditable` element
(`.inline-title`, 25.888 px / 700, i.e. H1 size).

### The property pills

The widget per property type, read from the rendered table:

| Type | Markup |
| --- | --- |
| list | `.multi-select-container` holding one `.multi-select-pill` per item (`.multi-select-pill-content > span` plus `.multi-select-pill-remove-button` with a `lucide-x`), then a trailing `.multi-select-input[contenteditable]` |
| text | `.metadata-input-longtext[contenteditable]`, placeholder `Empty` |
| number | `input.metadata-input.metadata-input-number[inputmode=decimal][step=any]` |
| checkbox | `input.metadata-input-checkbox` with `data-indeterminate` |
| date | `input.metadata-input.metadata-input-text.mod-date[type=date][max=9999-12-31]` |

The four open gestures:

- **Enter in the trailing input adds an item.** Typing `three` into the empty
  `.multi-select-input` and pressing Enter writes `  - three` into the list.
- **Backspace takes two presses.** With the input focused and empty, the first
  Backspace moves focus to the last pill (`document.activeElement` becomes
  `.multi-select-pill`) and changes nothing; the second removes that pill and
  writes the file. Repeating walks backwards a pill at a time, and once the
  list is empty focus returns to the input and further presses do nothing. So
  "Backspace removes the previous pill" was half right: it selects, then
  removes.
- **The remove × is always drawn, not revealed on hover.** With the mouse over
  the pill and with it parked in the corner, the button computes identically:
  `display: flex`, `visibility: visible`, `opacity: 1`, `width: 13.9978px`.
- **Removing the last item keeps the key with an empty value.**
  `tags:\n  - one\n  - two\n` emptied one pill at a time ends as `tags:\n` —
  not `tags: []`, and the key is not deleted.

### Clearing every property leaves a blank line

`markdown:clear-metadata-properties` is named **"Clear file properties"** and
is a `checkCallback`. `---\nalpha: one\nbeta: two\n---\n\nbody\n` becomes
`\nbody\n` — first byte `0x0A` — from the reading view and from Live Preview
alike, which is what Silica already writes. A note that is *only* frontmatter
(`---\nalpha: one\n---\n`) becomes an **empty file**, zero bytes.

### `graph:open-local` is not offered with no file open

With a note open, the command's `checkCallback(true)` returns `true` and it is
listed. With every markdown leaf detached, `checkCallback(true)` returns
undefined and `app.commands.listCommands()` **does not include the id at
all** — so Obsidian hides the row rather than offering a command that would
fail. Running it anyway opened nothing (`localgraph` leaf count stayed 0). So
Silica falling back to the global graph is a parity gap: the command should
not be runnable.

### The local graph opens at its own scale 1

`graph.json` held `scale: 8`. The **global** graph opened at
`renderer.scale: 7.926`, honouring it. The **local** graph opened at
`renderer.scale: 1`, with its own option block inside the leaf state rather
than `graph.json`'s:

```
{ "collapse-filter": true, "search": "", "localJumps": 1, "localBacklinks": true,
  "localForelinks": true, "localInterlinks": false, "showTags": false,
  "showAttachments": false, "hideUnresolved": false, "collapse-color-groups": true,
  "colorGroups": [], "collapse-display": true, "showArrow": false,
  "textFadeMultiplier": 0, "nodeSizeMultiplier": 1, "lineSizeMultiplier": 1,
  "collapse-forces": true, "centerStrength": 0.518713248970312, "repelStrength": 10,
  "linkStrength": 1, "linkDistance": 250, "scale": 1, "close": false }
```

Its leaf title is `Graph of <basename>` and its icon `lucide-git-fork`. This
confirms the earlier reading: Silica keeping the global graph's pinned zoom for
a local graph is a finding, not a defensible choice.

### A half-typed query: tolerated, or nothing at all

Result counts read from the search view's own `dom.resultDomLookup`, over a
vault where two notes contain `foo`:

| Query | Results |
| --- | --- |
| `foo` | 2 |
| `foo OR` | 2 — the dangling operator is ignored |
| `foo -` | 2 |
| `foo )` | 2 |
| `(foo` | 2 |
| `"foo`, `foo "` | 2 |
| `foo OR bar` | 2 |
| **`foo AND`** | **0**, "No matches found." |
| **`foo (`** | **0**, "No matches found." |
| `OR` alone | 0 |
| `(` alone | 0 |

So Obsidian is not uniform: a dangling `OR`, `-`, `)`, a leading `(` and an
unclosed quote all match as if they were not there, while a dangling `AND` and
an unclosed trailing `(` make the whole query match nothing.

Built on 2026-10-28. Both of the second group parse to a group holding no term
at all, which the matcher now fails: an `All` node with no children used to be
vacuously *true*, so `foo OR` matched every note in the vault — the dangling
branch became an empty `All` under the `Any`, and the whole vault satisfied it.
`Any` with no children already answered false, so the asymmetry was the bug;
fixing it in the matcher rather than in `Search::is_empty` is deliberate,
because `foo (` and `foo AND` do hold a matcher — Obsidian runs the search and
prints "No matches found.", which is a real zero and not a gated pane. A
dangling `-` no longer searches for a literal hyphen either.

What stays unprobed is whether `AND` is an operator at all. The observation
above is consistent with two readings — `AND` as a dangling operator, or `AND`
as an ordinary word that happens to appear in neither note — because the probe
vault's two `foo` notes contain no `and`. Only the trailing shape is
implemented, so `a AND b` is still three terms with `AND` among them. OBSERVE:
whether `a AND b` matches the notes holding both words and nothing else, and
what the explain panel prints for `foo AND` and `foo (`.

### The search collation tie, explained

`notes/apple` ahead of `apple.md` was not a fluke, and the rule is not a path
tie-break ascending. Ten notes, eight of them named `apple`:

```
zzz/apple.md   z/apple.md   m/n/apple.md   m/apple.md
apple.md       a/b/c/apple.md   a/apple.md   APPLE.md
```

Under **File name (A to Z)** the search pane, and the quick switcher, return
them in exactly that order — which is the **full path descending**, and is
also exactly the order `app.vault.getMarkdownFiles()` returns. Under
**File name (Z to A)** only the basename groups swap (`banana` before
`apple`); inside a tie the order is unchanged.

So: **sort by basename ascending, case-insensitively, as a stable sort over a
file list that is already in full-path-descending order.** That is the whole
rule, and it is why a note in a folder sorts ahead of a root note of the same
name.

### Ctrl+E inside a text input does nothing

With the caret at offset 3 of `foobar` in the search pane's input, Ctrl+E left
the value, the selection offset and the focus untouched, and did **not** change
the markdown leaf's mode. The same in the quick switcher's `.prompt-input`:
value and caret unchanged, the prompt still open. In the editor the same chord
does toggle the mode (`markdown:toggle-preview`, default `Mod+E`, named "Toggle
reading view"), `source` → `preview`. So Obsidian neither moves the caret to
the end of the field nor toggles reading view from a field: the chord is
simply swallowed. Silica's Ctrl+E-is-End in a text field is a divergence.

### The file explorer's context menus

Right-clicking with a real mouse, `---` marking a separator. Note the two
consecutive separators before Rename in both menus — an empty section.

**A file row**

| Item | Icon | Section |
| --- | --- | --- |
| Open in new tab | `lucide-file-plus` | `open` |
| Open to the right | `lucide-separator-vertical` | `open` |
| Open in new window | `lucide-picture-in-picture-2` | `open` |
| `---` | | |
| Make a copy | `lucide-files` | `action` |
| Move file to... | `lucide-folder-tree` | `action` |
| Bookmark... | `lucide-bookmark` | `action` |
| Merge entire file with... | `lucide-git-merge` | `action` |
| `---` | | |
| Copy path | `lucide-clipboard` | *(none)* |
| `---` | | |
| Open version history | `lucide-history` | `view` |
| `---` | | |
| Open in default app | `lucide-arrow-up-right` | `system` |
| Show in system explorer | `lucide-arrow-up-right` | `system` |
| `---` `---` | | |
| Rename... | `lucide-edit-3` | `danger` |
| Delete | `lucide-trash-2` | `danger` |

**A folder row**

| Item | Icon | Section |
| --- | --- | --- |
| New note | `lucide-edit` | `action-primary` |
| New folder | `lucide-folder-open` | `action-primary` |
| New canvas | `lucide-layout-dashboard` | `action-primary` |
| New base | `lucide-layout-list` | `action-primary` |
| `---` | | |
| Make a copy | `lucide-files` | `action` |
| Move folder to... | `lucide-folder-tree` | `action` |
| Search in folder | `lucide-folder-search` | `action` |
| Bookmark... | `lucide-bookmark` | `action` |
| `---` | | |
| Copy path | `lucide-clipboard` | *(none)* |
| `---` | | |
| Show in system explorer | `lucide-arrow-up-right` | `system` |
| `---` `---` | | |
| Rename... | `lucide-edit-3` | `danger` |
| Delete | `lucide-trash-2` | `danger` |

**The empty space below the last row** gives only the four `action-primary`
items: New note, New folder, New canvas, New base.

Two payloads the capture did not record, both decided when the menus were
built on 2026-10-28 and both still worth a probe. **OBSERVE: what "Copy path"
puts on the clipboard** — Silica writes the vault-relative path
(`Notes/Plan.md`), which is the only path the rest of the app speaks, but
Obsidian may well write the absolute one. **OBSERVE: what "Search in folder"
puts in the search box** — Silica writes `path:"<folder>/"`, with the trailing
slash so that `Notes/` does not also match `Notes archive/`. Neither the item's
payload nor the resulting query string was read out of Obsidian, only the menu
row itself.

Two Lucide names in the tables above are Obsidian's older aliases, and the
bundled drawing is under the current name instead: `lucide-edit` is
`square-pen` and `lucide-edit-3` is `pen-line`, both from Lucide's own
`aliases` metadata rather than from a guess. The same already held for
`check-circle-2`, `alert-triangle` and `help-circle` in the callout table.

### The modal backdrop

Read from the quick switcher and from the delete confirmation, in a 1024×800
window:

| Element | Value |
| --- | --- |
| `.modal-bg` | `background-color: rgba(10, 10, 10, 0.4)`, `opacity: 0.85`, `position: absolute`, no `backdrop-filter`, filling the window |
| `.modal-container` | class `modal-container mod-dim`, transparent, `z-index: 50` |
| `.prompt` | `width: 700px`, `max-width: 819.31px` (80% of the window), `border-radius: 12px`, `background: rgb(28, 28, 28)` |
| `.modal` (confirm) | `width: 560px`, same max-width, radius and background |
| box-shadow (both) | `rgba(0,0,0,0.07) 0 1.8px 7.3px`, `rgba(0,0,0,0.114) 0 6.3px 24.7px`, `rgba(0,0,0,0.2) 0 30px 90px` |

The delete confirmation is titled **"Delete file"**, reads
`Are you sure you want to delete “<name>”?` followed by
`It will be moved to your system trash.`, and carries two buttons:
`Cancel` (`mod-cancel`) and `Delete` (`mod-cta mod-destructive`). There is no
`.modal-close-button` on it. The Settings modal has **no `.modal-bg` at all**,
although `app.setting.containerEl.ownerDocument` is the main window's.

### The attachment-location dropdown

Settings → Files and links → "Default location for new attachments" is a
native `<select>` with four options, each storing a different shape of
`attachmentFolderPath` and revealing a different dependent row:

| Option label | `attachmentFolderPath` | Dependent row revealed |
| --- | --- | --- |
| Vault folder | `/` | none |
| Same folder as current file | `./` | none |
| In subfolder under current folder | `./Attachments` | **Subfolder name**, placeholder `Attachments` |
| In the folder specified below | `Attachments` | **Attachment folder path**, placeholder `Attachments` |

Picking an option writes `app.json` immediately — with an empty `app.json` the
first pick leaves exactly `{"attachmentFolderPath": "/"}`.

What each stored value resolves to, asked of
`app.fileManager.getAvailablePathForAttachment('photo.png', note)` for a note
at the root and one in `Sub/`:

| Stored value | From a root note | From `Sub/Nested.md` |
| --- | --- | --- |
| `""` | `photo.png` | `photo.png` |
| `/` | `photo.png` | `photo.png` |
| `./` | `photo.png` | `Sub/photo.png` |
| `./Attachments` | `Attachments/photo.png` | `Sub/Attachments/photo.png` |
| `Attachments` | `Attachments/photo.png` | `Attachments/photo.png` |
| `att/deep/new` | `att/deep/new/photo.png` | `att/deep/new/photo.png` |

So a leading `./` means "relative to the note's folder" and anything else is a
vault-absolute path; `/` and the empty string both mean the vault root.

The whole Files and links tab, for the Settings parity work — `hidden` is a row
Obsidian builds but does not show at the default settings:

| Row | Control |
| --- | --- |
| Default file to open | dropdown: `""` Last opened, `new` New note, … |
| File to open | *(hidden)* |
| Always focus new tabs | toggle, on |
| Default location for new notes | dropdown of **three**: `root` Vault folder, `current` Same folder as current file, `folder` In the folder specified below — the same labels as the attachment dropdown bar its subfolder option, but storing `newFileLocation`'s own enum rather than a path |
| Folder to create new notes in | text, placeholder `Example: folder 1/folder 2` *(hidden)* |
| Default location for new attachments | dropdown (above) |
| Attachment folder path | text, placeholder `Attachments` *(hidden)* |
| Subfolder name | text, placeholder `Attachments` *(hidden)* |
| Links | heading |
| New link format | dropdown: `shortest` Shortest path when possible, … |
| Automatically update internal links | toggle, off |
| Use [[Wikilinks]] | toggle, on |
| Show all file types | toggle, off |
| Trash | heading |
| Confirm before deleting files | toggle, on |
| Delete attachments when deleting files | dropdown: `ask` Ask each time, `always`, … |
| Deleted files | dropdown: `system` Move to system trash, `local`, … |
| Advanced | heading |
| Excluded files | button row |
| Override config folder | text, placeholder `.obsidian` |
| Allow URI callbacks | toggle, off |
| Rebuild vault cache | button `Rebuild` |

**Unprobed:** what a *paste* does with each of the four options, which is the
`attachment-paste` block's own question rather than this row's; the labels of
the two dropdowns whose option lists were truncated in the capture (Default
file to open, Delete attachments when deleting files, Deleted files) beyond
their first entries; and whether the Settings modal's lack of a `.modal-bg` is
a 1.14.5 change or has always been so.

## Observed properties-in-reading-view behavior, Obsidian 1.14.4

Observed on 2026-10-18 with a throwaway plugin in a scratch vault, on Obsidian
1.14.4 (installer build 1.13.7), reading the reading view's DOM, the two
properties panes and `app.metadataTypeManager`, and the vault's files back as
raw bytes. That run read and clicked; it never succeeded in typing into a
property, so what it settled is structure and layout. The editing path was
settled on 2026-10-19 by a CDP-driven run
([[Development#Driving Obsidian over CDP, 2026-10-18]]): real browser-injected
keystrokes and mouse clicks against the same build, with every file read from
disk by the host rather than through the API. Its vault was fresh —
`.obsidian/app.json` written as `{}` and nothing else — holding `Note A.md`
(`Status: draft`, `zebra: alpha-note`, `alpha: 1`, `tags` one/two, `aliases`
Alpha, `count: 3`, `done: true`, `due: 2026-10-18`, `link: "[[Note B]]"`,
`long value`), `Note B.md` (`status:`), `Note C.md` (`STATUS:`), `Dotted.md`
(`book.title`, `book.year`), `Nested.md` (`shelf:` holding a map),
`Past Date.md` (`due: 2020-03-04`), `No Frontmatter.md` and `Plain.txt`; the
Properties and Daily notes core plugins were already on. All thirteen run steps
and both post-restart steps completed. Two things about that session bound
every pixel below: the window was 1024×800, and the File properties pane
opened in step 6 stayed open for the rest of the run, so the main leaf was
680 px wide for the first text edits and 380 px (block width 304) from then
on. Where the two runs disagree the bytes win; where only the plugin run saw
something, it says so.

`propertiesInDocument`, with a ten-property note open in reading mode:

| Value | Reading view | Side panes |
| --- | --- | --- |
| `visible` | the properties table | all 10 rows / all 13 registry rows |
| `source` | the **same table** — the reading view never shows raw YAML | unchanged |
| `hidden` | the block is gone from the rendered text; `div.metadata-container` is still in the DOM, with its text run together | unchanged |

So `source` is an editor-only mode: only the editor swaps the widget for the
YAML. `hidden` drops the block from the document in both views and changes
neither pane. The CDP run adds the mechanism: under `hidden` the preview root
(`div.markdown-preview-view`) loses its `show-properties` class and the
container's computed display is `none`; under `visible` and `source` the class
is present and the container is `block`. `--metadata-display-reading` reads
`block` in all three modes, so the class, not the variable, carries the
switch. The File properties pane kept all 11 rows under every mode. (That
run's block happened to be folded throughout the step, so under `source` it
saw only the 51 px heading; the table itself under `source` rests on the
plugin run.) In all three modes the frontmatter also emits a
`div.el-pre.mod-frontmatter.mod-ui` element holding a
`pre.frontmatter.language-yaml` with `style="display: none"` whose `code` is
the YAML as Prism tokens (`span.token.key.atrule` for each key) — the plugin
run read it as empty because it sampled text, and it never reaches the
rendered text. Under `hidden` the note's `div.mod-header.mod-ui` element holds
only the inline title; under the other two it holds the title and the whole
block.

The write of the setting is **deferred**. Immediately after the probe restored
`propertiesInDocument`, `.obsidian/app.json` still read `{\n  "foldHeading": true\n}`
with no such key; the key appeared only in the final read about 4.8 s later, as
`{\n  "foldHeading": true,\n  "propertiesInDocument": "visible"\n}` —
two-space indent, no trailing newline, written even though `visible` is the
default. The file held one prior key, which also sorts before the new one, so
this run cannot tell insertion order from sorted output; it was also not a
clean baseline, since `foldHeading` flipped from `false` to `true` mid-session
without the scenario touching it. The CDP run's baseline was clean, and it set
each mode with `app.vault.setConfig` (the Settings dropdown was not used):
the file's mtime landed 1001, 1002 and 1003 ms after the three calls, and each
1 s read already held the new bytes — `{\n  "propertiesInDocument": "source"\n}`
(38 bytes), then `"hidden"` (38), then `"visible"` (39, explicit again) — so
the save is debounced about one second. Each save also rewrote
`appearance.json` (`{}`, 2 bytes) with an mtime within 2 ms of `app.json`'s,
though nothing in it changed.

The in-document block, from the reading view's DOM:

```
div.metadata-container[tabindex=-1][data-property-count="10"]
  div.metadata-error-container                      style="display: none;"
  div.metadata-properties-heading[tabindex=0]
    div.collapse-indicator.collapse-icon > svg.right-triangle
    div.metadata-properties-title                   "Properties"
  div.metadata-content > div.metadata-properties
    div.metadata-property[tabindex=0][data-property-key="status"]
      div.metadata-property-key
        span.metadata-property-icon[aria-disabled=false] > svg.lucide-text
        input.metadata-property-key-input[autocapitalize=none]
                                        [enterkeyhint=next][type=text]
                                        [aria-label="Status"]
      div.metadata-property-value[data-property-type="text"] > <widget>
      div.clickable-icon.metadata-property-warning-icon
                      [aria-label="Type mismatch, expected Text"]  display:none
```

- The type is an attribute of the **value wrapper**, `data-property-type`, not
  of the row: the row element carries no type attribute at all.
- Rows follow the file's key order, unsorted. `data-property-key` is the key
  lower-cased; the key input's `aria-label` keeps the file's casing — `Status:`
  in one note gives key `status` labelled "Status", `status:` in another gives
  `status` labelled "status".
- Every row carries the hidden warning icon, the `link` row included. The six
  labels seen are "Type mismatch, expected" Text, Number, Tags, Aliases,
  Checkbox, Date.
- The block's text ends "Add property": the add affordance is inside the
  in-document container too, not only in the sidebar. It is
  `div.metadata-add-button.text-icon-button[tabindex=0]` holding
  `span.text-button-icon` (lucide-plus) and `span.text-button-label`.
- `data-property-count` follows the row count: 10 at the start, 11 once
  `demo` was added.

Value markup by inferred type, from one note's ten properties:

| Type | Value element |
| --- | --- |
| text | `div.metadata-input-longtext[contenteditable=true][placeholder=Empty][spellcheck=true][tabindex=0]` |
| number | `input.metadata-input.metadata-input-number[type=number][inputmode=decimal][step=any][placeholder=Empty]` |
| checkbox | `input.metadata-input-checkbox` |
| date | `input.metadata-input.metadata-input-text.mod-date` |
| tags, aliases | `div.multi-select-container` holding a contenteditable `div.multi-select-input` |

A `text` property whose value is a wikilink (`link: "[[Note B]]"`, widget
`text` in the registry) is drawn instead as a non-editable `div.metadata-link`:
a `div.metadata-link-inner.internal-link[data-href="Note B"][draggable=true]`
with the link text and a `div.metadata-link-flair` holding a pencil
(lucide-pencil). A tag pill is `div.multi-select-pill[tabindex=0]` holding
`div.multi-select-pill-content > span` and a `div.multi-select-pill-remove-button`
(lucide-x). The `date` row also showed a visible 22×22 `clickable-icon`
labelled "Open daily note" (lucide-link) inside its value area, with the
daily-notes core plugin on. It is not tied to today's date: `Past Date.md`'s
`due: 2020-03-04` row drew the same icon at the same size and x.

Key-column icons, one per type: text `lucide-text`, number `lucide-binary`,
checkbox `lucide-check-square`, date `lucide-calendar`, tags `lucide-tags`,
aliases `lucide-forward`. `multitext` is `lucide-list`, seen only on the All
properties row for `cssclasses`; a key whose value is a nested map (`shelf`)
gets `lucide-file-question` there.

The two views and their five commands:

| Id | Name |
| --- | --- |
| `properties:open` | Properties view: Show all properties |
| `properties:open-local` | Properties view: Show file properties |
| `editor:toggle-fold-properties` | Toggle fold properties in current file |
| `markdown:add-metadata-property` | Add file property |
| `markdown:clear-metadata-properties` | Clear file properties |

The last three are `checkCallback` commands; the two view commands are plain
callbacks. The registered view types are `all-properties` and
`file-properties`. `properties:open-local` created a 300 px-wide
`file-properties` leaf in the right split, shrinking the main leaf from 680 to
380 — identically in both runs.

The File properties pane, titled "File properties", draws the rows with **no**
"Properties" heading and ends with
`div.metadata-add-button.text-icon-button` reading "Add property"
(lucide-plus). (The CDP run's dump of the pane while its tab was hidden shows a
`div.metadata-properties-heading` inside the pane's `metadata-container`, so the
heading is in the markup and its absence is a matter of style; whether CSS
hides it was not measured.) Its key inputs are editable in every row. Its
"More options" menu holds only "Split right" (disabled) and "Split down" —
nothing property-specific. For a Markdown note with no frontmatter the pane
shows zero rows and its visible text is just "Add property" (a "No properties
found." element — `div.pane-empty`, `display: none` — exists in the DOM but is
not in that text); for a `.canvas` file the pane's text *is* "No properties
found." with no rows and no "Add property". The view stays bound to the file in
both cases.

**The pane's text drops input-valued widgets.** A note with `status: shipped`,
`zebra: beta-note` and `count: 7` renders pane text
`"shipped\nbeta-note\nAdd property"` — the number lives in `input.value`, as do
checkbox and date. Any fixture built from these strings silently loses them.

**Editing in the reading view**, from the CDP run. Clicking a text value opens
a field in place: `document.activeElement` becomes that row's
`div.metadata-input-longtext` — contenteditable, inside
`.markdown-reading-view`, no sidebar involved — with a collapsed caret where
the click landed: at the end of the 10-character `alpha-note` when the click
fell past its text, at offset 35 of the 97-character `long value` when it fell
on the text. Typing changes the field and nothing else: 200 ms after
` edited` was typed the file still read `zebra: alpha-note`. Clicking the note
body commits: at the 400 ms read the file held `zebra: alpha-note edited`, and
the 5 s read was identical. The line diff against the fixture is exactly one
line — unquoted, `link: "[[Note B]]"` keeps its double quotes, key order,
indentation and the trailing newline unchanged. Nothing in the run shows a
write before blur; the samples are 200 ms after typing and 150 ms before
Escape, so a longer debounce that blur or Escape pre-empts is not excluded.

Keys inside an open value field:

| Key | Field | Focus afterwards | File |
| --- | --- | --- | --- |
| Enter, on the untouched `long value` | no newline; text unchanged | the row's `div.metadata-property`, selection `rangeCount` 0 | unchanged |
| Escape after typing `zzz` | the `zzz` is gone, the committed text is back | the row container, `rangeCount` 0 | unchanged — still `zebra: alpha-note edited` |
| Tab after typing `qqq` | committed | the **next** row's `div.metadata-property` (`alpha`), `rangeCount` 1 — not its key input, not its value, not out of the table | `zebra: alpha-note editedqqq`, one line changed |
| Ctrl+Z twice, focus on that row container | — | stays on `alpha`'s container | unchanged; `editedqqq` still on disk and in the row after a body click |

Enter was only pressed on an untouched field, so it is known not to insert a
newline and to leave the field; that it commits a *modified* value is inferred
from Tab, not observed. Escape on the row container afterwards did nothing
visible. Ctrl+Z was pressed in reading mode with no editor focused: undo from a
property row does not reach a committed edit.

The key input is live in the reading view too. The run's clicks on `count` and
`due` landed on the key column (x 392–516 at that layout: the row's first
`input` is the key input), which turned the step into key renames. End, `9`
and a click elsewhere turned `count: 3` into `count9: 3` on the same line —
value and position kept, on disk at the 400 ms read after the blur. Ctrl+A and
`2026-10-20` in the `due` key input gave the unquoted line
`2026-10-20: 2026-10-18`, again in place, committed by the next click away.
So a UI rename keeps the entry where it is — unlike the delete-then-set rename
through `processFrontMatter` in [[#Observed properties behavior, Obsidian 1.14.4]],
which moves the key to the end. Two oddities on the way: the first click at
the `due` key input's left edge left focus on the *previous* row's
`input.metadata-input-checkbox` (`done`, 16×16 at x 524), and ArrowRight then
ArrowUp on that focused checkbox left it checked and the file at
`done: true`; a second click at the same point reached the `due` key input.
Why focus went there is unobserved.

Removing a tag writes the list back in block form. Hovering the `one` pill and
clicking its remove button dropped the pill at once; after a body click the
file read `tags:\n  - two` with every other line as before — no collapse to
`[two]`. Whether the removal writes on its own or on the blur was not
separated. (The click meant to add a tag left `body` as the active element, no
suggestion popup appeared and the typed `three` reached nothing, so adding is
unobserved.)

**Add property**, clicked in the in-document block, appends a blank row —
`div.metadata-property[data-property-key=""]` with
`span.metadata-property-icon[aria-disabled="true"]` (lucide-text), an empty key
input and a `text` value — focuses its key input and opens
`div.suggestion-container.mod-property-key` over it. The popup lists registry
keys the note does not already have, each a `suggestion-item.mod-complex` with
the type icon in `span.suggestion-flair` and the name in
`div.suggestion-title`, first item `is-selected`: `cssclasses`, `book.title`,
`book.year`, `shelf`, `count`, `due`, `other` — neither alphabetical nor by
count, and the ordering rule was not found. After typing `demo` the popup was
gone (no "create" row; sampled 400 ms later). Enter on the key writes at once:
the file gained `demo:` — bare colon, no `""` — as the last line of the block,
and focus moved into the new row's value field (an empty
`div.metadata-input-longtext`). Typing `hello` and clicking the body rewrote
that line as `demo: hello`; nothing else in the block changed. A second blank
row left with an empty key and clicked away wrote nothing — no `:`, no `""` —
and the row disappeared from the table.

**The type icon's menu.** Clicking the glyph left of a key opens a `div.menu`
at the icon: "Property type" (lucide-info, `has-submenu`, chevron-right); a
group with `data-section="clipboard"` — Cut (lucide-scissors), Copy
(lucide-copy), Paste (lucide-clipboard-check); two consecutive
`div.menu-separator`s; then "Remove" (`is-warning`, lucide-trash-2,
`data-section="danger"`). So a row's remove affordance lives in this menu, and
hovering a row adds nothing to its markup. The submenu was never expanded and
nothing was picked, so the type list, what a retype writes and what Remove
writes stay open. None of the run's edits created `.obsidian/types.json`, and
the type manager's raw map stayed null before and after the restart.

**Folding.** Clicking the heading's `collapse-indicator` folds the block:
`is-collapsed` is added to `div.metadata-container`,
`div.metadata-properties-heading` and the indicator, `div.metadata-content`
goes `display: none` and the container's text shrinks to "Properties".
`editor:toggle-fold-properties` runs in reading view (`executeCommandById`
returned `true` with the preview leaf active) and toggles the same fold: run
once it unfolded, run again it folded. The fold is per file and outlives the
view: Note B opened unfolded, Note A came back folded, and after a scripted
quit and relaunch Note A was restored in preview mode with the block folded.
The record is the one heading folds use
([[#Observed heading fold behavior, Obsidian 1.14.4]]): the `localStorage` key
`<vault id>-note-fold-<path>`, here `e1e76222977ba271-note-fold-Note A.md`,
value `{"folds":[{"from":0,"to":0}],"lines":20}` — the properties fold is the
range `0`–`0`, the opening fence line, and `lines` is 20 for the 19-line note
plus the empty line after its final newline. `app.json` was unchanged 10 s
after the fold (`{}`), and `workspace.json`, though rewritten by the note
switch (5439 → 5458 bytes), holds no fold state. `localStorage` is shared
across vaults and namespaced only by the id: the scratch vault's store also
listed `5c0r1ac0rpu50001-note-fold-Fold Probe.md` and
`5c0r1ac0rpu50001-file-explorer-unfold` from another vault.

At the 380 px main leaf the reading view's rows stayed 29 px tall at a 304 px
block width — key and value did not stack there.

The All properties pane holds one `div.tree-item` per registry key, each with
`div.tree-item-self tappable is-clickable`, a type icon, the display name and a
count. With `sortOrder: "frequency"` the plugin run's order was `tags`6, `STATUS`3,
`title`3, `zebra`3, `count`2, `aliases`1, `alpha`1, `done`1, `due`1, `link`1,
`long value`1, `other`1, `cssclasses`0 — counts merge casings, and
`cssclasses` is listed at zero. Its sort menu is "Property name (A to Z)",
"Property name (Z to A)", a separator, then "Frequency (high to low)"
(`mod-checked`, with the check icon) and "Frequency (low to high)". Its nav
header has "Change sort order" (lucide-sort-asc) and "Show search filter"
(lucide-search); the latter's target is already in the markup as a hidden
`div.search-input-container` holding
`input[type=search][enterkeyhint=search][spellcheck=false][placeholder="Search..."]`
and `div.search-input-clear-button[aria-label="Clear search"]`, so it reveals
an in-pane field rather than a popup. Clicking the `tags` row made the
already-open search leaf the active one at 300 px with 6 results; a
`searchQuery` of `["tags"]` was read back.

The CDP run filled the pane in. The fresh vault's default workspace already
held a hidden `all-properties` leaf in the right split; `properties:open`,
with the right sidebar open at 300 px and the File properties tab active
there, activated it in that same tab group at x 724, 300 px wide — the
sidebar's existing width, not one the command chose. Its tab header is
`div.workspace-tab-header[data-type="all-properties"][aria-label="All properties"]`
(lucide-archive), and its right-click menu is Close, Pin, Link with tab...,
Move to new window, Split right (disabled), Split down. Closing the tab and
running `properties:open` again created a new leaf in the same place. Rows
are flat: `div.tree-item > div.tree-item-self.tappable.is-clickable` (inline
`padding-inline-start: 24px !important`), `div.tree-item-icon`,
`div.tree-item-inner > div.tree-item-inner-text`, `div.tree-item-flair-outer >
span.tree-item-flair` with the count; each row 25 px tall at x 736, 276 px
wide, 27 px apart, and no `tree-item-children` anywhere. Dotted keys
`book.title` / `book.year` are two depth-0 rows with their own types (text,
number); the nested map `shelf` is one depth-0 row with `lucide-file-question`.

Frequency order (18 rows, before the rename): `Status` 3, `zebra` 3,
`2026-10-20` 1, then `aliases`, `alpha`, `book.title`, `book.year`, `count`,
`count9`, `demo`, `done`, `due`, `link`, `long value`, `other`, `shelf`, `tags`
at 1, `cssclasses` 0. Ties run in ascending name order with digits first;
`Status`/`zebra` agree under both case rules, so the tie-break's case
sensitivity stays open. Picking "Property name (A to Z)" (after the rename)
gave `2026-10-20`, `aliases`, `alpha`, `book.title`, `book.year`, `count`,
`count9`, `cssclasses` (0, in place), `demo`, `done`, `due`, `link`,
`long value`, `other`, `shelf`, `state`, `Status` (2), `tags`, `zebra` (3):
case-insensitive (`state` before `Status`), digits first, zero-count keys not
segregated. The choice is view state, not a vault setting: the leaf's
`getViewState().state` went from
`{"sortOrder":"frequency","showSearch":false,"searchQuery":""}` to
`sortOrder: "alphabetical"`, `workspace.json` carried it inside that leaf's
`state` within 6 s and `app.json` did not change; a leaf created later started
at `frequency` again.

Rename: the row's right-click menu is Rename (lucide-edit-3,
`data-section="action"`), "Property type" (lucide-menu, `has-submenu` — the
icon menu's twin uses lucide-info), a separator, "Delete property"
(`is-warning`, lucide-trash-2, `data-section="danger"`). Rename is in-pane: the
`tree-item-self` gains `has-focus is-being-renamed`, its
`div.tree-item-inner-text` becomes `contenteditable="true" spellcheck="true"`
and holds a non-collapsed selection (anchor 0, focus 1 — the whole name or its
first character; the sample cannot tell, and the run pressed Ctrl+A before
typing); no modal and no scope dialog appeared at any point. Typing `state`
and Enter rewrote **only Note A**, `Status: draft` → `state: draft` in place,
one line changed; `Note B.md` (`status:`) and `Note C.md` (`STATUS:`) were
byte-identical before and after. Whether that is current-file scope or a
case-sensitive match on `Status` the data cannot say — both fit, and the
view's `propertyRenameScope` read back only as an object. The reading view's
row followed at once (`state`, `draft`). The pane then listed `zebra` 3,
`Status` 2, …, `state` 1: the merged entry kept the display name `Status` with
no file holding `Status:` any more — the casing is fixed when the key is first
indexed and not recomputed on a rename; after the relaunch the same entry read
`status` 2, indexed afresh from `status:` and `STATUS:`.

Search filter: clicking "Show search filter" shows the
`div.search-input-container` (x 732, 284×30) and focuses its
`input[type=search]`; typing `zeb` filtered the list live to the single
`zebra` 3 row; Escape hid the container, put all 19 rows back and left focus
on `body`.

**Obsidian rewrites `.obsidian` files it did not change.** The fixture wrote
only `app.json`. At launch Obsidian rewrote it byte-identical (`{}`, mtime
moved) and created `appearance.json` (`{}`), `core-plugins.json` (666 bytes)
and `workspace.json`; on the relaunch all four were rewritten with unchanged
sizes (`app.json` …847403 → …880304, `appearance.json` …847403 → …880305,
`core-plugins.json` …761193 → …879846, `workspace.json` …875554 → …880394).
Byte-identical mtimes are not something Obsidian itself provides, so Silica's
stricter rule is not needed for compatibility — only for not being the thing
that changes a file.

`Plain.txt`: with `showUnsupportedFiles` false the explorer DOM still holds its
`nav-file-title[data-path="Plain.txt"]`, but with a zero rect — hidden — so
the click on it was skipped.

`app.metadataTypeManager` keys every property by its **lower-cased** name and
keeps one display name per key: `Status`, `status` and `STATUS` across three
notes give the single entry mapKey `status`, name "STATUS". Its
`registeredTypeWidgets` are `aliases`, `checkbox`, `date`, `datetime`, `file`,
`folder`, `multitext`, `property`, `number`, `tags`, `text` — there is no
`cssclasses` widget, and `cssclasses` is carried as `multitext`. None of the
inferred widgets is persisted: the manager's own raw types map is null and
`.obsidian/types.json` was absent before and after both runs;
`getPropertyInfo('zebra')` read `{name: "zebra", widget: "text"}` throughout.

Layout, from one session: the sidebar's key column held a fixed width beside
the value at pane widths 300 and 280, and at 180 the key and value **stack**,
each taking the full row width and the row growing from 29 px to 54 px. The
relevant variables are `--metadata-label-width: 9em` (computed 135 px, `flex:
0 0 auto`) and `--metadata-input-longtext-lines: 3`. That session's metadata
variables were visibly customized (`--metadata-property-radius: 0px`, a
`color-mix()` label background, a non-default font stack), so the pixel
constants are this vault's, not necessarily Obsidian's defaults; the stacking
behavior itself is not theme-dependent in any way the run could show.

**Unprobed:** the non-text widgets' writes — the number input (`count`'s
click hit the key column), the checkbox (`done`'s too; arrow keys on a focused
checkbox did not toggle it, but a click on the box itself was never made) and
the date input (quoted or not once changed) . Adding a tag, the pill
gestures and the bytes a removal writes are settled in
[[#Observed UI rules, Obsidian 1.14.5 (installer build 1.13.7)]], which also
records what `markdown:clear-metadata-properties` leaves in the file. Enter in a *modified* value field (only an untouched one was
tried). Undo from inside the body editor after a property commit (Ctrl+Z was
pressed on a row container in reading mode). Why the first click at the `due`
key input focused the `done` checkbox, and whether a key rename moves focus
into the row's value. Right-click menus on a *property row*, in the reading
view and in the File properties pane: both CDP right-clicks captured empty
menus here, so the only row menu seen is the icon's, and any
reading-vs-sidebar difference is open — although a right-click on a file
explorer row does raise its menu (see
[[#Observed UI rules, Obsidian 1.14.5 (installer build 1.13.7)]]), so an empty
capture is about the target rather than about `Input.dispatchMouseEvent`. "Remove" from that menu and the bytes
after it; the "Property type" submenu's items and order; what
`zebra: alpha-note editedqqq` becomes when retyped as Number; Cut, Copy and
Paste on a row — none was picked. The first shape `types.json` takes from a UI
retype (its API-written shape is in
[[#Observed properties behavior, Obsidian 1.14.4]]). The ordering rule of the
key-suggestion popup, and what it offers on a partial match (it closed on
`demo`, which matched nothing). The Settings dropdown path for
`propertiesInDocument` (the run used `setConfig`). The All properties rename's
scope — current file or case-sensitive match. Whether the File properties
pane's heading element is hidden by CSS. Whether the fold record is written on
unfold as well as on fold (the key was first read after folding), and where
`properties:open` puts its leaf when the right sidebar is collapsed. Pane
geometry on a default theme beyond the row rects above. What clicking
`Plain.txt` does and what the File properties pane shows for it. Whether the
frequency tie-break is case-sensitive and whether keys other than `cssclasses`
list at count 0 (this fixture discriminates neither). Whether `types.json`
entries and the reserved keys `tags`/`aliases` match case-insensitively (no
type assigned, no `Tags:`). Which casing wins when a key is first indexed
(`Status` in the run, `status` after the relaunch with `Status:` gone — "first
file indexed" fits but is unproven). `markdown:add-metadata-property` was not run
(`markdown:clear-metadata-properties` now is, above); and the exact string the
search box receives from a property row was not re-read (`["tags"]` from the
plugin run stands).

## Observed auto-pair behavior, Obsidian 1.14.4

Observed on 2026-10-18 with a throwaway plugin in a scratch vault, against
Obsidian 1.14.4 (installer build 1.13.7) — those are the API and installer
versions the plugin read back; the running app version came back `null` and
was not observed. Typing is `document.execCommand("insertText")` on the
CodeMirror contentDOM, the one JS path that reaches the input handler; the
run proved the path first (typing `(` on an empty note gave `()`) and every
typing row took it. **Backspace and undo rows are different**: they are
synthetic, untrusted `KeyboardEvent`s, so they record what CodeMirror's
keymap and Obsidian's handlers did together, with no native editing
fallback behind them.

Both toggles read back `true` when the run opened. The run then flipped
them: all four combinations of `autoPairBrackets` and `autoPairMarkdown`
were typed through on an empty document, and the captured starting values
were restored at the end (nothing was restored from a guessed default).

**The two settings own disjoint character sets**, on an empty document with
the cursor at 0:

| Typed | `autoPairBrackets` | `autoPairMarkdown` | Closes when |
| --- | --- | --- | --- |
| `(` `[` `{` | `()` `[]` `{}` | — | `autoPairBrackets` |
| `"` `'` | `""` `''` | — | `autoPairBrackets` |
| `` ` `` `*` `_` | — | ` `` ` `**` `__` | `autoPairMarkdown` |
| `<` `~` `=` `$` `%` | — | — | never |

Each set follows its own toggle and ignores the other's; with both off
nothing closes. `<`, `~`, `=`, `$` and `%` never close under any
combination, and typing a second one just adds the character (`~~`, `==`,
`$$`, `%%`). The membership claim is bounded by the thirteen characters
probed — no other quote or bracket form was tried.

**A second identical character nests for brackets and skips for the rest.**
`(` `(` gives `(())` with the cursor at ch 2; `"` `"` leaves `""` and only
moves the cursor to ch 2. The same skip holds for `'`, `` ` ``, `*` and
`_`.

**Closing depends on the characters either side of the cursor**, not on the
setting alone. With both settings on, seven characters were typed in five
positions:

| Cursor | `(` `[` `{` | `"` `'` `` ` `` `*` |
| --- | --- | --- |
| `|word` — word char ahead | no (`(word`) | no (`"word`) |
| `don|t` — inside a word | no (`don(t`) | no (`don't`) |
| `word|` — after a word char, end of line | closes (`word()`) | no (`word"`) |
| `say |` — after a space, end of line | closes (`say ()`) | closes (`say ""`) |
| `(|)` — before a `)` | closes (`(())`) | no (`(")`) |

So brackets close unless the next character is a word character, and the
quote and markdown characters were seen to close only where the preceding
character is a space or the start of the document and the line ends at the
cursor. Two mid-line controls sharpen that: in `para| text here` (word char
before, space after) `(` closes to `para() text here` while `*` does not
(`para* text here`), and in `x =| 1` (`=` before, space after) `(` closes
and `*` does not — a preceding `=` suppresses markdown pairing even though
it is not a word character. Two collapsed cursors on `alpha` / `beta` at ch
0 each got a bare `(` for the same reason: `(alpha` / `(beta`.

**A fenced code block and a `$$` math block do not suppress bracket
pairing**: `let x| = 1` inside a ```js fence gives `let x() = 1`, and
`x =| 1` inside `$$` gives `x =() 1`, the same as the plain-paragraph
control.

**Frontmatter follows the same rule, with no YAML special case.** In source
mode at `title: |`, `"`, `'`, `[` and `*` all close (`title: ""`,
`title: ''`, `title: []`, `title: **`), and on the body line `|body text`
`"` does not. In Live Preview the YAML is replaced by the properties widget
and is not CodeMirror-editable text, so these rows are source mode only.

**Typing a closer over an auto-inserted one moves the cursor instead of
inserting.** `(|)` typed `)` leaves `()` with the cursor at ch 2. It still
skips after an unrelated edit — typing `(`, then `q` after the pair, then
coming back inside, still skips. Typing `)` at `x|)`, where the `)` was
planted rather than inserted by the editor, writes `x))`; that document has
no `(` anywhere, so what decides — provenance or an unmatched opener — is
not settled (see Unprobed).

**Backspace between an empty pair deletes both characters**, whether or not
the editor inserted the pair: an auto-inserted `(|)` and a planted `(|)`
both leave an empty document, against a control where `pla|in` loses one
character. With content inside, `(|x)` leaves `x)` — only the opener goes.
`*|*` also leaves an empty document, and `**|` leaves `*`; the run does not
record whether that `**` was typed or planted, so the "whoever inserted it"
half is established for brackets only.

**With a selection, every pair character wraps it** and keeps the selection
over the inner text: `alpha` selected and `(` typed gives `(alpha) beta`
with `alpha` still selected. This includes `~`, `=`, `$` and `%`, which
never close at a bare cursor. `<` is the exception — it replaces the
selection (`< beta`). Wrapping is per range: two selections and one `(`
give `(alpha)` and `(beta)`, both selections kept.

**`[` twice gives `[[]]`** with the cursor at ch 2, and the file suggester
opens there (70 items, four registered suggesters). With `autoPairBrackets`
off the same two keys give `[[` and the suggester still opens, so the popup
does not depend on the closers. Sending Escape left the document `[[]]`,
unchanged.

**One undo removes the opener and its auto closer together**: typing `(`
at `seed|` gives `seed()`, and one undo gives `seed`. After skipping over a
closer the same single undo reaches `seed`, so the skip adds no undo step
of its own. The probe left 600 ms between keystrokes, so each keystroke was
its own CodeMirror history group, and the second undo in each case
re-entered the probe's own `setValue` seeding — those snapshots are noise.

**The closer is ordinary text.** Typing `(` then `a` and running save left
the file reading `(a)` (char codes 40, 97, 41, decimal). That is a decoded
string rather than a byte read; the file's prior content and whether a
trailing newline survived were not recorded.

### The 2026-10-25 pass: what decides, in one rule

These rows ran against **Obsidian 1.14.5**, not the 1.14.4 the rest of this
section was observed on: the window title of the probe's own launch reads
`Obsidian 1.14.5`, so the app had updated itself since 2026-10-18. Nothing the
earlier pass recorded changed.

The rows this section listed as unprobed were run on 2026-10-25 over CDP
(`tools/obsidian-probe/editing-typing.mjs`, results in
`tools/obsidian-probe/results/2026-10-25-editing-typing.json`). These
keystrokes go through `Input.dispatchKeyEvent`, so unlike the 2026-10-18 pass
they are injected by the browser and the Backspace, Delete and undo rows are
no longer synthetic events with nothing behind them.

**One rule covers every position either pass ran.** A pair closes when:

- **brackets** `(` `[` `{` — the character *ahead* is whitespace, the end of
  the line, or a closing bracket. The character behind does not matter:
  `say.|` closes (`say.()`) and `word|` closes.
- **the markdown and quote characters** `*` `_` `` ` `` `"` `'` — the
  character ahead is whitespace or the end of the line **and** the character
  behind is whitespace or the start of the line.

The five positions that settle it, with the previous pass's rows folded in:

| Caret | `(` | `*` `_` `` ` `` `"` |
| --- | --- | --- |
| `say \|` end of line | closes | closes |
| `say \| word` space ahead, more text after | closes | **closes** |
| `say \|x` word ahead | no | no |
| `say \|.` punctuation ahead | **no** | no |
| `say.\|` end of line, punctuation behind | closes | **no** |
| `\tx \|` after a space that follows a tab | closes | closes |
| `\|` empty document | closes | closes |
| `\|word` at ch 0 | no | no |

So it is the character *ahead* that carries the markdown rule, not "the line
ends here": `say | word` pairs. And the bracket rule is narrower than the
earlier reading — punctuation ahead blocks a bracket too.

`_` behaves exactly like `*`. `~ = $ % <` close in none of the five positions,
and a second one just adds the character (`~~`, `==`, `$$`, `%%`, `<<`).

**Skip-over is provenance-tracked for every closer, not only `)`.** Typing
the opener and then its own closer leaves the pair alone and moves the caret
past it, for `)` `]` `}` `"` `'` `` ` `` `*` `_` alike, both on an empty
document and after a space. A **planted** closer with no opener in the
document is inserted instead: `x|)` typed `)` gives `x))`, and the same for
every other pair. The provenance survives moving within the line — type `(`,
press End, press Home, press Right, type `)` and it still skips — but **not
leaving the line**: in `first(|)` / `second`, Down then Up then `)` gives
`first())`.

**No context suppresses pairing.** A blockquote, a nested blockquote, a
fenced code body, a `$$` block, a heading, a list item and a task item all
pair exactly like the plain control, in Live Preview and in source mode. The
one place a pair does not form is inside a closed inline code span, and that
is the ahead-character rule rather than a context rule: at
``x `say |` `` the character ahead is the closing backtick, so neither `(`
nor `*` closes. (In Live Preview the caret cannot be put inside a rendered
code span by offset, so the Live Preview rows for a mid-line span typed
outside the widget and are not comparable.)

**Forward Delete between an empty pair deletes only the closer**, whether the
editor inserted the pair or it was planted: `(|)` becomes `(`, and the same
for `[]`, `""`, `**` and ` `` `. That is ordinary forward deletion — the
control `pl|ain` gives `plin`, and `(|x)` gives `()` — and it is the opposite
of Backspace, which deletes both characters.

**`[[` auto-pairs, and `]` skips over.** Typing `[` `[` gives `[[]]` with the
caret at ch 2 and the file suggester open; typing `]` then `]` moves the
caret to ch 3 and ch 4 without changing the document, and Escape first does
not stop the skip. With a name typed the same holds: `[[Ty|]]` plus `]` `]`
leaves `[[Ty]]` with the caret at ch 6. A third `[` nests (`[[[]]]`), and
`!` `[` `[` gives `![[]]`.

**An abandoned closer is never removed.** Type `(` at `seed|`, press End, type
` tail`: the document is `seed() tail` and saving writes exactly those bytes.

**A run of characters typed inside a fresh pair undoes as one step.** `(`
then `a` `b` `c` at about seven keys a second, then one Ctrl+Z, leaves `seed`
— opener, closer and all three characters go together, unlike the 2026-10-18
pass whose 600 ms gaps made each key its own history group.

**Unprobed:** whether both toggles are Obsidian's factory defaults (they were
read from the API in a scratch vault, not from a pristine `app.json`); whether
the 2026-10-18 "source mode" rows really ran in source mode, since nothing
read the mode back after the switch and they are byte-identical to the Live
Preview rows; whether the `**` in that pass's backspace rows was typed or
planted; the caret inside a rendered inline-code widget in Live Preview, which
cannot be reached by offset; and behavior under Vim mode, on mobile, or with
an IME composition in flight.

## Observed attachment paste behavior, Obsidian 1.14.4

Observed on 2026-10-18 with a throwaway plugin in a scratch vault, on Obsidian
1.14.4 (installer build 1.13.7): synthetic `paste` events carrying a `File`
item were dispatched on the CodeMirror content element, and the vault listing,
the editor text, the bytes written and the bytes on disk were read back around
each one. Twenty-three pastes over seventeen steps, every step reporting `ok`.
The vault's baseline was `attachmentFolderPath: "/"`, `newLinkFormat:
shortest`, `useMarkdownLinks: false`, `alwaysUpdateLinks: false`, and the
naming, collision, format, cursor and undo steps all ran at that baseline, so
their files land at the vault root.

**Naming.** A pasted image is saved as `Pasted image ` + a 14-digit
**local**-time stamp `yyyyMMddHHmmss` + the extension. The run's clock
separated the two readings (local `20261018205534`, UTC `20261019015534`,
`America/Chicago`) and the file took the local form.

Which clipboard items get that name, and which keep their own:

| Clipboard item | Written as |
| --- | --- |
| `image/png`, clipboard name not recorded | `Pasted image 20261018205534.png` |
| `image/png`, clipboard name `image` (no extension) | `Pasted image 20261018205535 1.png` |
| `image/jpeg`, clipboard name `image.jpg` | `Pasted image 20261018205535.jpg` |
| `image/webp`, clipboard name `image.webp` | `image.webp` |
| `image/gif`, clipboard name `image.gif` | `image.gif` |
| `text/plain` file item, clipboard name `notes.txt` | `notes.txt` |

The run does not separate "the item's own name is ignored" from "the item's
name carried no usable extension": the only png whose clipboard name was
recorded was called `image`, and the jpeg's name already ended `.jpg`. Every
one of these, the plain-text file included, was inserted as an embed
(`![[notes.txt]]`).

**Collisions** append a space and an ascending integer before the extension,
and the comparison is against the whole file name, extension included:
`Pasted image 20261018205535.png` and `Pasted image 20261018205535.jpg` both
exist unsuffixed, while a second png in that same second became `Pasted image
20261018205535 1.png`. Three pastes inside one wall-clock second gave ` 1`,
` 2`, ` 3`. `fileManager.getAvailablePathForAttachment` resolves the same way
and continues past existing suffixes (asked for a name whose ` 1`–` 3` were
taken, it returned ` 4`); it returns a free name, a dotted name and a name
with no dot unchanged, and creates no file.

**Bytes.** For all five formats the written byte count equals the source byte
count (png 68/68, jpeg 160/160, webp 44/44, gif 42/42, text 24/24) and the
first sixteen bytes carry each format's own magic (`89 50 4e 47 …`,
`ff d8 ff e0 … JFIF`, `RIFF…WEBP`, `GIF89a`). No hash or byte-for-byte
comparison was recorded and the fixtures are 24–160 bytes, so this is evidence
against re-encoding rather than proof of its absence.

**Insertion.** The embed goes in at the cursor with no padding added on either
side (`BEFORE ![[…]]AFTER`). Over a selection it replaces it (`BEFORE `
selected gives `![[…]] AFTER`). With two cursors one paste writes **one** file
and inserts the same link at both (`one![[…]]\ntwo![[…]]`), leaving two
selections. Right after a paste the note was on disk with exactly the editor's
text — 49 bytes, trailing newline kept.

One undo put the text back to `BEFORE AFTER` in a single step and left the
attachment alone: the file stayed in the vault index through every entry of
the undo trace and was still listed on disk at 68 bytes at the end of the run.
The trace's later entries show another step's buffer, so only the first undo
is clean evidence of the step size.

A clipboard carrying only `text/plain` and no `File` item creates nothing:
`/home/user/pictures/snap.png` was inserted verbatim. That is a path-shaped
clipboard, not an OS file-manager copy.

Every paste that recorded the DOM event came back `defaultPrevented` — the
text-only one that wrote no file included — so the flag does not separate
Obsidian's handler from CodeMirror's own paste handling.

**Where the file lands.** `fileManager.getAvailablePathForAttachment` for each
`attachmentFolderPath` and two source notes (API only, no file written):

| `attachmentFolderPath` | Source note | Path returned |
| --- | --- | --- |
| `""` or `/` | `Target.md`, `Sub/Nested.md` | vault root |
| `Attachments` | `Target.md`, `Sub/Nested.md` | `Attachments/` |
| `./att` | `Target.md` | `att/` |
| `./att` | `Sub/Nested.md` | `Sub/att/` |
| `Sub` | `Target.md`, `Sub/Nested.md` | `Sub/` |
| `Blocker.md` (an existing **file**) | `Target.md`, `Sub/Nested.md` | vault root |

Three settings were confirmed by a real paste, each from a single source note:
`Attachments` from `Target.md` wrote `Attachments/…`, `./att` from
`Sub/Nested.md` wrote `Sub/att/…`, and `Blocker.md` from `Target.md` wrote to
the vault root. The rest of the table is API output only.

**The link a real paste writes**, from `Sub/Nested.md` to a uniquely named
file in `Attachments/`:

| `useMarkdownLinks` | `newLinkFormat` | Inserted |
| --- | --- | --- |
| false | `shortest` | `![[Pasted image 20261018205538 1.png]]` |
| false | `relative` | `![[../Attachments/Pasted image 20261018205539.png]]` |
| false | `absolute` | `![[Attachments/Pasted image 20261018205539 1.png]]` |
| true | `shortest` | `![](Pasted%20image%2020261018205539%202.png)` |
| true | `relative` | `![](../Attachments/Pasted%20image%2020261018205539%203.png)` |

`absolute` carries no leading slash. A markdown link has **empty** link text
and its spaces encoded as `%20`; no name in the run held any other character
whose encoding would show. `app.fileManager.generateMarkdownLink` over the
same file returns these same forms without the leading `!`.

`shortest` is not "the bare name unless the basename is ambiguous". Three real
pastes wrote the same base name into three folders, in this order:
`Attachments/Pasted image 20261018205538.png` from `Target.md` gave
`![[Pasted image 20261018205538.png]]`; then the same name under `Sub/att/`
from `Sub/Nested.md` gave the full path `![[Sub/att/Pasted image
20261018205538.png]]`; then the vault-root copy from `Target.md` gave the bare
name again, with the other two already in place. What the three agree on is
the shortest form that resolves back to *that* file from the source note.

**Notices.** The eight pastes that recorded a `notices` array recorded none —
including the paste whose attachment folder pointed at an existing file, and
the text-only paste — as did the final restore step. The other fifteen pastes
record no notice field at all.

At the end of the run, after the probe restored `attachmentFolderPath` to `/`,
`newLinkFormat` to `shortest` and `useMarkdownLinks` to `false`,
`.obsidian/app.json` held `{"foldHeading": true, "propertiesInDocument":
"visible"}` — none of the three keys present, while `propertiesInDocument` sat
there at its own default. How the restore wrote those three keys is not
recorded, so this does not settle whether setting a key to its default removes
it (see [[Specification#Observed settings write behavior, Obsidian 1.14.4]],
which observed the opposite through `vault.setConfig`).

**Unprobed as of 2026-10-18** — the 2026-10-25 pass below answers most of
this paragraph: whether a *paste* creates a missing attachment folder — the
folders already existed by the time the real-paste step ran — and whether the
API call is what created them: `Attachments`, `att` and `Sub/att` were folders
afterwards and `att` never received a file, but the run's "before" listing is
a file listing and cannot show a folder was absent. `attachmentFolderPath =
"./"` (Obsidian's "Same folder as current file") was never set, nor a deep
non-existent path. `useMarkdownLinks` with `absolute` was never pasted for
real, only generated by the API. No png with a real, distinct clipboard name
(`photo.png`) was ever pasted, so the rename rule for named png and jpeg items
is unsettled, as is the extension chosen for a `image/jpeg` item named `*.jpeg`
or unnamed. Nothing beyond png, jpeg, webp, gif and `text/plain` was pasted —
no svg, avif, bmp or pdf — and no attachment name contained `#`, `?`, `&`,
parentheses or non-ASCII, so only the space is known to be percent-encoded. A
real OS file-manager copy and drag-and-drop were not exercised; neither were
reading view, a canvas or any non-markdown view. Whether the note's save was
the paste's doing or the probe's is unrecorded, as is whether any further undo
ever removes the attachment, whether a Notice appears for the fifteen pastes
that recorded none, whether the `Pasted image` prefix is localized or
configurable, and what a collision across different folders does.
`alwaysUpdateLinks` was read (`false`) but never varied, and `appVersion` was
null, so only the API version 1.14.4 and installer 1.13.7 identify the build.

### The 2026-10-25 pass: a real clipboard image, and a real file drop

`tools/obsidian-probe/attachment-paste.mjs` against **Obsidian 1.14.5**,
results in `tools/obsidian-probe/results/2026-10-25-attachment-paste.json`.
The image goes on the system clipboard through Electron
(`nativeImage.createFromPath` on a file **outside** the vault) and the paste is
a real Ctrl+V; the drop goes through `Input.dispatchDragEvent` with a `files`
payload, which is how a file manager's drag arrives.

**A paste creates the whole missing folder chain.** With
`attachmentFolderPath: "att/deep/new"` and no `att` folder in the vault, one
Ctrl+V produced `att/deep/new/Pasted image 20261025103853.png` and wrote
`BEFORE ![[Pasted image 20261025103853.png]]AFTER` — the three folders were
created by the paste, with no notice.

**`./` is the note's own folder.** With `attachmentFolderPath: "./"`, pasting
in `Sub/Nested.md` wrote `Sub/Pasted image 20261025103857.png`.

**An image on the clipboard is always a PNG named `Pasted image` plus fourteen
digits** — `Pasted image YYYYMMDDHHMMSS.png`, local time, no space before the
digits and no counter. Because the name carries a second-resolution timestamp,
a collision is practically unreachable: pasting twice into a vault that already
held a root-level `photo.png` produced two differently-named files and no
numbering. (The svg, bmp and pdf rows of this run are void: `nativeImage`
cannot read those files, so the clipboard still held the previous PNG. What a
*named* non-image file does on the clipboard is still unprobed; the drop below
is the nearest answer.)

**With `useMarkdownLinks: true` and `newLinkFormat: absolute`** the paste
writes `![](Attachments/Pasted%20image%2020261025103931.png)` — a Markdown
embed, the vault-absolute path with **no leading slash**, and spaces
percent-encoded as `%20`.

**A paste into reading view does nothing at all**: no file written, no notice,
the note byte-identical.

**A paste into a canvas adds a file node.** `Board.canvas` went from
`{"nodes":[],"edges":[]}` to one node, and the attachment landed under
`attachmentFolderPath` as usual:

```
{
	"nodes":[
		{"id":"73e07abbeb4607bd","x":0,"y":0,"width":400,"height":400,"type":"file","file":"Attachments/Pasted image 20261025103944.png"}
	],
	"edges":[]
}
```

So a pasted image becomes a 400×400 `type: "file"` node at the origin, in the
one-line-per-node tab-indented shape
[[#Observed canvas write behavior, Obsidian 1.14.4 (installer build 1.13.7)]]
records.

**The note reaches disk about two seconds after the paste.** Polling the file
while nothing else touched it: the editor held the embed immediately, and disk
still read `BEFORE AFTER\n` at 0, 500 and 1000 ms and the embed at 2000 ms and
after. So the paste itself does not save; the ordinary autosave does.

**One undo removes the whole paste, and the attachment file is never
removed.** The first Ctrl+Z took `BEFORE ![[Pasted image …]]AFTER` back to
`BEFORE AFTER` in a single step; further presses walked back into earlier
history rather than doing anything more to the paste. Through five undos the
attachment folder kept all six files it had accumulated — nothing deletes the
image.

**A real file drop into the editor keeps the file's own name.** Dropping
`photo.png` on the editor wrote `Attachments/photo.png` with the source's exact
74 bytes and inserted `![[Attachments/photo.png]]` — the **full path**, not the
shortest form, because a `photo.png` already existed at the vault root and the
basename alone would have been ambiguous.

**Unprobed after this pass:** what a *named* non-image file on the clipboard
does (the harness can only put an image there); a collision between two
attachments of the same name, which the timestamped name makes hard to
provoke; a drop onto a folder row in the explorer or onto a canvas; and whether
the `Pasted image` prefix is localized.

## Observed file explorer drag-and-drop and inline rename behavior, Obsidian 1.14.4

Observed on 2026-10-18 with a throwaway plugin in a scratch vault, Obsidian
1.14.4 (installer build 1.13.7), dispatching synthetic `DragEvent`s over real
explorer rows, calling the explorer view's own rename entry points, and reading
the vault back after every step. Every move below is backed by both the
`fileManager.renameFile` call the probe saw and the vault diff it read
afterwards. The run forced `alwaysUpdateLinks` on (it had been off) and left it
on; `newLinkFormat` was `shortest`, `useMarkdownLinks` off.

Every row in the file explorer, file and folder alike, carries
`draggable="true"` — all 37 rows present. The payload the row sets has the keys
`source, type, icon, title, file`:

| Dragged | `type` | `icon` | `title` | `file` |
| --- | --- | --- | --- | --- |
| `Root.md` | `file` | `lucide-file` | `Root` | `Root.md` |
| `Deep` (folder) | `folder` | `lucide-folder-open` | `Deep` | `Deep` |

So a file's `title` is its basename without the extension. Only `.md` files
were ever dragged.

Where a drop lands:

| Dropped on | Result |
| --- | --- |
| a folder row | the file moves into that folder (`Root.md` → `Dest/Root.md`) |
| a file row | the file moves into *that file's parent folder* (`Inbox/Gamma.md` dropped on `Notes/Beta.md` → `Notes/Gamma.md`) |
| the folder it already lives in | nothing: no `renameFile` call, no notice, no change on disk — though the hover highlight and the `Move into “Dest”` label still appear |
| `.nav-files-container` (the scrolling file list) | the file moves to the vault root (`Dest/Root.md` → `Root.md`) |
| a workspace tab header | accepted, but as a different action: the label reads `Open as new tab` and a `.workspace-drop-overlay` element appears |
| `.nav-header`, `.nav-buttons-container`, `.workspace-leaf-content`, `.workspace-drawer-vault-name` | nothing is accepted: no highlight, no new drag action |

The drag ghost is a `.drag-ghost` element holding the item's title plus an
action line, `Move into “Dest”`, in curly quotes. Over the root zone the name
in it was `probe` — which is both the vault's name and its folder name in this
run, so the two are not distinguished. A ghost with no action line is 31 px
tall against 46 px for one that carries one.

The hover highlight is the class `is-being-dragged-over`, and it lands on the
*container*, not on the row that received the event: on the outer
`.tree-item.nav-folder` while `.tree-item-self.nav-folder-title` is unchanged,
and on `.nav-files-container` itself for the root zone. A file row, although it
accepts the drop, takes no highlight anywhere — the ghost's action line is the
only affordance.

A name collision at the destination neither prompts nor refuses. `Notes/Alpha.md`
dropped onto `Dest`, which already held `Alpha.md`, moved in as `Dest/Alpha 1.md`
— space before the digit — leaving `Dest/Alpha.md` untouched and showing no
modal. That is one collision, not a series.

A move that changes the file's basename rewrites the links to it in `shortest`
form and shows a Notice reading exactly `Updated 1 link in 1 file.` — `Linker.md`
went from `- unrelated: [[Notes/Alpha]]` to `- unrelated: [[Alpha 1]]`. With
`alwaysUpdateLinks` on no prompt or modal appears at any point in the run. Moves
that leave the basename alone produced no notice at all, and the note read
immediately after `Root.md` moved to `Dest/Root.md` still said
`- markdown: [Root](Root.md)` and `- wikilink: [[Root]]`.

A folder dragged onto its own descendant (`Deep` onto `Deep/Sub`) did nothing:
no highlight, no action line on the ghost, no drag action recorded, vault
unchanged. No folder was dragged onto a *valid* folder anywhere in the run, so
this is not isolated from "folder drags offer no move under synthetic
`DragEvent`s" — only the vault-unchanged half is certain. The API behind it is
not guarded the same way: `fileManager.renameFile("Deep", "Deep/Sub/Deep")`
fails with a raw `EINVAL: invalid argument, rename '…/Deep' -> '…/Deep/Sub/Deep'`,
and leaves no partial state (`Deep/`, `Deep/Sub/`, `Deep/Sub/Leaf.md` all still
there), while `renameFile("Deep", "Deep")` is a silent no-op.

A collapsed folder hovered continuously during a drag spring-loads open between
the 700 ms and the 800 ms tick (collapsed with 1 child row at 700, expanded with
2 at 800). One run, sampled at 100 ms, with synthetic events.

Inline rename edits the row in place. The row's
`.tree-item-inner.nav-file-title-content` div becomes `contenteditable="true"
spellcheck="true"` and the row gains `is-active has-focus is-being-renamed`:

```
<div class="tree-item-self nav-file-title tappable is-clickable is-active has-focus is-being-renamed" data-path="blurme.md" draggable="true">
  <div class="tree-item-inner nav-file-title-content" contenteditable="true" spellcheck="true">blurme</div>
</div>
```

The editable holds the basename alone and the whole of it is selected when the
editor opens. For a non-markdown file the extension sits outside the editable in
a sibling `.nav-file-tag` div — `cover.png` renders as `cover` + `png` — and is
not part of the editable text.

Blur commits: `blurme.md` became `blurred.md`, extension intact, with the
editable closed afterwards. Escape abandons the edit, and a blur dispatched
after the Escape does not commit the discarded text (`escme.md` survived both).
The probe set the editable's text by a means it did not record, so how a typed
character reaches the commit path (`onFileRenameInput` versus plain
`textContent`) is not established by this run. The explorer view exposes the
whole state machine: `startRenameFile`, `onFileRenameInput`, `onTitleBlur`,
`saveRename`, `exitRename`, `onKeyEnterInRename`, `onKeyEscInRename`,
`onKeyRename`, `displayError`.

The command `workspace:edit-file-title` is present, is named `Rename file`, and
the registry declares F2 with no modifiers as its default (user hotkeys: none).
That is registry data: every F2 keystroke the run actually dispatched did
nothing, and all three inline renames were opened by calling
`view.startRenameFile` directly. Invoked with focus in a CodeMirror editor the
command made a title element editable with its whole basename selected and
renamed nothing by itself — but the two invocations, from the same
`cm-content` focus, produced two different elements: once `.inline-title`
(text `Linker`), once `.view-header-title` (text `Beta`). Which element the
command targets, and on what, is not settled by this run.

Ctrl+click on explorer rows opened files rather than selecting them: markdown
leaves went from 0 to 2, and afterwards the only row carrying an extra class was
`Dest/Keep.md` with `is-active`. The probe's read of the view's selection list
returned null (a failed read, not an observed empty), and the drag payload it
then captured was for a row it had not clicked, so nothing here shows whether a
selection travels with a drag. The API says multi-selection is modelled:
`dragManager` has `dragFiles` alongside `dragFile` and `dragFolder`, the
explorer view has `dragFiles` and `onDeleteSelectedFiles`, and every file and
folder item carries a `selectable` field.

Two recording caveats for anyone re-running this. The recorder double-records:
the one blur rename appears twice in `renameFileCalls` and the one link notice
twice in `notices`, so no count may be read off those arrays — only the text.
And the highlight detector watches class churn document-wide: one capture picked
up an unrelated `search-result-container … is-loading`, so a positive is not
automatically drag-related (the negatives are the stronger evidence). The scratch
vault's `alwaysUpdateLinks` was deliberately left forced on, which will bias a
re-run from the same vault.

**Unprobed as of 2026-10-18** — the 2026-10-25 pass below answers most of
this paragraph: whether F2 pressed with focus genuinely on an explorer row opens
the in-row editor, and what `workspace:edit-file-title` does with focus in the
explorer rather than the editor (the "explorer" context never took focus — the
active element was `cm-content` in both invocations); whether Enter commits an
inline rename (`onKeyEnterInRename` exists but no Enter was dispatched); what an
inline rename onto a name already taken does, and whether committing an inline
rename on a non-markdown file keeps its extension (the `cover.png` editor was
opened but never committed); inline rename of a *folder*, never opened at all;
whether a drag-move prompts before rewriting links with `alwaysUpdateLinks` off;
whether a markdown-style link's destination is rewritten on a move at all, and
after what delay, since the only read was immediate and the file later moved
back; whether `fileManager.renameFile` auto-numbers or errors on a collision
(the direct API check could not run — the drop had already moved the file); what
a drop on a file row does when the destination folder already holds that name;
whether a folder dropped on a valid, non-descendant folder moves at all under
these synthetic events, the control this run lacks; whether `.view-content`,
and a collapsed folder's children area, are drop zones (the `.view-content`
probe grabbed `.workspace-leaf-content` twice, and a planned `root-fileItem`
probe found no element); whether the explorer multi-selects under any other
modifier and whether `dragManager.dragFiles` can carry several paths; when the
`is-being-dragged-over` and `is-being-dragged` classes are torn down (one step
still carried both after its drop, four did not, and no `dragend` was ever
dispatched); whether dropping between rows reorders or inserts; what a drop on
an editor pane or another vault window does; and whether a drag-move offers an
undo path.

### The 2026-10-25 pass: real drags, and the inline rename committed

`tools/obsidian-probe/explorer-dnd.mjs` against **Obsidian 1.14.5**, results in
`tools/obsidian-probe/results/2026-10-25-explorer-dnd.json`, with
`alwaysUpdateLinks: false` in `app.json` — the opposite of the 2026-10-18
run's forced-on setting, which is the configuration this block's open
questions were about.

`Input.dispatchMouseEvent` never raises `dragstart`, which is why the earlier
pass could not establish what a drop does. This one uses
`Input.setInterceptDrags` plus `Input.dispatchDragEvent` (`cdp.mjs`'s
`dragAndDrop` and `dragOverOnly`), so Chrome itself produces the drag the
compositor would, and the probe can also read the payload the source offered
and hold the drag over a target before releasing.

**The drag payload.** A file row offers two items, both the same URI:

| MIME type | Data |
| --- | --- |
| `text/plain` | `obsidian://open?vault=<vault name>&file=Root` |
| `text/uri-list` | the same |

The file name is the **basename without the extension**. A *folder* row
offers `text/plain` only, no `text/uri-list`.

**The highlight classes.** While a file is held over a folder row, the source
row carries `is-being-dragged` and the **target folder's outer `tree-item`**
carries `is-being-dragged-over` (`tree-item nav-folder is-being-dragged-over`),
not the title element. Both are gone after the drop. Held over the empty area
below the last row, `.nav-files-container` itself takes
`is-being-dragged-over` — so the gap below the tree is a drop zone (the vault
root), which the earlier pass could not confirm.

**What a drop does.**

| Drop | Result |
| --- | --- |
| file on a folder row | moves the file into that folder (`Root.md` → `Dest/Root.md`), no notice |
| file on a folder row, name already taken | **auto-numbers**: `Notes/Alpha.md` dropped on `Dest/` where `Dest/Alpha.md` exists becomes `Dest/Alpha 1.md` — space, then `1` — and the existing file is untouched |
| file on a **file** row | moves it into that file's folder: `Inbox/Gamma.md` dropped on `Dest/Keep.md` became `Dest/Gamma 1.md`, `Dest/Gamma.md` untouched |
| folder on a folder row | moves the whole folder (`Deep/` → `Dest/Deep/`, children included) |
| folder on its own descendant | **nothing** — the tree is byte-identical; this is the control the earlier run lacked |
| `dragCancel` instead of a drop | nothing moves |

So `fileManager.renameFile` auto-numbers on a collision rather than erroring,
and the suffix is ` 1`.

**With `alwaysUpdateLinks: false`, Obsidian asks — but only when a link would
actually change.** Moving `Root.md` into `Dest/` left `Linker.md`
byte-identical immediately and four seconds later, with no modal: the shortest
-path link `[[Root]]` still resolves, so nothing had to change. Moving
`Notes/Alpha.md` to `Dest/Alpha 1.md`, which changes the basename, raised a
modal:

| | |
| --- | --- |
| Title | **Update links** |
| Text | `Do you want to update internal links that link to this file?` then `This will affect 1 link in 1 file.` |
| Buttons | **Always update**, **Just once**, **Do not update** |

Dismissed with Escape, `Linker.md` stayed byte-identical.

**There is no undo for a drag-move.** Ctrl+Z with the explorer focused after
the file-row drop left the tree exactly as the drop had left it, and raised no
notice.

**Ctrl+click does not multi-select; Shift+click does.** Ctrl+clicking three
file rows in turn left exactly one row marked (`is-active`) and the view's own
selection list empty. Shift+clicking back to the first row marked all three
with **`is-selected`** — a contiguous span.

**The inline rename.** F2 only works with focus genuinely on a row, which a
click cannot give: clicking a file row opens the file and hands focus to
CodeMirror. Entering the tree through `file-explorer:reveal-active-file`
leaves the row focused (`has-focus`), and F2 then turns the row's own label
into the editor:

- the editable element is the row's `.tree-item-inner.nav-file-title-content`
  (and `.nav-folder-title-content` for a folder), made `contenteditable`
- the row gains **`is-being-renamed`** beside `has-focus` and `is-active`
- the **basename without the extension** is pre-selected (`Beta`, not
  `Beta.md`; `cover`, not `cover.png`)
- `document.activeElement` stays `BODY` — the selection is inside the
  contenteditable but focus is not reported on it

| Committed with Enter | Result |
| --- | --- |
| `Beta` → `Beta Renamed` | `Notes/Beta.md` → `Notes/Beta Renamed.md`, no notice, no modal, editor closes |
| `cover` → `renamed cover` on a `.png` | `renamed cover.png` — **the extension is kept** without typing it, and `![[cover.png]]` in another note was *not* updated |
| `Keep` → `Alpha` where `Dest/Alpha.md` exists | **refused silently**: the tree is unchanged, there is no notice and no modal, and the inline editor **stays open** showing `Alpha` |
| the empty name | the same: nothing renamed, no notice, editor stays open |

So an inline rename refuses a collision where a *drag* auto-numbers it.

**A folder's inline rename comes from the context menu.** Clicking a folder
row does not focus it (`has-focus` on no row, `activeElement` BODY), so F2 does
nothing; picking **Rename...** from the folder's context menu opens the same
in-row editor with the folder name pre-selected and no modal.

## Observed graph.json write behavior, Obsidian 1.14.4 (installer build 1.13.7)

Observed on 2026-10-18 with a throwaway plugin in a scratch vault (47 nodes, 15
links), polling `.obsidian/graph.json`'s bytes, size and mtime around each
interaction. One run, one vault, no restart; the version is the baseline step's
`appVersion` — the run's own `meta.appVersion` is null.

**Obsidian creates the file; an enabled plugin does not.** At baseline the graph
core plugin was found and enabled, and `.obsidian` held only `app.json`,
`appearance.json`, `community-plugins.json`, `core-plugins.json`,
`workspace.json` and a `plugins/` folder — no `graph.json`. Opening the global
graph view created it, once, with nobody touching the controls. It was present
at the first poll after the view opened, and the poller ran at roughly 120 ms
(14 polls over 1691 ms), so every latency below is a poll-granularity upper
bound, not a measurement.

The created file is a complete twenty-key document — never sparse — in this
order, with these values:

```json
{
  "collapse-filter": true,
  "search": "",
  "showTags": false,
  "showAttachments": false,
  "hideUnresolved": false,
  "showOrphans": true,
  "collapse-color-groups": true,
  "colorGroups": [],
  "collapse-display": true,
  "showArrow": false,
  "textFadeMultiplier": 0,
  "nodeSizeMultiplier": 1,
  "lineSizeMultiplier": 1,
  "collapse-forces": true,
  "centerStrength": 0.518713248970312,
  "repelStrength": 10,
  "linkStrength": 1,
  "linkDistance": 250,
  "scale": 1,
  "close": false
}
```

All four control sections start collapsed. Two-space indentation, one key per
line, no trailing newline — the last byte is `}` and the file is 494 bytes.
Numbers come out the way JavaScript prints them: integers with no decimal point
(`0`, `1`, `10`, `250`) and everything else at full round-trip precision, with
no fixed number of decimal places — `0.518713248970312` at fifteen significant
digits, and `5.0625` for a zoom. Later states measured 503, 498 and 497 bytes.

**The file is a serialization of the graph core plugin's own options object.**
The plugin instance's `options` holds exactly the same twenty keys in exactly
the same order as the file, and its prototype carries `saveOptions`,
`onExternalSettingsChange`, `openGraphView` and `openLocalGraph`. That accounts
for the fixed key order, the whole-document rewrite, and for no `local*` key
ever reaching the file.

Every observed file state carries those same twenty keys in that same order —
the creation write, the writes whose bytes were recorded afterwards, and every
later snapshot. One interaction changes exactly one value and leaves the other
nineteen untouched; nothing is appended and nothing is reordered. Whether a
write is atomic is not observable from size/mtime polling.

What wrote, and what it wrote:

| Interaction | Written | Seen |
| --- | --- | --- |
| Open the global graph view | the whole file, created | first poll (~120 ms grain) |
| Click the `Filters` section header | `collapse-filter` `true` → `false` | one write, ~300 ms after the click |
| Click `Groups` | `collapse-color-groups` `true` → `false` | one write, ~300 ms |
| Click `Display` | `collapse-display` `true` → `false` | one write, ~300 ms |
| Click `Forces` | `collapse-forces` `true` → `false` | one write, ~300 ms |
| Wheel zoom | `scale` `1` → `5.0625`, then a second write `5.0625` → `8` | one write each, see below |
| Click the panel's Close button | `close` `false` → `true` | one write, ~100 ms |
| Open a local graph | nothing | 0 writes in ~2.8 s |
| Detach every graph view | nothing | 0 writes in ~1.2 s |

Only the expand direction of a section header was exercised: every click went
`true` → `false`, and re-collapsing was not tried. The ~300 ms figures are
quantized by the same ~120 ms polling, so they say "a write landed about a third
of a second later", not "the debounce is 300 ms".

**Controls panel.** The panel is `div.graph-controls[data-ignore-swipe="true"]`.
Each section is `div.tree-item.graph-control-section` plus one of `mod-filter`,
`mod-color-groups`, `mod-display`, `mod-forces`, and expanding removes
`is-collapsed` from that div; the header div keeps `tree-item-self
mod-collapsible` unchanged. While a section is collapsed its setting rows are
absent from the DOM, not merely hidden — a query for setting-items under the
collapsed panel returned none. The panel has four
`div.clickable-icon.graph-controls-button`: `mod-close` ("Close"), `mod-open`
("Open graph settings"), `mod-animate` ("Start timelapse animation"),
`mod-reset` ("Restore default settings"). Clicking Close adds `is-close` to
`.graph-controls` and writes `"close": true`; the graph view itself stays open.

**Zoom.** Wheel zoom persists to `scale`, and both values that reached the file
match a wheel-tick target rather than the animating scale: `5.0625` is exactly
1.5⁴, and the second write was exactly `8` at a moment when the renderer read
`7.9319604902379455` at both samples taken around it. The renderer's `scale` was
never sampled at write time, so nothing here shows what the writer reads; what
it does not read is the animated value. The second write's mtime is exactly
2000 ms after the first; a pan drag was dispatched inside that window but moved
nothing (`panX`/`panY` identical before and after), so the second write cannot
be attributed to the drag, to the zoom settling, or to a periodic flush — the
three are indistinguishable in this run. The renderer's `targetScale` read `8`
at every sample after the wheel burst.

`graph.json` carries no pan or offset coordinates in any observed state, and the
key set never grows.

**Local graph.** Opening a local graph wrote nothing to `graph.json`. Its
options live in the workspace leaf state: `type: "localgraph"`, `state.file`
(`"Alpha.md"`), `state.options`. That option set is not `graph.json`'s — it
drops `showOrphans` and adds `localJumps` (`1`), `localBacklinks` (`true`),
`localForelinks` (`true`), `localInterlinks` (`false`). The renderer engine's
own live `options` object is a third, filter-only set: `showTags`,
`showAttachments`, `hideUnresolved`, `showOrphans`, `localFile`, `localJumps`,
`localInterlinks`, `localForelinks`, `localBacklinks`. The `local*` keys exist
in the engine and the leaf state and never appear in the file.

The local graph opened at a point when `graph.json` already held
`collapse-filter: false`, `scale: 8`, `close: true`; its own options read
`collapse-filter: true`, `scale: 1`, `close: false` — the values the file was
created with, and its four control sections were all collapsed while the global
panel's four had been expanded. So a local graph does not start from the current
`graph.json`. Whether it starts from built-in defaults or from a snapshot of the
plugin's options taken earlier in the session cannot be told from one local
graph in one session.

**Confirmed independently on 2026-10-25** against a hand-written `graph.json`
holding `scale: 8`: the global graph opened at `renderer.scale: 7.926` and the
local graph at `renderer.scale: 1`, with the same own-options block
(`localJumps: 1`, `localBacklinks`/`localForelinks` true, `localInterlinks`
false, `scale: 1`). Its leaf title is `Graph of <basename>`, not "Local graph".
The same run found that **`graph:open-local` is not offered at all when no file
is open** — see
[[#Observed UI rules, Obsidian 1.14.5 (installer build 1.13.7)]].

The view types are `graph` (title "Graph view") and `localgraph` (title "Local
graph"), both icon `lucide-git-fork`. `getViewState()` on the global graph leaf
reports an empty `state` object; no write to `workspace.json` was observed, but
the baseline for that file was taken after both leaves were already open.
Detaching every graph view wrote nothing further; the file kept the last state
written while a view was open.

**Unprobed as of 2026-10-18** — the 2026-10-25 pass below answers most of
this paragraph: whether a filter checkbox (`showTags`, `showAttachments`,
`hideUnresolved`, `showOrphans`) writes at all — the sections were still
collapsed, so no row existed to click; whether a slider writes on `input` or
only on `change`, and which key the probed slider owns — its five synthetic
frames ended on the slider's starting value and fired no `change` event, so the
zero writes prove nothing, and only `min -3 / max 3 / step 0.1` was captured
(that range matches the Display section's text-fade control, which makes
`textFadeMultiplier` the likely owner, unconfirmed); whether toggling a *local*
graph option writes anything — that sub-step silently did not run (`toggled`,
`boxKind` and both `boxClass` fields all null) even though the step reported
success; whether pan persists anywhere — the synthetic drag never moved the
renderer; re-collapsing a section (`false` → `true`); the search box,
`colorGroups` edits, `showArrow`, and the "Restore default settings" and
timelapse buttons, none of which were touched (`search` stayed `""` and
`colorGroups` `[]` throughout); the real debounce and whether writes are
periodic; whether a write is atomic (temp file plus rename) or in place; whether
`8` is a zoom clamp, and what the lower bound is; whether
`onExternalSettingsChange` actually makes Obsidian reload `graph.json` when
something else rewrites it, which the prototype suggests but the run never
tested; whether unknown keys in a hand-written `graph.json` survive, since the
file did not exist at baseline and Obsidian only ever rewrote its own; and
whether a local graph's options ever reach `workspace.json`. Every figure above
comes from a single ~14 s run against one 47-node scratch vault: the file shape
and key order are robust to that, the timings and the saturation at `8` are not.

### The 2026-10-25 pass: every control, under real input

`tools/obsidian-probe/graph-writeback.mjs` against **Obsidian 1.14.5**, results
in `tools/obsidian-probe/results/2026-10-25-graph-writeback.json`. The earlier
run's sections were collapsed, so no checkbox or slider row existed to click
and its "zero writes" proved nothing. This one expands all four sections
first, clicks every toggle, drags every slider with a real mouse (scrolling
each into view — the Forces sliders sit below an 800 px window and a drag at
their reported coordinates hits nothing), types in the search box, and polls
`graph.json` every 100 ms to time the write.

**The control panel, in full.** Four `.tree-item` sections, each with
`is-collapsed` when shut:

| Section | Row | Kind | Range | Key it writes |
| --- | --- | --- | --- | --- |
| Filters | *(search box)* | text, placeholder `Search files...` | | `search` |
| Filters | Tags | toggle | | `showTags` |
| Filters | Attachments | toggle | | `showAttachments` |
| Filters | Existing files only | toggle | | `hideUnresolved` |
| Filters | Orphans | toggle | | `showOrphans` |
| Groups | *(no rows)* | | | |
| Display | Arrows | toggle | | `showArrow` |
| Display | Text fade threshold | slider | −3 … 3 step 0.1 | `textFadeMultiplier` |
| Display | Node size | slider | 0.1 … 5 step any | `nodeSizeMultiplier` |
| Display | Link thickness | slider | 0.1 … 5 step any | `lineSizeMultiplier` |
| Display | *(button)* `Animate` | button | | — |
| Forces | Center force | slider | 0 … 1 step any | `centerStrength` |
| Forces | Repel force | slider | 0 … 20 step any | `repelStrength` |
| Forces | Link force | slider | 0 … 1 step any | `linkStrength` |
| Forces | Link distance | slider | 30 … 500 step 1 | `linkDistance` |

So "Existing files only" is `hideUnresolved`, "Text fade threshold" is
`textFadeMultiplier` — the range the earlier run guessed from — and **there is
no "Restore default settings" button**: Display's only button is `Animate`.
Each section's own open/shut state is its `collapse-<section>` key, and
re-collapsing writes `collapse-filter: true` 300 ms later.

**Every control writes, and almost immediately.** Each toggle and each slider
changed exactly one key, and the file had changed by the first 100 ms poll
after the gesture. The search box is the one debounced row: typing `alpha`
reached the file after **401 ms**, and clearing it wrote `search: ""`. So the
"real debounce" question is answered — about 100 ms for a discrete control,
about 400 ms for keystrokes.

**The zoom clamps.** Twenty-five wheel-ups saturate the renderer at
`scale: 7.927` and the file records **8**; forty wheel-downs saturate at
`0.00789` and the file records **0.0078125** (2⁻⁷). So `8` is the upper clamp,
the lower one is 0.0078125, and the written value is not the renderer's live
scale but a rounded one.

**Pan is not persisted anywhere.** `graph.json` has no `panX`/`panY` key in any
state this run produced, and reopening the graph restored only the scale
(`0.0078125` in the file came back as a renderer scale of `0.00788`).

**Unknown keys do not survive.** A hand-written `graph.json` carrying
`silicaUnknownKey: "please keep me"` lost it on Obsidian's first write — the
file is rewritten wholesale. Worth contrasting with Silica's own rule: for
`graph.json`, preserving unknown keys is *not* what Obsidian does.

**An external rewrite is not picked up live, but is read on the next open.**
Writing `scale: 3` into the file from outside left the running renderer at
`0.00788`; detaching and reopening the graph gave `2.9707`. So
`onExternalSettingsChange` does not reload this file while a view is open.

**The local graph keeps its options in `workspace.json`, not `graph.json`.**
Toggling "Incoming links" on a local graph left `graph.json` untouched through
a 6 s poll and changed the leaf's own state instead; `workspace.json`
afterwards mentions both `localgraph` and `localJumps`. Its control panel is
the global one plus four rows at the top of Filters:

| Row | Kind | Key |
| --- | --- | --- |
| Depth | slider | `localJumps` |
| Incoming links | toggle | `localBacklinks` |
| Outgoing links | toggle | `localForelinks` |
| Neighbor links | toggle | `localInterlinks` |

**Unprobed after this pass:** whether a write is atomic (temp file plus
rename) or in place; `colorGroups` edits and the Groups section's own add-group
affordance, which has no `.setting-item` row to click; the `Animate` button;
and whether a slider writes on `input` or only on `change`, which a real drag
cannot separate because it fires both.

## Observed search pane behavior, Obsidian 1.14.4

Two runs. The first, on 2026-10-18, used a throwaway plugin in a scratch
vault, on Obsidian 1.14.4 (installer build 1.13.7): the Search view was opened
through the `global-search` plugin's `openGlobalSearch`, then the view object,
its header elements, its result DOM, the menus its surfaces open, `.obsidian/`
and `localStorage` were read back. `Menu.showAtMouseEvent`,
`Menu.showAtPosition`, `Modal`, `navigator.clipboard` and Electron's
`clipboard` were all patched to record anything the pane did. **That run never
changed the sort order and never invoked the copy action.** The 2026-10-19
observations came from a CDP-driven run — real browser-injected keystrokes and
mouse clicks, with file bytes read from disk by the host
([[Development#Driving Obsidian over CDP, 2026-10-18]]) — on the same Obsidian
1.14.4 (installer build 1.13.7) in a fresh scratch vault of twelve fixtures:
eleven `.md` notes containing `needle` (`_needle.md`, `9 needle.md`,
`Apple Pie.md`, `apple.md`, `apple2.md`, `apple10.md`, `kiwi.md`, `mango.md`,
`zebra.md`, `notes/apple.md`, `notes/banana.md`) and `plain.txt`, every file
given a distinct mtime. Three bounds on that run: each sort switch was a
script-dispatched `change` event on the native `<select>` — ArrowDown on the
focused select changed nothing — which reached `setSortOrder` through the
dropdown's `changeCallback`; the copy modal's two selects were set the same way
(its toggle, the count control, the menu items and the buttons were real
clicks); and the query was empty by the time the toggle and keyboard steps ran,
so those steps settled state keys and handler sources, not visible effects.
A sentence below that names no date holds for both runs.

The view:

- Type `search`, icon `lucide-search`, display title `Search`; the command is
  `global-search:open`, "Search: Search in all files".
- Its serialized state is exactly six keys — `query`, `matchingCase`,
  `explainSearch`, `collapseAll`, `extraContext`, `sortOrder`. `getState` reads
  the first three off the view and the last three off the result-DOM object:
  `t.collapseAll=this.dom.collapseAll,t.extraContext=this.dom.extraContext,t.sortOrder=this.dom.sortOrder`.
  Sort order and the two display flags live on the result list, not on the view.
- In a vault where none was ever chosen, `sortOrder` is `alphabetical` and all
  four flags are `false`. A never-searched leaf already serializes all six:
  `.obsidian/workspace.json` read right after launch, before any search, held
  `"query": "", "matchingCase": false, "explainSearch": false, "collapseAll": false, "extraContext": false, "sortOrder": "alphabetical"`
  for the search leaf, in that order — the keys are emitted even at their
  defaults — and the file-explorer leaf uses the same
  `"sortOrder": "alphabetical"` string.
- `setSortOrder` is `function(e){var t=this.dom;t.sortOrder=e,t.changed()}`: it
  assigns `dom.sortOrder` and calls `dom.changed()`, and nothing else.
  `dom.changed()` is a generic debounce wrapper; its inner body was not captured
  in either run.
- The result list is virtualized — the DOM object carries `infinityScroll`,
  `vChildren`, `resultDomLookup`, `focusedItem`, `pusherEl`, `emptyStateEl`,
  `showingEmptyState` and `working`.
- The view's container holds, in order, `search-row`, `search-info-container`,
  `search-params`, `search-results-info` and
  `search-result-container mod-global-search`. The sort control is a native
  `<select class="dropdown">`, not a menu, and it sits in the
  `search-results-info` bar beside the result count — not in the settings
  section, which holds only the three toggles. (The bar's text content is the
  count, the six option labels and the current label once more, the last from
  the hidden `is-measuring` twin select the dropdown component sizes itself
  with.) The view owns `sortOrderDropdown`, `collapseResultsToggle`,
  `extraContextToggle`, `explainSearchToggle`, `filterSectionToggleEl` and
  `searchParamsContainerEl`, with `toggleFilterSection` on the prototype.
  `onPaneMenu` is `function(e,t){}`, an empty body.
- Other prototype surface that bounds the feature: `saveSearch` (with
  `requestSaveSearch` on the instance), `startSearch`/`stopSearch`,
  `setQuery`/`getQuery`, `renderSearchInfo`, `onCopyResultsClick`,
  `onTabHeaderClick`, and the focus-key handlers `onKeyEnterInFocus`,
  `onKeyArrow{Up,Down,Left,Right}InFocus`, `onKeyShowMore{Before,After}`.
- `startSearch` (source, not driven) calls `stopSearch`, empties the result
  list and clears the focused item, compiles the input with the `matchingCase`
  flag (`new NL(n,o,this.matchingCase)`) and returns early when that yields no
  matcher, re-renders the search-info block, feeds each file through
  `addResult`/`removeResult`, sets the count from
  `labelResultCount({count: dom.getMatchCount()})` when its queue stops, and
  ends with `app.workspace.requestSaveLayout()` then `requestSaveSearch()` —
  every query run schedules a layout save and a recent-searches save.

The header holds three elements:

| Element | Class | Icon | Label |
| --- | --- | --- | --- |
| Match case | `input-right-decorator clickable-icon` | `uppercase-lowercase-a` | `Match case` |
| Search settings | `clickable-icon` | `lucide-sliders-horizontal` | `Search settings` |
| Result count | `clickable-icon search-results-result-count` | `lucide-more-horizontal` | none |

A real click on Search settings switches `search-params` from `display: none`
to `display: flex` and marks the icon `is-active`. The section holds exactly
three `setting-item mod-toggle` rows with empty descriptions, top to bottom:
`Collapse results`, `Show more context`, `Explain search terms`. Match case is
not in it: it is the decorator inside the input, and it gains `is-active` when
on.

The count element's text is `<span>7 results</span>` for four files holding 1,
1, 3 and 2 matches: it counts matches, not files, and names no file count. Its
markup is a `clickable-icon` wrapping a `more-options-icon`. On 2026-10-19 it
read `16 results` over eleven rows whose flairs sum to 14; the two files whose
names contain the term (`_needle`, `9 needle`) carry flair 1 each and render
`needle` in their titles inside `search-result-file-matched-text`. That fits
filename hits counting as results without adding to the file's flair — an
inference from one count, since `getMatchCount` was not read. The same text
read `0 results` once the query had been emptied in-session, and an empty
string right after a restart with no search run.

**A real left-click on the count opens a two-item menu**, both items section
`action`: `Copy search results` (`lucide-copy`) and `Bookmark...`
(`lucide-bookmark`). Escape closes it. The first is the control for
`onCopyResultsClick` — see Copying results; the second was never clicked.

Results, for the query `needle`:

- On 2026-10-18 the order was `apple.md`, `notes/banana.md`, `mango.md`,
  `zebra.md` under `sortOrder: alphabetical` — basename order. Path order
  would put `notes/banana.md` last; creation order (ctime 870845, 870851,
  870852, 870854, 870863 for apple, kiwi, mango, banana, zebra) would put
  mango second; the match counts 1, 1, 3, 2 rule out ordering by number of
  matches.
- Rows start expanded (`collapsed: false` on every row) and carry a match
  count: apple 1, banana 1, mango 3, zebra 2. With `extraContext: false`, each
  match rendered as a single short string.
- A write to `kiwi.md` during the 2026-10-18 run (82 bytes before, 32 after —
  overwritten, not appended) put a fifth file in the list within the 451 ms the
  probe waited, `kiwi.md` inserted between `notes/banana.md` and `mango.md`,
  with `sortOrder` unchanged. 451 ms is the wait the probe used, not a measured
  latency.
- Default search is case-insensitive: on 2026-10-19 `kiwi.md` contained only
  `Needle` and was listed with flair 1 for the lowercase query under
  `matchingCase: false`.
- A `.txt` file holding the term is never listed: `plain.txt` was absent in
  every sort order and again in the copy run, while present in the vault on
  disk.
- A real click on a row's collapse icon (`tree-item-icon collapse-icon`,
  `right-triangle`) collapses that row only (`is-collapsed`) and opens no file
  (`getActiveFile()` stayed null). The click meant to re-expand it later landed
  inside the rect the count control occupied, so re-expansion by a second
  click is unobserved. Each match line (`search-result-file-match tappable`)
  carries two hidden hover buttons labelled `Show more context` —
  `search-result-hover-button mod-top` (`lucide-chevron-up`) and `mod-bottom`
  (`lucide-chevron-down`) — so context expands per match, above and below.

**Sort orders.** The `<select>` has exactly six options and index 0 is selected
in a vault that never chose one. The native popup is not in the DOM and was
never opened; the options and labels were read from its `<option>` children.
Paths below drop `.md`.

| Stored value | Label | Order observed for `needle` |
| --- | --- | --- |
| `alphabetical` | File name (A to Z) | `_needle`, `9 needle`, `notes/apple`, `apple`, `Apple Pie`, `apple2`, `apple10`, `notes/banana`, `kiwi`, `mango`, `zebra` |
| `alphabeticalReverse` | File name (Z to A) | `zebra`, `mango`, `kiwi`, `notes/banana`, `apple10`, `apple2`, `Apple Pie`, `notes/apple`, `apple`, `9 needle`, `_needle` |
| `byModifiedTime` | Modified time (new to old) | `zebra`, `mango`, `_needle`, `apple`, `apple2`, `Apple Pie`, `notes/apple`, `kiwi`, `9 needle`, `notes/banana`, `apple10` |
| `byModifiedTimeReverse` | Modified time (old to new) | the exact reverse of `byModifiedTime` |
| `byCreatedTime` | Created time (new to old) | `apple10`, `notes/banana`, `9 needle`, `kiwi`, `notes/apple`, `Apple Pie`, `apple2`, `_needle`, `mango`, `apple`, `zebra` |
| `byCreatedTimeReverse` | Created time (old to new) | `apple`, `zebra`, then the other nine in their `byCreatedTime` order |

What the orders show:

- Alphabetical compares the basename without extension, not the path:
  `notes/apple` sits with `apple`, `notes/banana` between `apple10` and
  `kiwi`. Case is folded (`Apple Pie` between `apple` and `apple2`); digit
  runs compare numerically (`apple2` before `apple10`); a space sorts before a
  digit (`Apple Pie` before `apple2`); a bare prefix sorts first (`apple`
  before both); `_` sorts before `9`, which sorts before letters. Byte order
  is ruled out three times over — it puts `apple10` before `apple2`,
  `9 needle` before `_needle`, and `Apple Pie` first among the apples — and
  the data is consistent with a locale-aware numeric collation.
- `alphabeticalReverse` reversed every pair except the one basename tie:
  `notes/apple` stayed ahead of `apple` in both directions. **Why, settled on
  2026-10-25** (`tools/obsidian-probe/search-collation.mjs`, and
  [[#Observed UI rules, Obsidian 1.14.5 (installer build 1.13.7)]]): the sort
  is a *stable* sort on the basename over a file list that is already in
  **full-path descending** order. Eight notes named `apple` at different
  depths come back `zzz/`, `z/`, `m/n/`, `m/`, root, `a/b/c/`, `a/`, `APPLE`
  under File name (A to Z) — exactly the path descending, exactly the order
  `app.vault.getMarkdownFiles()` returns, and unchanged under Z to A, which
  reverses only the basename groups. The quick switcher orders the same tie
  the same way. So a note in a folder leads a root note of the same name, and
  the tie-break to implement is full path **descending**.
- `byModifiedTime` matched the fixtures' mtime order exactly, newest first;
  its reverse is the exact reverse. No two fixtures shared an mtime.
- Created time put nine files in one group and `apple`/`zebra`, the two
  written first, in an older one; inside a group the rows kept the order they
  had before the switch (the nine stand in their `byModifiedTimeReverse` order
  with `apple` and `zebra` removed, in both directions). The host's `fs.stat`
  has `apple` and `zebra` at birthtime `…566.33` ms, `kiwi` at `…567.58` and
  the rest from `…567.62` to `…568.04`, with one identical `ctimeMs` for all
  twelve. The grouping fits Obsidian keying on the birthtime rounded to whole
  milliseconds — floor would have split `kiwi` from the rest, `ctime` proper
  would have made a single group — but those are the host's stats, not
  Obsidian's own, so this stays likely.

**Changing the sort order re-sorts the existing list without re-running the
query.** With every method on the result DOM and on the view wrapped in
counters, each of the five switches recorded `view.setSortOrder` 1,
`changeFocusedItem` 1 and `setFocusedItem` 1 (absent in one case), and never
`startSearch`, `stopSearch`, `emptyResults`, `addResult` or `removeResult`.
The count stayed `16 results`, though the sampler started 120 ms after the
change, so it is the counters, not the samples, that rule out a transient. The
one row collapsed by hand before the switches (`_needle`) stayed collapsed
through all five orders and after `alphabetical` was restored. Four of the five
switches left one row carrying `has-focus` (`_needle`, `_needle`, `apple`,
none, `zebra`); which row gets it was not worked out.

Where state lives:

- The six keys persist in the search leaf's `state` in
  `.obsidian/workspace.json`, and nowhere else that was looked. With
  `byModifiedTime`, `extraContext` and `explainSearch` set, the file after
  Ctrl+Q held
  `"query": "", "matchingCase": false, "explainSearch": true, "collapseAll": false, "extraContext": true, "sortOrder": "byModifiedTime"`;
  `app.json` stayed `{}` with no sort key at any point; and after a restart the
  dropdown showed `Modified time (new to old)` and the two toggles came back
  enabled. When the write happened is not known: the bytes were read at launch
  and after quit, and `startSearch` ends in `requestSaveLayout()`, so the save
  may well precede the quit.
- Recent searches live in `localStorage` under `<appId>-recent-searches`,
  holding a JSON array of query strings — `["needle"]` on 2026-10-19, the
  vault's only key, and it survived the restart; `["[\"tags\"]"]` on
  2026-10-18, one of six keys for that vault. That lines up with
  `saveSearch`/`requestSaveSearch` and with `bookmarks:bookmark-current-search`.
- `.obsidian/global-search.json` appeared between launch (when `.obsidian/`
  held `app.json`, `appearance.json`, `core-plugins.json`, `workspace.json`)
  and the post-quit listing; no step read its bytes, so what it holds — and
  whether the first search's `requestSaveSearch` wrote it — is unknown.
- The 2026-10-18 vault's `.obsidian/` held `app.json`, `appearance.json`,
  `community-plugins.json`, `core-plugins.json`, `graph.json` and
  `workspace.json` plus a `plugins/` folder — there was no `search.json`.
  `app.json` was `{"foldHeading": true, "propertiesInDocument": "visible"}`,
  with no `searchSortOrder`, `sortOrder` or `searchSort` key; that absence now
  reads as expected, since the sort order is leaf state.
- The 2026-10-18 scratch vault held `Notes/` and `notes/` as two distinct
  folders, so nothing here speaks to folder-name case handling.

**Copying results opens a modal; nothing reaches the clipboard until its button
is pressed.** `onCopyResultsClick` is
`function(e){e.preventDefault(),new x2(this.app,this.dom).open()}`, and a real
click on the menu item left a sentinel clipboard untouched. The modal is titled
`Copy search results`, has a close button, and its content holds, in DOM
order, a `textarea.copy-search-result-textarea` (prefilled, editable,
`rows=2`), three settings rows, and two buttons:

| Control | Label | Description | Values (stored → label) | Default |
| --- | --- | --- | --- | --- |
| toggle | Show path | Display full path of the file rather than just the file name. | — | off |
| select | Link style | Optionally turn each file result in to a link. | `none` → None, `wikilink` → Wikilink, `markdown` → Markdown link | `none` |
| select | List prefix | Optionally add a list item prefix to each file result. | `none` → None, `-` → Dash (-), `*` → Asterisk (*), `num` → Numbered | `none` |

Buttons: `Copy results` (`mod-cta`) and `Done`. The textarea holds one line
per file in the list's order — the list was in default alphabetical order in
this run; copying under another sort was not tried — joined by `\n` with no
trailing newline, no match lines, no indentation; a file with two matches gives
one line. All 24 combinations were read from the textarea over `apple.md`,
`Apple Pie.md`, `notes/banana.md`, `mango.md`; the `notes/banana.md` line per
combination, before any prefix:

| Show path | Link style | Line |
| --- | --- | --- |
| off | None | `banana` |
| off | Wikilink | `[[banana]]` |
| off | Markdown link | `[banana](banana)` |
| on | None | `notes/banana.md` |
| on | Wikilink | `[[notes/banana.md]]` |
| on | Markdown link | `[banana](notes/banana.md)` |

So the name form is the basename without extension, the path form is the
vault-relative path with extension and folder kept, a Markdown link's text is
always the basename and its target is not percent-encoded
(`[Apple Pie](Apple Pie.md)`). The prefix prepends `- `, `* ` or `1. `, `2. `,
… (numbered from 1 in list order) to every line. A real click on
`Copy results` put the textarea text on the system clipboard byte for byte —
`1. [apple](apple.md)\n2. [Apple Pie](Apple Pie.md)\n3. [banana](notes/banana.md)\n4. [mango](mango.md)`,
LF only, no trailing newline, read through Electron's clipboard — showed the
notice `Results copied to your clipboard.`, and left the modal open; Escape
closed it. `Done` was never pressed, and no `paste.md` was created, so the
bytes a paste lands in a note are unobserved.

204 commands are registered. The ones the 2026-10-18 probe's filter surfaced
that touch copying or search are `workspace:copy-path`,
`workspace:copy-full-path`, `workspace:copy-url`, `bases:copy-table`,
`bookmarks:bookmark-current-search` and `global-search:open`. The other ids
were not recorded, so nothing is established about them.

**The four toggles each flip one state key**, clicked for real with an empty
query, and clicking again restores `false`:

| Toggle | Where | State key | Class when on | Also set on the result DOM |
| --- | --- | --- | --- | --- |
| Collapse results | settings section | `collapseAll` | `checkbox-container is-enabled` | `dom.collapseAll` |
| Show more context | settings section | `extraContext` | `checkbox-container is-enabled` | `dom.extraContext` |
| Explain search terms | settings section | `explainSearch` | `checkbox-container is-enabled` | (the probe read only `collapseAll`, `extraContext`, `sortOrder` off the DOM object) |
| Match case | input decorator | `matchingCase` | `input-right-decorator clickable-icon is-active` | — |

Enabling `Explain search terms` moved its own row down 45 px (y 164 at the
click that enabled it, 209 at the click that disabled it) while the other three
rows stayed put, so it renders something above the settings section even with
an empty query — `search-info-container` precedes `search-params`. The probe's
`[class*="explain"]` selector found nothing, so that block is not classed
`explain`. What the other three do to a populated list was not seen.

Context menus:

- 2026-10-18, synthetic right-clicks: a result's file title opens a 13-item
  menu. A match line opens the same menu with "Delete file" dropped — 12 items,
  every callback byte-identical to the file-title one (item 0 is
  `function(){r.app.workspace.openLinkText(t,n,"tab")}` in both). A match row's
  menu acts on the file; nothing in it is match-specific — no copy-match, no
  line target.
- The dump below is the menu's insertion order with each item's `data-section`,
  not the order a reader sees: Obsidian renders a menu grouped by section,
  which is why three titles are fragments that only read under a Copy heading.

| # | Title | Section |
| --- | --- | --- |
| 0 | Open in new tab | `open` |
| 1 | Open to the right | `open` |
| 2 | Rename... | `action` |
| 3 | as Obsidian URL | `info.copy` |
| 4 | Open in default app | `system` |
| 5 | Move file to... | `action` |
| 6 | from vault folder | `info.copy` |
| 7 | from system root | `info.copy` |
| 8 | Show in system explorer | `system` |
| 9 | Open in new window | `open` |
| 10 | Reveal file in navigation | `system` |
| 11 | Bookmark... | `action` |
| 12 | Delete file | `danger` |

- Right-clicking the result container (`search-result-container`) and the
  results children (`search-results-children`) opened no menu at all on
  2026-10-18. The pane's tab header did open one, with exactly one item:
  "Close" (section `close`, icon `lucide-x`, callback
  `function(){return t.leaf.detach()}`). A real right-click on 2026-10-19,
  read 735 ms later, gave the same one item, which removes most of the worry
  that the synthetic click read the menu before it was populated. A real
  right-click on the `workspace-tab-header-spacer` beside the tabs opened
  nothing, and the view has no `nav-header` element in this layout. The two
  empty result surfaces stay weak negatives: on 2026-10-19 there was no empty
  space below the last result to click.
- The 2026-10-19 run could not re-dump the search rows' menus: no rows existed
  when that step ran. Between the copy step and the menu step the rows
  disappeared and the query read empty from then on. The copy step's real
  click had opened one modal; because the clipboard stayed unchanged the
  script then called `onCopyResultsClick` directly and opened a second,
  pressed Escape once, and the next step began by re-running
  `global-search:open` — a stacked modal is the likely cause, but nothing
  recorded the moment the query emptied.
- The Files view's menu for `apple.md`, from a real right-click, in displayed
  order:

| Title | Section | Note |
| --- | --- | --- |
| Open in new tab | `open` | `lucide-file-plus` |
| Open to the right | `open` | `lucide-separator-vertical` |
| Open in new window | `open` | `lucide-picture-in-picture-2` |
| --- | | |
| Make a copy | `action` | `lucide-files` |
| Move file to... | `action` | `lucide-folder-tree` |
| Bookmark... | `action` | `lucide-bookmark` |
| Merge entire file with... | `action` | `lucide-git-merge` |
| --- | | |
| Copy path | none | `has-submenu`, `lucide-clipboard` |
| --- | | |
| Open version history | `view` | `lucide-history` |
| --- | | |
| Open in default app | `system` | `lucide-arrow-up-right` |
| Show in system explorer | `system` | `lucide-arrow-up-right` |
| --- | | two separators in a row |
| --- | | |
| Rename... | `danger` | `lucide-edit-3` |
| Delete | `danger` | `is-warning`, `lucide-trash-2` |

  Against the synthetic search-row dump it has `Rename...` in `danger` rather
  than `action`, `Delete` rather than `Delete file`, a `Copy path` submenu in
  place of the three `info.copy` fragments, no `Reveal file in navigation`,
  and three items the search dump lacks (`Make a copy`,
  `Merge entire file with...`, `Open version history`); whether a search row's
  real menu matches it item for item is still open.

Keyboard, from the handler sources on the view prototype — none was driven
against a populated list:

| Key | Handler |
| --- | --- |
| ArrowDown | `e.defaultPrevented` aside, calls `cE()` (unread) then `dom.changeFocusedItem("forwards")`; no input guard |
| ArrowLeft | nothing while the search input is the active element; else on a file row `setCollapse(true, true)`; on a match row collapses the parent file row and moves focus up to the nearest uncollapsed ancestor; on the list root `setCollapseAll(true)` |
| ArrowRight | same input guard; on the list root `setCollapseAll(false)`; on a file row `setCollapse(false, true)` |
| Enter | `focusedItem.onResultClick(e)` when the focused item is a file row or a match row |

So file rows and match rows are both focusable, and the Ctrl/Shift Enter
variants are decided inside `onResultClick`, which was not read. The driven
sequence (five ArrowDown, ArrowUp, two ArrowLeft, two ArrowRight, Enter,
Ctrl+Enter, Shift+Enter) ran over an empty list: `focusedItem` stayed null,
the input stayed the active element for the arrows, and no Markdown leaf,
popout window or vault file appeared after any Enter — which says nothing was
focused, not that Enter does nothing.

**Unprobed:** the visible effect of each toggle on a populated list — the
`Needle` hit set under Match case (the capitalised query never landed), whether
Collapse results drops match lines, how many lines Show more context adds and
whether they are trimmed, and what Explain search terms renders (only its
layout shift was seen). Keyboard navigation as driven: what the focus ring
moves over, what Enter, Ctrl+Enter and Shift+Enter open and where, and what
`cE()` in the ArrowDown handler is. The real context menu of a result's file
title and of a match line, whether it matches the Files view's menu for the
same file item for item, and the menu — if any — of the empty space below the
last result. Ctrl+C on a focused result row (never pressed; the only keyboard
step around the count menu pressed ArrowDown then Enter and read an unchanged
clipboard). What `Bookmark...` in the count menu does, what `Done` in the copy
modal does, whether the modal's textarea tracks the result list after an edit
or a sort change, and the bytes a paste of the copied text lands in a note.
The contents of `.obsidian/global-search.json` and which action writes it.
What a restored non-empty `query` does on restart (the query was empty at
quit). Whether the sort re-sort moves existing DOM nodes or re-renders rows,
and which row takes `has-focus` after a switch. The tie-break of a fresh list
(only re-sort stability was seen, from one tie pair), mtime ties (none
existed), the exact created-time key (birthtime rounded to milliseconds fits
one grouping of host stats), and the native `<select>` popup itself, which was
never opened — every switch was a synthetic `change` event. Whether a second
click on a row's collapse icon re-expands it, and whether clicking a row's
title collapses or opens.

**Driven again on 2026-10-22, for the pane's look** (`tools/obsidian-probe/search-shots.mjs`,
results in `results/2026-10-22-search-shots.json`; the same Obsidian 1.14.4 on installer build
1.13.7, a fresh scratch vault of eight notes holding `needle`, real clicks and keystrokes over
CDP, `Page.captureScreenshot` for the pictures and `getComputedStyle` for the numbers; a 300 px
dock). What the pane is made of, top to bottom, in the dark theme unless said otherwise:

- `search-row`: margin 12px 12px 8px, 30 px tall, a flex row with a 4 px gap. Inside it the
  `search-input-container` fills the width; the `<input>` is 30 px tall, 13 px text, padding
  `4px 8px 4px 30px` (62 px at the right once the decorators show), background `rgb(46,46,46)`,
  a hairline border `rgb(51,51,51)`, radius 5. Over its right end sit the Match case decorator
  (`input-right-decorator clickable-icon`, 24 by 20, padding 2px 4px, radius 4, a 16 px icon at
  85 % opacity, muted) and then the clear button (`search-input-clear-button`, 28 by 30, shown
  only with text: a filled disc in the muted colour with a dark cross). The Search settings
  button stands after the container: 28 by 24, padding 4px 6px, radius 4.
- `is-active` on either button is the accent as text (`rgb(166,138,249)`) over the accent at a
  tenth as background.
- `search-info-container` (Explain search terms): padding 4px 16px 8px, 12 px muted text, one
  `<div>` per line. For `needle` it read exactly `Matches text: "needle"` — the term in double
  quotes, which the 2026-10-18 label dump had not recorded.
- `search-params`: margin 4px 16px, a flex column with an 8 px gap; each `setting-item
  mod-toggle` is 19 px tall with 8 px of margin below (so rows sit 35 px apart), the name 13 px
  in the normal text colour, the `checkbox-container` 34 by 19 with an 18 px radius, background
  `rgb(63,63,63)` off and the accent on. With the section open the results bar moved from y 90
  to y 195.
- `search-results-info`: padding 0 12px 8px, a flex row, the two controls 24 px tall. The count
  (`clickable-icon search-results-result-count`) is padding 4px 6px, radius 4, 12 px muted
  text, and ends in `more-options-icon`: a filled disc the colour of muted text carrying three
  dark dots; under the pointer the control takes a white wash at 6.7 %. The sort control is the
  native `<select class="dropdown">`, 24 px tall, 12 px muted text, no border or background at
  rest, padding `0 28.8px 0 9.6px`, a chevron pair at its right.
- `search-result-container`: padding 12 on every side; the list scrolls inside it.
- A result's title row (`tree-item-self search-result-file-title`): 24.9 px tall (13 px text on
  a 16.9 px line, padding 4px 8px 4px 24px), margin-bottom 2, radius 8. The collapse icon is 16
  by 17 at `margin-left: -20px` (so it sits 4 px in), a 10 px `right-triangle` in the faint text
  colour at 85 % opacity, pointing down; collapsed it points right. The flair is 12 px faint text
  at the right. A name hit is `search-result-file-matched-text` inside the title, same weight.
- The card (`search-result-file-matches`): the note background `rgb(28,28,28)`, radius 4,
  margin 4px 0 8px, no border in the dark theme; in the light theme white with a 1 px ring of
  `rgb(228,228,228)` drawn as a box shadow. Each `search-result-file-match` is padding
  `8px 20px 8px 12px`, 12 px muted text on a 15.6 px line, `white-space: pre-wrap` — a long line
  wraps to three lines and the snippet itself ends in `...`. A hairline separates one match from
  the next.
- `search-result-file-matched-text`: background `oklch(0.88 0.132 107.9 / 0.3)` (a yellow at
  30 %) in the dark theme, `oklch(0.771 0.158 86.6 / 0.3)` in the light one; weight 400, the
  text colour unchanged in a title and the normal text colour in a match row.
- A match row under the pointer: background the accent at 33 % (`oklch(0.603 0.217 292.5 /
  0.33)`), its text the normal colour, and its `search-result-hover-button`s shown — each a 16
  px chevron in the faint colour with padding 1px 3px and radius 4, `mod-top` at the top right
  and `mod-bottom` at the bottom right. On a note's first line only the bottom one appeared.
- Keyboard, now driven against a populated list: with the keyboard in the input, ArrowDown put
  `has-focus` on the first result and drew a 2 px ring around its title row; ArrowLeft then
  collapsed that result (`is-collapsed`, its card gone, the ring staying on the title row) and
  ArrowRight re-opened it. So the "nothing while the input is the active element" reading of
  the ArrowLeft source above does not describe what happens once a row has focus.
- Toggles on a populated list: Collapse results folded every row and left the count at
  12 results; Show more context turned mango's first row into three lines, `line before first`
  / `mango needle first` / `line after first`, in one row; Match case dropped `kiwi` (its only
  hit is `Needle`) and the count read 11 results. Enabling Explain search terms put the
  `Matches text` line above the three toggles.
- The count's menu: 171 by 63, background `rgb(40,40,40)`, radius 8, a hairline border
  `rgb(63,63,63)`, three box shadows; each `menu-item` 24.9 px tall, padding 4px 8px, radius 4,
  a 16 px muted icon, an 8 px gap, 13 px text.

## Observed shell typography and chrome, Obsidian 1.14.4 (installer build 1.13.7)

Read from a running Obsidian on Linux over CDP on 2026-10-20 with
`tools/obsidian-probe/frame-typography.mjs`, `frame-settings.mjs` and
`frame-setting-key.mjs` (raw runs in `tools/obsidian-probe/results/2026-10-20-frame-*.json`);
every number is a computed style or a bounding box in CSS pixels at zoom 1 in a 1024 by 800 window
on a fresh vault. These are the values `crates/silica/src/theme.rs` and the shell renderers carry.

**The window frame.** `body` carries `is-frameless is-hidden-frameless mod-linux`: on Linux the
default "Window frame style" is **Hidden**. The setting lives in Settings → **Interface** (not
Appearance), labelled "Window frame style", described "Determines the styling of the title bar of
Obsidian windows. Requires a full restart to take effect.", with options `hidden` "Hidden (default)",
`custom` "Obsidian frame" and `native` "Native frame". Choosing any of them wrote **nothing** under
`.obsidian` (`appearance.json` stayed `{}` and `app.vault.config` stayed empty): the choice is kept
outside the vault, so Silica takes it from `--frame` or `SILICA_FRAME` and writes no key. Obsidian
1.14 opens Settings in a popout window; its rows are only reachable through
`app.setting.activeTab.containerEl`.

In the hidden frame the title row is **39 px** tall (`.titlebar`, transparent, over the 40 px tab
header containers) and holds, left to right: the left sidebar toggle (`.sidebar-toggle-button.mod-left`,
44 by 39, its icon in a 28 by 24 button padded `4px 6px`), the left dock's tab header, the root tab
header, the right sidebar toggle (36 by 39, at the right end of the root header: left of the right
dock's tab header while the sidebar is open, before the window buttons while it is collapsed; it
never sits in the right dock's own header) and the window buttons `.titlebar-button.mod-minimize`, `.mod-maximize`,
`.mod-close`, each **44 by 39** padded `0 16px`, in a `.titlebar-button-container.mod-right` 132 px
wide in the sidebar colour. `--frame-right-space` is `calc(126px - 0px)` and the root tab header
is padded `0 134px 0 8px` to stay clear of them. The `.titlebar-text` is 13 px, weight 700, muted at
opacity 0.85 and reads "New tab - <vault> - Obsidian 1.14.4"; the hidden frame gives it no box.
`--titlebar-height` is 30 px and `--window-border-radius` is unset; the window's corners are
nevertheless rounded, about 10 px on all four in a screenshot of 2026-10-21 (KDE Wayland, 2x),
so the rounding is the hidden frame's own rather than a theme's.

**The macOS frame.** Read from Obsidian 1.14.5's bundle on macOS on 2026-10-23 (its stylesheet
and main-process script; not yet a vault probe): the window is opened with `titleBarStyle` hidden
and `trafficLightPosition: {x: 19, y: 12}`, and `.mod-macos.is-hidden-frameless` sets `.titlebar`
to `display: none`, so the title row has no text and no window buttons of Obsidian's.
`--frame-left-space` is `calc(80px - var(--frame-ribbon-left))` (36 px beside the 44 px ribbon,
80 px without it) and `--frame-right-space` is 0; `.is-fullscreen` zeroes both. The tab header
container in the top-left corner (`mod-top-left-space`: the left dock's while the sidebar is
open, else the root's first) is padded `calc(var(--size-4-2) + var(--frame-left-space))` on the
left. The ribbon's corner is its usual header-height `::before` drag region, which on every
platform carries the headers' bottom line (`border-bottom: var(--tab-outline-width) solid
var(--tab-outline-color)`) and no divider: `.workspace-ribbon.mod-primary` itself starts under the
title row (`margin-top: var(--header-height)`), so its `border-inline-end` begins there too and
never crosses the corner. On macOS the workspace does not prepend `leftSidebarToggleButtonEl` to the ribbon: it is appended to the left
dock's tab header container while the sidebar is open, so it sits at that header's right end against
the dock's edge, and is prepended to the root's top-left one while the sidebar is collapsed. The
same two placements apply on every platform while the ribbon is hidden. `rightSidebarToggleButtonEl` goes into the right dock's own tab header container
while that sidebar is open (unlike Linux), and the stylesheet fixes `.sidebar-toggle-button.mod-secondary`
at `top: 0; inset-inline-end: var(--frame-right-space)` with the top-right header padded
`calc(38px + var(--frame-right-space))`, so it sits in the window's top-right corner either way.
Silica's `--frame hidden` on macOS asks gpui for a transparent title bar with the traffic lights at
(19, 12), pads the top-left header by the same 80 px less the ribbon, keeps the left toggle in the
headers and the right one in the corner, and draws no window buttons.

**The hidden frame's shadow.** A window capture at scale 1 on 2026-10-21 (KDE Wayland, the
surface's own pixels with alpha) shows the content in a transparent margin of **23 px at the left
and right, 15 px above and 46 px below**. The margin holds a shadow that is pure black at a
varying alpha: along a side it rises from 0.016 at the outer edge to 0.16 at the content, along
the top from 0.004 to 0.06, and along the bottom from 0.016 at the outer edge to 0.27 at the
content, which fits a gaussian of sigma 13 at 32 % black offset 12 px downward (CSS
`0 12px 27px rgba(0,0,0,0.32)`). The last pixel before the content is darker than the ramp —
alpha 0.27 at the sides, two pixels of 0.15 and 0.11 at the top — a one pixel outline on the
shadow rather than a border inside the content, whose first pixel is `--background-secondary`.

**Fonts.** `--font-interface` is `'??', '??', '??', ui-sans-serif, -apple-system, BlinkMacSystemFont,
system-ui, "Segoe UI", "Google Sans Flex", Roboto, "Inter Variable", "Inter", …` (the `??` are the
theme and override slots, empty here); `--font-text` is the same without the first slot;
`--font-monospace` is `'??', '??', ui-monospace, SFMono-Regular, "Cascadia Mono", "Roboto Mono",
"DejaVu Sans Mono", "Liberation Mono", Menlo, Monaco, "Consolas", "Source Code Pro", monospace`.
`document.fonts` lists Inter 100–900 normal and italic as **loaded** and Source Code Pro as unloaded:
Inter is what the shell renders in. Silica bundles Inter's static faces (`crates/silica/assets/fonts`)
and defaults both families to it.

**Sizes.** `--font-text-size` 16 px; `--font-ui-smaller` 12, `--font-ui-small` 13, `--font-ui-medium`
15, `--font-ui-large` 20; `--line-height-normal` 1.5, `--line-height-tight` 1.3; `body` is 15 px at
19.5 px. Headings: `--h1-size` 1.618em, h2 1.462, h3 1.318, h4 1.188, h5 1.076, h6 1em; weights
700, 680, 660, 640, 620, 600; line heights h1 1.2, h2 1.2, h3 1.3; `--bold-weight` is
`calc(400 + 200)`. In a note: `h1` 25.888 px / 31.07 line / weight 700 / letter-spacing −0.388 px
(−0.015em), margin `0 0 16px`; `h2` 23.392 px / 28.07 / 680 / −0.257 px, margin `40px 0 16px`; `p`
16 px / 24 px line, margin `16px 0`; `.inline-title` is an h1 with margin-bottom 12.944 px; the
preview and the editor are padded 32 px (`--file-margins: 32px 32px`, `--file-line-width` 700 px);
a tag is 14 px in a 28 px pill padded `3.5px 9.1px` on a 10 percent accent wash.

**Chrome.** `--header-height` 40 px: every tab header container and the view header are 40 px, the
view header padded `0 12px` with an 8 px gap, its title 13 px, its nav arrows and actions 28 by 24
clickable icons padded `4px 6px` with a 4 px radius, the arrows at opacity 0.4 with nowhere to go.
`--ribbon-width` 44 px: `.workspace-ribbon.mod-left` (classes `workspace-ribbon side-dock-ribbon
mod-primary`) starts 40 px down, padded `8px 4px 12px`, its `.side-dock-actions` a column with a
6 px gap of 28 by 24 buttons whose 16 px icons sit at opacity 0.85; `.side-dock-settings` is at the
bottom. Sidebars: `.mod-left-split` and `.mod-right-split` are **300 px** wide on a fresh vault, the
right one collapsed at first launch; the dock tab header is padded `0 8px`, its tabs 28 by 25.3 with
a 4 px radius, the active one on `--background-modifier-hover`, showing only the view's 16 px icon;
the `.workspace-leaf-resize-handle` is a 3 px strip on the dock's inner edge (`cursor: col-resize`,
a `--divider-color` hairline, `--divider-color-hover` the accent at `--divider-width-hover` 3 px).
The root tab: `.workspace-tab-header` 200 by 34.3 (`--tab-width` 200, `--tab-max-width` 320),
padded `1px 4px 4px`, radius `10px 10px 0 0`, the active one in `--background-primary`; its title
13 px; its close button 20 by 20 padded 2 px with an 8 px radius (`lucide-x`); the new-tab button
24 wide (`lucide-plus`) with margin `0 4px 0 8px`; the tab list button `lucide-chevron-down`.
Inside the tab, `.workspace-tab-header-inner` is the box the 1/4/4 padding leaves: 192 by 29.3,
padded `0 3px 0 6px` with a 2 px gap, radius 4 px (`--tab-radius`). **Hover** paints that inner
box, not the tab: `--background-modifier-hover` (white at 6.7 percent in dark, black at 6.7 percent
in light) on the 4 px-rounded inner box, so the wash sits 1 px below the tab's top and 4 px in from
its sides and bottom, while the tab's own 10 px top corners stay unpainted. The title's colour does
not change on hover (`--tab-text-color-focused`, #b3b3b3, before and after), the close button
appears (`display: none` → `flex`) and, under the pointer itself, draws no wash of its own, only
its cross going from `--text-muted` to `--text-normal`. The active tab shows nothing on hover: its
inner box stays transparent. Between two inactive tabs a divider: the inner box's `::after`, a 1 px
line 15 px tall 8 px below the tab's top, at the tab's **right** edge straddling the gap
(`right: -0.5px`), in `--tab-divider-color` (#3f3f3f dark, #dadada light). The active tab's is
`opacity: 0`, and the one on the tab before the active tab is painted over by the active tab, so
the first gap and the two around the active tab are bare. Hover changes no divider: both lines
beside a hovered tab stay, the wash stopping 4 px short of them. (Read over CDP on 2026-10-22,
`tools/obsidian-probe/results/2026-10-22-tab-hover.json`, both themes; the earlier note here that a
hovered tab hides its divider, and the #555555 measured off a screenshot, were wrong.) The dock
tabs hover the same way: the hovered 28 by 25 tab's inner box takes `--background-modifier-hover`
at the 4 px radius, the active one carries the same wash at rest.

**The strip's right end** (read over CDP on 2026-10-22,
`tools/obsidian-probe/results/2026-10-22-tab-list.json`). After the last tab comes the new-tab
button, `.workspace-tab-header-new-tab`: a 24 by 24 `clickable-icon` padded 4 px around the 16 px
`lucide-plus`, at y 8.66 in the 40 px strip (so centred on the titles rather than on the 34 px
tab), margin `0 4px 0 8px`. Pressing it, or running `workspace:new-tab`, appends an empty "New
tab" at the strip's right end whichever tab was active, and makes it the active tab; a second
press appends another after it (`tools/obsidian-probe/results/2026-10-22-new-tab.json`, with the
second of four tabs active). The empty tab's view offers "New note Ctrl + N", "New base", "New
canvas" and "Go to file Ctrl + O". The last tab's own `::after` divider stays when the tab is inactive, so
a hairline parts the tabs from the button. Then the spacer, then the tab-list button
`.workspace-tab-header-tab-list`, the same 24 px `clickable-icon` around `lucide-chevron-down` at
y 8.66 with margin `0 2px 0 0`, and, with the right sidebar collapsed, the sidebar toggle, a 28 by
24 `clickable-icon` padded `4px 6px` at y 8. Clicking the chevron opens `.menu.mod-tab-list` at the
pointer: `--background-secondary`, 8 px radius, a 0.69 px `--background-modifier-border` border,
three shadows, padded 6 px; rows 24.9 px tall padded `4px 8px`, 4 px radius, an 8 px gap between
the 16 px icon and the 13 px `--text-normal` title, `--background-modifier-hover` under the pointer;
separators 0.69 px with margin `6px -6px`, so they run the menu's full width. The rows: **Stack
tabs** (`lucide-layers`, section `action`; reads **Unstack tabs** once the group is stacked,
`mod-stacked` on `.workspace-tabs`), **Bookmark N tabs...** (`lucide-bookmark`, N the tabs with a
file, absent when none has one; opens the "Add N bookmarks" modal with a Title field whose
placeholder is the current date and time, and Cancel / Save), a separator, **Close all**
(`lucide-x`, section `close`; every tab in the group closes and, in the window's only group, one
empty "New tab" is left active), a separator, then one row per tab (`lucide-file`, section
`tablist`, the tab's title) with `mod-checked` and a trailing check on the group's active tab;
picking a row activates that tab. Escape closes the menu.
Explorer rows: `.nav-file-title` 24.9 px tall padded `4px 8px 4px 24px`, margin-bottom 2 px, radius
8 px, 13 px text, muted until active (then `--text-normal` on the hover wash); `.nav-header` 40 px
padded 8 px with a 2 px gap of 28 by 24 buttons. The status bar is a tab in the window's bottom right
rather than a bar across it: `.status-bar` padded `4px 8px 4px 4px`, min-height 18, the sidebar
colour, a hairline on its left (and top), radius `8px 0 0 0`, 12 px text, items padded `3px 4px`
with a 4 px gap. Icons everywhere are 16 px with a 2 px stroke on Lucide's 24 unit grid (`--icon-s`,
`--icon-m` 16, `--icon-l` 18, `--icon-xs` 14).

**The vault profile.** Settings is not at the bottom of the ribbon: the probe found
`.side-dock-settings` 0 px tall. In 1.14 the left sidebar ends with a footer, the vault profile,
seen on 2026-10-21 in a screenshot of the running app (the docs vault open, dark theme, 2x): a
hairline across the top, then a row the height of a header with, on the left, a `chevrons-up-down`
icon at muted strength and the vault's name in 13 px semibold `--text-normal`, and on the right two
clickable icons, Help (`circle-help`) and Settings (`settings`). The name and its chevrons open the
vault switcher, Help opens the help site, the gear opens Settings. The footer is part of the sidebar,
so it goes with it when the sidebar collapses, and it is there whether or not the ribbon is shown.
Its exact padding and the switcher's hover shape are still to be read over CDP; Silica draws the row
40 px tall, padded 8 px, its buttons the ribbon's 28 by 24, and runs `app:open-vault` from the name,
`app:open-help` from Help and `app:open-settings` from the gear.

**The vault switcher.** The footer's name, and `app:switch-vault` ("Change vault..."), drop a menu up
from the name: every vault this machine has opened whose folder is still there, in the registry
document's own order with a check on the one showing, a hairline, then "Manage vaults..."
(`app:open-vault`, the vault manager below). A click outside closes it, and so does a second click on
the footer's own row — the menu closes on a mouse *down* outside itself, so the row's click handler
has to leave its own press alone or the press closes the menu the click reopens. The list is Obsidian's `obsidian.json`
document — `{"vaults": {<16 hex digits>: {"path", "ts", "open"}}}`, `ts` in Unix milliseconds,
`open` on the vault a window shows — written as `silica.json` under `~/.config/silica` (Linux),
`%APPDATA%\silica` (Windows) or `~/Library/Application Support/silica` (macOS), or
`$SILICA_CONFIG_DIR`; unknown keys survive a rewrite. Every vault loaded goes on the list except the
sandbox, which is a fresh `Silica Sandbox <ms>` folder under the temporary directory holding a
`Welcome.md` and an `Another note.md`, opened on `Welcome`. Switching puts the vault that was open
away as quitting would (the note's edits and `workspace.json` written if the session changed them),
closes every tab group, and restores the next vault from its own `workspace.json`.

Obsidian's own switcher was read over CDP on 2026-10-29
(`tools/obsidian-probe/vault-switcher.mjs`, `results/2026-10-29-vault-switcher.json`), with a
registry holding six vaults — two of them folders named `vault` under different parents, one of them
a folder that had been deleted. The footer is `.workspace-sidedock-vault-profile` around a
`.workspace-drawer-vault-switcher` (a `.workspace-drawer-vault-switcher-icon` and the
`.workspace-drawer-vault-name`) and a `.workspace-drawer-vault-actions`. Clicking it opens a plain
`.menu` whose vault rows are `.menu-item tappable`, each an empty `.menu-item-icon`, then a
`.menu-item-title` holding **the folder's name alone** — no path, no `aria-label`, no icon — and on
the open vault a trailing `.menu-item-icon.mod-checked` drawing `lucide-check`. Three things follow
from the run: two vaults that share a folder name are two identical rows, and Obsidian neither
disambiguates them nor hangs the path off them; a vault whose folder is gone is left out of the menu
altogether; and the order is the document's key order,
not recency — the open vault was the fourth of six rows, where its key sat, and the newest `ts` was
third. Under a `.menu-separator` comes a single row, "Manage vaults..." with the `open-vault` icon,
which raises Obsidian's vault-manager window. Silica matches the rows, the omission, the order and
that tail, drawing the one row with Lucide's `folder-open` (Obsidian's `open-vault` icon is its own),
and it keeps the folder's path as the row's tooltip. A machine that has
opened dozens of vaults has a list taller than the window, which would carry the rows under the
hairline off the bottom edge whichever way the menu is snapped: the menu is capped at the window's
height less 96 px and scrolls, the way Obsidian's own menus scroll inside their `.menu-scroll`.

**The vault manager.** "Manage vaults..." and `app:open-vault` ("Manage vaults") raise a window of
its own rather than a modal: in Obsidian it is `app://obsidian.md/starter.html`, 800 by 650, body
`starter`, the same window the app shows when it is started with no vault. It was read over CDP on
2026-10-29 (`tools/obsidian-probe/vault-manager.mjs`,
`results/2026-10-29-vault-manager.json`), with the same registry fixture as the switcher run.
`.starter-screen-inner` is a `.recent-vaults` column 280 px wide beside a 520 px `.splash`.

A row of the list is a `.recent-vaults-list-item` whose `aria-label` is the vault's **full path**
(tooltip to the right), holding a `.recent-vaults-list-item-name` with the folder's name, a
`.recent-vaults-list-item-path` with the **parent folder's** path (which wraps to a second line
rather than eliding), and a `.recent-vaults-list-item-option-button` ("More options",
`lucide-more-vertical`) at the right. Clicking the row opens that vault.

Two things the run settled. The list is **`ts` descending**, most recently opened first — not the
switcher menu's key order: the six rows came back in exactly the order the document's `ts` values
rank them, with equal `ts` keeping the document's order. And a vault whose folder is gone is left out
here too; by the time the window was up, Obsidian had also dropped that entry from `obsidian.json`
altogether. Silica leaves the entry in `silica.json` and only drops the row — a vault on an unplugged
drive should not be forgotten — which is the one deliberate difference.

The ellipsis opens a `.menu` of seven rows: "Copy vault ID" (`lucide-copy`), a separator, "Rename
vault..." (`lucide-edit-3`) and "Move vault..." (`lucide-folder-tree`), a separator, "Reveal vault in
system explorer" (`lucide-folder-open`), a separator, and "Remove from list"
(`.is-warning`, `lucide-x`). Silica draws three of them — Copy vault ID, Reveal vault in system
explorer, Remove from list — in that order with their separators; Rename and Move rename and move the
user's folder on disk and are not built yet.

The `.splash` is the mark (90 px), the name, "Version x.y.z", then a `.setting-group` of rows, each a
`.setting-item-name` over a `.setting-item-description` with a button on the right: "Create new
vault" / "Create a new Obsidian vault under a folder." / an accent `mod-cta` **Create**; "Open folder
as vault" / "Choose an existing folder of Markdown files." / **Open**; "Open vault from Obsidian
Sync" / "Set up a synced vault with existing remote vault." / **Sign in**; and a language dropdown
with a help icon. Silica keeps the first two with Obsidian's wording (its own name in the first), puts
"Open sandbox vault" (`app:open-sandbox-vault`) where Sync is, and has no language row: it is English
only. **Create** asks the desktop where the vault goes and what it is called, then makes the folder
and the `.obsidian` in it; **Open** is the folder picker `app:open-another-vault` uses. Whatever is
picked loads into the workspace window and the manager closes. Escape closes it, and raising it again
while it is up brings the one window forward, as the Help window does.

`app:switch-vault` ("Change vault...") raised neither a menu nor a window in Obsidian in the same run,
though it reported itself as run; Silica gives it the switcher's menu.

**The Help window.** `app:open-help` and the footer's Help open a window of Obsidian's shape rather
than the browser (seen in a screenshot of 1.14.4, 2026-10-21): the mark, the name, "Version x.y.z",
then a card of rows, each an icon on the left, a title over a line of explanation, and a button on
the right — Obsidian's are the help site (an accent "Visit"), Discord ("Join"), the forum ("Visit")
and the sandbox vault ("Open"). Silica's four are the manual (silica.cc, accent "Visit"), the source,
the issue tracker and the sandbox vault; the window is 700 by 860 with the desktop's own frame,
Escape closes it, and opening help again brings the one up forward.

**Icons named in the DOM** (`svg.lucide-*`, so Silica draws the same Lucide drawings from Lucide's
own repository): ribbon `file-search` (switcher), `git-fork` (graph), `layout-dashboard` (canvas),
`calendar` (daily note), `files` (template), `terminal` (palette), `layout-list` (base); left dock
`folder-closed`, `search`, `bookmark`; right dock `links-coming-in` and `links-going-out`
(Obsidian's own; Silica draws its own two under those names, Lucide's chain at the top left with a
horizontal arrow beneath it pointing in or out), `tags`, `archive` (All properties), `list` (outline),
in that order; view header `arrow-left`,
`arrow-right`, actions `bookmark` and `more-vertical`; explorer header `edit`, `folder-plus`,
`sort-asc`, `gallery-vertical`, `chevrons-up-down`; search `search`, `list`, `move-vertical`; tags
`folder-tree`; bookmarks `bookmark-plus`, `folder-plus`, `chevrons-down-up`; folder collapse
`right-triangle` and the sidebar toggles `sidebar-toggle-button-icon` are Obsidian's own.

**Palette.** Dark (`theme-dark`): `--background-primary` #1C1C1C, `-primary-alt` #232323,
`-secondary` #282828, `-secondary-alt` #333333, `--background-modifier-border` #333333,
`--background-modifier-hover` `color-mix(in oklch, white 6.7%, transparent)`, `--text-normal`
#dadada, `--text-muted` #b3b3b3, `--text-faint` #666666, `--interactive-accent` `hsl(258, 88%, 66%)`
(the picker shows #8a5cf5), `--text-accent` and `--link-color` `hsl(255, 89.76%, 75.9%)` (computed
rgb(166, 138, 249)), `--tab-text-color` #666666, `--tab-text-color-active` #b3b3b3,
`--tab-background-active` #1C1C1C, `--nav-item-color` #b3b3b3, `--nav-item-color-active` #dadada,
`--icon-color` #b3b3b3, `--divider-color` #333333, `--code-background` #232323, `--text-selection`
a 33 percent accent wash, `--scrollbar-thumb-bg` a 10 percent white wash. Light (`theme-light`):
#ffffff, #fafafa, #f6f6f6, #fcfcfc, border #e4e4e4, hover `black 6.7%`, text #222222, muted
#5c5c5c, faint #ababab, accent and link `hsl(258, 88%, 66%)`, tab text #ababab / #5c5c5c,
selection a 20 percent wash. Radii `--radius-s` 4, `-m` 8, `-l` 12; `--clickable-icon-radius` 4.

**What a sidebar writes.** `workspace.json`'s `left` and `right` nodes carry `"width": 300` on a
fresh vault; dragging the left handle 120 px wrote `"width": 419.50215911865234` (the dock's new
`style="width: 419.502px"`), and the sidebar toggle adds `"collapsed": true` after `width` and
removes it again. `app:toggle-left-sidebar`, `app:toggle-right-sidebar` and `app:toggle-ribbon`
are the commands ("Toggle left sidebar", "Toggle right sidebar", "Toggle ribbon").

## Observed tab group behavior, Obsidian 1.14.4 (installer build 1.13.7)

One CDP-driven run on 2026-10-20 (`tools/obsidian-probe/splits-bookmarks.mjs`,
raw results in `tools/obsidian-probe/results/2026-10-20-splits-bookmarks.json`)
in a fresh scratch vault of four notes (`A.md`, `B.md`, `C.md`, `sub/D.md`),
reading `app.workspace.rootSplit` after every step and `workspace.json` after
`app.workspace.saveLayout()`. Everything below was seen, not read from docs.

The layout:

- `main` is always a `split` with `direction: "vertical"`, even around one
  group: `{"id", "type": "split", "children": [...], "direction"}` in that key
  order. A group is `{"id", "type": "tabs", "children": [leaves]}` plus
  `"currentTab": n` after `children` when the active tab is not the first.
- `workspace:split-vertical` ("Split right", `hasCheck`) on a group inside a
  vertical split inserts a new `tabs` node **right after it** in the same split.
  `workspace:split-horizontal` ("Split down") on a group inside a vertical split
  **replaces the group's node with a `split` of `direction: "horizontal"`** whose
  children are the old group and the new one. The rule is the same both ways:
  a sibling when the enclosing split already runs that way, a wrapping split
  otherwise. Splitting a group inside the horizontal split with "Split right"
  wrapped it in a vertical split, three levels deep.
- The new group holds **a copy of the active leaf**: same file, same `mode` and
  `source`, a fresh leaf id. It becomes the active leaf. Splitting an empty tab
  makes a new group with an `empty` leaf; splitting the graph makes one with a
  `graph` leaf.
- Dragging the divider between two children writes **`"dimension"`** on *both*
  of them: a percentage of the split, `29.411764705882355` / `70.58823529411765`,
  inserted **after `type` and before `children`** (`id, type, dimension,
  children`; for a split, `id, type, dimension, children, direction`). The DOM
  carries it as `flex-grow: 29.4118`. Before any drag no child has the key.
  Splitting a child that has a dimension gives each half of it (`29.41` became
  `14.70` and `14.70`); closing a child rescales the rest to a hundred (`14.70`,
  `70.59` became `17.24`, `82.76`); a split left with one child loses the key.
- Closing a group's last tab (`workspace:close`) **removes the group**, unless it
  is the only group, which keeps an empty leaf. A split left with one child is
  **not collapsed**: the root kept a nested horizontal split around a single
  `tabs` node. A split left with no child goes.
- `workspace:close-tab-group` ("Close this tab group") removes the active group
  with all its tabs. `workspace:close-others-tab-group` ("Close others in tab
  group") closes the **other tabs of the active group**, not other groups.
- After a group closes, the active leaf becomes the **most recently active leaf
  elsewhere** (the one active before the closed group was), not the layout's
  first.
- `editor:focus-left/right/top/bottom` ("Focus on tab group to the left/right",
  "above/below") step to the sibling on that side of the nearest enclosing split
  that runs that way; when the sibling is itself a split, focus lands on the
  group used most recently inside it (from the left column, focus-right went
  back to the bottom-right group that had been active last, not the top-right).
  At an edge nothing happens. All four are `hasCheck`.
- `editor:open-link-in-new-split` ("Open link under cursor to the right",
  default hotkey Mod+Alt+Enter) is "Split right" and then the link opened in
  the new group — a **new** sibling next to the current group, even when a group
  already exists to its right.
- The tabs container of the active group carries `mod-active`; its active tab
  header has `mod-active is-active`, an inactive group's active tab only
  `is-active`. A click in a group's content makes it active.
- A nested layout survives a quit and a relaunch byte for byte: `workspace.json`
  after the restart equalled the file written at quit, and was not rewritten
  within 3 s of the layout being ready.

Silica implements all of this since 2026-10-20 ([[Development#Tab groups, 2026-10-20]]),
with one graph view shared between groups: a second group showing the graph
says so in place of it.

Since 2026-10-21 a tab header can also be dragged ([[Development#Dragging tabs, 2026-10-21]]).
The rule Silica uses is **not probed**, it is the common desktop one: dropped on
another tab it goes in front of that tab, on a strip's open space at its end,
and over a group's content the outer band of each side (30 percent of the
shorter side, the nearer edge winning in a corner) makes a new group on that
side the way "Split right" and "Split down" place theirs — a sibling in front
or behind when the enclosing split runs that way, a wrapping split otherwise —
while the middle joins the strip. The dropped tab is the active tab of the
group it landed in, and a group left with no tab closes as it does when its
last tab is closed. Obsidian's zone geometry, highlight and ghost are still
to be observed.

**Unprobed:** the `dimension` a drag writes when only *some* siblings have one;
`workspace:close-others` (without `-tab-group`) across groups; dragging a tab
header into another group or onto a split edge (where the zones start, what is
highlighted, whether a split from a drop halves a `dimension` as "Split right"
does); `workspace:toggle-stacked-tabs`; what "Split right" does from the
sidebar's "More options" menu.

## Observed bookmarks behavior, Obsidian 1.14.4 (installer build 1.13.7)

Same run and vault as [[#Observed tab group behavior, Obsidian 1.14.4 (installer build 1.13.7)]],
with the `bookmarks` core plugin enabled and `.obsidian/bookmarks.json` read
from disk after every command.

The file:

- `{"items": [...]}`, two-space indented, **no trailing newline**. New items are
  appended at the end. It is written within a second of the change.
- Every item has `"type"` first and `"ctime"` (Unix milliseconds) second; what
  follows depends on the type:

| Type | Keys after `type`, `ctime` | Made by |
| --- | --- | --- |
| `file` | `path`; `subpath` (`"#Second heading"` or `"#^b56db2"`) when a heading or block | `bookmarks:bookmark-current-view`, `bookmark-current-heading`, `bookmark-current-section` |
| `search` | `query` | `bookmarks:bookmark-current-search` with the Search pane showing a query |
| `graph` | `title`, `options` (the whole graph config: `collapse-filter`, `search`, `showTags`, ... `close`) | `bookmark-current-view` with the graph active; the default title is `Untitled Graph` |
| `group` | `items` (nested items), `title` | `bookmarks:bookmark-all-tabs`; the default title is the moment formatted `MMMM D, YYYY h:mm A` (`October 20, 2026 1:49 PM`); every file item in it shares one `ctime`, the group's is later |
| `folder` | `path` | added through the plugin API in this run, so the key order Obsidian's own UI writes is unprobed |
| `url` | `url`, `title` | likewise through the API |

- `"title"` is written only when the modal's Title field was filled in; an
  empty field writes no key, and the pane shows the path (`A`,
  `A#Second heading`, `A#^b56db2`), the query, or the group's title.
- `bookmarks:bookmark-current-section` **writes a block id into the note** when
  the paragraph has none: ` ^b56db2` appended to the line (six lowercase
  hex-ish characters, a space before the caret), then bookmarks
  `"subpath": "#^b56db2"`.
- `bookmarks:unbookmark-current-view` ("Remove bookmark for the current file")
  removed **one** plain `file` item for the open file — in this run the one
  inside the group that had been added last, leaving the older top-level one.
  It is a lookup by path, last added wins.
- Running `bookmark-current-view` on a file already bookmarked opens
  **"Edit bookmark"** (with a Remove button) and writes nothing on Save.

The modal, "Add bookmark": a `Path` row (the file, read-only text input holding
`A`, `A#Second heading`, `A#^b56db2`; `Query` for a search), a `Title` row (an
empty input whose placeholder is that same text), a `Bookmark group` row, and
`Cancel` / `Save`. The graph's has no Path row and its Title input starts
filled with `Untitled Graph`. "Bookmark all tabs" opens "Add 3 bookmarks" with
only Title (placeholder the date) and Bookmark group. Enter saves.

The pane (view type `bookmarks`): a `nav-header` of four buttons in order —
"Bookmark the active tab..." (`lucide-bookmark-plus`), "New group"
(`lucide-folder-plus`), "Collapse all" (`lucide-chevrons-down-up`), "Show
search filter" (`lucide-search`) — then one `tree-item` per item in file order.
Icons by type: file `lucide-file`, heading `lucide-heading`, block
`lucide-toy-brick`, search `lucide-search`, graph `lucide-git-fork`, folder
`lucide-folder`, url `lucide-globe-2`; a group is a collapsible row with the
`right-triangle` and its items indented under it. The row for the open file
carries `is-active`.

Commands and names: `bookmarks:open` "Bookmarks: Show bookmarks";
`bookmarks:bookmark-current-view` "Bookmarks: Bookmark..." (check: a file or
graph view is active); `bookmarks:bookmark-current-search` "Bookmarks: Bookmark
current search..." (check); `bookmarks:unbookmark-current-view` "Bookmarks:
Remove bookmark for the current file" (check); `bookmarks:bookmark-current-section`
"Bookmarks: Bookmark block under cursor..." (check); `bookmarks:bookmark-current-heading`
"Bookmarks: Bookmark heading under cursor..." (check); `bookmarks:bookmark-all-tabs`
"Bookmarks: Bookmark all tabs..." (no check). None has a default hotkey.

Silica implements this since 2026-10-20 ([[Development#Bookmarks, 2026-10-20]]): the file
in these bytes, the seven commands, and the pane less its search filter and
the "Bookmark group" dropdown.

**Unprobed:** what clicking a search or heading bookmark in the pane does (the
clicks dispatched in this run landed on nothing); the item context menu and the
file menu's "Bookmark..." entries (the explorer was hidden behind the Search
pane when they were tried); the order of `folder` and `url` keys as the UI
writes them; the "Bookmark group" dropdown; renaming and dragging items; the
search filter.

## Observed file explorer header behavior, Obsidian 1.14.4 (installer build 1.13.7)

Read from a running Obsidian on Linux over CDP on 2026-10-21 with
`tools/obsidian-probe/explorer-header.mjs` (raw run in
`tools/obsidian-probe/results/2026-10-21-explorer-header.json`), on a scratch
vault of ten files in four folders, created in one order and given
modification times in another so the time sorts could be told apart.

**The header.** `.nav-header` is 40 px tall with 8 px of padding and no
border; inside it `.nav-buttons-container` is a flex row, `justify-content:
center`, `gap: 2px`, `flex-wrap: wrap`. Five `clickable-icon nav-action-button`
of 28 by 24 (padding `4px 6px`, radius 4 px, `--text-muted`), in order, each
with its `aria-label` as the tooltip (`.tooltip` below the button):

| Label | Icon | Does |
| --- | --- | --- |
| New note | `lucide-edit` (Lucide's `square-pen`) | what `file-explorer:new-file` does: `Untitled.md` in the folder `newFileLocation` names (`root` → the vault root, `current` → the open note's folder), opened in a new tab with the inline title focused |
| New folder | `lucide-folder-plus` | `Untitled/` at the vault root, its row in inline rename (`is-being-renamed`, `contenteditable`); Escape keeps the folder under that name |
| Change sort order | `lucide-sort-asc` (`arrow-up-narrow-wide`) | the menu below |
| Auto-reveal current file | `lucide-gallery-vertical` | toggles the leaf's `autoReveal`; the button carries `is-active` while on |
| Expand all | `lucide-chevrons-up-down` | opens every folder at every depth; while any folder is open the same button reads "Collapse all" with `lucide-chevrons-down-up` and closes them all |

A sixth, "Show search filter" (`lucide-search`), is in the DOM with a zero
rect in this build. The last button's label follows the tree, not the last
press: one folder opened by hand turns "Expand all" into "Collapse all".

**The sort menu.** Six items in three pairs separated by `menu-separator`,
the one in force `mod-checked` with a `lucide-check` icon: "File name (A to
Z)", "File name (Z to A)", "Modified time (new to old)", "Modified time (old
to new)", "Created time (new to old)", "Created time (old to new)". The leaf's
`sortOrder` takes `alphabetical`, `alphabeticalReverse`, `byModifiedTime`,
`byModifiedTimeReverse`, `byCreatedTime`, `byCreatedTimeReverse`; it is
written into the file-explorer leaf's `state` in `workspace.json` (key order
`sortOrder`, `autoReveal`, `showSearch`, `searchQuery`) and `app.json` stays
`{}` — the `fileSortOrder` key of [[#Observed settings write behavior, Obsidian 1.14.4]] is not what the
header writes. **Folders keep their name order under every sort**; only the
files of each folder move. Observed, with folders `Archive/old`, `Empty`,
`Projects/{Deep/Inner, Budget, Plan}` and root files `Alpha`, `beta`, `cover.png`,
`Gamma`, `n2`, `n10`:

| `sortOrder` | Root files | Projects |
| --- | --- | --- |
| `alphabetical` | Alpha, beta, cover, Gamma, n2, n10 | Budget, Plan |
| `alphabeticalReverse` | n10, n2, Gamma, cover, beta, Alpha | Plan, Budget |
| `byModifiedTime` | newest first by mtime | newest first |
| `byModifiedTimeReverse` | the exact reverse | the exact reverse |
| `byCreatedTime` | newest first by birth time (`cover.png`, created last, led) | newest first |
| `byCreatedTimeReverse` | the exact reverse | the exact reverse |

**Auto-reveal.** Off in a fresh vault (`autoReveal: false`; `view.autoReveal`
is undefined, the flag lives in the state). With it off, opening
`Projects/Deep/Inner.md` through a link expanded nothing: the tree stayed as
it was and no visible row was `is-active`. `file-explorer:reveal-active-file`
then expanded `Projects` and `Deep` and marked `Inner` active. Clicking the
button turned it on, wrote `autoReveal: true` and **revealed the open file at
once**; opening `Archive/old.md` next expanded `Archive` and marked its row.
Turning it off again left the tree expanded.

Silica implements this since 2026-10-21 ([[Development#Files header, 2026-10-21]]):
the five buttons with the same Lucide drawings and labels, the sort menu and
its six orders (files by name, mtime or birth time; folders by name), the
leaf's two keys read from and written back into `workspace.json`, and
`FileExplorer::follow`, which highlights the open file and only expands to it
with auto-reveal on. New folder goes through Silica's prompt modal rather
than inline rename, and the search filter is not built.

**Unprobed:** the computed colour and background of the `is-active` button
(Silica paints the accent on the hover wash); whether a visible row of the
open file is `is-active` with auto-reveal off (Silica highlights it); the
menu's position relative to the button.

## Observed file explorer keyboard behavior, Obsidian 1.14.4 (installer build 1.13.7)

Read from a running Obsidian on Linux over CDP on 2026-10-21 with
`tools/obsidian-probe/explorer-keyboard.mjs` (raw run in
`tools/obsidian-probe/results/2026-10-21-explorer-keyboard.json`), on a scratch
vault of seven notes in three folders. The pane's `tree` object exposes
`focusedItem`, and one `.tree-item-self` carries `has-focus` for it.

**Getting the keyboard into the tree.** `file-explorer:reveal-active-file`
expands to the open file, marks its row `has-focus` and makes the explorer
the active leaf; `file-explorer:open` makes the explorer the active leaf and
leaves `has-focus` where it was (or nowhere). A click on a row that opens a
file gives the keyboard to the editor (`cm-content` is the active element,
the explorer is not the active leaf, and the arrows then move the caret); a
click on the row of the file that is already open puts `has-focus` on it with
the explorer active; a click on a folder toggles it, makes the explorer
active and sets no `has-focus` — but the keyboard position is that folder:
Down then lands on the row under it.

**The keys**, with `has-focus` on a row and the explorer active:

| Key | On a file | On a folder |
| --- | --- | --- |
| Down / Up | the next / previous visible row, clamped at both ends; the file does not open | the same |
| Enter | opens the file; the keyboard goes to the editor; the row keeps `has-focus` and gains `is-active` | toggles the folder |
| Space | nothing | toggles the folder |
| Right | — (not probed) | collapsed: expands it, ring stays; open: the ring moves to its first child |
| Left | the ring moves to the parent folder, which collapses | open: collapses it, ring stays; collapsed at the root: nothing |
| Home / End / PageUp | nothing | nothing |
| Escape | the keyboard goes to the editor; the row keeps `has-focus` | — |
| F2 | inline rename of the row (`tree-item-inner` becomes the active element, contenteditable) | — |
| Delete | the "Delete file" dialog: "Are you sure you want to delete “Alpha.md”? It will be moved to your system trash." — Cancel / Delete, Delete focused | — |
| a letter, Tab | nothing | — |

Ctrl+Enter on the already-open file put the keyboard in the editor, like Enter.

**The ring.** `.tree-item-self.has-focus`: `box-shadow: 0 0 0 2px rgb(85, 85, 85)`
in the dark theme (`--background-modifier-border-focus`), the text at
`--text-normal`; with `is-active` as well, the hover wash behind it.

Silica implements this since 2026-10-21 ([[Development#Explorer keyboard, 2026-10-21]]):
an "Explorer" key context on the tree with Up, Down, Left, Right, Enter,
Space, Escape, F2 (the rename prompt) and Delete (the delete prompt); the
ring is a 2 px border; the two commands and the three kinds of click focus as
above; with no ring on a visible row, Down and Up start from the open file's
row, or the first. The socket's `focus` method takes `explorer`.

**Unprobed:** Right on a file; where Down starts with no `focusedItem` at
all; the light theme's ring colour (Silica uses `#c0c0c0`); whether Up or
Down from a hidden `focusedItem` (its folder since collapsed) starts over.

## Observed outgoing links pane behavior, Obsidian 1.14.4 (installer build 1.13.7)

Read from a running Obsidian on Linux over CDP on 2026-10-22 with
`tools/obsidian-probe/outgoing-links.mjs` (raw run in
`tools/obsidian-probe/results/2026-10-22-outgoing-links.json`), on a scratch
vault of five notes. `Alpha.md` carried a `related: "[[Gamma]]"` property and,
in its body, `[[Beta]]`, `[[Beta#Heading|the alias]]`, `[[Beta]]` again,
`[[Missing]]`, `[Beta link](Beta.md)`, `![[Beta]]`, `[[#Local]]`,
`[[Gamma#^blk]]` and `[[Sub/Deep]]`, then a line mentioning `Gamma`, `gamma`,
`GammaAlias` (an alias of Gamma), `Delta`, `Beta` and `delta` without links.
The right split was 300 px wide. The dark theme's values are the ones quoted;
`--text-normal` is `#dadada`, `--text-muted` `#b3b3b3`, `--text-faint`
`#666666`, `--background-primary` `#1C1C1C`, `--background-modifier-border`
`#333333`.

**The leaf.** `outgoing-links:open` activates a leaf of type `outgoing-link`
(tab icon `links-going-out`, title "Outgoing links") whose state is
`{"file": "Alpha.md", "linksCollapsed": false, "unlinkedCollapsed": true}` —
the unlinked section starts folded. Both keys follow the headers' folds and
are written to `workspace.json` on quit and read back on restart. The pane has
no `nav-header` of its own: `.view-content` holds `.outgoing-link-pane`
directly, padded `12px 12px 32px`.

**Section headers.** "Links" and "Unlinked mentions" are each a
`div.tree-item-self.is-clickable` (25 px tall, `padding: 4px 8px`, 8 px radius,
2 px below; `aria-label` "Click to collapse" / "Click to expand") holding a
`span.tree-item-icon.collapse-icon` that is `display: none` even under the
pointer, a `div.tree-item-inner` with the label at 13 px, weight 500,
`--text-normal`, and a `tree-item-flair` with the count at 12 px,
`--text-faint`. A folded header (`is-collapsed`) is `--text-faint`
throughout and shows no count. Hovering a header paints
`--background-modifier-hover` and turns the count `--text-muted`. Clicking
the Links header folds its list and flips `linksCollapsed`; a second click
unfolds it.

**The Links section** listed seven rows for the ten links written, in this
order: `Gamma`, `Beta`, `Heading` / `Beta`, `Missing`, `Local`, `blk` /
`Gamma`, `Sub/Deep` / `Sub`. So the property link comes first, the body
follows in order of first occurrence, and one row stands for every link to
the same target and subpath: `[[Beta]]`, `[Beta link](Beta.md)` and
`![[Beta]]` collapse into the one "Beta" row, and the alias `the alias` is
shown nowhere. Each row is `div.tree-item-self.is-clickable.outgoing-link-item`
(`padding: 4px 8px 4px 24px`, 8 px radius, 2 px below, `--text-muted`, 13 px;
25 px tall with one line of text, 39 px with two) holding a
`span.tree-item-icon` (16 by 17, `margin-left: -20px` so it sits in the left
padding, opacity 0.85, a 14 px icon) and a `div.tree-item-inner` with a
`div.tree-item-inner-text` and, for some rows, a `div.tree-item-inner-subtext`
under it at 0.85 em (11.05 px) in `--text-faint`. The icon says what the link
points at: `lucide-link` for a note, Obsidian's own `heading-glyph` for a
heading subpath, `lucide-layout-list` for a block subpath, and
`lucide-file-plus` for a link that resolves to nothing, whose row also carries
`aria-label="Not created yet"` and is otherwise styled like the rest (no
unresolved colour, opacity 1). The text is the subpath's last element for a
heading or block link (`Heading`, `blk`, `Local`), else the path as written
(`Beta`, `Missing`, `Sub/Deep`); the subtext is the note's name under a
heading or block link to another note (`Beta`, `Gamma`) — a self link
(`[[#Local]]`) has none — and the folder under a note link (`Sub`), so a root
note has none either. Hovering a row paints `--background-modifier-hover` and
turns the text `--text-normal`.

**Clicks.** The `Heading` / `Beta` row made `Beta.md` the active file in the
root leaf, in source mode with the cursor at line 4 column 0 — the heading's
line. The `Missing` row created an empty `Missing.md` at the vault root and
opened it, with no dialog. A right click on a row opens the file menu: Open in
new tab, Open to the right, Open in new window, ---, Rename..., Move file
to..., Bookmark..., ---, Copy path, ---, Open in default app, Show in system
explorer, Reveal file in navigation.

**Unlinked mentions** (count 6: every occurrence, case-insensitively, aliases
included, `Delta` and `delta` both). The section is a `search-result-container`
(`padding: 4px 1px 16px`) holding one `div.tree-item.search-result` with
*no* `search-result-file-title` row — the file is the open note itself — and
a `div.search-result-file-matches` (background `--background-primary`, 4 px
radius, `margin: 4px 0 8px`, 12 px, line height 1.3, `--text-muted`, no
border). Each `div.search-result-file-match.tappable` is padded
`8px 20px 8px 12px`, wraps (`white-space: pre-wrap`), holds the whole line
with the mention in a `span.search-result-file-matched-text` (background
`--text-highlight-bg`, text `--text-normal`), and under it, 6 px down, a
`div.search-result-file-match-destination-file` chip
(`aria-label="Link this file"`, `padding: 4px 6px`, background
`--background-modifier-border`, 4 px radius) holding a 14 px `lucide-link` in
`--text-faint` and the mentioned note's name at 12 px in `--text-muted`. Under
the pointer a match paints the accent at 33% (`oklch(0.60 0.217 292 / 0.33)`)
and two `search-result-hover-button`s (`mod-top` with `lucide-chevron-up`,
`mod-bottom` with `lucide-chevron-down`, both labelled "Show more context")
appear at its right. Clicking the chip under `Gamma` rewrote the line to
`[[Gamma]] and gamma and GammaAlias …` with nothing else in the file touched
and no notice; the pane then listed five mentions. Clicking a match's text
moved the editor's cursor to the mention's first character (line 9, column 24
for `GammaAlias`) and left the note open.

**The Backlinks pane in the same run**, for comparison with the search pane's
rows it reuses: the same section headers (flair `--text-faint`, no visible
triangle); a `div.tree-item-self.search-result-file-title.is-clickable` per
file (`padding: 4px 8px 4px 24px`) with a *visible* `right-triangle` collapse
icon in the left padding, the name at 13 px `--text-normal` and a faint count;
then the same `search-result-file-matches` card. Two links on one line
(`[[Alpha]] and [[Alpha#Local|local]]`) are one match with two highlighted
spans and count two. An unlinked mention's `button.search-result-file-match-replace-button`
reading "Link" (12 px, `padding: 4px 8px`, background
`--background-modifier-border`, 5 px radius, `--text-normal`) is absolutely
placed 2 px from the top and 24 px from the right of the match and has no size
until the match is hovered; clicking it rewrote `Mentions Alpha once.` to
`Mentions [[Alpha]] once.`. An empty Linked mentions section shows
`div.search-empty-state` "No backlinks found." at 13 px `--text-faint`, 8 px in,
12 px below. The pane's header buttons are `lucide-list` (Collapse results),
`lucide-move-vertical` (Show more context), `lucide-sort-asc` (Change sort
order) and `lucide-search` (Show search filter), and the pane under the header
is padded `4px 12px 32px`.

**Unprobed:** what the Links section shows when the note has no links at all;
what a link to an image or other non-note file looks like; whether the
subtext under a note link is the written path's folder or the resolved file's
(the fixture made them the same); the per-match "Show more context" buttons'
effect; keyboard navigation of either pane.

## Observed tags pane behavior, Obsidian 1.14.4 (installer build 1.13.7)

Read from a running Obsidian on Linux over CDP on 2026-10-22 with
`tools/obsidian-probe/dock-panes.mjs` (raw run in
`tools/obsidian-probe/results/2026-10-22-dock-panes.json`; the same script
also covers the All properties and Outline panes below), on a scratch vault
of eight notes. The tags written: `project/alpha` and `common` in `Alpha.md`'s
frontmatter and inline again in its body; `project/beta` and `common` in
`Beta.md`'s frontmatter with `#Project`, `#common` and `#Lambda` inline;
`#project/beta/deep`, `#topic`, `#lambda` and `#common` in `Gamma.md`;
`#common` twice in `Delta.md`; `#Common`, `#common` and `#Kappa` in
`Epsilon.md`; `project/alpha` and `meta` in `Props.md`'s frontmatter; `single`
in `Outline.md`'s frontmatter plus `#topic` in one of its headings; and `#a` in
`NoHeadings.md`. The right split was 300 px wide; the dark theme's values are
quoted (`--text-normal` `#dadada`, `--text-muted` `#b3b3b3`, `--text-faint`
`#666666`, `--background-modifier-hover` white at 6.7%,
`--nav-indentation-guide-color` white at 12%).

**The leaf.** `tag-pane:open` ("Tags view: Show tags") activates a leaf of
type `tag` (tab icon `lucide-tags`, title "Tags") whose state is
`{"sortOrder": "frequency", "useHierarchy": true, "showSearch": false,
"searchQuery": ""}`. All four keys are written to `workspace.json` and read
back on restart: a run that quit with
`{"sortOrder": "frequencyReverse", "useHierarchy": false, "showSearch": true,
"searchQuery": "com"}` came back with the flat list in that order, the search
box shown with `com` in it and the rows already filtered to `common`. The
pane's `.workspace-leaf-content[data-type="tag"]` holds a `div.nav-header`
(40 px: `padding: 8px` around a `div.nav-buttons-container` 284 by 24,
`display: flex`, `justify-content: center`, `gap: 2px`), the hidden
`div.search-input-container`, and `div.tag-container` with
`padding: 12px 12px 32px` and a 13 px font, so the first row starts 12 px
under the header (y 92 with the header at y 40–80).

**Header buttons**, left to right, each a
`div.clickable-icon.nav-action-button` 28 by 24 (`padding: 4px 6px`, 4 px
radius) holding a 16 px icon in `--text-muted` at opacity 0.85: "Change sort
order" (`lucide-sort-asc`), "Show nested tags" (`lucide-folder-tree`,
`is-active` while `useHierarchy` is true), "Expand all"
(`lucide-chevrons-up-down`, `aria-disabled="false"`) and "Show search filter"
(`lucide-search`, `is-active` while the box is shown). Hovering a button
paints `--background-modifier-hover` and leaves the colour muted; an
`is-active` button wears that wash permanently with the icon in
`--text-normal` at opacity 1. About a second under the pointer a `div.tooltip`
appears 8 px below the button, centred on it: the label at 13 px in
`rgb(250, 250, 250)` on `oklch(0 0 0 / 0.9)`, `padding: 8px 12px`, 12 px
radius.

**Rows.** With `useHierarchy` on the default view listed `common` 9,
`project` 6 (folder of `alpha` 3 and `beta` 2, which holds `deep` 1),
`Lambda` 2, `topic` 2, `a` 1, `Kappa` 1, `meta` 1, `single` 1. Counts are
occurrences, not notes (`common` is 9 for five notes: two each in Alpha, Beta,
Delta and Epsilon), merged case-insensitively (`#Common` counts toward
`common`), and a parent's count is its own occurrences plus every
descendant's (`project` 6 = `#Project` 1 + `alpha` 3 + `beta` 2, `beta` 2
including `deep` 1). The display casing is the first one indexed: `Kappa`
(only ever capitalised) shows as `Kappa`, `Lambda` (capitalised in Beta,
lower in Gamma) as `Lambda`, and `#Project` (Beta) folds into `project`
(Alpha first). `app.metadataCache.getTags()` returns the same keys and
counts, `#project` 6 included. Each row is `div.tree-item >
div.tree-item-self.tag-pane-tag.is-clickable` (`mod-collapsible` when it has
children), 276 by 25 (`padding: 4px 8px 4px 24px`, `margin-bottom: 2px`, so
27 px pitch, 8 px radius, 13 px, `--text-muted`, `align-items: baseline`),
holding `div.tree-item-inner > div.tree-item-inner-text` with a
`span.tag-pane-tag-parent` (the path up to the last `/`, `display: none` in
hierarchy mode) and a `span.tree-item-inner-text` with the last segment, no
`#`; then `div.tree-item-flair-outer` (`padding-left: 4px`, `flex: 0 0 auto`,
pushed to the right edge) holding `span.tag-pane-tag-count.tree-item-flair`
with the count at 12 px, line height 12 px, `--text-faint`, no background.
Children sit in `div.tree-item-children` (`margin-left: 12px`,
`padding-left: 4px`, `border-left: 0.69px solid` the indentation guide
colour) and each nested self carries inline
`margin-inline-start: -17px !important; padding-inline-start: 41px !important`
at depth 1, `-33px`/`57px` at depth 2 (`-50px`/`74px` would follow), so the
hover wash spans the full 276 px while the text indents 17 px per level (x
760, 777, 793). A `mod-collapsible` row has `div.tree-item-icon.collapse-icon`
(16 by 17, `position: absolute`, `margin-left: -20px`, so at x 740, 757, 773
by depth, opacity 0.85) holding a 10 px `right-triangle` in `--text-faint`,
pointing down when open. Hovering a row paints `--background-modifier-hover`,
turns the text `--text-normal`, the count and the triangle `--text-muted`;
no tooltip appears on a row.

**Sort menu.** A real click on "Change sort order" opens a `.menu` 204 by
126 at the pointer (its top-right corner at the click, since the button sits
near the window's right edge): "Tag name (A to Z)", "Tag name (Z to A)", a
separator, "Frequency (high to low)", "Frequency (low to high)", each a
`div.menu-item.tappable` 190 by 25 with the current one `mod-checked` and
carrying a `lucide-check`. The stored values are `alphabetical`,
`alphabeticalReverse`, `frequency` (default) and `frequencyReverse`. Orders
read back: A to Z `a, common, Kappa, Lambda, meta, project, project/alpha,
project/beta, project/beta/deep, single, topic` — case-insensitive, the
tree kept, children sorted the same way; Z to A the exact reverse, children
included (`project, project/beta, project/beta/deep, project/alpha`);
frequency `common 9, project 6 (alpha 3, beta 2 (deep 1)), Lambda 2,
topic 2, a 1, Kappa 1, meta 1, single 1` — ties by name A to Z,
case-insensitive; low to high `a, Kappa, meta, single, Lambda, topic,
project (beta (deep), alpha), common` — ascending by count with ties still A
to Z, and the children ascending too. The menu is the same list in flat mode.

**Nested tags toggle.** Clicking "Show nested tags" sets `useHierarchy`
false, drops the button's `is-active` and redraws every tag as a depth-0 row:
the parent span becomes `display: inline` so `project/alpha` reads as
`project/` + `alpha` in one line (both spans `--text-muted`, 81 px wide
together), the inline margin/padding return to `0px`/`24px`, no row has a
triangle, and `project` is still listed with its inclusive 6. A click on the
flat `project/alpha` row searched `tag:#project/alpha` as the nested row
does. Clicking the button again restored the tree, every folder open.

**Folding.** A click on `project`'s triangle gave its `tree-item`
`is-collapsed`, removed its `tree-item-children` from the DOM (11 rows
became 8) and left the count at 6; the state did not change and a fold is not
in `workspace.json`. The "Expand all" button is odd: with every row open its
label already read "Expand all" with `lucide-chevrons-up-down`; the first
click changed nothing, the second gave every `tree-item` — leaves included —
`is-collapsed` (8 rows), the third opened them all again, and the label and
icon never changed across the three clicks (the Outline's twin does swap to
"Collapse all" / `lucide-chevrons-down-up`).

**Search filter.** "Show search filter" shows the `search-input-container`
(284 by 30, `margin: 4px 0`, the header growing to 78 px and the rows moving
down 38 px) and focuses its `input[type=search][enterkeyhint=search]
[spellcheck=false][placeholder="Search..."]` (13 px, background
`rgb(46, 46, 46)`, 0.69 px border `rgb(85, 85, 85)` while focused, 5 px
radius, `padding: 4px 8px 4px 30px`, the right padding 28 px once the
`div.search-input-clear-button[aria-label="Clear search"]` — 28 by 30, 50%
radius, `right: 2px`, otherwise `display: none` — appears with text). State:
`showSearch: true`, then `searchQuery` follows each keystroke. Typing `proj`
left `project`, `project/alpha`, `project/beta` and `project/beta/deep` and
gave every other `tree-item` `is-filtered-out` (zero rect); `zzz` filtered
everything out with no empty-state text. Escape in the box is not handled by
the pane: focus jumped to the editor's `.cm-content`, the native search
input dropped its text, but the rows stayed filtered, `showSearch` stayed
true and `searchQuery` was already `""` — the view had been told nothing. The
clear button put the rows back; the header button hid the box, set
`showSearch` false and left focus on `body`.

**Clicks.** A left click on a row opens the global search: the search leaf
became active (its input focused), `getViewState().state.query` and the
input read `tag:#project/alpha` for the leaf row (2 results: Alpha with two
matches, Props with one), `tag:#project` for the parent row (4 results, the
search's `tag:` matching nested tags) and `tag:#project/beta/deep` for the
depth-2 row; the tags pane kept no selected row and its state did not change.
A click on the row `a` with focus then in the search input: ArrowDown and
Enter did nothing to the pane. A right click raised no menu through CDP (as
on the file-properties rows). The tags pane does not depend on the active
note: switching notes left it unchanged.

**Unprobed:** whether the search filter matches the full path or the last
segment (`proj` cannot tell), and whether it is case-insensitive; why the
"Expand all" label is stuck; the row context menu (if any); whether a tag
with its first occurrence in a later-named file keeps that casing after a
re-index; a vault without tags; drag of a tag row; what the tooltip of a
truncated tag shows (no row showed one).

## Observed all properties pane behavior, Obsidian 1.14.4 (installer build 1.13.7)

Read in the same `dock-panes.mjs` run as the tags pane above (raw run in
`tools/obsidian-probe/results/2026-10-22-dock-panes.json`). `Props.md`
carried `title`, `summary` (text), `list` (two items), `count: 42`,
`done: true`, `due: 2026-10-22`, `when: 2026-10-22T09:30:00`, `tags`,
`aliases`, `cssclasses`, a nested map `shelf: {row: 1, label: top}`,
`book.title: Dune`, `book.year: 1965` and `status: draft`; `Alpha.md` had
`title` and `Status`, `Beta.md` `status` and `count`, `Gamma.md` `STATUS` and
`title`; `.obsidian/types.json` named `summary` text, `list` multitext,
`count` number, `done` checkbox, `due` date and `when` datetime. This extends
the paragraphs above that start "The All properties pane holds one
`div.tree-item` per registry key" and "The CDP run filled the pane in"; where
this run differs it says so.

**The leaf.** `properties:open` ("Properties view: Show all properties")
activates the `all-properties` leaf (`lucide-archive`, "All properties")
whose state is `{"sortOrder": "frequency", "showSearch": false,
"searchQuery": ""}`; a run that quit with `alphabeticalReverse`, `showSearch`
true and `searchQuery: "st"` came back with the box shown, `st` in it and the
rows already filtered (`Status` 4, `list` 1). The leaf content holds the same
`nav-header` (40 px, buttons centred: "Change sort order" `lucide-sort-asc`
and "Show search filter" `lucide-search`, both 28 by 24 and styled as the
tags pane's), the hidden search container, a hidden `div.view-header` (zero
rect; "All properties" title, Bookmark and More options buttons in markup)
and `div.view-content` with `padding: 4px 12px 32px`, so the first row is at
y 84.

**Rows** (14 at frequency): `status` 4, `tags` 4, `title` 3, `count` 2, then
`aliases`, `book.title`, `book.year`, `cssclasses`, `done`, `due`, `list`,
`shelf`, `summary`, `when` at 1 — counts are notes, merged case-insensitively
(`Status`, `status`, `STATUS`, `status` → 4), ties A to Z, and `cssclasses`
listed at 1 since a note carries it (the earlier run's 0 was a vault with
none). The display casing was `status` in the session that indexed the vault
fresh and `Status` (from `Alpha.md`, the first file) after the relaunch, as
the paragraphs above found. Each row is `div.tree-item >
div.tree-item-self.tappable.is-clickable` with the same geometry as a tags
row (276 by 25 at 27 px pitch, `padding: 4px 8px 4px 24px`, 8 px radius,
13 px `--text-muted`, inline `margin-inline-start: 0px !important;
padding-inline-start: 24px !important`), holding `div.tree-item-icon` (16 by
17, `position: absolute`, `margin-left: -20px`, at x 740, opacity 0.85, a
14 px icon in `--text-muted`), `div.tree-item-inner > div.tree-item-inner-text`
with the key, and `div.tree-item-flair-outer > span.tree-item-flair` with the
count at 12 px `--text-faint`. The icon per widget: `lucide-text` for text
(`status`, `title`, `summary`, `book.title`), `lucide-list` for multitext
(`list`, `cssclasses`), `lucide-binary` for number (`count`, `book.year`),
`lucide-check-square` for checkbox, `lucide-calendar` for date,
`lucide-clock` for datetime, `lucide-tags` for `tags`, `lucide-forward` for
`aliases`, and `lucide-file-question` for the nested map `shelf`, whose
widget `app.metadataTypeManager` reports as `unknown`. Dotted keys are two
flat rows. Hovering a row paints `--background-modifier-hover`, the key
`--text-normal`, the count `--text-muted`, the icon unchanged.

**Sort menu**, 196 by 126 at the pointer: "Property name (A to Z)",
"Property name (Z to A)", separator, "Frequency (high to low)" (checked by
default), "Frequency (low to high)"; values `alphabetical`,
`alphabeticalReverse`, `frequency`, `frequencyReverse`. A to Z gave `aliases,
book.title, book.year, count, cssclasses, done, due, list, shelf, status,
summary, tags, title, when`; Z to A the reverse; low to high `aliases …
when` (the 1s A to Z), `count` 2, `title` 3, `status` 4, `tags` 4 — ties
ascending by name in both frequency orders.

**Search filter and clicks.** The box is the tags pane's (284 by 30 in the
header, the header growing to 78 px); typing `boo` left `book.title` and
`book.year`; `zzz` left an empty list with no text; Escape here *is* handled:
it hid the box, cleared the query, set `showSearch` false and put the 14 rows
back. A click on a row opens the global search with the key quoted in
brackets: `["book.title"]` (1 result), `["status"]` (4), `["tags"]` (4), the
search leaf becoming active; the pane keeps no selected row. The row's
right-click menu is Rename (`lucide-edit-3`), Property type (`lucide-menu`,
`has-submenu`), a separator, Delete property (`is-warning`,
`lucide-trash-2`), as recorded above — this run reached it through a CDP
right click.

**Unprobed:** keyboard navigation of the list; whether the filter matches
beyond a prefix; the tooltip of a long key; a `types.json` type that
contradicts a value's shape; the `file` and `folder` widgets' icons; what the
pane shows in a vault with no properties.

## Observed outline pane behavior, Obsidian 1.14.4 (installer build 1.13.7)

Read in the same `dock-panes.mjs` run (raw run in
`tools/obsidian-probe/results/2026-10-22-dock-panes.json`). `Outline.md` had
frontmatter then `# Top heading`, `## Second level`, `### Third level under
second`, `#### Fourth level`, `## Another second`, `#### Fourth under second,
skipping third`, `# Second top`, `### Third under top, skipping second`,
`## Heading with **bold**, *italic*, `code`, [[Alpha]] and a #topic tag`,
a `##` heading of 160 characters, `# Last top`, then `## Extra 1` … `## Extra
24`, with 14 (3 for the extras) filler lines under each heading;
`NoHeadings.md` had none. The headings' lines were 4, 19, 34, 49, 64, 79, 94,
109, 124, 139, 154.

**The leaf.** `outline:open` ("Outline: Show outline") activates the
`outline` leaf (`lucide-list`, title "Outline" — not the note's name) whose
state is `{"file": "Outline.md", "followCursor": false, "showSearch": false,
"searchQuery": ""}`; `file` follows the active markdown leaf and is absent
while an empty tab is active. `followCursor` and `showSearch` round-tripped
through `workspace.json` as `true`; folds are not in the state and did not
survive the restart. The `searchQuery` typed two seconds before quitting came
back in one run and as `""` in another (the tags and properties queries typed
the same way always came back), and the restored query sat in the box
without filtering the rows until the next keystroke. The leaf content is the
nav header (buttons centred: "Show search filter" `lucide-search`,
"Auto-scroll to current section" `lucide-gallery-vertical`, `is-active` while
`followCursor` is true, and "Collapse all" `lucide-chevrons-down-up`), the
hidden search container, a hidden `view-header`, and `div.view-content` with
`padding: 4px 12px 32px`.

**Rows.** One `div.tree-item` per heading, nested structurally: a heading
is a child of the nearest preceding heading of a smaller level, whatever the
numbers, so `#### Fourth under second, skipping third` sits at depth 2 under
`## Another second` and `### Third under top` at depth 1 under `# Second
top`; indentation is by depth, 17 px per level, with the same inline
`margin-inline-start`/`padding-inline-start` pairs, `tree-item-children`
guides and `collapse-icon` placement as the tags pane. The self is
`div.tree-item-self.is-clickable` (`mod-collapsible` with children,
`draggable="true"`) holding only `div.tree-item-inner` with the heading's
plain text: `**bold**` → `bold`, `` `code` `` → `code`, `[[Alpha]]` →
`Alpha`, `#topic` kept. No flair, no icon. The inner is `white-space:
pre-wrap` with no ellipsis or line clamp, so long headings wrap at the
227 px available at depth 1: the formatted heading took two lines (row 42
px), the 160-character one four (row 92 px), at 16.9 px per line. The row
for the heading that holds the cursor is `is-active`: the hover wash
`--background-modifier-hover` and `--text-normal` text, regardless of
`followCursor`. Hovering any row paints the same wash and colour and turns
its triangle `--text-muted`.

**Current heading** (source mode, cursor moved with `setCursor` and by a
real click in the editor): the marked row is the heading whose section holds
the cursor's line — `Top heading` at line 7, `Fourth level` at 51, `Another
second` on its own line 64, `Fourth under second, skipping third` at 80 and
81, `Extra 24` at the end — and no row is marked with the cursor in the
frontmatter (line 1). With `followCursor` false the pane does not scroll:
the `Extra 20` row was marked at y 992 under a 720 px tall `view-content`
(scroll height 1079) that stayed at `scrollTop` 0. Clicking "Auto-scroll to
current section" set `followCursor` true (button `is-active`); the same move
then scrolled `view-content` to 239 so the row sat at y 753 — scrolled into
view at the bottom edge, not centred — and moving back to the top scrolled
it to 0. In reading mode the marking follows the scroll: after each wheel
step (`previewMode.getScroll()` 23.4, 47.2, 71.0, 94.3) the marked row was
the heading whose section holds the line at the top of the view (`Second
level`, `Third level under second`, `Another second`, `Second top`), and with
the view scrolled to 0 — the frontmatter at the top — nothing was marked.

**Clicks.** In source mode a click on a row's text moved the cursor to the
heading's line, column 0 (`#### Fourth level` → line 49, `# Second top` →
94, the long heading → 139), focused the editor (`.cm-content`) and scrolled
so the heading's line sat at 348 px (344, and 261 for the four-line
heading) from the top of the 682 px tall scroller — centred. In reading mode
a click scrolled the heading to the top edge of `.markdown-preview-view`
(the `h1`/`h4` at 0 and −1 px, `getScroll()` 94.0 and 49.0, the heading's
line) and focused the preview. A right click raised no menu through CDP.

**Folding.** Clicking `Top heading`'s triangle gave its `tree-item`
`is-collapsed` and removed its descendants from the DOM (35 rows became 30);
with the cursor then in a hidden child's section no row was marked, the
folded parent included. "Collapse all" folded every top-level item (3 rows
stayed) and swapped the button to "Expand all" with `lucide-chevrons-up-down`;
clicking that opened everything and swapped it back.

**Search filter.** The same box; `second` kept every row whose text contains
the query case-insensitively *and their ancestors* (`Top heading` stayed for
its matching children), hid the rest with `is-filtered-out`, and wrapped each
match in `span.suggestion-highlight` (`Second`, `second`); `zzz` emptied the
list with no text; Escape hid the box, cleared the query and showed all rows.

**Switching notes and edits.** Opening `NoHeadings.md` replaced the list
with `div.pane-empty` "No headings found." (13 px, `--text-faint`,
`padding: 24px 16px`, `margin: 8px 0`, centred); an empty tab shows the same
text with no `file` in the state. Opening `Alpha.md` listed its one heading
with nothing marked (cursor at line 0, inside the frontmatter); returning to
`Outline.md` listed all 35 rows unfolded. Typing ` X` at the end of `# Top
heading` changed the row to `Top heading X` 2.1 s later, at the moment the
file reached disk — the outline follows the metadata cache, not the buffer.

**Unprobed:** keyboard navigation; dragging a row (`draggable`); the
`followCursor` scroll when the row is above the view (only a row below was
tried); headings inside code fences or callouts; a heading with a trailing
`#`; what the pane does for a canvas or base leaf; the reading-mode marking
with `followCursor` true; the text of an HTML heading.

## Observed Bases write behavior, Obsidian 1.14.5 (installer build 1.13.7)

Observed on 2026-10-25 over CDP on a scratch vault
(`tools/obsidian-probe/bases.mjs`, raw results in
`tools/obsidian-probe/results/2026-10-25-bases.json`). Nothing about Bases had
ever been observed before this run, so everything here is new; where it
disagrees with the published syntax documentation, the observation wins and
says so. The fixture was one `.base` carrying every documented top-level key in
a scrambled order, both YAML quoting styles, comments at three depths, flow
collections, four-space indentation, nested `and`/`or`/`not` filters and
unknown keys at the top, view and property level, in five copies with five
different fates — never opened, opened and closed, edited, driven through the
menus, rewritten from outside — plus a CRLF copy, a BOM copy, one with no
trailing newline, a zero-byte file, `{}`, and eight files broken in eight
different ways.

**The writer is one function, and it is decompiled.** A base view holds a
parsed `query`; `view.getViewData()` returns `query.toString()`, which is
`getSerializable()` passed to a YAML emitter (`""` when the serializable
object is empty), and Obsidian's own text-file save writes that. So every
question below is a question about `getSerializable()`:

```js
getSerializable() {
  var e = Object.assign({}, this.unrecognizedData);
  if (this.filters) e.filters = this.filters.serialize();
  if (this.formulas) { var t = Object.keys(this.formulas);
    if (t.length > 0) { e.formulas = {}; for (r of t) e.formulas[r] = this.formulas[r].toString(); } }
  if (this.summaryFormulas) { e.summaries = {};
    for (r of Object.keys(this.summaryFormulas)) e.summaries[r] = this.summaryFormulas[r].toString(); }
  for (var s = {}, l = !0, r of Object.keys(this.properties)) {
    var h = this.properties[r].serialize(); if (null !== h) { l = !1; s[r] = h; } }
  if (!l) e.properties = s;
  if (this.views) e.views = this.views.map((e) => e.serialize());
  if (this.newItemFolder) e.newItemFolder = this.newItemFolder;
  if (this.newItemTemplate) e.newItemTemplate = this.newItemTemplate;
  return e;
}
```

The other side, `setViewData`, parses the text and on failure keeps the **error
object** as the query (`this.query = null` for the view's purposes), which is
why a base that cannot be parsed is never written.

**When Obsidian writes.** Opening a base writes nothing — not within six
seconds, and the mtime does not move. Closing it writes `query.toString()`
whenever that differs from the bytes that were read, even with nothing edited:
the 1352-byte fixture came back 1256 bytes, comments gone. Where the file is
already in that canonical form the mtime does not move either (`Minimal.base`,
39 bytes, and `BrokenView.base` both survived a close untouched), so the close
write is skipped rather than written identically. A base that is only indexed,
never opened, is never touched. One real edit — a column dragged wider — lands
on disk within about a second and writes the whole file in the same canonical
form; there is no surgical write.

So **a `.base` is canonicalized, not preserved**: a comment, a quoting choice,
flow style, CRLF line endings, a UTF-8 BOM and a missing trailing newline all
survive being opened and are all lost on the close that follows. This is the
same open/close asymmetry canvas has, and it is stronger than canvas's: canvas
keeps the file's own key order on an unedited close, Bases does not.

**Top-level key order.** Unknown keys are spread **first**, in their original
file order, and every other key is then assigned in the serializer's fixed
order — but a key already present keeps the position the spread gave it. One
documented key is in both places: `summaries` is left in `unrecognizedData`
*and* parsed into `summaryFormulas`, so it is written at the position it had in
the file, with its statements re-serialized. The fixture's scrambled order came
back:

| Written | Source |
| --- | --- |
| `summaries`, then every unknown top-level key | `unrecognizedData`, in file order |
| `filters` | `filters.serialize()` |
| `formulas` | each statement's `toString()`; the whole key is omitted when there are none |
| `summaries` | `summaryFormulas`; present only when the file had the key |
| `properties` | each `PropertyConfig.serialize()`; **the whole key is omitted when every one of them returns null** |
| `views` | each `ViewConfig.serialize()` |
| `newItemFolder`, `newItemTemplate` | written when truthy |

`newItemFolder` and `newItemTemplate` are two keys the published schema never
mentions; both round-tripped from the fixture. A property whose config is empty
(`note.unused: {}`) is **dropped**: its `serialize()` returns null.

**A view's key order** is `type`, `name`, `filters`, `groupBy`, `groupOrder`,
`order`, `sort`, `limit`, `summaries` — `type` and `name` always, the rest when
set — and then every key of the raw parsed object that is not one of the
config's own fields, in file order, appended at the end. That last loop is
where unknown keys *and* every layout key live, which is why a layout key
written by the UI lands after a plugin's unknown key rather than among the
documented ones. `groupBy`, `order`, `sort` and the keys of `summaries` are
passed through a property-id normalizer; a hand-written bare `status` came back
`status`, so it does not qualify ids that are already written bare.

**A property's key order** is its own unknown keys first, then `displayName`
(`unknownPropKey` came back before `displayName`, which the fixture had second).

**The YAML emitter.** Two-space indentation; a sequence's `- ` is indented one
level past its key (`views:` at column 0 has items at 2, `and:` at 2 has items
at 4); a mapping inside a sequence item continues at the item's text column.
Block style only — the fixture's `order: [file.name, status, formula.ppu]` and
`groupBy: {property: status, direction: ASC}` both came back as block
collections. One trailing newline, LF, no BOM. No line folding: a 121-character
statement was written on one line. Scalars are written plain where YAML allows
it and quoted where it does not, **preferring double quotes, except that a
value containing a `"` and no `'` is single-quoted** — so
`'"#" + status'`, `'if(status == "a", "x: y", "z")'`, `'[1, 2, 3].join(", ")'`
and `'status + "x" '` came back single-quoted while a list separator of `", "`
came back `", "` in double quotes. Numbers are written bare (`limit: 103`,
`cardSize: 630`, `imageAspectRatio: 2` for 2.00).

**Statement bytes survive; only their quoting changes.** A parsed statement
keeps the original string in `formula.text` beside its AST, and `toString()`
returns that string, so `'type + " "  + file.name'` came back with its double
space intact as `type + " "  + file.name`. Even a statement the expression
parser rejects is kept verbatim (`'foo(((('` → `foo((((`). Silica can therefore
keep statement text byte-for-byte and still match Obsidian's output, as long as
it matches the quoting rule above.

**The layout keys**, which the published documentation says are unpublished,
read two ways that agree: off each registration's `options()` table
(`plugin.getRegistrations()`, whose source names every key with its widget,
range and default) and off the file after driving the real "Configure view"
page — the views toolbar menu lists one row per view, and the row's own icon
opens that view's settings.

| Layout | Key | Control | Values / range | Default |
| --- | --- | --- | --- | --- |
| `table` | `rowHeight` | dropdown | `""` Short, `medium`, `tall`, `extra` (Extra tall) | `""` (key absent) |
| `table` | `columnSize` | a header drag | a map of property id to integer px (`columnSize:` / `  file.name: 232`) | absent |
| `cards` | `cardSize` | slider | 50 to 800 step 10 | 200 |
| `cards`, `kanban` | `image` | property picker | a property id, written qualified (`note.aliases`, `formula.ppu`) | absent |
| `cards`, `kanban` | `imageFit` | dropdown | `""` Cover, `contain` | `""` (key absent) |
| `cards` | `imageAspectRatio` | slider | 0.25 to 2.5 step 0.05 | 1 |
| `kanban` | `columnWidth` | slider | 200 to 500 step 10 | 280 |
| `kanban` | `imageAspectRatio` | slider | 0.25 to 2.5 step 0.05 | 0.5 |
| `list` | `markers` | dropdown | `bullet`, `number`, `none` | `bullet` |
| `list` | `separator` | text | any string | `", "` |
| `list` | *(Indent properties)* | toggle | — | false |

The four registered layouts are `table` (Table, `lucide-table`), `cards`
(Cards, `lucide-layout-grid`), `list` (List, `lucide-list`) and `kanban`
(Kanban, `lucide-kanban-square`), and the Configure view page's Layout
combobox offers exactly those four. Picking another one **rewrites `type` and
leaves the previous layout's keys in place**: a list view with `markers:
number` and `separator: "; "` became `type: kanban` with both keys still
there. (The combobox marks *Table* as the selected row whatever the view's real
layout is — a 1.14.5 bug, not a key.)

**What the toolbar writes.** The Bases toolbar does not open menus of rows: each
control opens a `.menu.bases-toolbar-menu` whose body is a panel. The sort and
group panels hold `.combobox-button` pairs (property, direction) with a
`lucide-trash-2` button; the filter panel holds a `<select class="conjunction">`
(`and`/`or`/`not`) per group, a property combobox, an operator combobox, a
value input and a `filer-expression-mode-toggle` (`lucide-code-xml`) per rule,
in two sections, "All views" and "This view"; the properties panel is a
checkbox list of every property with "Add *name* property", "Add formula" and
"Hide all"; the results panel holds a "Limit number of results" number input
and the rows *Show all (n)*, *Copy to clipboard* and *Export CSV...*. A column
header's own context menu is *Hide column*, *Summarize...*, separator, *Sort A →
Z*, *Sort Z → A*, (*Clear sort*, once one exists), separator, *Edit property...*.
*Sort Z → A* **prepends** `{property, direction: DESC}` to the view's `sort`
list; *Hide column* removes the property from `order`; the limit input writes
`limit` as a bare number.

**What the commands write.**

| Command | Palette name | Effect |
| --- | --- | --- |
| `bases:new-file` | Bases: Create new base | creates `Untitled.base` as a **zero-byte file** and opens it; the parsed query of an empty file is one default view, so closing it writes `views:` / `  - type: table` / `    name: View` |
| `bases:add-view` | Bases: Add view | appends a view named `View 2` and opens its Configure view page |
| `bases:insert` | Bases: Insert new base | creates another zero-byte base (`Untitled 1.base`) and inserts `![[Untitled 1.base]]` on a line of its own at the cursor |
| `bases:copy-table` | Bases: Copy table to clipboard | the visible columns as TSV — one header row of display names, tab separators, no trailing newline (`status\tPrice per unit\ntodo\t0.02\ntodo\t0.06`) — and a "Copied to your clipboard" notice. It needs the view focused: run from the palette with focus elsewhere it reports success and shows "Unable to copy to your clipboard" |
| `bases:change-view` | Bases: Switch view... | not probed |
| `bases:add-item` | Bases: Add item | not probed |

The results panel's own *Copy to clipboard* row produces the same TSV.

**A broken base is never written.** Five failure modes, five outcomes, and in
every case where the parse fails the bytes and the mtime are untouched:

| Fixture | What the view shows | Bytes |
| --- | --- | --- |
| a YAML syntax error | `.bases-error`: "Unable to parse your base file:" and the YAML library's own message ("Flow sequence in block collection must be sufficiently indented and end with a ] at line 5, column 1: …") | untouched, open and close |
| a view with no `name` | "Unable to parse your base file: Missing or invalid "name" in view 1" | untouched |
| a view with `name: ''` | the same message — **an empty name is invalid too** | untouched |
| an unknown key beside `and` in `filters` | "Unable to parse your base file: "filters" may only have one of an "and", "or", or "not" keys." — unknown keys are tolerated at every level **except inside a filter group** | untouched |
| `type: nonsense` | "Unknown view type: nonsense", no rows; the file parses, `type` is kept | canonicalized on close (unchanged here, because the fixture already was) |
| an unparseable statement in a filter, a formula or a summary | no error banner at all; the statement is kept as an `invalid` AST node with its text | canonicalized on close, statement text intact |

So `name` is required and non-empty on every view — the one hard structural
rule the published schema does not state.

**An external rewrite.** With the base open, rewriting the file from outside
reloaded the view within four seconds (the renamed view was in the live query),
and the close that followed wrote the canonical form **of the external
content** — the external name and limit both survived. Nothing clobbers an
outside edit; it is only canonicalized.

**Unprobed:** `bases:change-view` and `bases:add-item`; the *Summarize...*
submenu (the row opens something that is not a `.menu`, so its options were not
read) and therefore what a summary chosen from the UI writes; the key the
list layout's "Indent properties" toggle writes (the toggle produced no write,
and the registration's source names it only as a minified constant); renaming
a view through the Configure view page's name field; whether `groupOrder` is
ever written by the UI; an embedded base in a note (`![[x.base]]`) and what
`createAndEmbedBase` writes; a base inside a folder rather than the vault root;
whether a write is atomic (temp-plus-rename) rather than in place.

## Observed Bases expression language, Obsidian 1.14.5 (installer build 1.13.7)

Observed on 2026-10-25 over CDP (`tools/obsidian-probe/bases-expr.mjs`, raw
results in `tools/obsidian-probe/results/2026-10-25-bases-expr.json`). Three
oracles, each of which makes a whole class of question cheap:

1. **The parser.** `mockContext.evaluateIdent(s)` is
   `evaluateFormula(MQ.parse(s))`, so patching `evaluateFormula` on the
   prototype captures the parse of an arbitrary string. A parsed statement is
   plain JSON, so precedence and associativity are *read off the tree*, not
   inferred from values.
2. **A real context.** `controller.results` is a `Map` whose values are the
   per-file evaluation contexts; `node.getValue(thatContext)` is what a cell
   does. The `mockContext` is not bound to a file and answers 0 or null for
   every ident, so it cannot be used for values.
3. **The formula editor's completion list.** "Add formula" in the properties
   panel opens a CodeMirror editor whose autocompletion lists exactly the
   functions and properties the receiver has, each with
   `name(param: Type?)` and a sentence of Obsidian's own documentation. It also
   shows a live `Valid formula` / `Invalid formula` status.

A value is an object `{icon, data}` whose `icon` is its type tag:
`lucide-text` String, `lucide-binary` Number, `lucide-check-square` Boolean,
`lucide-calendar` Date, `lucide-clock` DateTime, `lucide-calendar-range`
Duration, `lucide-list` List and Object, `lucide-tags` Tags, `lucide-link`
Link, `lucide-file` File, `lucide-regex` RegExp, `lucide-square-function`
Formulas, `lucide-file-question` null. The names in error messages are String,
Number, Boolean, List, Date, Object, Link, File, RegExp, Duration and Formulas.

**The parse tree.** Thirteen node types, all plain JSON:

| Type | Fields |
| --- | --- |
| `primitive` | `value` (string, number, `true`, `false`, `null`) |
| `ident` | `id`. `this` is one of these and not a node type of its own: `this.file.name` parses as `object_access(object_access(ident "this", "file"), "name")` |
| `array` | `elements` |
| `regexp` | the literal |
| `paren_expr` | `value` |
| `arithmetic` | `operator` (`+ - * / %`), `left`, `right` |
| `comparison` | `operator` (`== != < <= > >= && \|\|`), `left`, `right` |
| `negate`, `not` | `expr` |
| `function` | `name`, `subject` (null for a global call), `args` |
| `object_access` | `object`, `index` (the name as a string) |
| `array_access` | `array`, `index` (an expression) |
| `invalid` | `value` (the original text), `parseError` |

One node type, `comparison`, carries both the comparison and the boolean
operators. A failed parse is a node, not an exception: `{type: "invalid",
value: "foo((((", parseError: "Unable to parse formula. (position 0)"}`.

**Precedence, highest first, every level left-associative** — read off the tree,
case by case:

| Level | Operators | Evidence |
| --- | --- | --- |
| postfix | `.name`, `.name(...)`, `[expr]` | `a.b.c` → `(a.b).c`; `a.b().c()` → `(a.b()).c()`; `a.b[0]` → `(a.b)[0]` |
| unary | `-`, `!` | `!a.b` → `!(a.b)`; `-a.b` → `-(a.b)`; `!a && b` → `(!a) && b`; `!a == b` → `(!a) == b` |
| multiplicative | `*`, `/`, `%` | `1 + 2 * 3` → `1 + (2*3)`; `1 % 2 * 3` → `(1%2)*3` |
| additive | `+`, `-` | `1 - 2 + 3` → `(1-2)+3` |
| comparison | `<`, `<=`, `>`, `>=`, `==`, `!=` | `a + b > c + d` → `(a+b) > (c+d)`; `a == b == c` → `(a==b)==c`; `a < b < c` → `(a<b)<c`; `a >= b <= c` → `(a>=b)<=c` |
| and | `&&` | `a && b == c` → `a && (b==c)` |
| or | `\|\|` | `a && b \|\| c` → `(a&&b) \|\| c`; `a \|\| b && c` → `a \|\| (b&&c)` |

A negative number is folded into the literal (`-1 + 2` has `primitive -1` on
the left; `- 1 * 2` is `(-1) * 2`), while `-` in front of anything else is a
`negate` node.

**What is not in the language**, each tried and rejected by the parser:
the ternary `a ? b : c`; `??`; `in`; `contains` as an operator; the word
operators `and`, `or`, `not`; `&` and `|`; `**` and `^`; `===` and `!==`;
assignment; an object literal `{}`. Array literals *are* in (`[1, 2]`, `[]`,
and `[1, 2][0]`).

**Literals.** Double- and single-quoted strings, with **JSON escapes** — an
unknown escape fails with `Bad escaped character in JSON at position 3`, which
says the lexer hands the quoted token to `JSON.parse`; `\n`, `\t`, `\\`,
`\uXXXX` and raw UTF-8 all work, an unterminated string does not, and backticks
are not a string at all. Numbers: `1`, `1.5`, `.5`, `1.`, `01` and `-1` parse;
`+1`, `1e3`, `0x10` and `1_000` do not. `true`, `false` and `null` are
primitives; `undefined`, `NaN` and `Infinity` parse as idents and evaluate to
null. `/re/` and `/re/i` are regexp literals.

**Every name is case-insensitive — values are not.** `NOW()`, `nOw()`,
`"abc".LOWER()`, `12.3.TOFIXED(1)`, `[1,2].SUM()`, `file.NAME`, `when.YEAR`,
`file.properties.STATUS` and even the bare ident `DENSITY` all work.
`listish.contains("ALPHA")` is false where `("alpha")` is true, so values are
compared case-sensitively; tags are the exception — `file.hasTag("Rock")` and
`file.hasTag("#rock")` are both true.

**Coercion is for comparison, not arithmetic.** `"3" > 2` is true, `2 > "3"`
false, `"3" == 3` true, `true == 1` true, `"abc" > 1` false, and two strings
compare lexicographically (`"10" > "9"` is false). Arithmetic is typed and
refuses: `1 + true` and `true + 1` both fail with `Invalid operator between
Number and Boolean`, and `when - "2026-01-01"` with `Invalid operator between
Date and String`. `+` with a String on either side concatenates (`1 + "2"` is
the String `12`, `"a" + 1` is `a1`, and a text property `"42" + 1` is `421`).
Numbers are IEEE doubles printed as JavaScript prints them: `0.1 + 0.2` is
`0.30000000000000004`, `1 / 0` is `Infinity`, `0 / 0` and `1 % 0` are `NaN`.

**Null absorbs everything.** A missing property is null, and null answers every
function call and every property access with null: `nosuchprop + 1`,
`nosuchprop.length`, `nosuchprop.toFixed(2)` and `nosuchprop.nosuch.deeper` are
all null, with no error. Only three things differ: `isEmpty()` is true,
`isTruthy()` is false, and `toString()` is the String `null`. Comparison is
three-valued: `nosuchprop > 1` is **null**, not false, while `nosuchprop == null` is
true and `nosuchprop == ""` and `== 0` are false. `&&` and `||` coerce through
truthiness, so `nosuchprop && 1` is false and `nosuchprop || 1` is true. An
empty string is not null: `empty == null` is false and `empty.length` is 0.

**Dates and durations.** `now()` is a DateTime, `today()` a Date; a Date
compared against a String parses the String as a date (`when > "2026-01-01"`,
`when == "2026-01-02"`), but Date arithmetic needs a Duration, and a String in
that position is parsed as one: `now() - "1 week"`, `when + "1 month"`,
`when - "1M"` (a month: 2025-12-02) and `when - "1m"` (a minute: 2026-01-01
23:59). The unit letters are `y M w d h m s`, and the **`M` versus `m`
distinction is real**. `duration("1w")` and `duration("1 week")` agree;
`duration("1y 2M 3d")` is **null**, so compound durations are not supported.
Subtracting two dates gives a Duration, which prints humanized ("10 months",
"7 days", "a minute", "a few seconds"); its components are **properties, not
functions** — `days`, `hours`, `minutes`, `seconds`, `milliseconds`, `weeks`,
`months`, `years`, each fractional (`(now() - when).days` → 296.6033578819444).
All but `months` and `years` are an exact SI division of the total
milliseconds; those two are **calendar** quantities, and the eight recorded
reads pin the rule down exactly. Each read re-evaluated `now()`, so each landed
on its own millisecond — 25626530121 for `days` through 25626530131 for `years`,
recoverable from the exact conversions and monotonic in the run's own reading
order — and `months` is then the duration laid on the calendar **from the Unix
epoch**: 25626530126 ms after 1970-01-01T00:00:00 is 9 whole months (273 days)
plus 23.603 days of an October 31 days long, and `9 + 2039330126/2678400000` is
the recorded `9.761398643219833` to the last bit, where a fixed-length month
gives 9.7449 (30.436875 days), 9.7513 (365/12) or 9.8868 (30 days). `years` is
exactly `months / 12`, so a month is worth January 1970's 31 days and a year
1970's 365. `duration("1d") + duration("1d")`, `duration("1d") * 2` and
`duration("1d") > duration("1h")` all work. `date("not a date")` is null.
`relative()` answers a *clock-tagged* value — `lucide-clock "10 months ago"`,
not a String — and `now().relative()` is "a few seconds ago", so a zero
difference reads as past.

**There is no lambda, and that is the whole answer.** `=>` does not parse in any
form — `filter(x => x != "beta")`, `filter((x) => …)`, `map(x => x * 2)`,
`reduce((a, b) => a + b)` are all parse errors. The argument to `filter` and
`map` is an ordinary expression evaluated once per element with two magic idents
in scope, **`value`** and **`index`**; `reduce` takes two arguments, the
expression and the initial accumulator, and its accumulator ident is **`acc`**.
Obsidian's own completion text says so: `filter(value: Boolean)` — "Filter the
elements of this list by calling a filter function, which uses the variables
`index` and `value`"; `reduce(value: Any, acc: Any)` — "uses the variables
`value` and `acc`". Observed, on `nums: [3, 1, 2]` and
`listish: [alpha, beta, alpha]`:

| Expression | Value |
| --- | --- |
| `listish.filter(value != "beta")` | `alpha, alpha` |
| `nums.filter(index > 0)` | `1, 2` |
| `nums.map(value * 2)` | `6, 2, 4` |
| `nums.map(index)` | `0, 1, 2` |
| `nums.map(item * 2)` | `null, null, null` — an unknown ident is null, not an error |
| `nums.reduce(acc + value, 0)` | `6` |
| `nums.reduce(value, 0)` | `2` — the last element, because the expression ignores `acc` |
| `nums.map(value, index)` | `Cannot call function "map", too many arguments.` |

A summary formula gets the ident **`values`** instead.

**The function table.** It is *not* the JavaScript prototype — `12.345.toFixed(2)`
works although Number's prototype carries only `renderTo`, and `[3,1,2].length()`
is refused although List's prototype has `length`. The table was read two ways
that agree: the formula editor's completion list per receiver, and a brute-force
pass over candidate names using the four distinguishable refusals
(`Cannot find function "N" on type T`, `Cannot find "N" on type T` for a
property, `Cannot call function "N", not enough / too many arguments.`, and
`Type error in "N", parameter "P". Expected X, given Y.`). Every type has
`isTruthy()`, `isType(type: String)` and `toString()`.

| Receiver | Functions | Properties |
| --- | --- | --- |
| String | `contains(value: String)`, `containsAll(values: String)`, `containsAny(values: String)`, `endsWith(query: String)`, `isEmpty()`, `lower()`, `repeat(count: Number)`, `replace(pattern: String,RegExp, replacement: String)`, `reverse()`, `slice(start: Number, end: Number?)`, `split(separator: String,RegExp, n: Number?)`, `startsWith(query: String)`, `title()`, `trim()` | `length` |
| Number | `abs()`, `ceil()`, `floor()`, `isEmpty()`, `round()`, `toFixed(precision: Number)` | — |
| Boolean | — | — |
| List | `contains(value: Any)`, `containsAll(values: Any)`, `containsAny(values: Any)`, `earliest()`, `filter(value: Boolean)`, `flat()`, `isEmpty()`, `join(separator: String)`, `latest()`, `map(value: Any)`, `max()`, `mean()`, `median()`, `min()`, `reduce(value: Any, acc: Any)`, `reverse()`, `slice(start: Number, end: Number?)`, `sort()`, `stddev()`, `sum()`, `unique()` | `length` |
| Date, DateTime | `date()` (drops the time), `format(format: String)` (Moment.js), `isEmpty()`, `relative()`, `time()` | `year`, `month`, `day`, `hour`, `minute`, `second`, `millisecond`, `timestamp` |
| Duration | — | `years`, `months`, `weeks`, `days`, `hours`, `minutes`, `seconds`, `milliseconds` |
| Link | `asFile()`, plus the whole String set (a Link is a String receiver: `lower()`, `split()`, `slice()`, …), `linksTo(file: File)`, `isEmpty()` | `length` |
| File | `asLink(display: Any?)`, `hasLink(dest: File,Link,String)`, `hasProperty(property: String)`, `hasTag(values: String)`, `inFolder(folder: String)` | `name`, `basename`, `fullname`, `path`, `folder`, `ext`, `size`, `ctime`, `mtime`, `tags`, `links`, `embeds`, `backlinks`, `properties`, `file` |
| Object | `filter()`, `isEmpty()`, `keys()`, `map()`, `values()` | any key of the map; an unknown key is null |
| RegExp | `matches(value: String)` | — |
| null | every name answers null | every name answers null |

There is **no `upper()`** — `lower()` and `title()` exist and both readings
agree that nothing uppercases a string. `List.toString()` joins with `", "`; `Link.toString()` is `[[Pumice]]`;
`Object.toString()` is JSON. `file.folder` for a root file is `/`, `file.tags`
carry their `#`, and `file.links`/`backlinks`/`embeds` are Lists of Links.

**The globals** are sixteen, and the completion list is the whole set:

| Global | Signature and Obsidian's own description |
| --- | --- |
| `now()` | a date object representing the current moment (a DateTime) |
| `today()` | the current date (a Date) |
| `date(input: String)` | parses a timestamp or date object; `date("not a date")` is null |
| `duration(input: String)` | parse a string as a duration |
| `number(…)` | attempt to return the provided value as a number; a bad input gives `Unable to convert "RegExp" to a number.` |
| `list(element)` | wrap the element in a list, or return it unchanged if it is one; exactly one argument |
| `link(path, display: Any?)` | parses a string path and returns a Link |
| `file(input: String)` | attempt to locate a file for a path or link |
| `icon(input: String)` | a Lucide icon name, rendered as the icon |
| `image(input: String)` | an image object the view renders |
| `html(input: String)` | converts a string into a code snippet that renders as HTML |
| `escapeHTML(input: String)` | escapes special characters for inclusion in HTML |
| `if(condition, true_result: Any, false_result: Any?)` | the third argument is optional |
| `min(…)`, `max(…)` | the smallest/largest of all the numbers given; with no arguments, null |
| `random()` | a random number between 0 and 1 |

Names the table does **not** know, each tried: `string`, `bool`, `choice`,
`ifs`, `switch`, `coalesce`, `sum`, `average`, `mean`, `median`, `round`,
`floor`, `ceil`, `abs`, `sqrt`, `pow`, `log`, `trunc`, `sign`, `regexp`,
`object`, `type`, `empty`, `notEmpty`, `concat`, `join`, `contains`, `length`,
`count`, `first`, `last`, `reverse`, `sort`, `unique`, `filter`, `map`,
`reduce`, `markdown`, `embed`, `tag`, `prop`, `property`, `note`, `formula`,
`parseInt`, `asDate`, `asNumber`, `asString`, `asLink`, `asFile`, `dateDiff`.
(`note`, `formula`, `file` and `this` exist, but as idents, not functions.)

**The namespaces, and what `this` is.** A bare ident resolves to a frontmatter
property, so `density` and `note.density` are the same value, and `note` itself
is an Object of the note's frontmatter. `file` is the row's File. `formula.<name>`
is a namespace of its own, of type `Formulas`, holding the base's own formulas;
asking for one that does not exist gives `Cannot find "ppu" on type Formulas`.
And **`this` is the base file, not the row**: in a `.base` view `this.file.name`
is `Main.base`, `this.status` and `this.density` are null, `this.note` is an
Object and `this.formula` is `Formulas[]`. Silica must not implement `this` as
the current row.

**An invalid statement, by where it sits.** All three are kept verbatim in the
file and canonicalized like any other statement:

| Where | What happens |
| --- | --- |
| a filter | **silently empties the view** — 0 results, no error banner, no notice |
| a formula | the view renders; a cell whose statement failed to *parse* is empty, and one whose evaluation failed shows the error text as the cell's content (`Cannot find function "nosuchfunction"`) |
| a summary | kept, no error, nothing computed |

A boolean formula renders as a disabled checkbox, so its cell's text is empty
even though the value is `true`; a cell carries
`data-property-type="text|number|checkbox"`, which is the rendered type tag.

**Unprobed:** the `sort`, `group`, `filter` and property panels' own expression
mode (`filer-expression-mode-toggle`) and whether it writes a statement or a
structured rule; `isType`'s accepted type names; `format()`'s full Moment.js
token set; `image()` and `icon()` as rendered output rather than values;
`html()`'s sanitizer; the `tags` type's own function set (only `includes` was
reachable, and it refused); `Object.filter`/`map`'s idents (`value` and `index`
are assumed, not shown); how a Duration prints at every magnitude; operator
behaviour between a List and a scalar; a formula that references another formula
(`formula.x` inside a formula); and the editor's completion list for Number,
which `12.` could not reach because it is a valid number literal.

Building the evaluator added to that list, and these are the cheap probes that
would close it: whether `String.replace` replaces every occurrence or only the
first (the implementation assumes every, on the strength of a plural in
Obsidian's own sentence, and it is the riskiest guess in the crate); what
`[1, "a", 3].mean()` divides by, which is the only thing that separates
"mean over the numbers" from "mean over the list"; `fullname`'s value on a file
whose extension is not `.md`; whether `hasTag` matches a parent of a nested
tag; whether `inFolder` reaches subfolders; what `escapeHTML` escapes, which
was never called at all; `timestamp`'s unit; and what a further call on the
value `relative()` returns does.

## Observed Bases table view, Obsidian 1.14.5 (installer build 1.13.7)

What a `.base` *shows*, as against what it writes. Read off the same two runs
(`tools/obsidian-probe/results/2026-10-25-bases.json` and `-bases-expr.json`)
while the table view was being built, and in four places it corrects what the
write-behavior section above would lead you to assume.

**A column's header.** `properties.<id>.displayName` when the base gives one,
and the key is the **qualified** id — `properties['note.status']` lands where a
hand-written `properties.status` matches nothing, even though `order` carries
the id bare. Failing that it is Obsidian's own label for a `file.*` property,
not the id's last segment: `order: [file.name, status, formula.ppu]` renders
`file name`, `status`, `Price per unit`. The properties panel names the whole
set — `file backlinks`, `file base name`, `created time`, `file embeds`,
`folder`, `file full name`, `file links`, `modified time`, `file path`,
`file size`, `file tags`.

**A filter that empties the view leaves the header row drawn.** `BadFilter.base`
reports "0 results" with `headers: ["file name"]` and the error banner hidden:
the failure is silent in the view's body, not in its chrome.

**An empty `.base` is one default table of the whole vault.** A zero-byte file
and `{}` both parse to `views: [{type: table, name: View}]`, and the run's own
22-note vault rendered 22 rows.

**The no-value group comes first under an ascending `groupBy`.** The copied
TSV of a view grouped by `status` ascending, sorted by `status` descending,
is `status\tPrice per unit` then two blank-status rows and then the `todo`
ones — so grouping orders the rows before the sort does, and the group with no
value leads.

**Metrics.** The header row is 30 px. The three header boxes of `Layout.base`
measured x 344/436/524 and w 92/88/487: they abut exactly, the first is flush
with the pane's left edge, the last stretches far past its text, and 88 px is
the floor (the layout object carries `minColWidth` and `maxColWidth`). A
resized column writes `columnSize:` as a px map (`file.name: 232`), and the
header then measures 232. `rowHeight` is `""` (Short, the default and the key
absent), `medium`, `tall` or `extra`.

**The two error shapes.** A file that does not parse renders
`<div class="bases-error"><p>Unable to parse your base file:</p><p>…</p></div>`,
the second paragraph being the YAML library's own message. A view whose `type`
is outside the four registered layouts (`table`, `cards`, `list`, `kanban`)
renders the same block with one paragraph, `Unknown view type: nonsense`, and
no rows; the file still parses and its bytes are untouched.

**Unprobed, and the cheap probes that would close it.** `groupOrder`'s
semantics are **not observed at all**: the key appears in the view object's own
property list and in the published schema, but nothing in either run ever wrote
one, so the reading that a `null` entry is the no-value group — and the order
unnamed groups come in — is schema-reading, not observation. Drag a group
header and read the file. Also unobserved: a grouped table's own DOM, including
a group header's text; what an empty table renders; a row's measured height at
each `rowHeight` step; a cell's padding; whether a number cell is right-aligned;
the `data-property-type` of a Date, List or Link column (only `text`, `number`
and `checkbox` were ever rendered); how a Link cell with a display text draws;
and what a base leaf records in `workspace.json`, the showing view index
included.

## What building the evaluator settled, 2026-10-25

Implementing `silica-bases`'s evaluator against the same run turned up eleven
things the reading above got wrong or did not say. Every one is from the raw
results, not from the summary, and where the two disagreed the raw run won.

**One rule runs through every refusal, and it is not the obvious order.** A
call is checked **minimum arity, then the declared parameter types, then
maximum arity**. So `"s".contains(/zz/, /zz/, /zz/)` answers the *type* error
on `value` rather than "too many arguments.", and `date(/zz/, /zz/, /zz/,
/zz/)` likewise; "too many arguments." is only reachable once the declared
parameters type-check, which is why `number(/zz/, /zz/)` — whose parameter is
untyped — does answer it. The same order holds for `join`, `slice`, `map`,
`sort`, `toFixed`, `hasLink` and `link`.

**The namespaces.**

- `this` is not a File. `this.file.name` is the base file's name, `this.note`
  an Object and `this.formula` `Formulas[]`, but the completion list for
  `this.` offers only `file`, `note`, `formula`, `this` and the vault's
  frontmatter names — **none** of the File properties. So `this` is the base
  file's own row context, and `this.path` answers nothing where a File would
  answer a path.
- `Object.filter` and `Object.map` bind **`key` and `value`**, not `index`:
  Obsidian's own completion text says so for both.
- `index` *is* bound inside `reduce`, which the lambda note did not say
  (`nums.reduce(index, 0)` → 2), and `reduce`'s second argument is required.
  Nine other accumulator spellings — `accumulator`, `total`, `result`,
  `previous`, `carry`, `sum`, `current`, `prev`, `a` — bind nothing.

**Types and tags.**

- A **Tags** value names itself `List` in a refusal: `tags.includes()` is
  `Cannot find function "includes" on type List`. Only its icon is its own.
- `icon()` and `image()` answer values tagged **`lucide-image`**, a tag the
  list above omits.
- `relative()` answers a **clock-tagged** value, not a String.
- `min()` and `max()` with no arguments answer a *bare* null, with no `icon`
  at all — unlike every other null, which carries `lucide-file-question`.
- `Object.toString()` is JSON in which **every value is a string**:
  `{"density":"1.21","flag":"true","nums":"3, 1, 2"}`, the number, the boolean
  and the list all quoted.
- `Link.toString()` is `[[Pumice]]` only without a display; with one it is
  `[[path|display]]` (`file.links` → `[[Pumice|Pumice]]`).

**The two readings do not always agree.** The claim above that the completion
list and the brute-force pass agreed holds everywhere but Link: the completion
list for `linkish.` is the String set alone, with no `asFile()` and no
`linksTo()`, while the brute-force pass reached both. Where they differ the
brute force is the one that evaluated something.

**Smaller corrections.** `link()`'s first parameter is named **`self`**, not
`path` (`Type error in "link", parameter "self"`). `date()` accepts a Date as
well as a String, although its refusal still says `Expected String`.
`file.name` is the file name with a `.md` dropped and any other extension kept
(`Silica`, but `Main.base`), `basename` is always the stem and `file.size` is
in bytes. `List.stddev()` is the **population** deviation. `[3,1,2].unique()`
answers `1,2,3` while `listish.unique()` answers `alpha,beta`, so distinctness
is by rendered string and the order is a plain JavaScript object's key order,
not first-seen.

**One trap for anyone reading the raw run.** A step's `str` is not
`toString()`: it is `String(value.data)`, so a List cell shows `3,1,2` where
`[3,1,2].toString()` is `3, 1, 2`, and a Link cell shows `Pumice` where its
`toString()` is `[[Pumice]]`. Matching an implementation to `str` would build
the wrong `toString`. Note what this does *not* settle: those two readings are
inferred from `str` rather than read off a rendered cell, because neither a
List nor a Link column was ever rendered. A cell's own rendering is
[[#Observed Bases table view, Obsidian 1.14.5 (installer build 1.13.7)]].

## What building the rest of the Bases view settled, 2026-10-25

The other three layouts, the toolbar, editing through the table, the `bases:*`
commands and an embedded `![[x.base]]` were built against the same two runs.
Almost nothing here is a new observation — the runs rendered a table and
nothing else — so this is the list of **readings**: what the build had to
decide with no observation behind it, each of them a place where a cheap probe
would replace a guess. The keys themselves, their ranges and their defaults
are observed and are in
[[#Observed Bases write behavior, Obsidian 1.14.5 (installer build 1.13.7)]].

**The two layout keys that describe a box.** `cardSize` is read as a card's
**width** and `imageAspectRatio` as its image box's **width over height**, so
the kanban default of 0.5 is a portrait box twice as tall as a card is wide —
which is also the likeliest reason its default differs from the cards layout's
1. Both layouts read the key the same way, so a probe that settles one settles
the other. The reader never clamps a hand-written value and the drawing always
does, which is the split the sliders imply.

**What a row shows outside a table.** A card, a list line and a kanban card all
take their **title from the row's first cell** — `order`'s first id, usually
`file.name` — in the link colour when the column would draw as a link, with
the file's own name as the fallback for a row whose first cell rendered
nothing. The body is every other cell with the empty ones left out: stacked
under the title on a card, joined by `separator` on a line, one indented line
each under "Indent properties". A click anywhere on a card or a line opens the
row's file. `markers: number` restarts at 1 **inside each group** and `bullet`
draws `•`.

**Grouping outside a table.** Every layout takes `Table::groups` as it stands,
so `groupBy`, `groupOrder`, the direction and the leading no-value group are
the engine's answer and no layout reorders anything. A grouped cards or list
view draws the table's own group header between the runs; a kanban column *is*
a group, its header the key verbatim, "Empty" for the no-value one, and an
ungrouped board is one unnamed column of every row. **Dragging a card between
kanban columns is deliberately absent**: nothing is observed about whether such
a drag exists or what it would write, and a wrong guess rewrites a user's note.

**Metrics, none of them measured.** No cards, list or kanban view was ever
rendered, so every box is the shell's own: a 30 px group header (the table's
observed header row reused), 8 px gaps, 16 px grid padding, a 6 px radius on a
card, 20 px title and 18 px value lines in a kanban card, a 20 px marker
column. The same goes for the toolbar — a 30 px bar, 8 px padding, 22 px
buttons, five control icons in an order of Silica's choosing, and a panel
anchored at the pointer because no button box was ever measured.

**What the toolbar does not write, and why.** Three observed controls are drawn
inert or left out because what each writes is not derivable: **"Hide all"** is
unresolvable as written — `order` is exactly the shown columns, but an *absent*
`order` shows the implied ones, so a list that means "none" has no spelling;
**"Add formula"** never had its dialog opened; and **"Export CSV..."** never
had its file, location or dialect read. *Summarize...* and anything touching
`summaries` are out for the same reason, the Configure view page's name field
is drawn as text because renaming was never driven, the list layout's "Indent
properties" is read but never written, and **nothing anywhere writes or clears
`groupOrder`** — a `groupOrder` the file carries survives a `groupBy` change
untouched. The filter panel's **operator combobox** is the one structural
omission: a rule's shape was read but never its operator strings, so the panel
offers the expression mode the `filer-expression-mode-toggle` switches into
rather than inventing statement text. Each is a probe of one click.

**Two deliberate divergences from 1.14.5.** The Configure view page's Layout
combobox marks the view's **real** layout as selected, where 1.14.5 always
marks Table — that is a bug, not a key. And a control writes only when its
value differs, so opening a panel never writes the file.

**What a sort or an `order` edit writes.** *Sort Z → A* prepending
`{property, direction: DESC}` is the only insertion into `sort` the run pinned
down; *Sort A → Z* and the sort panel's own add row are read as its mirror and
prepend too. Either drops an entry the list already had for that property,
because otherwise clicking twice leaves a second entry that can never win and
the file grows on every click. *Clear sort* clears that column's entries only.
Neither the header menu nor the properties panel will **empty `order`**: an
empty list is omitted by the serializer and the implied columns come straight
back, so the last column is kept instead. Conversely, checking a box on a view
with **no** `order` materializes the implied ids *and* the new one — appending
the id alone would collapse the view to a single column. Ids are written as the
caller spells them: the header menu uses the column's own id, a panel uses what
its picker produced, which is qualified the way the observed `image` picker's
is.

**Editing a cell at all is a reading, and the one observation argues against
it**: the single cell control the run captured is a *disabled* checkbox. The
gesture is therefore the conventional one — a click on a frontmatter cell opens
a field holding what the note really has, Enter commits, Escape cancels, a
press outside or opening another cell commits, a click on an editable checkbox
flips the property, and a click on a `file.*` or `formula.*` cell opens the
note instead, those two having no bytes in it to write. A last column that
`columnSize` names is drawn at that width rather than stretching, which the
fixture's one sized column (its first) could not settle.

**The two unprobed commands.** `bases:change-view` is read as a suggester over
the view names and `bases:add-item` as a new note in the base's
`newItemFolder` from its `newItemTemplate`, opened in a new tab, writing
nothing into the `.base`. `View 2` is observed for a base's second view; `View
3`, `View 4`, ... by position is the reading of that one datum, and a name
already taken is stepped past byte for byte.

**An embedded base.** Everything but the rows is a reading, the embed being in
the write-behavior section's own unprobed list: the shell's note-embed box with
the view name as its title, **no toolbar**, always the file's **first** view
(a leaf remembers an index and an embed has nowhere to), a cap of 50 rows with
a line counting the rest, a row click opening that row's file, and a cards,
list or kanban base drawing the same table rows under its own name. What *is*
observed is the rows themselves, and both error shapes reach an embed
unchanged.

**One wording, used in one more place than it was observed.** A view that
resolves to no rows draws "0 results", which is what `BadFilter.base` reports;
where Obsidian puts that count, and what an empty table's body looks like, were
never read.

## Observed PDF view behavior, Obsidian 1.14.5 (installer build 1.13.7)

Observed on 2026-10-26 over CDP on a scratch vault
(`tools/obsidian-probe/pdf.mjs`; raw results in
`tools/obsidian-probe/results/2026-10-26-pdf.json`, 36 steps over five
launches, screenshots beside the run). `document.title` said 1.14.5, the user
agent `obsidian/1.13.7`. Window 1024 by 800 CSS px at a device pixel ratio of
1.45; the leaf was 680 px wide with the left sidebar open, 980 with it
collapsed. Fixtures: `doc.pdf` (typst 0.14.2, five A4 pages, page 3
landscape, a two-level outline, a 40 pt sentence per page), `small.pdf` (one
US letter page), `broken.pdf` (a `%PDF-1.4` header and 191 bytes of junk),
`Embeds.md` with eleven embeds and four links, `Board.canvas` with two file
cards and a text card, `Tall.md` with the embed 40 paragraphs down.

**One warning first.** pdf.js keeps a per-document record in
`localStorage['pdfjs.history']` (shared by every vault this Obsidian opens),
keyed by the PDF's fingerprint, and the first pass of this run left a 90°
rotation in it; every later open of `doc.pdf` — leaf, embed, canvas card —
came back rotated until the entry was stripped. The `b` steps of the results
file are the clean pass and are what this section reads; the first pass is
kept as evidence of the history itself. An implementer who ignores this
record will differ from Obsidian on every re-open.

### The leaf, and what persists where

The view type is `pdf`, registered for the extension `pdf` in
`app.viewRegistry.typeByExtension`; the embed registry has `pdf` too. There
is no PDF core plugin, no PDF setting (the only Settings row matching "PDF"
is the hotkey for `workspace:export-pdf`), no vault config key, and no
command whose id contains `pdf` other than `workspace:export-pdf`. The tab
icon is `lucide-file-text`, the tab's `aria-label` and the view header title
are the basename (`doc`), and the Files pane tags the row `PDF`.

The leaf record is the whole of what `workspace.json` carries, exactly:

```json
{"id":"24f2def7e86c6422","type":"leaf","state":{"type":"pdf","state":{"file":"doc.pdf"},"icon":"lucide-file-text","title":"doc"}}
```

`leaf.view.getState()` is `{"file":"doc.pdf"}` and stays so after
scrolling, zooming, rotating and `requestSaveLayout()`; nothing about page
or zoom is ever written to the vault. Page, zoom, scroll offset, rotation,
sidebar and spread live in `localStorage['pdfjs.history']`, shape
`{"files":[{"fingerprint":"c150ba4f803124202a289f715c5f156b","page":3,"zoom":150,"scrollLeft":-4,"scrollTop":539,"rotation":0,"sidebarView":0}]}`
(`zoom` is `"page-width"`, `"page-height"` or a percent number;
`spreadMode` appears only once set; `scrollTop`/`scrollLeft` are pdf.js's
in-page destination offsets in PDF units, not pixels). The entry is
rewritten on every scroll and zoom. Two more keys: `pdfjs.sidebarView`
(`"0"`, `"1"` thumbnails, `"2"` outline — the last sidebar view, global) and
`<appId>-pdfjs-search-settings`
(`{"highlightAll":false,"caseSensitive":true,"matchDiacritics":false,"entireWord":false}`).
"Adapt to theme" also survived a re-open in the same session; where it is
stored was not found.

**Restart fidelity.** Quit at page 3, scale 1.5, scrollTop 3514; after
relaunch the leaf came back active at page 3 with the same in-page offset
(`location.top` 539 before, 543 after) but at `page-width`, scale 0.559 —
the page and rotation and sidebar come back through the history, the zoom
level does not: every open starts at `page-width`. A same-session re-open
after rotating showed the same: page 3 and rotation 90 kept, zoom reset.
`lastOpenFiles` listed `doc.pdf` like any file.

### The view's markup

`.view-content` (flex column, `--pdf-background` `#1C1C1C` dark / `#ffffff`
light) holds `.pdf-toolbar` then `.pdf-container` > `.pdf-content-container`
> `.pdf-viewer-container` (absolute, `overflow:auto`, the scrolling box;
`tabindex` via its text layers) > `.pdfViewer` > `.page` × N, plus
`.pdf-sidebar-container` and `.pdf-findbar.pdf-toolbar.mod-hidden`. The
view header has the Bookmark and More options actions only. The pdf.js
`PDFViewer` is at `view.viewer.child.pdfViewer.pdfViewer`; its owner
`view.viewer.child.pdfViewer` carries `subpath`, `height`, `isEmbed`,
`applySubpath`, `pdfSidebar`, `toolbar`, `findBar`, `pdfOutlineViewer`,
`pageInvert`, `pageBackground`. `textLayerMode` is on (`.textLayer` per page,
opacity 0.2, `span.textLayerNode` with transparent text); the annotation
layers are present and hidden.

**Toolbar.** 38.69 px tall, full width, `padding: 4px 12px`, flex, background
`--pdf-background`, text `rgb(218,218,218)` dark / `rgb(34,34,34)` light.
`.pdf-toolbar-left` (gap 4) in order: *Toggle sidebar* (`lucide-layout-list`),
*Sidebar options* (`lucide-chevron-down`), a 12 px `.pdf-toolbar-spacer`,
*Zoom out* (`lucide-zoom-out`), a `.pdf-toolbar-divider` 1 by 16 px
(`#333333` dark, `#e4e4e4` light), *Zoom in* (`lucide-zoom-in`), *Display
options* (`lucide-chevron-down`), a spacer, `input.pdf-page-input`
(`type=number`, `max=5`, 52 by 30, `padding: 4px 8px`, radius 5, 13 px,
background `rgb(46,46,46)` with a 0.69 px `#333333` border dark / white with
`#e4e4e4` light, value right-aligned), `span.pdf-page-numbers` "of 5" (13 px).
Every button is a 28 by 24 `.clickable-icon` (`padding: 4px 6px`, radius 4,
16 px icon) in `rgb(179,179,179)` dark / `rgb(92,92,92)` light; the sidebar
toggle while open is `is-active`: accent text `rgb(166,138,249)` on the
accent at 10 % alpha. `.pdf-toolbar-right` (gap 4) is empty in a leaf; in an
embed and on a canvas card it holds a spacer and *Open*
(`lucide-more-vertical`), and on a canvas card also *Save current position
in document* (`lucide-file-symlink`). There is no zoom `<select>`, no
rotation button and no find button.

**Pages.** `.pdfViewer` carries `--scale-factor` = scale × 4/3 (0.791 →
1.0550, 1 → 1.3333, 1.5 → 2). A `.page` is the PDF size × `--scale-factor`
(the A4 page 628 by 888 CSS px at 0.791) inside a 4.14 px transparent
border, so page boxes are 636.27 by 896.27 and abut with no gap (8.28 px
between canvases). The `.canvasWrapper` carries `--pdf-shadow`:
`0 0 0 1px #333333` dark, `0 0 0 1px rgba(0,0,0,0.05), 0 2px 8px
rgba(0,0,0,0.1)` light; the page itself has no background, no radius. The
landscape page is 896.27 by 636.27 and centred. The canvas bitmap is 909 by
1287 for the 628 by 888 page (DPR 1.45). The `::selection` on the text layer
is the accent at 0.33 alpha. Page 1 starts at the top of the scroll box with
no top margin; the viewer opened with `scrollLeft` 15.9.

### Zoom

The default on every open is `currentScaleValue = "page-width"` (0.791 at
668 px of client width; 1.169 at 968 after collapsing the left sidebar — the
scale follows a resize at once; 0.615 at 528 with the PDF sidebar open).
*Zoom in* from 0.791 gives 0.9, 1, 1.1, 1.3, 1.5, 1.7; *Zoom out* gives 0.7,
0.6, 0.5, 0.4, 0.3, 0.2, 0.1, 0.1 (pdf.js's ×1.1 step rounded out to a tenth,
floor 0.1); after a step `currentScaleValue` is the number as a string.
Setting 10 holds (ceiling 10). *Display options* is a `.menu`: **Fit width**
(`lucide-move-horizontal`, checked by default), **Fit height**
(`lucide-move-vertical`), ---, **Single page** (`lucide-rectangle-vertical`,
checked), **Two-page (odd)** (`rectangle-vertical-double`, not a Lucide
name), **Two-page (even)**, ---, **Adapt to theme** (`lucide-palette`). Fit
height sets `currentScaleValue` to `page-height` (0.584 here, the same scale
pdf.js's `page-fit` gives; the A4 page then 467.51 by 659.52, centred).
Two-page (odd) sets `spreadMode` 1 and wraps pages in `.pdfViewer >
.spread` pairs at half the width (0.396, pages 318 by 448); (even) sets 2
and puts page 1 alone on the right. Adapt to theme adds `mod-themed` to
`.pdf-container`, sets the owner's `pageInvert` true and `pageBackground`
`#1C1C1C`; the canvas has no CSS filter, so the inversion is in pdf.js's
render. `Ctrl+=`, `Ctrl+-`, `Ctrl+0` are Obsidian's window zoom
(`webFrame` 1 → 1.1 / 0.91 / 1), not the viewer's; `Ctrl+wheel` zooms the
viewer (100 units down: 0.791 → 0.5; 100 up: → 1.1).

### Keyboard, scrolling and the page number

With the viewer focused (a click on a page focuses its `.textLayer`): Down /
Up scroll 40 px; PageDown, Space / PageUp, Shift+Space scroll 566 px in a
672 px box; Home / End go to 0 / the end (page 5, 3550); **Right / Left go
to the next / previous page's top** (896 per page) — they and Escape are the
only keys in `view.scope` (`ArrowLeft`, `ArrowRight`, `Escape`, no
modifiers). `j`, `k`, `n`, `p`, `r`, `Shift+r`, `Ctrl+e` do nothing; `Ctrl+a`
selects nothing; `Ctrl+f` opens the find bar; `Ctrl+p` and `Ctrl+o` open
Obsidian's own modals from inside the viewer.

A 900-unit wheel scrolls 900 px. `currentPageNumber` is pdf.js's: the page
the largest fraction of which is visible, earlier page on a tie — at
scrollTop 886 (10 px above page 2's top) it is already 2, at 538 (60 % down
page 1) still 1, 10 px above page 3's top already 3. `openLinkText` and
the page input land on a page's `offsetTop` exactly (page tops 0, 896,
1793, 2429, 3325 at 0.791).

**Page input.** Typing a number and Enter goes to that page's top; `99`,
`0` and `abc` leave the page alone and the input is put back to the current
page; blurring with a changed value also commits. The label is `of N`.

### Sidebar

*Toggle sidebar* adds `sidebarOpen` to `.pdf-content-container`;
`.pdf-sidebar-container` is 140 px wide, absolute at the left, background
`--pdf-sidebar-background` (`#1C1C1C` / `#ffffff`), with a 3 px
`.pdf-sidebar-resizer` at its right edge; the viewer container moves right
by 140 and `page-width` recomputes. It has no tabs: `.pdf-sidebar-content`
holds `.pdf-thumbnail-view` (padding 12) and `.pdf-outline-view` (padding 8,
`hidden` class when not shown), and `data-view` 1 / 2 says which. *Sidebar
options* is a `.menu`: **Thumbnails** (`lucide-layout-grid`, checked),
**Table of contents** (`lucide-list`), ---, **Reveal page in table of
contents** (`lucide-list-start`). A thumbnail is `a[href="#page=N"] >
.thumbnail[data-page-number][data-page-label][data-loaded] >
img.thumbnailImage` (a PNG data URL) with `--thumbnail-width: 98px;
--thumbnail-height: 138px` (69 for the landscape page), a 7.59 px border
(transparent; white at 6.7 % on `.selected`), radius 4, so 113 by 153 per
box; clicking one goes to that page and moves `selected`. The outline is
Obsidian's tree: `.tree-item.is-collapsed > .tree-item-self.mod-collapsible
.is-clickable` (24.88 px, `padding: 4px 8px 4px 24px`, margin-bottom 2,
radius 8, 13 px `rgb(179,179,179)`) with `.collapse-icon > svg.right-triangle`
(10 px, `rgb(102,102,102)`), `.tree-item-inner` ("1 One") and
`.tree-item-flair` (the page number, 12 px, `rgb(102,102,102)`); children
start collapsed. Clicking "3 Three" went to page 3; no row was marked
current after the click or after *Reveal page in table of contents*. The
open state and the view survive a re-open through the history entry's
`sidebarView`.

### Find bar

Only `Ctrl+f` opens it. `.pdf-findbar.pdf-toolbar` (42.67 px, padding 6,
gap 4, 13 px) is inserted above the viewer container, which shrinks by the
`--findbar-height: 42px` set on `.pdf-content-container` (class
`findbarOpen`). Right-aligned `.pdf-search-wrapper`: the count
`span.pdf-toolbar-label.pdf-find-results-count` ("1 of 3 matches",
`rgb(179,179,179)`, hidden while empty), a `.search-input-container
.global-search-input-container` with `input[type=search]` placeholder
`Search...` (300 by 30, `padding: 4px 8px 4px 30px`, background
`rgb(46,46,46)`, border `rgb(85,85,85)`, radius 5), its clear button, the
`.input-right-decorator` *Match case* (icon `uppercase-lowercase-a`), two
`button.pdf-toolbar-button` *Shift + F3* (`lucide-arrow-up`) and *F3*
(`lucide-arrow-down`), `.pdf-findbar-settings-btn` *Search settings*
(`lucide-sliders-horizontal`) and *Close* (`lucide-x`). Typing searches as
you type: `three` gave "1 of 3 matches", scrolled to page 3 and wrapped the
hit in `.highlight.selected.appended` (`rgb(138,92,245)`); Enter and the
down button step to "2 of 3", "3 of 3"; the up button steps back. A query
with no hit empties the count and gives the input a `rgba(251,70,76,0.6)`
border and 2 px ring. Escape and Close hide the bar (`mod-hidden`), clear
the highlights and return focus to the body. *Search settings* toggles an
inline `.pdf-findbar-settings` panel (absolute, 200 px, radius 8, border
`#333333`, padding `4px 8px`) of three `.setting-item.mod-toggle` rows (27
px, 13 px names) **Highlight all**, **Match diacritics**, **Whole words**
with `.checkbox-container.mod-small` 34 by 19 toggles; the settings key is
written as above. Match case caught a `THREE` search as case-sensitive;
the three panel toggles were not confirmed to take (the clicks did not
flip them in this run).

### Links and `#page=`

`openLinkText('doc.pdf#page=N')` into an existing pdf leaf reuses that leaf
(no new tab) and scrolls to page N's top. Out of range (`99`, `0`, `-1`)
leaves the view where it is; `abc` goes to page 1; a bare `doc.pdf` while
at page 3 stays at 3; `#height=300` and `#foo` left it at page 1 where
`#page=abc` had put it, so they are not separated from "ignored";
`#page=3&height=200` goes to page 3. `openLinkText(..., true)` opens a
second pdf leaf. In reading view the links render as `a.internal-link`
(`href` = `data-href` = `doc.pdf#page=4`, `target=_blank`,
`rel="noopener nofollow"`; the Markdown `[text](doc.pdf#page=2)` the same);
a plain click **navigates the note's own leaf** to the PDF (the markdown
leaf is gone, the leaf count unchanged) at the linked page; Ctrl+click and
middle click open a new tab at that page. Hovering `[[doc.pdf]]` for 1.5 s
with page-preview on opened `.popover.hover-popover` (451 by 401, radius 8,
border `#333333`) holding a `.pdf-embed.is-loaded` 450 by 400 with its
toolbar `display:none` and the five pages at `page-width`.

### Embeds

Reading view: `span.internal-embed.pdf-embed.is-loaded[src][alt]` inside
the paragraph; Live Preview: `div.internal-embed.pdf-embed.is-loaded
[tabindex=-1][contenteditable=false]` placed directly in `.cm-content` —
there is no `.cm-embed-block` wrapper. `alt` is the link text:
`doc.pdf`, `doc.pdf > page=3`, `alias text`; `![](doc.pdf)` has no `alt`,
`![alt](doc.pdf#page=2)` has `alt="alt"`. The box is the line's full width
(604.43 at readable line width), `border: 0.69px solid #333333`, radius 4,
transparent background, `overflow:hidden`, a flex column of the same
`.pdf-toolbar` (with *Open* on the right) over `.pdf-container`; each
embed owns a full `PDFViewer` with its own hidden sidebar and find bar, at
`page-width` (0.745 at 591 px). **Height** is an inline style and, in
every case measured, this:

| embed | inline height | what it is |
| --- | --- | --- |
| `![[doc.pdf]]`, `![[doc.pdf\|alias text]]`, `![](doc.pdf)` | `873.843px` | toolbar 38.69 + page 1 at page-width 834 + borders |
| `![[doc.pdf#page=3]]`, `![alt](doc.pdf#page=2)` | `873.843px` | the same — sized from page 1, not the target page |
| `![[small.pdf]]` | `802.824px` | toolbar + the letter page's 764 |
| `![[doc.pdf#height=300]]` | `338px` once loaded (`300px` before) | `height=` + the toolbar |
| `![[doc.pdf#page=2&height=200]]` | `200px` in every pass | `height=` as the whole box |
| `![[broken.pdf]]` | none; computed 800 px | the stylesheet's default |

So the default is "one page tall", and whether the toolbar is added on top
of `height=` differed between the two `height=` cases; this run did not
isolate why. `#page=2` embeds sit exactly on page 2's top (scrollTop 844 =
its offset); the two `#page=3` embeds sat 353 px below page 3's top in
every pass, with the input reading 4 — page 3 is the landscape page, and
the cause was not isolated. `![[missing.pdf]]` is
`span.internal-embed.is-loaded.file-embed.mod-empty-attachment` with the
text `“missing.pdf” could not be found.` (padding 8, radius 8, background
`rgb(35,35,35)`, `rgb(179,179,179)`, 14 px, 37 px tall). `![[broken.pdf]]`
builds the full chrome with `0` in the input and `of 0`, no pages, no
error text, no notice. Source mode shows the text with the usual
`cm-formatting-embed` / `cm-hmd-embed cm-hmd-internal-link` spans and no
widget.

Embeds build lazily in both modes: in `Tall.md` the embed element did not
exist until the view was scrolled to it, and only the two pages near the
viewport got a canvas; in Live Preview the off-screen embeds of `Embeds.md`
are not in the DOM either. A wheel over an embed scrolls the embed's pages
(300 units → inner scrollTop 300, the note unmoved); once the embed is at
its end the next wheel scrolls the note. The embed's page input works like
the leaf's (3 Enter → page 3's top), and nothing about an embed reaches the
note's `getState()` (`{file, mode, source}`), its ephemeral state
(`{scroll}`) or `workspace.json`. *Open* in an embed's toolbar opens a
`.menu`: **Open in new tab** (`lucide-file-plus`), **Open to the right**
(`lucide-separator-vertical`), **Open in new window**
(`lucide-picture-in-picture-2`), ---, Rename..., Move file to...,
Bookmark..., ---, Copy path, ---, Open in default app, Show in system
explorer, Reveal file in navigation; *Open in new tab* from the embed
opened a pdf leaf at the embed's current position.

### Canvas

A `file` node for `doc.pdf` (600 by 500) renders
`.canvas-node-content.pdf-embed.is-loaded` with the same inline one-page
height (`868.186px`, clipped by the card), the toolbar with *Open* and
*Save current position in document* on the right, the pages at
`page-width` for the card (0.74 at 587 px), and the `.canvas-node-label`
`doc.pdf` above the card (`rgb(102,102,102)`, 15 px). `"subpath":
"#page=2"` puts the card's viewer on page 2 (scrollTop 840 = its offset).
A text card `![[doc.pdf#page=4]]` renders the markdown embed of the
previous section inside `.canvas-node-content.markdown-embed`, with *Open*
but no save button. A wheel over the card pans the canvas (300 units → 279
canvas units), never the pages, toolbar included; a click selects the
card; a double-click on the pdf card leaves it `is-focused` and opens
nothing (a double-click on the text card enters its edit mode). *Save
current position in document*, clicked with the card's viewer at page 3,
opened no menu, showed no notice and changed neither the node's `subpath`
nor the file. *Open* → *Open in new tab* opened a pdf leaf at page 1;
*Open to the right* split; *Open in new window* opened a popout.

### Menus, selection and copy

Right-click on a page with no selection, on the text, on the toolbar, in
the gap between pages and in the margin opens no menu. The tab header
menu is the generic file one (Close, Close others, Close all, ---, Pin,
Link with tab..., ---, Move to new window, Split right, Split down, Open in
new window, ---, Rename..., Move file to..., Bookmark..., ---, Copy path,
---, Open version history, Open linked view, ---, Open in default app, Show
in system explorer, Reveal file in navigation, ---, Delete file); More
options is the same without the Close and Pin groups. A drag across the 40
pt sentence selects `This is page one.`; right-click on it opens **Copy**,
**Copy as quote**, **Copy link to selection** (all `lucide-copy`, section
`selection`), which write exactly:

```
This is page one.
> This is page one.\n\n[[doc.pdf#page=1&selection=8,0,8,17|doc, page 1]]
[[doc.pdf#page=1&selection=8,0,8,17|doc, page 1]]
```

`selection=a,b,c,d` is start node, start offset, end node, end offset in
pdf.js's text-layer numbering; the alias is `<basename>, page <n>`.
`Ctrl+c` copies the plain text; a selection across lines reads with `\n`
in `getSelection()` but copies as one space-joined line.

### Panes and the broken file

With the pdf leaf active the Outline pane shows `No headings found.`, the
Backlinks pane `Linked mentions 13` (the eight `doc.pdf` embeds and four
links of `Embeds.md`, and `Tall.md`), Outgoing links `Links 0`. `small.pdf`
opens at `page-width` 0.770 with `of 1`. `broken.pdf` opens as a normal pdf
leaf (`{"type":"pdf","state":{"file":"broken.pdf"}}`, tab `broken`) whose
toolbar reads `0` `of 0`, with `.pdfViewer` empty, no `pdfDocument`, no
error element, no notice, and nothing more after three seconds.

### Not observed

Whether the toolbar is added to `height=` (the two cases disagree); why a
`#page=` embed onto the landscape page lands 353 px low; where "Adapt to
theme" persists; the three find-settings toggles taking effect; Shift+Enter
in the find bar (no movement seen, possibly the harness); a click on the
`[text](doc.pdf#page=2)` link (the click missed; it renders like a
wikilink); any effect of *Save current position in document*; whether
`#foo` or `#height=300` on a link resets the page or is ignored;
`#page=2.7`; the page count label with page labels; a password-protected
PDF; printing/export; what the viewer does under a narrow leaf (below 300
px); the light theme's thumbnail and outline colours beyond the custom
properties; the first pass's `display:none` text layer under the stale
rotation.

## What building the PDF view settled, 2026-10-26

The view, the embed and the canvas card were conformed to
[[#Observed PDF view behavior, Obsidian 1.14.5 (installer build 1.13.7)]];
what follows is this build's own reading where the run did not reach, each
marked `OBSERVE:` in the code.

- **What is written.** Nothing about a page, a zoom or a scroll offset
  reaches the vault, as observed; pdf.js's `localStorage['pdfjs.history']`
  is outside the vault and is not reproduced. A tab's page and zoom are
  remembered in memory for the session, which is the observable equivalent
  of Obsidian's leaf staying alive behind a tab.
- **Fit height.** pdf.js pads a `page-fit` scale by 5 px; the one observation
  (0.584 at 683 px) implies 27. The observed value is used.
- **The embed's height.** One page tall by default (toolbar plus the first
  page at page-width), `#height=` plus the toolbar when given, which matches
  the `#height=300` case; the `#page=2&height=200` case measured 200 and was
  not isolated.
- **The viewer's ground** between and beside the pages is the shell's
  secondary background; the box of an embed is transparent as observed.
- **A loading file** draws the broken file's chrome (`0` `of 0`) until the
  pages are known; Obsidian's loading frame was not captured.
- **The outline's empty state** says "No outline available"; unprobed.
- **Inert by design**, with the menus saying so: Two-page (odd/even), Adapt
  to theme, Search settings' three toggles, the sidebar resizer, the embed's
  *Open* menu (the button opens the file in the tab), *Save current position
  in document*, the alias size hint (`|300`) which stands in for `#height=`
  only when there is none.
- **Not drawn at all**: a text layer, so no selection, no copy strings and no
  highlight of a find match (find scrolls to the page); the hover popover;
  the `rectangle-vertical-double` icon, which is not Lucide's (the single
  rectangle stands in).

## What building math rendering settled, 2026-10-26

Math was not probed against Obsidian for this build; what follows is the
build's own reading of MathJax's documented behaviour and of how Obsidian
styles its output, each to be checked in a later pass. The engine is
KaTeX-compatible, so the syntax accepted is KaTeX's.

- **Delimiters** are unchanged: the parser's `$x$` and `$$x$$` rules in
  [[#Parser notes]] decide what is math; the engine only sees the inside.
- **Inline math** (`$x$`) is set in TeX text style at the surrounding font
  size, in the text colour, on the text's baseline, and takes exactly the
  width the engine gives it plus a slack smaller than a narrow no-break
  space. A formula taller than the font grows its line in Live Preview; in
  the reading view it draws over the line above or below (gpui lays the
  paragraph's lines out at one line height). OBSERVE: whether Obsidian's
  inline MathJax grows the line box in reading view (it should, as an inline
  SVG with `vertical-align`).
- **Display math** (`$$x$$`, on its own lines or on one line) is set in
  display style, centred in the note's width, with half an em above and
  below, and scrolls sideways when wider than the note. OBSERVE: MathJax's
  display container has `margin: 1em 0` in its own stylesheet; Obsidian's
  theme may override it.
- **`$$x$$` inside a paragraph** breaks the paragraph into text, the block
  and text. In a heading or a table cell it stays inline, in display style.
- **Colour.** Black (the engine's default) is repainted in the theme's text
  colour; `\color` and `\textcolor` keep theirs. OBSERVE: Obsidian's math
  inherits `color` the same way.
- **Errors.** A formula the engine rejects shows its source in
  `--text-error` red (`#e93147`), the text font, in place: inline in the
  reading view, centred as a block. In Live Preview it falls back to the
  pre-math behaviour, source in the accent colour with the dollars hidden.
  OBSERVE: MathJax renders `merror` as the source in red with the message
  as a `title` tooltip; the message is not shown here.
- **What does not parse here but does in Obsidian**: `\require{...}`,
  `\newcommand` defined in one formula and used in another (KaTeX's macros
  are per formula), and AMS or physics extensions MathJax loads on demand.
  Each shows as an error, above.

## What building the editing commands settled, 2026-10-26

The commands were conformed to
[[#Observed editing command behavior, Obsidian 1.14.5 (installer build 1.13.7)]]
and the 2026-10-25 block of [[#Observed auto-pair behavior, Obsidian 1.14.4]];
what follows is this build's own reading where the run did not reach.

- **The cycle over mixed list lines.** Obsidian oscillates on a selection of
  list lines that differ only in their checkbox. Silica advances each line
  one step on its own (`- a` / `- [ ] b` → `- [ ] a` / `- [x] b`), which
  converges; the observed normalisation — a line that is not a list item
  pulls every line back to a bullet — is kept as it is.
- **Mod+L over a selection** follows the observed three rows; the inverting
  third row is implemented as observed, since one press of it is what was
  measured and a second press is the user's.
- **The plain `editor:toggle-highlight` over a coloured highlight** takes the
  whole highlight off, emoji included. OBSERVE: whether Obsidian strips it or
  only drops the colour.
- **A colour emoji in Live Preview** stays visible in the source text; the
  reading view consumes it. OBSERVE: whether Obsidian's Live Preview hides
  the emoji inside the rendered mark.
- **The highlight washes** are `--color-<name>` at 0.3 alpha. Red was read
  back; the other five use the same tokens the canvas presets use (and
  `--color-blue`, `#027aff` dark / `#086ddd` light). OBSERVE: the five
  computed values.
- **The selection after a toggle.** One line keeps the selection on its text
  (the toggle then undoes itself); several lines leave it over the whole
  wrapped text, markers included, so that the second press finds a pair in
  every line's part. OBSERVE: where Obsidian's selection lands after a
  multi-line wrap.
- **A bare caret on whitespace** gets an empty pair with the caret between
  the halves; on a word it takes the word, as observed, and leaves the word
  selected.
- **`insert-horizontal-rule` at the start of a non-blank line** inserts
  `\n\n---\n` at the caret like any other mid-line position, giving two blank
  lines, the rule and the line. OBSERVE: ch 0 of a non-blank line.
- **`insert-footnote` when the document already ends in blank lines** adds
  only the newlines needed for one blank line before the definition.
  OBSERVE: a document ending in `\n\n`.
- **`clear-formatting` and line prefixes.** A line's `#`s, `> ` and list
  marker go when the selection covers them from the line's start; a quoted
  list item loses both. Indentation stays. A selection that starts inside the
  line leaves its prefix.
- **Skip-over's "leaving the line"** is a vertical move or a click; moving
  within the line by any means keeps the planted closer, and any edit that
  lands on the closer, or an undo, forgets it. A planted closer is also
  forgotten lazily when the caret is found on another line at the next
  keystroke, which covers moves the editor makes without a hook.
- **Enter on an empty nested item** keeps the previous one-step outdent
  (`lists::outdent_width`): the Shift+Tab rows did not probe Enter.
- **Tab with `useTab: false`** inserts four spaces whatever `tabSize` says,
  as the calibration row observed under `tabSize: 2`; the same unit serves a
  plain line and a list item. OBSERVE: Tab on a plain line under
  `useTab: false, tabSize: 2`.

## What building folding settled, 2026-10-26

The editor, the reading view and the record were conformed to
[[#Observed heading fold behavior, Obsidian 1.14.4]] and
[[#The 2026-10-25 pass: the cold restart and the rest of the block]]; what
follows is this build's own reading where the run did not reach.

- **A fold's identity across edits** is its line's kind and text and which
  of the lines with that text it is; an edit to the folded heading's text
  keeps the fold by its line. CodeMirror's folds are positional; the
  difference shows only when a folded heading is retyped as another
  heading's text, which keeps the fold here.
- **The caret inside a fold.** A fold command leaves the caret where it is
  (so `fold-less` from inside a folded section still finds it, as
  observed); a caret that arrives in a fold by a link, a search hit, an
  outline row or an edit opens the fold. OBSERVE: what Obsidian does to a
  caret already inside a section when `editor:toggle-fold` folds it, and
  whether typing inside a fold unfolds it.
- **The indicator is shown while folded**, not only on hover, since the
  placeholder `…` alone does not say where to click. OBSERVE: the folded
  line's `cm-fold-indicator` opacity when not hovered.
- **List folds in the reading view** collapse nothing: Obsidian's reading
  view list collapse was not probed. The keys are carried so a switch to the
  editor and back keeps them.
- **The record's home** is `note-fold/<vault id>.json` under Silica's
  configuration folder, one file per vault with a map by vault-relative
  path, the per-note value in Obsidian's own shape. Obsidian's is
  `localStorage`, which has no file to match.
- **A fold is saved on leaving the note and on quit**, read as the note
  opens, and the stored `to` is ignored in favour of the heading's extent
  (observed). A note left before its first parse has settled keeps the
  record it was opened with.
- **The reading-view chevron** sits 28 px into the left margin, centred on
  the heading's line box; Obsidian's is at `left: 31.99px` of a container
  whose padding was not measured. OBSERVE: the offset from the heading
  text.
- **`fold-all` with the caret inside a section** folds everything and
  leaves the caret in place, which is what the observed run reported (three
  rendered lines, the caret unrecorded).
- **`foldHeading: false`** removes the reading view's chevron entirely
  rather than keeping a hidden, clickable one.
