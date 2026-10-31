---
tags:
  - silica/dev
  - silica/probe
---
# Interaction probes

The automated probe batch of 2026-10-18 ([[Development#The probe batch, 2026-10-18]])
settled formats, storage keys, DOM shapes and command ids, and wrote them up as the
`## Observed ...` sections of [[Specification]]. It could not settle **interaction**:
there is no input automation on this machine, and a synthetic event carries no focus,
so nothing that needs a real keystroke, a real drag or a real dropdown came back.

This note is that remaining pass, done by hand. Each block closes the `**Unprobed:**`
paragraph of one specification section.

> [!warning] Not in the gate vault
> Use a disposable scratch vault. **Not** `~/bstore2/raga/plmcrzy/PLMCRZY` — a
> byte-level gate run is measuring it — and not `docs/silica`. Each block names the
> fixture notes it needs.

> [!tip] Most of this is now automatable
> After this note was written, Obsidian turned out to speak the Chrome
> DevTools Protocol (`obsidian --remote-debugging-port=9222`), whose input
> events carry real focus — see
> [[Development#Driving Obsidian over CDP, 2026-10-18]]. The auto-pair block
> below was answered that way in about a minute rather than the fifty
> estimated, and the search-pane and properties blocks were ported to scripts
> the next day ([[Development#Porting two checklist blocks to CDP, 2026-10-19]]);
> all three sets of findings are in [[Specification]]. Before
> working through a block by hand, check whether CDP can do it: nearly all of
> these steps can be, and what is left for a person is the aesthetic
> judgement — how something looks, whether an affordance feels right.

## Order of work

**Every block of this note has now been run.** Blocks 1-3 went over CDP in
October 2026 and blocks 4-10 on **2026-10-25**, in six scripts under
`tools/obsidian-probe/` whose raw runs are in `results/2026-10-25-*.json`. Block
11, the Bases groundwork, went the same day in two more scripts; it is the one
block that closed no **Unprobed** paragraph, because it opened the two
sections it writes. What
each one found is written into the matching `## Observed ...` section of
[[Specification]], under a `### The 2026-10-25 pass` heading, and the lines
they settle are struck from those sections' **Unprobed** paragraphs. The
estimate below was about 8 hours of hand work; over CDP the seven blocks took
about twenty minutes of wall clock.

What genuinely stays for a person is aesthetic judgement — how something looks,
whether an affordance feels right — plus the handful of rows each pass names as
still unprobed, which are collected in each section rather than here.

| # | Block | Cost | Why here |
| --- | --- | --- | --- |
| ~~1~~ | [[#Auto-pair interaction\|Auto-pair]] | ~~50 min~~ **done** | answered over CDP on 2026-10-18; kept below only for the few rows still open |
| ~~2~~ | [[#Search pane\|Search pane]] | ~~40 min~~ **ported** | run over CDP on 2026-10-19 (`tools/obsidian-probe/search.mjs`, `search-copy-modal.mjs`); what is left is in the section's Unprobed paragraph |
| ~~3~~ | [[#Properties in reading view — hands-on pass\|Properties in reading view]] | ~~50 min~~ **ported** | run over CDP on 2026-10-19 (`tools/obsidian-probe/properties.mjs`); what is left is in the section's Unprobed paragraph |
| ~~4~~ | [[#Attachment paste\|Attachment paste]] | ~~50 min~~ **done** | run over CDP on 2026-10-25 (`tools/obsidian-probe/attachment-paste.mjs`): a real clipboard image and a real file drop |
| ~~5~~ | [[#File explorer drag-and-drop and inline rename\|Explorer drag and drop]] | ~~50 min~~ **done** | run over CDP on 2026-10-25 (`explorer-dnd.mjs`), with `Input.setInterceptDrags` for the drags and F2 for the rename |
| ~~6~~ | [[#Code-block copy\|Code-block copy]] | ~~40 min~~ **done** | run over CDP on 2026-10-25 (`code-copy.mjs`): hover, real click, clipboard read back through Electron |
| ~~7~~ | [[#Heading fold: the human interaction pass\|Heading fold]] | ~~55 min~~ **done** | run over CDP on 2026-10-25 (`heading-fold.mjs`), the cold restart included |
| ~~8~~ | [[#Raw HTML in reading view — human pass\|Raw HTML]] | ~~55 min~~ **done** | run over CDP on 2026-10-25 (`raw-html-remote.mjs`): real anchor clicks, guarded so the frame cannot navigate |
| ~~9~~ | [[#Graph view (`.obsidian/graph.json` write-back)\|graph.json write-back]] | ~~50 min~~ **done** | run over CDP on 2026-10-25 (`graph-writeback.mjs`): every section expanded, every control driven |
| ~~10~~ | [[#Remote images — human pass\|Remote images]] | ~~50 min~~ **done** | run over CDP on 2026-10-25 (`raw-html-remote.mjs`), measured rather than eyeballed |
| ~~11~~ | [[#Bases: the format, the write-back and the expression language\|Bases]] | ~~2 h~~ **done** | run over CDP on 2026-10-25 (`bases.mjs`, `bases-expr.mjs`): phase-4 groundwork — nothing about Bases had ever been observed |

## How to record a result

Write the answer back into the matching `## Observed ...` section of [[Specification]]
and delete the line it settles from that section's `**Unprobed:**` paragraph. An
observation that is not written down did not happen. Where a step asks for bytes, read
the file (`xxd`, `cat -A`), not the screen.

### Auto-pair interaction

Settles the Unprobed list under "Observed auto-pair behavior, Obsidian 1.14.4": skip-over provenance, skip-over for closers other than `)`, cursor round trips and abandoned closers, `_` word boundaries, which half of the markdown rule fails, context suppression, whether the source-mode rows were really source mode, typed-vs-planted `**`, the `[[` popup and Escape, fast-typing undo grouping, and the factory defaults.

Throughout: type with the keyboard, never paste, except where a step says "paste" (paste is the only way a person can plant a character the editor did not insert). Clear `Probe Autopair.md` with Ctrl+A Backspace between rows. Where a step asks for file bytes, press Ctrl+S and read the file in the terminal with `xxd`, not the editor pane.

1. Before opening Settings at all, run `cat .obsidian/app.json` in the new vault. Then open Settings → Editor and look at "Auto pair brackets" and "Auto pair Markdown syntax"; close Settings with Escape, touching nothing, and `cat .obsidian/app.json` again.
   - Record: the exact bytes of `app.json` both times (or "file does not exist"), and whether each of the two toggles showed on or off. This is the only reading that says whether `true`/`true` is Obsidian's factory default or just what the probe vault happened to hold, and whether merely opening the Editor pane writes the keys.

2. In `Probe Autopair.md`, paste the two characters `()` from the external clipboard (Ctrl+V — do not type them), click between them so the cursor is at ch 1, and type `)`.
   - Record: the resulting line verbatim (`()` or `())`) and the cursor column. If it skips, skip-over keys off an unmatched opener, not off provenance, and Silica needs no bookkeeping of auto-inserted closers at all. Then repeat the same paste-and-type with `[]`, `{}` and `""` to confirm the answer is not paren-specific, recording each line.

3. On fresh empty lines, type the opener and then immediately the matching closer, for `[` `]`, `{` `}`, `"` `"`, `'` `'`, `` ` `` `` ` ``, `*` `*`, `_` `_`.
   - Record: for each pair, the line after the second keystroke and the cursor column — `[]` with cursor 2 (skipped) versus `[]]` (inserted). The probe only ever tested `)`.

4. Type `seed` then `(` at the end of the line (giving `seed()`), press End, type ` tail`, click back between the parens, then press Up/Down to another line and back. Press Ctrl+S, then switch to Reading view and back to editing, close the tab and reopen the note.
   - Record: the line verbatim at each of those three moments, and the `xxd` output of the file after Ctrl+S. This settles both whether the auto-inserted pair survives a cursor round trip and whether an abandoned closer is ever removed — if `()` is still on disk, nothing reaps it.

5. On an empty line type `*` once (giving `**`, cursor at ch 1) and press Backspace; record. Clear the line, type `*` twice (cursor at ch 2) and press Backspace; record. Clear again, paste `**` from the clipboard, click to ch 1, press Backspace; record.
   - Record: the three resulting lines. The probe could not say whether its `**` was typed or planted, so this is the markdown half of "Backspace between an empty pair deletes both".

6. In `Probe Autopair.md`, build the five probe positions one line at a time and type `_` at each: `word` with cursor at ch 0; `dont` with cursor at ch 3; `word` with cursor at ch 4 (end of line); `say ` with cursor at ch 4 (end of line); `()` with cursor at ch 1. Then add two new positions and type both `*` and `_` at each: `say  word` with the cursor between the two spaces (space before, space and a word ahead, not end of line), and `say x` with the cursor at ch 4 (space before, word char ahead).
   - Record: each resulting line verbatim. The first five give `_`'s word-boundary row; the last two say which half of the markdown rule actually fails — if `*` closes at `say | word` then the rule is "preceding whitespace", and the end-of-line part of the current spec sentence is wrong.
   - Leaving out: word-boundary rows for `~`, `=`, `$`, `%` and `<`. They close in no position under any toggle, so a boundary rule cannot change their outcome; only the selection-wrap behavior distinguishes them and that is already settled.

7. Open `Probe Autopair Code.md`, press Ctrl+P and run "Toggle Live Preview/Source mode", and confirm by eye that the raw markers (` ``` `, `$$`, `> `) are visible as plain text. In this mode, at each of these cursor positions type a space and then `(`, note the result, undo twice, then type a space and `*`, note the result, undo twice: end of `para text here` (the control); just before the closing backtick of `` `inline code` ``; end of the fence body `let x = 1`; end of the math body `x = 1`; end of `> quote text`.
   - Record: the ten resulting lines verbatim, plus a yes/no that the raw markers were visible. Typing a space first makes every position "after a space", which is the shape where both brackets and markdown close in a plain paragraph — so any row that does not close is the context suppressing it. This is also the only reading that confirms a source-mode row really ran in source mode; the probe's source rows were byte-identical to its Live Preview rows with nothing reading the mode back.

8. Still in `Probe Autopair Code.md`, run "Toggle Live Preview/Source mode" again to return to Live Preview and repeat just the two inline-code rows and the two blockquote rows from step 7.
   - Record: the four lines verbatim, and whether the cursor visibly landed where you clicked (Live Preview can push a cursor out of a widget-replaced range). Any difference from step 7 is a mode difference, which the spec currently cannot claim either way.

9. In `Probe Autopair.md`, type `[` twice at normal speed and look at the popup. Press Escape, then look again. Clear the line, type `[` twice again, and type `]`.
   - Record: the document after the second `[` and the cursor column; whether the file suggester was open; whether Escape closed the popup and what the line read after Escape; and the line and cursor after typing `]` inside `[[|]]`. Also record whether holding the two `[` presses to a deliberate half-second changes anything — the probe left 600 ms between keys and never typed `[[` at human speed.

10. Type `seed`, then type `(abc` as fast as you can without pausing, and press Ctrl+Z once; record the line. Press Ctrl+Z again; record the line. Repeat the whole row with a deliberate one-second pause after the `(`.
   - Record: the line after each undo in both runs. The probe's 600 ms gaps put every keystroke in its own CodeMirror history group, so its "one undo removes the opener and its auto closer together" says nothing about real typing.

11. Settings → Editor → Advanced → turn on "Vim key bindings", reopen `Probe Autopair.md`, press `i` to enter insert mode, type `say ` then `(` at the end of the line, press Escape, and `cat .obsidian/app.json`.
    - Record: the line after `(`, the line after Escape (does the closer survive leaving insert mode), and the `vimMode` value in `app.json`. Turn the setting back off afterwards.
    - Leaving out: mobile and IME-composition behavior. Mobile needs an Android/iOS device with the vault synced, and IME needs a composing input method installed and switched to, so neither is a step a person can run at this desk; both stay unprobed.

### Search pane

Settles what the 2026-10-18 plugin probe could not reach without real focus: the sort dropdown's option labels and the order each produces, where a changed sort order is persisted, what the copy-results control writes, what the result-count more-options button opens, and the real contents of the pane's menus.

Create the fixture notes listed above in a disposable vault first, then open it in Obsidian 1.14.4 and run `Search: Search in all files` (default `Ctrl+Shift+F`). Type `needle` into the search box by hand. Keep a plain text file open for the observations.

1. Click the **Search settings** icon (the sliders glyph, `lucide-sliders-horizontal`, to the right of the search box) to expand the settings section under the input.
   - Record: every control in the section, top to bottom, with its exact visible label — the sort dropdown's own label (if any) and each toggle's label (expect four: match case, collapse results, show more context, explain search; write what the UI actually calls them).

2. Click the sort dropdown and leave it open.
   - Record: every option, verbatim and in menu order (e.g. "File name (A to Z)"), and which one is marked selected on a vault that never changed it. This is the list of legal `sortOrder` values from the user's side; step 4 gets their stored spellings.

3. Select each sort option in turn. After each, read the result list top to bottom.
   - Record, per option: the file order as shown, including where `Apple Pie.md`, `apple2.md`, `apple10.md`, `_needle.md`, `9 needle.md`, `notes/apple.md` and `plain.txt` land; whether `plain.txt` appears at all; and for the two basename ties (`apple.md` vs `notes/apple.md`) which comes first. Also note whether rows reset to expanded and whether the `N results` count flickers/recomputes when you switch — that is the only visible tell for whether `setSortOrder` re-renders or re-runs the query.

4. Leave the sort set to a non-default option (pick the modified-time one), then quit Obsidian completely (`Ctrl+Q`, not just close the window) so `workspace.json` is flushed.
   - Record: `cat .obsidian/app.json` and `cat .obsidian/workspace.json` bytes — specifically whether any `sortOrder` / `searchSortOrder` key appeared in `app.json`, and the full `state` object of the search leaf in `workspace.json` (the exact string stored for the option you chose). Also run `strings ~/.config/obsidian/Local\ Storage/leveldb/* | grep -i -e recent-searches -e sort` and write down any hit. Then reopen the vault and record whether the sort order survived the restart.

5. Click the **result count** control at the top left of the results (`7 results` with the three-dot `more-options` glyph).
   - Record: whether a menu opens at all; if it does, every item verbatim in displayed order, with any separator positions. If an item copies the results, that is the control for `onCopyResultsClick` — say so.

6. Invoke the copy-results control found in step 5 (or, if there is none there, look in the settings section from step 1 and in the pane's tab-header menu, and record where it actually lives). Then create a new note `paste.md`, paste with `Ctrl+V`, save, and quit Obsidian.
   - Record: the bytes of `paste.md` from the shell (`cat -A paste.md`) — whether files are written as path or basename, whether `notes/banana.md` keeps its folder, the per-file heading form, the indentation (tabs vs spaces) of match lines, whether one line is emitted per match or per file, and the line ending. The file's bytes are the answer, not the screen.

7. With each of the four toggles from step 1 off, turn them on one at a time and leave the others off.
   - Record, per toggle: what visibly changes in the result list (match-case: does `Needle` vs `needle` change the hit set — type `Needle` to check; collapse results: do rows lose their match lines; show more context: how many extra lines appear around a match and whether they are trimmed; explain search: what the explanation block renders). Then quit and record the four flag values in `.obsidian/workspace.json`'s search leaf state.

8. Click into the search input and drive the keyboard only: press `Down` repeatedly, then `Up`, then `Right` and `Left` on a focused file row, then `Enter`.
   - Record: what the focus ring moves over (file rows only, or file rows and match lines), what `Right`/`Left` do to a focused file row (expand/collapse?), what `Enter` opens and in which pane/tab, and whether `Ctrl+Enter`/`Shift+Enter` differ.

9. Right-click, in order: (a) a result's file title, (b) a match line under it, (c) the empty space below the last result, (d) the Search tab header at the top of the sidebar, (e) the sidebar's own header area above the search box. Then open the Files view and right-click `apple.md` there.
   - Record: for each surface, the menu's items verbatim in displayed order with separators (or "no menu"), and whether the result-row menu for `apple.md` is item-for-item identical to the Files view menu for the same file. The probe's synthetic clicks saw a 13-item file-title menu, a 12-item match menu, and an implausibly thin one-item tab-header menu — this step is what makes those real.

**Not covered here, deliberately:** whether `setSortOrder` re-renders existing rows or re-runs the query is only approximated by step 3's flicker note — settling it properly needs a devtools/profiler pass, not a click; and the `nav-header` surface the probe never found is folded into step 9(e) rather than given its own step, since it may simply not exist in this layout.

### Properties in reading view — hands-on pass

Settles the whole editing path the plugin probe could not reach: whether clicking a property value opens a real field, what reaching disk looks like byte for byte, what Enter/Tab/Escape/Ctrl+Z do inside one, what Add property / remove / type-change / rename write, and which of these touch `.obsidian/app.json` and `.obsidian/types.json`.

Create the fixture notes listed above in a throwaway vault (`~/scratch/props-hand`), launch Obsidian on it, enable the Properties core plugin (Settings → Core plugins), and open Note A in **reading mode** (Ctrl+E until the rendered properties table is showing, not the YAML).

1. **Record the clean baseline.** Before touching anything in the app, in the terminal run `cat .obsidian/app.json; echo; ls -l .obsidian/types.json; cat "Note A.md"`.
   - Write down: app.json's exact bytes, whether types.json exists at all, and Note A's frontmatter verbatim (this is the diff base for every later step).
2. **Reading-view text edit → disk.** Click directly on the text `alpha-note` (the `zebra` row's value) in the rendered properties table of Note A. Type ` edited`, then click on the note body below the table. Wait 5 seconds, then `cat "Note A.md"` in the terminal.
   - Write down: whether a caret actually appeared in the value (yes/no), and the file's frontmatter verbatim afterwards — specifically whether the line is `zebra: alpha-note edited`, whether any other line was re-quoted or re-indented, whether `link: "[[Note B]]"` kept its double quotes, whether key order changed, and whether a trailing newline was added or lost.
3. **Enter / Tab / Escape / Ctrl+Z inside an open field.** Still in Note A's reading view, click the `long value` text value; press Enter — note what happens; click the `zebra` value, type `zzz`, press Escape, `cat "Note A.md"`; click it again, type `qqq`, press Tab; then press Ctrl+Z twice and `cat "Note A.md"` again.
   - Write down, one line each: whether Enter inserts a newline in the value or commits and leaves the field; where focus lands after Tab (next row's value, the key input, or out of the table); whether Escape reverted the typed text on screen AND in the bytes; and what the bytes are after Ctrl+Z — did undo in a property reach the file, or only the body editor.
4. **Non-text widgets.** In the same table: click the `count` number field and type `9`; click the `done` checkbox; click the `due` date field and pick a different day; click inside the `tags` pill area, type `three`, press Enter; then click the `x` on the `one` pill. Click the body, wait 5 seconds, `cat "Note A.md"`.
   - Write down the full frontmatter afterwards: how the date is serialized (`due: 2026-10-20`, quoted or not), how the boolean is written (`true`/`false`, any case change), whether `tags` stayed a block list (`- one`) or collapsed to `[one, three]`, and where the new tag was appended.
5. **Add property.** Click "Add property" at the bottom of Note A's rendered table. Record the UI that appears, type `demo`, press Enter, then type `hello` and click away. Then do it a second time but leave the value empty and click away. `cat "Note A.md"` after each.
   - Write down: whether a key-suggestion dropdown opened and the first five suggestions in order; where `demo: hello` was inserted (end of the block or elsewhere); and for the empty one, whether an empty key/value reaches the file at all, and if so whether it writes `key:`, `key: ""`, or nothing.
6. **Remove and type change (and the daily-note icon).** Right-click the `other`-style row — use the `alpha` row in Note A — and record the context menu; pick the remove item. Then click the type **icon** (the small glyph left of the key name) on the `zebra` row and record the dropdown; choose "Number". Finally open `Past Date.md` in reading mode and look at its `due` row.
   - Write down: the context menu's items in order and whether the menu differs between the reading-view table and the File properties sidebar pane; Note A's frontmatter after the removal (is the key gone, is anything else reformatted); the type dropdown's items in order; what `zebra: alpha-note edited` became once retyped as Number (dropped, quoted, emptied?); the exact bytes of `.obsidian/types.json` after this (`cat .obsidian/types.json` — it did not exist before, so this is the file's first observed shape); and whether `Past Date.md`'s 2020 date row shows the "Open daily note" link icon or not.
7. **Folding.** Click the "Properties" heading's collapse triangle in Note A's reading view. Then Ctrl+P → run `Toggle fold properties in current file`. Switch to another note and back, then quit and relaunch Obsidian with Note A open.
   - Write down: whether the triangle click actually hides the rows; whether the command does the same thing or something different (e.g. acts only in the editor); whether the folded state survives a note switch and an app restart; and whether any file under `.obsidian/` changed — `cat .obsidian/app.json` and `ls -l .obsidian/` 10 seconds after folding (the probe showed this write is deferred ~5 s).
8. **`propertiesInDocument` modes, from the real dropdown.** Settings → Editor → "Properties in document": set it to **Source**, close settings, look at Note A in reading mode; set it to **Hidden**, look again; set it back to **Visible**. After each change wait 10 seconds and `cat .obsidian/app.json`.
   - Write down: what reading view shows under Source (rendered table, raw YAML, or nothing) and under Hidden; app.json's exact bytes after each of the three (key name, value spelling, indentation, key order relative to any other key, trailing newline); and — optional, Ctrl+Shift+I → Elements — whether `div.markdown-preview-view` keeps the `show-properties` class in all three modes.
9. **All properties: rename and sort.** Ctrl+P → `Properties view: Show all properties`. Right-click the `STATUS` row and record the menu; rename it to `state`. Then `cat "Note A.md" "Note B.md" "Note C.md"`. Click the sort icon (lucide-sort-asc) in that pane's header and choose "Property name (A to Z)".
   - Write down: the row's context menu items in order; whether a confirmation dialog or a scope choice appeared (the view has `propertyRenameScope` fields — say which wording it used); the resulting key line in each of the three notes (did `Status:`, `status:` and `STATUS:` all become `state:`, and did the casing differ per file); the pane's row order after the A-to-Z sort, including where `cssclasses` lands; whether `Dotted.md`'s `book.title` / `book.year` appear as two flat rows or as a nested `book` tree item; and whether the sort choice is persisted anywhere under `.obsidian/` (diff the folder before/after).
10. **Pane placement and the leftover file kinds.** Close the All properties leaf entirely (right-click its tab → Close), then Ctrl+P → `Properties view: Show all properties`. Then click `Plain.txt` in the file explorer, and click the "Show search filter" (magnifier) icon in the All properties header.
    - Write down: which split the recreated all-properties leaf opened in (left sidebar, right sidebar, main) and its pixel width from the sidebar edge; what happens when `Plain.txt` is clicked (opens in a tab, opens in the system editor, or nothing) and what the File properties pane shows while it is selected — "No properties found.", "Add property", or empty; and whether the magnifier puts a focused text field inside the pane (type `zeb` and record whether the list filters as you type).

**Left out deliberately.** Pane geometry and row heights (the earlier numbers came from a theme-customized vault; settling them needs devtools measurement on a default theme, not clicking, and nothing in the editor model depends on them); the exact query string a property row hands the search box (already read back as `["tags"]`); whether the frequency tie-break is case-sensitive and whether keys other than `cssclasses` can list at count 0 (cheap to re-probe automatically with a crafted fixture, and no implementation decision hangs on it).

### Attachment paste

Settles what the 2026-10-18 plugin probe could not reach without real focus: the attachment-folder dropdown's own labels and stored values, whether a paste creates a missing folder, what a real OS file-manager copy or drag does to a file that already has a name and extension, how non-ASCII and `#&()` are encoded in a markdown link, and whether undo ever takes the attachment back.

Create the fixtures above first. After every paste, read the result in the terminal (`ls -R`, `cat -A Target.md`), not in the editor.

1. **Record the build and the untouched baseline.** Settings → About; then in the terminal `cat .obsidian/app.json`.
   - Write down: the version and installer version strings verbatim (the probe recorded `appVersion: null`, so this is the only build id we get), and the full bytes of `app.json` before anything is changed — including whether the file exists at all.

2. **Open the attachment-folder dropdown and walk every option.** Settings → Files and links → the dropdown for the default location of new attachments. Open it, then select each option in turn, and after each selection `cat .obsidian/app.json` in the terminal. For the two options that reveal a text field, type `Attachments` into it and re-read `app.json`.
   - Write down: every option label in the dropdown, verbatim and in order; and for each one the exact `attachmentFolderPath` value written to `app.json` (quote it: we specifically need whether "same folder as current file" stores `./`, `.`, or something else, and whether the subfolder option stores `./Attachments`).

3. **Paste into a deep folder that does not exist.** Set the dropdown to the "folder specified below" option and the path to `att/deep/new`. Confirm with `ls` that no `att` folder exists. Click into `Target.md` between `BEFORE ` and `AFTER`, copy `photo.png` in Dolphin (Ctrl+C on the file), and press Ctrl+V in the editor.
   - Write down: whether `att/deep/new/` now exists and holds the file (`ls -R att`), the file's name verbatim, whether any toast/Notice appeared in the bottom-right corner (and its text), and the resulting line from `cat -A Target.md`.

4. **Paste from a note in a subfolder with "same folder as current file".** Set the dropdown to that option. Open `Sub/Nested.md`, put the cursor after the `x`, Ctrl+V the same clipboard image.
   - Write down: the full path the file landed at (`Sub/...` or vault root), and the exact inserted line from `cat -A Sub/Nested.md`.

5. **Copy real named files from the file manager, one per format.** Set the attachment folder back to the vault root option. In `Target.md`, on its own fresh line each time, paste a Dolphin Ctrl+C of: `photo.png`, `shot.jpeg`, `vec.svg`, `pic.bmp`, `doc.pdf` — pressing Enter between pastes.
   - Write down, as a five-row table: for each one, the file name that appeared in the vault verbatim (kept as `photo.png`, or renamed to `Pasted image <stamp>.png`), the extension chosen (`.jpeg` vs `.jpg` matters), the exact inserted text, and for any that produced nothing: what went into the note instead and whether a Notice appeared.

6. **Collide across folders.** Copy `photo.png` into the vault root by hand (`cp ~/scratch/paste-fixtures/photo.png .`), set the attachment folder to `Attachments`, and paste `photo.png` from Dolphin into `Target.md`.
   - Write down: the path written — `Attachments/photo.png` or `Attachments/photo 1.png` — i.e. whether the collision check looks at the target folder only or the whole vault.

7. **Markdown links, absolute format, and ugly characters.** In Settings → Files and links turn the wikilinks toggle OFF and set the link format to the absolute option (write down both labels verbatim). Keep the attachment folder at `Attachments`. Open `Sub/Nested.md` and paste `a#b&c (1) café.png` from Dolphin.
   - Write down: the file name as it landed on disk (`ls Attachments | cat -A`) and the inserted line as raw bytes (`xxd Sub/Nested.md | head`) — we need exactly which of space, `#`, `&`, `(`, `)` and `é` are percent-encoded, and whether the absolute form still carries no leading slash. This is the only real paste of markdown-link + absolute; the probe only had the API's answer.

8. **Drag a file in from the file manager.** Restore wikilinks ON and the shortest link format. Drag `photo.png` from Dolphin and drop it (a) in the middle of `Target.md`'s text, (b) on a folder row in the file explorer pane, (c) onto the open `Board.canvas`.
   - Write down for each drop: what drop indicator or overlay appeared, whether a file was written and where, the inserted text (or the canvas JSON diff — `cat Board.canvas`), and whether anything was refused outright.

9. **Paste into reading view and into a canvas.** With `Target.md` open, press Ctrl+E to switch to reading view and press Ctrl+V with the image on the clipboard. Then open `Board.canvas`, click empty canvas, Ctrl+V.
   - Write down: for reading view, whether any file was created and whether anything at all happened visibly; for the canvas, whether a node appeared, and the exact JSON the canvas file gained (`cat Board.canvas`) — node type, `file` path, and where the attachment was written.

10. **Undo depth and when the note hits disk.** In `Target.md` (source mode), paste once, then without clicking anything run `cat -A Target.md` in the terminal immediately and again after ~5 seconds. Then press Ctrl+Z once, check the vault; keep pressing Ctrl+Z, up to five times, checking after each.
    - Write down: whether the note's new text was on disk on the first read or only the second (this settles whether the paste itself saves); then, press by press, the note's text and whether the attachment file is still present in the vault (`ls`), i.e. whether any number of undos ever deletes or trashes it, and whether anything lands in `.trash/`.

**Left out on purpose:** whether the `Pasted image` prefix is localized or configurable — settling it means switching Obsidian's display language and restarting, and it only bites non-English installs, so it is cheap to defer. Varying `alwaysUpdateLinks` is also skipped: it governs renames, not pastes, and belongs with the file-explorer rename checklist. No `avif` fixture is included; add one to step 5 if you have an encoder to hand.

### File explorer drag-and-drop and inline rename

Settles what a synthetic `DragEvent` could not: whether F2 and Enter actually drive the in-row rename, what a rename or drop does when the name is already taken, whether a folder moves, and whether Obsidian prompts before rewriting links with `alwaysUpdateLinks` off.

Setup: build the scratch vault and fixtures listed above, open it in Obsidian 1.14.4, and keep a terminal on the vault root. Before anything else confirm Settings → Files and links → "Automatically update internal links" is **off**.

1. Click the `Dest/Keep.md` row once so the explorer, not the editor, holds focus (the row should pick up a focused look). Without clicking anywhere else, click `Notes/Beta.md` and press **F2**.
   - Record: did an in-row editor open on the `Beta` row (text box inside the row), or did the editable appear somewhere else — the inline title at the top of the note, or the tab's header title? Write down which element got the caret, and whether the whole basename was preselected. This is the one step that settles both the F2-with-real-focus question and what `workspace:edit-file-title` targets from explorer focus.
2. With that editor still open from step 1, type `Beta Renamed` and press **Enter**.
   - Record: did Enter commit? Run `ls Notes/` and write the resulting filename verbatim. Note any Notice text word for word, and whether the editor closed.
3. Press F2 on `Dest/Keep.md`, select all, type `Alpha` (which `Dest/` already holds), press Enter.
   - Record: did a modal appear (write its title, body and button labels verbatim) or did it go through silently? Then `ls Dest/` and write the exact resulting names — is it `Alpha 1.md` with a space, as the drop path produced, or something else? `cat Dest/Alpha.md` and confirm its "keep me" line is intact, i.e. nothing was overwritten.
4. Press F2 on the `cover.png` row, type `cover shot`, press Enter.
   - Record: `ls *.png` and write the filename verbatim — did the `.png` survive the commit even though the extension sat outside the editable? Note whether the `png` badge still shows on the row.
5. Right-click the `Deep` folder row. Write down every menu item in order, top to bottom, exactly as labelled. Then choose the rename item, type `Deepest`, press Enter.
   - Record: was the folder editor an in-row contenteditable like a file's, or a modal? What text was preselected? Then `find Deepest -type f` and write the full path of the leaf note, plus any Notice text.
6. Drag the `Deepest` folder row with the mouse and drop it on the `Dest` folder row (a valid, non-descendant folder — the control the probe never ran).
   - Record: the drag ghost's action line verbatim (the probe saw `Move into “Dest”` with curly quotes), whether a highlight appeared on the target, and then `find Dest -type f` — did the folder actually move, with its subtree? Note any Notice.
7. Drag `Root.md` onto the `Dest` folder row. Do not touch anything afterwards. `cat -A Linker.md` immediately, again after 60 seconds, then quit Obsidian and `cat -A Linker.md` once more.
   - Record: all three readings of the `- markdown: [Root](Root.md)` line, byte for byte — is a markdown-style link's destination rewritten at all with `useMarkdownLinks` off, and if so at which of the three readings did it change? Also record the `- wikilink:` and `- subpath:` lines, and whether any prompt or Notice appeared (the basename is unchanged here, so the expectation is none).
8. Reopen the vault and drag `Notes/Alpha.md` onto the `Dest` folder row — a move that both collides (`Dest/Alpha.md` exists) and changes the basename, with `alwaysUpdateLinks` still off.
   - Record: did a prompt appear before the link rewrite? Write its exact title, body and both button labels, and which one you clicked. Then `ls Dest/` for the resulting name, and `cat -A Linker.md` for the `- unrelated:` line's exact bytes (the forced-on run produced `- unrelated: [[Alpha 1]]`). Note the Notice text verbatim.
9. Drag `Inbox/Gamma.md` and drop it on the **file** row `Dest/Keep.md`, while `Dest/Gamma.md` already exists. Immediately after the drop, with the explorer still focused, press **Ctrl+Z**.
   - Record: the ghost's action line verbatim before you release; then `ls Dest/ Inbox/` — what name did it land under, and did `Dest/Gamma.md`'s "keep me" line survive (`cat Dest/Gamma.md`)? Then write down what Ctrl+Z did: any Notice, and whether `ls` shows the file back in `Inbox/`.
10. Ctrl+click three file rows in turn, then Shift+click a span of rows, then drag the group onto `Dest`. Before releasing, hover over three places in turn and note each: the open editor pane in the middle of the window, the gap between two file rows, and a collapsed folder's row area.
    - Record: do multiple rows show as selected under Ctrl, under Shift, or neither? The group ghost's text verbatim (does it name a count, e.g. `Move 3 files into “Dest”`)? For each of the three hover targets: whether any highlight or insertion line appeared and what the ghost's action line read. Finally `ls Dest/` — did all selected files move, or only the one you grabbed?

Left out on purpose:
- Whether `fileManager.renameFile` auto-numbers or errors on a collision — that is an API call, not a click; it needs a plugin re-run, and step 3 settles the behavior the UI actually exposes.
- Exactly when the `is-being-dragged-over` / `is-being-dragged` classes are torn down — class churn is not observable by eye; it needs a devtools instrumented re-run.
- A drop on a second vault window — needs a second vault open and blocks nothing in Silica's current explorer work.

### Code-block copy

Settles the interaction half of "Observed code-block copy behavior": whether a real hover-and-click copies the same bytes a synthetic click did, whether the write reaches the system clipboard at all, whether Tab/Enter reach the button, what the editor's `code-block-flair` does when clicked, and whether a transcluded embed or a setting changes any of it.

Create the fixtures above first (shell for the byte-exact ones, so Obsidian never normalizes them), then open `~/silica-copy-manual` in Obsidian and run these in order. Keep a terminal beside the window — several steps capture the clipboard with `wl-paste`.

1. Confirm the fixtures on disk before touching the app: `for f in ~/silica-copy-manual/*.md; do printf '%s ' "$f"; wc -c < "$f"; done` and `xxd ~/silica-copy-manual/'CRLF Code.md' | head`.
   - Record: the byte size of each fixture, and that `CRLF Code.md` shows `0d 0a` (six of them) while `Byte Probe.md` shows `0a` only.
2. Open `Code Copy.md`, switch to reading view (Ctrl+E until the pane shows rendered text), and move the real mouse over the `CASE_LANG` fence. Then click the copy button with the mouse and run `wl-paste --no-newline > /tmp/copy-capture/lang.bin; xxd /tmp/copy-capture/lang.bin`.
   - Record: whether the button only appeared on hover and where it sat (top-right inside the pre, or elsewhere); whether anything other than the icon changed (a Notice at the bottom, a tooltip, a background flash); the full hex of `lang.bin` — specifically whether it ends `3b` or `3b 0a`; and by eye whether the check icon reverted in under or over one second. (The exact revert delay between 400 and 900 ms is not settleable by a person watching — leave it bounded.)
3. Still in reading view, hover and click the copy button on the **empty** fence, then immediately `wl-paste --no-newline > /tmp/copy-capture/empty.bin; wc -c /tmp/copy-capture/empty.bin; xxd /tmp/copy-capture/empty.bin`.
   - Record: whether the capture is zero bytes (the clipboard was genuinely set to empty) or still holds `const CASE_LANG = 1;` from step 2 (the write was skipped). This is the one anomaly the probe could not explain.
4. Hover the `CASE-BYTES` fence in `Byte Probe.md` and click, then the `CRLF line one` fence in `CRLF Code.md` and click, capturing each with `wl-paste --no-newline > /tmp/copy-capture/bytes.bin` and `.../crlf.bin`, and `xxd` both. Do the same for the single fence in `Unicode Code.md` into `.../unicode.bin`, then diff it against the file's own fence body (`sed -n '/^```txt/,/^```/p' ~/silica-copy-manual/'Unicode Code.md' | sed '1d;$d' | xxd`).
   - Record: whether `bytes.bin` keeps the three trailing spaces (`20 20 20`) and the leading tab (`09`) and ends in exactly two `0a`; whether `crlf.bin` contains any `0d`; and for the unicode capture, each line's hex versus the file's — whether `&amp;` arrives as five characters or as `&`, whether the decomposed `é` stays `65 cc 81`, and whether the zero-width space (`e2 80 8b`) and the non-breaking space (`c2 a0`) survive.
5. With `Code Copy.md` still in reading view, click once on the note's body text (not a code block), press Tab repeatedly, and watch for a focus ring.
   - Record: how many Tab presses until a copy button takes focus and whether a focus ring is visible on a button that is not hovered; which block's button focuses first (if the hidden frontmatter `pre` is in the order at all — that settles whether a person can ever reach the frontmatter button); then press Enter on a focused button and `wl-paste --no-newline | xxd` to record whether Enter copied and whether Space does too.
6. Open the devtools (Ctrl+Shift+I), hover a fence so its button is visible, and inspect the button and its `pre`.
   - Record: the button's painted box from the Elements overlay (width × height in px, and its offset from the pre's top and right edges); and in the Styles pane for the `pre`, the stylesheet file and line that sets `white-space: pre-wrap` — the one rule the probe could not attribute, and the one that contradicts the "fenced code never wraps" row.
7. Open `Flair Langs.md` and switch to Live Preview (Ctrl+E). Read the small label at the top right of each fence, top to bottom, then click the one on the ```js fence with the mouse and `wl-paste --no-newline > /tmp/copy-capture/flair.bin; xxd /tmp/copy-capture/flair.bin`.
   - Record: the seven labels verbatim and in order (does ```js read `JavaScript` and ```javascript read the same, what does ```c++ and the unregistered ```silica-made-up show, does the bare fence show anything) — that settles display-name table vs one-off; and whether clicking the flair copied anything, opened a menu, selected the block, or did nothing at all, plus the clipboard hex if it changed.
8. Open `Code Copy.md` in Live Preview and scroll through it, then turn Live Preview off (Settings → Editor → Default editing mode → Source mode, or the pane's ⋮ menu → Toggle Live Preview/Source mode) and scroll through it again.
   - Record, for each mode separately: which blocks show a flair label, which show a real round copy button, and whether the callout's code block differs from a plain fence. The probe's 10-vs-9 counts were taken over a container that still held the reading-view DOM, so these two counts by eye are the real answer.
9. Open `Embed Host.md` in reading view and hover the code blocks inside the transcluded `Code Copy` content.
   - Record: whether a copy button appears on an embedded fence at all, and if it does, click one and `wl-paste --no-newline | xxd` to record whether it copies the embedded block's own body.
10. Open Settings and walk Editor, Appearance, and Core plugins, reading every toggle label.
    - Record: any setting whose label mentions code blocks, copy buttons, or line wrapping (verbatim label and its tab); then flip any you find, close Settings, and `cat ~/silica-copy-manual/.obsidian/app.json ~/silica-copy-manual/.obsidian/appearance.json` to record the exact key name and value each one writes. If no such setting exists, write down "none in Editor/Appearance/Core plugins" — that is the answer the probe's eight-key filter could not give.

Not covered, deliberately: mobile behavior beyond the two `.is-mobile` rules (no device in this setup), and the check icon's exact revert delay (needs frame capture, not a person watching).

### Heading fold: the human interaction pass

Settles what a synthetic event could not: real mouse folding, real keystrokes, the Hotkeys pane, raw Source mode, the Fold heading setting's visible affordance, a cold restart, and the fold record for a note in a subfolder.

Create the five fixtures listed above first. Keep a terminal open at the vault root and the devtools pane open in Obsidian (Ctrl+Shift+I → **Application** tab → **Local Storage** → `app://obsidian.md`, with `fold` typed in the filter box) — every "record the fold record" step means reading the value of the `…-note-fold-…` key there, verbatim, not paraphrased.

1. **Baseline, before any fold.** In the terminal run `find .obsidian -type f | sort && md5sum $(find .obsidian -type f | sort)` and `cat .obsidian/app.json`. In devtools, clear the filter and read the whole Local Storage key list.
   - Record: the `.obsidian/` file list with md5s; the exact bytes of `app.json`; the vault's app-id prefix (the hex string before `-base-font-size`); and whether any `-note-fold-` key exists yet.

2. **Fold by mouse in Live Preview, and check independence.** Open `Fold Probe.md` in Live Preview. Hover the left margin beside `## Section A` until the triangle appears and click it; then do the same on `## Section C`; then click `## Section A`'s triangle again to unfold only that one.
   - Record: the fold record value after each of the three clicks; whether a `{"from":0,"to":0}` entry is present in a vault that has never been scripted; whether unfolding Section A left Section C's range untouched; and re-run the step-1 `md5sum` command and note any `.obsidian/` file whose hash changed.

3. **The key for a note in a subfolder.** Open `Sub Folder/Fold Probe.md`, fold `## Section A` by clicking its triangle.
   - Record: the new localStorage key **verbatim**, character for character — especially how the `/` separator and any spaces appear after `-note-fold-`.

4. **Hotkeys pane and real keystrokes.** Settings → **Hotkeys**, type `fold` in the search box.
   - Record: every row listed, in order, with its command name and the key shown (or "blank") — including whether `Toggle fold on the current line`, `Fold more`, `Fold less`, `Fold all headings and lists`, `Unfold all headings and lists` and `Toggle fold properties in current file` each have a default binding. Then close Settings, put the caret on `## Section B` in `Fold Probe.md` and press each key the pane showed. Record for each: whether the section folded, and the fold record after.

5. **`Fold more`, `Fold less`, `Toggle fold properties` with real focus.** With the caret on `## Section A` and nothing folded, open the command palette (Ctrl+P) and run **Fold more**; run it twice more; then **Fold less** twice; then put the caret in the body and run **Toggle fold properties in current file** twice.
   - Record: after every single invocation, the fold record and which source lines are still visible on screen; and for the properties command, whether the frontmatter/Properties block visibly collapsed and whether any new range appeared in the record.

6. **Raw Source mode.** Settings → Editor → set **Default editing mode** to *Source mode* (or run `Toggle Live Preview/Source mode`), reopen `Fold Probe.md`.
   - Record: whether a fold triangle appears in the left margin on hover at all; whether clicking the margin folds; whether the keystroke from step 4 folds; the fold record after; and whether the collapsed line still shows a `…` placeholder. Set the mode back to Live Preview afterwards.

7. **Reading view: what is actually clickable.** Switch `Fold Probe.md` to Reading view. Click, as three separate trials on three different headings: (a) the heading's text itself, (b) the empty space to the right of the heading text, (c) the triangle that appears on hover at the heading's left.
   - Record: for each of (a), (b), (c), whether the section collapsed; whether the triangle is visible without hovering once a section is collapsed; and the fold record after the collapse (confirming or refuting the `{h, h+1}` shape).

8. **`foldHeading` off, seen by eye.** Settings → Editor → turn **Fold heading** off. Immediately `cat .obsidian/app.json` in the terminal, then switch away to another note and `cat` it again.
   - Record: both `app.json` readings (does the write land at once, or one change late?). Then, with the setting off, in both Live Preview and Reading view, hover every heading in `Fold Probe.md`: record whether any triangle, cursor change, or other affordance appears, and whether clicking the heading or its margin collapses anything. Turn the setting back on and confirm `app.json`.

9. **Cold load after a reading-view fold.** With `foldHeading` on, open `Fold Probe.md`, fold `## Section A` and `## Section B`, switch the note to **Reading view**, and leave it there. Read and write down the fold record now. Quit Obsidian completely (close the window and confirm no `obsidian` process remains: `pgrep -a obsidian`). Relaunch, and let the vault reopen on that note.
   - Record: the fold record read from localStorage on first launch, before clicking anything; whether the note opens in Reading or Live Preview and whether each section is still collapsed; then switch to Live Preview and record whether each section is still folded and at what extent (does a stored `{11,12}` reopen as the full `{11,18}`, as one line, or not folded at all?). Also re-run `md5sum` on `.obsidian/` and `grep -rl "note-fold" .obsidian/ 2>/dev/null` to record whether anything outside localStorage carries fold state across the restart.

10. **Jump into a folded section by a real click.** In `Fold Probe.md` (Live Preview) fold `## Section B`. Open `Link Source.md` and click the `[[Fold Probe#Section B]]` link.
    - Record: whether Section B is unfolded on arrival or still collapsed; where the caret/scroll lands; and the fold record immediately after the jump and again after switching to another note and back (does the jump erase the stored range, or only unfold it on screen?).

11. **Edge-case headings.** Open `Fold Edges.md` and try to fold, by clicking the margin triangle, each of: the line-0 heading, the setext heading, the heading inside the callout, the heading inside the blockquote, and `## Empty Last Section`.
    - Record: for each, whether a triangle appears at all, whether it folds, and the fold record's range; then in the terminal `xxd Fold\ Edges.md | tail -2` to confirm the file still has no trailing newline and is byte-unchanged after folding.

**Left out deliberately:** `foldIndent: false` (list-indent folding, not heading folding — it only matters once heading folds are implemented), and the origin of the `{"from":0,"to":0}` entry (step 2 records whether it appears in a clean vault, but no amount of clicking explains it; that one needs the record read against a source-level reading of CodeMirror's fold state, not a human pass).

### Raw HTML in reading view — human pass

Settles the ten questions in the **Unprobed:** paragraph of "Observed raw HTML behavior": where a raw anchor click actually goes, whether `<details>`/`<sup>`/`<sub>` render in a real note, whether an iframe and a CSS `background-image` really fetch, whether the `?<digits>` query is the file mtime, whether pre-wrap is the stock default, which basename wins when two match, which tags the sanitizer unwraps, how raw HTML behaves inside callouts/blockquotes/cells/list items, and whether opening and clicking a note rewrites its bytes.

**Fixtures:** create the scratch vault and the files listed above before starting. The last Unprobed question — the markup behind the Embeds, Anchors, Pre and Sup-and-sub sections of `Raw HTML.md` — is settled by authoring the note rather than by a step; use exactly this body so the DOM lines up with what is recorded:

```markdown
# Raw HTML probe

## Images block

<img src="pic.png" alt="blk-root-plain">
<img src="assets/deep.png" alt="blk-subpath">
<img src="deep.png" alt="blk-basename-only">
<img src="pic two.png" alt="blk-space-raw">
<img src="missing.png" alt="blk-missing">
<img src="http://127.0.0.1:8731/raw-html-blk.png" alt="blk-remote">

## Embeds for comparison

![[assets/deep.png|md-sub]]

## Anchors

anchor row: <a href="Target Note">anchor-noext</a> then <a href="Target Note.md">anchor-ext</a> then <a href="assets/deep.png">anchor-attachment</a> then <a href="Missing Target">anchor-missing</a> then <a href="#Images block">anchor-hash</a> and [[Target Note]] for comparison.

## Pre

<pre>literal **not bold** [[Target Note]] &amp; <b>tag inside pre</b>
  two leading spaces kept?
LLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLL 0123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789 endoflongline
</pre>

<pre><code>pre-code **literal** stays literal
</code></pre>

```text
fenced for comparison **literal** LLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLL 0123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789 endoflongline
```

## Sup and sub

E<sup>2</sup> and H<sub>2</sub>O in a real note.

## Details

<details><summary>Sum</summary>Body</details>
```

1. Start the HTTP server (`python3 -m http.server 8731 --directory ~/scratch/http`) and take the baseline: `cd ~/scratch/silica-html && find . -name '*.md' -o -name '*.png' | sort | xargs sha256sum > /tmp/before.txt && stat -c '%n %.3Y' pic.png assets/deep.png 'html/sibling.png' >> /tmp/before.txt`.
   - Record: paste `/tmp/before.txt` into the note, in particular the `%.3Y` mtime of `pic.png` in milliseconds.
2. Open the vault in Obsidian, then open Settings and record the config the probe never captured: Appearance → Base colour scheme, Theme, CSS snippets list; Editor → Readable line length (on/off), Font size. Then quit nothing — just read `.obsidian/appearance.json` and `.obsidian/app.json` in a terminal.
   - Record: the two JSON files verbatim (the probe vault's were `{}` and an 8-key `app.json`), plus whether the theme is the stock default. Every measurement below is only valid under this config, so it goes at the top of the write-up.
3. Open `Raw HTML.md` and press Ctrl+E until the mode line says reading view, then scroll all the way to the bottom of the pane.
   - Record: whether the "Sup and sub" paragraph renders at all and whether the 2s sit raised/lowered; whether the `<details>` block appears, and whether it starts collapsed or expanded. Click the "Sum" summary and record whether it toggles. (The probe saw `details.count` 0 and no sup/sub paragraph, but its preview was truncated — this step says which it was.)
4. In the same reading view, click each of the five raw anchors left to right: `anchor-noext`, `anchor-ext`, `anchor-attachment`, `anchor-missing`, `anchor-hash`. Return to `Raw HTML.md` after each (Ctrl+Alt+Left / back arrow) and watch for an external browser window appearing behind Obsidian.
   - Record, per anchor: did a new tab or pane open, did the system browser open (name the URL it landed on), did a notice toast appear, did a file get created for `Missing Target`, or did nothing happen. For `anchor-hash` record whether the pane scrolled to "## Images block". Also hover each anchor with Ctrl held and record whether a page-preview popup appears (raw anchors have no `data-href`, so a popup would be a surprise).
5. Drag the pane divider until the note pane is roughly 300 px wide (split the window once if needed), and compare the long line in the raw `<pre>`, the `<pre><code>`, and the fenced block.
   - Record: which of the three wrap and which scroll sideways, and whether a horizontal scrollbar appears. Then toggle Settings → Editor → Readable line length and repeat; record whether the answer changes. This is the only evidence that the recorded `white-space: pre-wrap` is stock rather than a theme artefact.
6. Open the developer tools (Ctrl+Shift+I), Elements tab, and inspect the first image in "Images block"; copy its full `src`.
   - Record: the `app://` host hash and the `?<13 digits>` query, next to `stat -c '%.3Y' pic.png` from step 1 — same number or not. Then in a terminal run `touch pic.png`, close and reopen the note, and record the new query and the new mtime. That settles whether the query is the mtime cache-buster the embed section already claims.
7. Open `html/Nested HTML.md` in reading view with devtools still open.
   - Record: for the new `nested-ambiguous` line, whether the wide red root `dup.png` or the tall green `html/dup.png` renders, and the resolved `src` path from devtools. Also confirm the seven original lines still resolve as the table in the spec says (bare `deep.png` → `assets/deep.png`, bare `pic.png` → root).
8. Open `Unwrap Cases.md` in reading view and walk the devtools Elements tree for each `### <tagname>` section.
   - Record, per tag (object, embed, form, input, button, svg, style, link, meta, video, audio): whether the element survives in the DOM with its attributes, whether it is unwrapped to its text content, whether it vanishes entirely, and whether it is visible on screen. Note specifically whether the `<style>` rule took effect (a lime outline on the page) and whether the form's input is focusable and typeable — type into it and record.
9. Open `Containers.md` in reading view.
   - Record, for each of the five positions (callout, blockquote, table cell, list item, mid-paragraph block): whether the `<b>`, the `<img>` and the `<a>` rendered as live markup or as escaped literal text; whether the mid-paragraph `<img>` line split the paragraph into separate `div.el-p` blocks or stayed inside one `<p>` (read the block classes in devtools).
10. Open `Fetch Cases.md` in reading view, wait five seconds, then look at the `python3 -m http.server` terminal; afterwards close Obsidian and re-check the bytes with `cd ~/scratch/silica-html && sha256sum -c /tmp/before.txt`.
    - Record: the exact request lines logged (`frame.html`, `frame2.html`, `san.png`, `san2.png`) and which are missing; whether the sandboxed iframe rendered content and whether the unsandboxed one did; then every line of `sha256sum -c` output that is not `OK`, plus the fresh `stat -c '%n %.3Y'` of the three images. A clean run here is the first real evidence that reading and clicking never write to the vault.

### Graph view (`.obsidian/graph.json` write-back)

Settles what the 2026-10-18 plugin probe could not: whether real clicks on filter checkboxes, sliders, the search box and color groups reach the file, what a write physically looks like on disk, whether Obsidian preserves unknown keys and reloads the file when something else rewrites it, and what a local graph does to `workspace.json`.

Create the fixture notes above first. Every "record" below means paste the literal text into the note — bytes, not a description of them.

1. **Baseline the fresh vault, then create the file.** Open `~/scratch/graph-hand` in Obsidian. Before touching anything, in the terminal run `ls -la .obsidian/`. Then press `Ctrl+G` (command `graph:open`, "Graph view: Open graph view"). Wait 3 s, then run `cat .obsidian/graph.json; echo; tail -c 1 .obsidian/graph.json | xxd`.
   - Record: the `ls` output verbatim (confirming no `graph.json` existed), the full created file verbatim, the last byte from `xxd` (the probe saw `}` with no trailing newline), and the exact `centerStrength` value. The probe's run gave `0.518713248970312`; if this fresh vault gives the same digits, it is a constant, if not, it is per-install or random — say which.

2. **Start a write watcher and leave it running for the rest of the checklist.** In the terminal: `inotifywait -m --timefmt '%T' --format '%T %e %f' -e create,modify,close_write,moved_to,moved_from,delete .obsidian/`. Leave the graph view open and touch nothing for 60 s.
   - Record: the event lines (if any) from that idle minute — this is the only way to tell whether the probe's "second write exactly 2000 ms later" was a periodic flush or interaction fallout.
   - Record, for the first interaction in step 3: the full event sequence for that single change. `MODIFY graph.json` alone means in-place rewrite; a `CREATE`/`MOVED_TO` of a temp name means atomic temp-plus-rename. Copy the lines verbatim.

3. **Click all four filter checkboxes and type in the search box.** Click the `Filters` header in the `graph-controls` panel (top left of the graph canvas) to expand it. Click each checkbox in turn — wait 3 s between clicks, and after each run `cat .obsidian/graph.json | head -8`. Then click into the search input at the top of the Filters section, type `alpha` with the keyboard, and wait 5 s without clicking away.
   - Record: the visible label text of every row in the Filters section, top to bottom, in order (the probe never saw these rows — they do not exist in the DOM while collapsed).
   - Record: which JSON key each label changed, as `label → key: from → to`. Expected candidates are `showTags`, `showAttachments`, `hideUnresolved`, `showOrphans`; note any row that changed no key at all.
   - Record: the exact `"search"` line after typing, and whether it appeared before you clicked away (typing-only) or only after focus left the box.

4. **Drag one slider slowly, watching the event stream.** Expand the `Display` section. Drag the text-fade slider (the one whose range is -3..3, step 0.1 — confirm by its label) slowly from end to end over about 3 s, then release. Watch the `inotifywait` output as you drag.
   - Record: the slider's label text, and which key changed — this confirms or kills the spec's guess that `textFadeMultiplier` owns the -3..3 slider.
   - Record: how many write events landed *during* the drag versus after release, with their timestamps. That is the `input` vs `change` question, and the real debounce.
   - Record: the written value's exact digits for a non-integer stop (e.g. `-1.3`), to confirm full round-trip precision rather than fixed decimals.
   - Record: do the same for one `Forces` slider (e.g. center force) and one more `Display` slider (node size), noting key names only.

5. **Toggle `showArrow` and re-collapse a section.** In `Display`, click the arrows toggle. Then click the `Filters` header again to re-collapse it. Wait 3 s after each.
   - Record: the `"showArrow"` line, and the toggle's label text.
   - Record: whether `collapse-filter` went `false → true` — the probe only ever exercised `true → false`, so this is the untested direction.

6. **Create, color and delete a color group.** Expand `Groups`, click `New group`, type `path:Notes` into the group's query box, click the color swatch and pick any non-default color, wait 3 s and `cat .obsidian/graph.json`. Then delete the group with its `x` and `cat` again.
   - Record: the whole `"colorGroups"` value verbatim, including the key names inside each entry and the exact color encoding (`"rgb(...)"`, a number, a hex string — copy what is there). The spec has only `[]` and no element shape at all; this is the biggest unknown in the file.
   - Record: whether deleting the group returns the value to exactly `[]` or leaves a residue.

7. **Local graph: does it touch anything?** Open `Alpha.md`, then `Ctrl+P` → "Graph view: Open local graph" (`graph:open-local`, no default hotkey). Copy `graph.json` aside (`cp .obsidian/graph.json /tmp/g-before`). In the local graph's own controls, expand `Filters`, change the `Depth` slider and un-tick `Backlinks`. Wait 5 s, then `diff /tmp/g-before .obsidian/graph.json`. Then quit Obsidian fully (`Ctrl+Q`) and run `python3 -m json.tool .obsidian/workspace.json | grep -A30 localgraph`.
   - Record: the `diff` output (the probe saw zero writes; confirm a *real* toggle also writes nothing).
   - Record: the `localgraph` leaf's `state.options` from `workspace.json` verbatim — specifically whether `localJumps`/`localBacklinks` carry your changed values, i.e. whether local options persist there at all.

8. **Unknown-key survival.** With Obsidian still quit, hand-edit `.obsidian/graph.json`: add `"silicaUnknown": {"a": 1}` as the last key before `}`, and flip `"showArrow"` to the opposite of what it was. Relaunch Obsidian, press `Ctrl+G`, change one setting (click the `Forces` header), wait 3 s, `cat .obsidian/graph.json`.
   - Record: the full file. Is `silicaUnknown` still present, and at which position in key order? Did the hand-set `showArrow` take effect in the UI (is the toggle in the state you wrote)?
   - This is the ground-rule question — if Obsidian drops unknown keys, Silica still must not.

9. **External reload (`onExternalSettingsChange`).** Leave Obsidian running with the graph view open and the `Filters` section expanded. In the terminal, rewrite the file from outside: `python3 - <<'EOF'` reading `.obsidian/graph.json`, setting `showTags` to `true`, writing it back with the same 2-space formatting. Watch the panel for 10 s without clicking.
   - Record: did the `Tags` checkbox in the panel flip by itself, did the graph re-render, and did Obsidian write the file again afterwards (check `inotifywait`)? A yes means Silica must watch `graph.json` and reload, not just write it.

10. **Zoom bounds and pan.** With the graph view focused, scroll the wheel up until the graph stops zooming in; wait 3 s; `grep scale .obsidian/graph.json`. Then scroll down until it stops zooming out; wait 3 s; `grep scale` again. Finally drag the empty canvas background a long way with the left mouse button, wait 5 s, and `diff` the file against a copy taken before the drag.
   - Record: the maximum and minimum `"scale"` values that reach the file (the probe saturated at exactly `8` and never tested the lower bound).
   - Record: whether a real pan changed any key — the probe's synthetic drag moved nothing, so pan persistence is still entirely open.

**Left out, deliberately:** the timelapse button (`mod-animate`, "Start timelapse animation") — it animates the existing graph and has no obvious settings key, so it costs a step and blocks nothing in Silica. The "Restore default settings" button (`mod-reset`) is also left out *as a separate step* because running it would wipe the state steps 3–6 just built; if you have time at the very end, click it, note whether a confirmation dialog appears, and record the resulting file — that gives Obsidian's true defaults, including whether `centerStrength` resets to the same long decimal.

### Remote images — human pass

Settles what the 2026-10-18 plugin probe could not: which leaf owns the `image-wrapper`, whether dragging `div.image-resize-corner` on a *remote* image writes a `|WxH` hint back into the note's bytes, how layout behaves before an image arrives, lazy loading, cross-restart caching and revalidation, `https:`, and whether any user-facing remote-content switch exists in the six setting tabs the probe never rendered.

Create the setup files first (see list). Steps are ordered highest-value first — stopping after step 3 still pays for itself.

1. **Open the note in Live Preview only, and find which leaf the wrapper belongs to.** Launch Obsidian on `~/silica-scratch/remote`, close every tab (`Ctrl+W` until the empty state), then open `Remote Images.md`; make sure the mode switcher at the top right reads *Reading view* (i.e. you are in Live Preview, not reading). Press `Ctrl+Shift+I`, in Elements right-click the `<img src=".../rv-inline.png">` and copy the outerHTML of its parent, then walk up to the nearest `div.workspace-leaf` and read its `.view-content` / `data-mode` attributes.
   - Record: the parent's exact markup (is it `<div class="image-wrapper">…<div class="image-resize-corner"></div></div>`?), the class list of the enclosing `.workspace-leaf`, and the leaf's data-mode value. One line: "image-wrapper belongs to the <editor|preview> leaf."
2. **Drag the resize corner of the remote sized image, then read the file, not the screen.** Still in Live Preview, hover `Sized: ![sized alt|200x100](…/rv-sized.png)` until the corner handle appears at the image's bottom-right, drag it left by roughly half the image width, release, and wait two seconds for the autosave. In the terminal run `cat -A "$VAULT/Remote Images.md"`.
   - Record: whether a handle appeared at all; the exact bytes of that line after the drag (e.g. `Sized: ![sized alt|117x58](http://127.0.0.1:8731/rv-sized.png)$`) — copy it verbatim including the `$`; and whether only the width or both dimensions were written.
3. **Repeat the drag on the raw-HTML image and on the internal control, in the same session.** Try the handle on `Raw html: <img src=…/rv-rawhtml.png alt="rh">` and then on `Internal control: ![[probe-pic.svg]]`; after each, `cat -A` the file again.
   - Record, per case: handle present yes/no, and the line's bytes afterwards — in particular whether the raw-HTML case rewrites the `width=`/`height=` attributes in place, leaves the HTML untouched, or converts it to markdown, and whether the internal embed gains `|WxH` inside the wikilink.
4. **Watch layout while the five-second image is pending.** Kill and restart `server.py` (clears its log and any socket reuse), then in Obsidian close and reopen `Remote Images.md`. Within the first second, in devtools select the paragraph containing `/slow` and read its height in the Computed/box-model panel; take a screenshot (Spectacle, region) of the note area. After the image lands (about 5 s) read the same paragraph's height and screenshot again.
   - Record: the pending height in px, the loaded height in px, whether the text below moved down when the image arrived (yes/no), and the two screenshot paths.
5. **Look at the 404 with your eyes.** In the same screenshots, find the `Broken four-oh-four:` paragraph.
   - Record: what is visible at the image's position — nothing, a broken-image glyph, the alt text `broken alt`, or a placeholder box — plus the measured height of that `<img>` in devtools, and whether the paragraph's following text sits flush against the preceding text.
6. **Capture the reading view's own markup for the four cases the truncated `previewHTML` cut off.** Press `Ctrl+E` to switch to Reading view, and in devtools copy the outerHTML of the rendered nodes for `/slow`, `/404`, `/rv-sized.png` and `/rv-rawhtml.png` (grab the enclosing `div.el-p` for each).
   - Record: the four outerHTML strings verbatim, and for each whether `referrerpolicy="no-referrer"` is present and whether any `image-wrapper` / `image-resize-corner` exists in reading view.
7. **Test lazy loading by scrolling.** Open `Lazy Scroll.md` (reading view), do not scroll, and immediately run `curl -s http://127.0.0.1:8731/log | python3 -m json.tool | grep path`. Then scroll to the bottom of the note and run the same command again.
   - Record: whether `/lazy-bottom.png` appears in the log *before* scrolling (it did → no lazy loading) or only after, and the timestamps of both GETs.
8. **Check `https:` with a real certificate.** Open `Https.md` in reading view with network access.
   - Record: whether the image renders, whether any dialog, notice or prompt appeared (and its exact wording if so), and any error text in the devtools Console for that URL.
9. **Restart and check cache and revalidation.** Quit Obsidian entirely (`Ctrl+Q`, confirm the process is gone), leave `server.py` running, relaunch Obsidian on the same vault, reopen `Remote Images.md`, then `curl -s http://127.0.0.1:8731/log | python3 -m json.tool`.
   - Record: how many new GETs appeared after the restart and for which paths; for each, whether it carries `If-None-Match: "v1"` or `If-Modified-Since` (quote the header line); and whether any path was served from cache with no request at all.
10. **Sweep the six unrendered setting tabs for a remote-content switch.** Press `Ctrl+,` and visit General, Appearance, Hotkeys, Keychain, Core plugins and Community plugins in that order; in each, click every dropdown open so its options are visible, and type `image`, then `remote`, then `network` into the settings search box if one is present. Then `cat "$VAULT/.obsidian/app.json"` and `cat "$VAULT/.obsidian/appearance.json"`.
   - Record: per tab, any item whose label mentions image / remote / network / privacy / offline / cache — label text verbatim, its control type, and its current value; write "none" explicitly for tabs with no such item. Also record the two config files' contents verbatim, so a later toggle can be diffed against them.

**Left out of this checklist, deliberately:** why `app://local/<abs path>` fails while `app://<36 hex>/<abs path>` loads (internal URL resolution — no click settles it; needs another plugin probe); what `CachedMetadata` records for an external image (read the cache via the plugin API, not the UI); and redirects, non-image `Content-Type`, authentication and very large images (all are server-side variants — add routes to `server.py` and re-run the automated probe, which is cheaper and more exact than a human pass).

### Bases: the format, the write-back and the expression language

Settles the whole of Bases, which nothing had ever observed: Silica cannot open a `.base` at all, `silica-formats::base` is a guess at the published syntax, and the expression language has no parser. Two scripts, run one after the other because they share one Obsidian and one CDP port — `tools/obsidian-probe/bases.mjs` for the format and the write-back, `bases-expr.mjs` for the expression language — writing [[Specification#Observed Bases write behavior, Obsidian 1.14.5 (installer build 1.13.7)]] and [[Specification#Observed Bases expression language, Obsidian 1.14.5 (installer build 1.13.7)]]. Raw runs in `results/2026-10-25-bases.json` and `results/2026-10-25-bases-expr.json`.

Both scripts are built on two things worth knowing before re-running them: `query.getSerializable()` is the entire writer, and it is readable as source from the renderer; and `mockContext.evaluateIdent(s)` is `evaluateFormula(MQ.parse(s))`, so patching `evaluateFormula` on the prototype turns the private expression parser into an oracle for arbitrary strings.

The format and write-back pass:

1. **The registrations, as source.** `app.internalPlugins.plugins.bases.instance.getRegistrations()` returns one entry per layout, and each entry's `options()` function is a declarative table of that layout's config keys with widget, range and default.
   - Record: every layout's id, display name and icon, and the `options()` source verbatim. This is where the unpublished layout keys come from, and it is cheaper and more exact than the menus.
2. **Open the scrambled fixture and read the writer.** Open it in a leaf of its own, wait six seconds, and compare bytes *and* mtime.
   - Record: whether opening wrote anything; `query.getSerializable()`, `query.unrecognizedData`, `query.toString()`, and the source of `getSerializable`, `toString`, `save`, `setViewData`, `getViewData`, `saveQuery`, each view config's `serialize`, and a property config's `serialize`.
3. **Close it with nothing edited.** Detach the leaf and watch the file.
   - Record: the bytes that come back beside the fixture, byte for byte. Everything about key order, quoting, indentation, comments, flow style and the trailing newline is in that one diff. Check the mtime too: a file already in canonical form is *not* rewritten, and bytes alone cannot tell that from an identical write.
4. **The byte-level copies.** A CRLF copy, a BOM copy, one with no trailing newline, a zero-byte file and `{}`, each opened and closed.
   - Record: size, BOM, line endings, last byte and mtime before, after the open and after the close.
5. **One real edit.** Drag `.bases-table-header-resizer` 140 px.
   - Record: the column widths either side, the key the drag wrote, and where in the view's mapping it landed relative to the unknown keys that are already there.
6. **The toolbar, which is not a menu.** Click each `.bases-toolbar-item` and dump the whole `.menu.bases-toolbar-menu` — its `.bases-toolbar-section` headers and every combobox, `<select>`, slider and `.suggestion-item` inside it.
   - Record: each panel's markup and controls. Reading `.menu-item` titles here finds nothing, which is how this was discovered.
7. **The per-view configuration page.** The views panel lists one row per view; the row's own `.bases-toolbar-menu-item-icon` opens "Configure view". Operate every row of that form — a native `<select>` set with a change event, a slider clicked three quarters along its track, a combobox's option clicked in the `.suggestion-container` that opens *outside* the menu — re-opening the page between operations and reading the file in between.
   - Record: the key each control writes and its exact value. Do the table, cards, list and kanban layouts; a view's `name` is required, so a list or kanban fixture needs one.
   - Leave the Layout combobox alone inside that loop: picking in it rewrites `type` and replaces every control after it. Probe it once, on a file of its own, and record what survives the change.
8. **The column header's own menu and the results panel.** Right-click `.bases-table-header`; then the results panel's limit input, *Copy to clipboard* and *Export CSV...*.
   - Record: the menu's rows in order; what *Sort Z → A*, *Hide column* and the limit input write; and the clipboard's exact bytes after *Copy to clipboard*.
9. **The commands.** `bases:new-file`, `bases:add-view`, `bases:insert`, `bases:copy-table`.
   - Record: the file each creates and its bytes, the text `bases:insert` puts in the note, and the clipboard after `bases:copy-table`. Click a row in the view first: without focus the command reports success and Obsidian shows "Unable to copy to your clipboard".
10. **The broken ones.** A YAML syntax error, a view with no `name`, a view with `name: ''`, an unknown key beside `and` in `filters`, an unknown view type, and an unparseable statement in a filter, a formula and a summary.
    - Record: the `.bases-error` text verbatim, and the bytes *and mtime* after the open and after the close. The question that matters is whether a file Obsidian cannot understand is ever written.
11. **An external rewrite.** With the base open, rewrite the file from the host, wait, then close it.
    - Record: whether the view reloaded, and whether the close kept the external content or clobbered it.

The expression-language pass:

12. **Install the oracles.** Patch `evaluateFormula` on `mockContext`'s prototype to capture its argument; then find a per-file context by evaluating `density` against every candidate and keeping the one that answers `1.21`. `controller.results` is a `Map` whose values are those contexts; `controller.ctx` is not one.
    - Record: every candidate and what it answered. Re-install after opening a different base — the patch is on the shared prototype but the context is not.
13. **Precedence and associativity, structurally.** Parse every pair of operators that can meet and read the tree.
    - Record: the AST of each, and the parse error of each form the language does not have (`? :`, `??`, `in`, `and`/`or`/`not`, `&`, `|`, `**`, `^`, `===`, assignment, `{}`).
14. **The lambda question.** Parse and evaluate every syntax a person would try for `filter`, `map` and `reduce`.
    - Record: which parse at all, and the value each gives on a known list. Then read the formula editor's completion text for `filter`, `map` and `reduce`, which names the magic idents outright.
15. **Literals, coercion, null and durations.** Evaluate each against the real context.
    - Record: the escapes a string accepts and the error for one it does not; which number forms parse; what every cross-type comparison and every cross-type `+` gives; what a missing property does to a call, a property access and a comparison; and the duration units, including `M` against `m`.
16. **The function table.** For each receiver type, type `<seed>.` into the formula editor and dump the completion list and each row's info panel; then brute-force a candidate name list through the four distinguishable refusals.
    - Record: per type, the functions with their parameter names and types, the properties, and Obsidian's own one-line description. The two readings must agree; where they do not, say which.
17. **The globals, and case.** The same brute force with no receiver, plus `NOW()`, `nOw()`, `file.NAME`, `DENSITY`, `listish.contains("ALPHA")` and `file.hasTag("Rock")`.
    - Record: the whole global list, and exactly which of names and values are case-insensitive.
18. **An invalid statement in each of the three places.** A filter, a formula and a summary that cannot parse, each in its own base.
    - Record: the rows that survive, the cell text, the `.bases-error`, the AST that is kept, and the bytes after the close.
19. **The rendered cells as ground truth.** A view whose columns are the test expressions themselves, read off the real table.
    - Record: the header and cell text per row, and each cell's markup — a boolean renders as a disabled checkbox, so a blank cell is not a blank value, and the table only builds the columns that fit, so the oracle carries the rest.

**Left out, deliberately:** an embedded base in a note (`![[x.base]]`) and `createAndEmbedBase`, which is a rendering question rather than a format one; `bases:change-view` and `bases:add-item`; *Summarize...*, whose submenu is not a `.menu` and needs its own pass; and the `filer-expression-mode-toggle` in the filter panel, which is the one UI path that writes a raw statement rather than a structured rule and deserves a block of its own.

### PDF viewer

Run on 2026-10-26 over CDP (`tools/obsidian-probe/pdf.mjs`, raw run in `results/2026-10-26-pdf.json`, screenshots beside it), opening [[Specification#Observed PDF view behavior, Obsidian 1.14.5 (installer build 1.13.7)]]: nothing about the PDF view had been observed before, and the roadmap item needs the leaf record, the chrome, the embed sizes and the canvas card. Fixtures are made by `typst` in the fixtures phase: a five-page A4 document with one landscape page and an outline, a one-page letter file, a 200-byte file with only a `%PDF-1.4` header, a note with eleven embeds and four links, a canvas with two file cards and a text card, and a note with the embed below the fold.

Two things to know before re-running it. pdf.js keeps a per-document record in `localStorage['pdfjs.history']`, keyed by the file's fingerprint and shared by every vault this Obsidian opens: the first pass rotated the document once and every later open — leaf, embed, canvas card — came back rotated. The script's `b` steps strip the probe's own entries first and are the pass the specification reads; `s18` strips them again on the way out. And the view's `PDFViewer` is at `leaf.view.viewer.child.pdfViewer.pdfViewer`; `view.getState()` is only `{file}`, so every page and zoom reading goes through the viewer.

1. **The leaf.** Open `doc.pdf`, read `getViewState()`, `getState()`, the tab header, and walk the view for the pdf.js viewer.
   - Record: the leaf JSON `workspace.json` writes after a scroll, a zoom and a rotation, and the `pdfjs.history` entry beside it. The leaf never carries page or zoom; the history does.
2. **The chrome.** Dump `.view-content` and the computed styles of the toolbar, each control, the pages and the sidebar in the dark theme and after `app.changeTheme('moonstone')`.
   - Record: the control order with each Lucide name, sizes, paddings, colours per theme, the `--pdf-*` custom properties, the page border and the canvas shadow.
3. **Every control from a fresh default.** Toggle sidebar, Sidebar options, Zoom out six times and Zoom in eight, Display options with each of its rows, the page input with `3`, `99`, `0`, `abc` and a blur.
   - Record: the viewer's page, scale, `currentScaleValue`, `spreadMode`, `pagesRotation` and the DOM delta after each, the menus' rows and checks, the history entry after each row. Expect `page-height` where pdf.js says `page-fit`, and expect Adapt to theme to stick across a re-open.
4. **Sidebar and find bar.** Thumbnails, Table of contents, a row click, Reveal page; `Ctrl+f`, type `three`, Enter twice, the up and down buttons, a miss, the settings panel, Escape and Close.
   - Record: the sidebar's width and views, the thumbnail box, the outline rows as Obsidian's tree items, the find bar's controls and count label, the highlight class and colour, the no-match input style, the `<appId>-pdfjs-search-settings` key.
5. **Keys and scrolling.** The key list with the viewer focused through the API and through a real click; a 900-unit wheel and a `scrollTop` set; `scrollTop` placed just above and below each page boundary.
   - Record: page, scroll top and scale per key, which keys are in `view.scope`, which Obsidian hotkeys still reach the app, and the page-number rule. `Ctrl+=` is Obsidian's window zoom, not the viewer's.
6. **Links.** `openLinkText` with `#page=4`, `2`, `99`, `0`, `-1`, `abc`, `2.7`, `#foo`, `#height=300`, `#page=3&height=200`; real clicks, Ctrl+clicks and middle clicks on the note's links with the anchor guard armed; a 1.5 s hover.
   - Record: which leaf each used, the page reached, and that a plain click navigates the note's own leaf to the PDF.
7. **Embeds.** Every embed of `Embeds.md` in reading view and Live Preview, scrolled to by line; `Tall.md` before and after scrolling in both modes; a wheel inside an embed, at its end, and the embed's page input.
   - Record: per embed the class list, `src`, `alt`, the inline height and the box, the inner viewer's page and scale, the toolbar's right-hand *Open*; the missing and broken shapes; the lazy build; that nothing reaches the note's state or `workspace.json`.
8. **Canvas.** The two file cards and the text card at zoom 0: the content class, the inline height, the toolbar's extra *Save current position in document*, the `#page=2` subpath; a wheel, a click, a double-click, *Save current position* and each row of *Open*.
   - Record: the card's markup and rect, what the wheel moves, what the double-click does, and the file after the save button (unchanged in this run).
9. **Menus, selection, panes, the broken file, settings, restart.** Right-clicks on page, text, toolbar, gap and margin; the tab header and More options menus; a real drag across the 40 pt sentence and the three copy rows with the clipboard read back; Outline, Backlinks and Outgoing links with the pdf leaf active; `small.pdf` and `broken.pdf` as leaves; the settings tabs, `app.vault.config`, the plugin lists and the view and embed registries; quit at page 3 and 150 % and read the leaf after relaunch.
   - Record: the exact clipboard bytes of *Copy*, *Copy as quote* and *Copy link to selection*; the broken file's `0 of 0` toolbar with no error; that the page comes back after a restart and the zoom does not.

**Left out, deliberately:** annotations and the annotation editor, which the viewer builds hidden; printing and `workspace:export-pdf`; a password-protected PDF; a PDF with page labels; and the *Save current position in document* button, which did nothing visible here and needs a hand pass.
