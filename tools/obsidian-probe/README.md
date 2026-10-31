# Obsidian probe harness

Drives a real Obsidian over the Chrome DevTools Protocol to observe behavior
the specification needs. CDP input events are injected by the browser, so they
carry focus and take the same path a person's keyboard and mouse do — unlike
`el.dispatchEvent`, which does not and which is why the earlier
throwaway-plugin probes could not settle interaction.

Not part of `cargo test` and **not run in CI**: these scripts assert
*Obsidian's* behavior, not Silica's. A failure means Obsidian changed or the
app flaked, which is not a reason to block a merge. Run them by hand when the
Obsidian version moves, read the diff, and update the observation tables and
`docs/silica/Specification.md` together. The CI-side counterpart is
`crates/silica-suggest/tests/obsidian_rows.rs`: Silica asserted against the
tables these runs produce.

## Running a block

```
tools/obsidian-probe/run-block.sh tools/obsidian-probe/search.mjs /tmp/search-results.json
```

`run-block.sh` creates a fresh scratch vault, registers it in
`~/.config/obsidian/obsidian.json` under a fresh id, calls the script in
phases — `fixtures` (Obsidian not running: write the notes), `run` (Obsidian
up), and `after-restart` if `run` ended with `c.quit()` — and restores
`obsidian.json` byte-identical on exit. It refuses to start while an Obsidian
is already running. Never point it at a real vault: not the gate vault
(`~/bstore2/raga/plmcrzy/PLMCRZY`) while a byte-level gate run is open, and
not `docs/silica`. Scripts get `VAULT` and `RESULTS` in the environment.

Scripts, each one checklist block of `docs/silica/Interaction probes.md`:

| Script | Block | Phases |
| --- | --- | --- |
| `search.mjs` | the search pane: sort orders, persistence, menus, toggles, keyboard | fixtures, run, after-restart |
| `search-copy-modal.mjs` | the "Copy search results" modal and its 24 format variants; supersedes `search.mjs` step 09 | fixtures, run |
| `search-shots.mjs` | the search pane's look: `Page.captureScreenshot` PNGs of the pane with results, a hovered match, a keyboard-focused row, a collapsed row, the settings section, Explain search terms, Show more context, Collapse results, Match case, the count menu and the light theme (`SHOTS` names the folder), plus the computed styles and markup of every part | fixtures, run |
| `properties.mjs` | properties in the reading view: the editing path, Add property, remove, type change, fold, propertiesInDocument, All properties | fixtures, run, after-restart |
| `canvas.mjs` | canvas write-back: what an edited save, an unedited close and `canvas:new-file` write; key order per node and edge type, the formatter's one-line rule, unknown keys, group ordering | fixtures, run |
| `splits-bookmarks.mjs` | tab groups: what "Split right" and "Split down" put in `workspace.json`, the `dimension` a dragged divider writes, the four focus commands, closing a group and its last tab, "Open link under cursor to the right", restart fidelity; and the Bookmarks plugin: every item type's bytes, each command's modal, the block id `bookmark-current-section` writes, the pane's header and rows | fixtures, run, after-restart |
| `frame-typography.mjs` | the shell's chrome in the hidden frame: the title row, its window buttons and sidebar toggles, the ribbon, every header's height, the Lucide icon each control names, the computed font sizes, weights and colours of the shell and of a note, and what dragging a sidebar's handle and toggling a sidebar write to `workspace.json` | fixtures, run |
| `frame-settings.mjs` | the ribbon's own classes and icons, the dark and light palettes as computed custom properties | fixtures, run |
| `frame-setting-key.mjs` | the "Window frame style" setting: 1.14 opens Settings in a popout window, so its rows are reached through `app.setting.activeTab.containerEl`; records the setting's tab, label, options and default, and that no option writes anything under `.obsidian` | fixtures, run |
| `explorer-header.mjs` | the Files pane's header: its five buttons and their icons, the sort menu and what each of the six orders does to the rows and to `workspace.json`, the auto-reveal toggle with a note opened in a collapsed folder either way, the Expand all / Collapse all swap, and where the header's New note and New folder put their file | fixtures, run |
| `tab-hover.mjs` | the root tab strip's hover: which box the wash paints (the inner box, not the tab), its colour, radius and inset, what hover does to the title, the close button and the dividers of the hovered tab and its neighbours, the active tab and a dock tab hovered, in both themes, with a screenshot of the strip per state (`SHOTS` names the folder) | fixtures, run |
| `tab-list.mjs` | the strip's right end: where the new-tab, tab-list and sidebar-toggle buttons sit against the tab, the divider after the last inactive tab, and the tab-list menu: its rows, icons, checks and separators, what picking a tab, Close all, Stack tabs and Bookmark N tabs do | fixtures, run |
| `new-tab.mjs` | the new-tab button and `workspace:new-tab` with the active tab in the middle of the strip: where the tab goes, whether it is active, what its empty view offers | fixtures, run |
| `explorer-keyboard.mjs` | the Files pane's keyboard: what a click on a file, on the open file and on a folder does to focus, and Up, Down, Left, Right, Enter, Space, Home, End, Escape, F2 and Delete on file and folder rows, entering the tree through `reveal-active-file` | fixtures, run |
| `outgoing-links.mjs` | the Outgoing links pane: its markup and styles, one row per target with a glyph per kind, the subtext under a row, what clicking a heading row and an unresolved row do, the row menu, the unlinked mentions card with its "Link this file" chip and the bytes the chip writes, the two fold keys in `workspace.json`; and the Backlinks pane's rows, hover-only Link button and empty state for comparison | fixtures, run, after-restart |
| `dock-panes.mjs` | the right dock's Tags, All properties and Outline panes: each pane's markup, computed sizes and colours, header buttons and tooltips, the sort menu and the row order under every entry, the in-pane search filter (and how each pane treats Escape), hover, what a row click opens, the Tags pane's nested-tags toggle, counts and folds, the Outline's current-heading mark in source and reading mode and its auto-scroll toggle, the empty state, and every state key's round trip through `workspace.json` | fixtures, run, after-restart |
| `editing-commands.mjs` | the editing commands the backlog waited on: the bare `set-heading`, `cycle-list-checklist`, `toggle-checklist-status`, the six insert-block commands, the highlight colours, `clear-formatting`, Move line's hotkey, the formatting toggles across a multi-line selection, `insert-table` on a non-blank line | fixtures, run |
| `editing-typing.mjs` | the same block's keystroke rows: auto-pairing's open questions (skip-over per closer, the word-boundary rule, context suppression, forward Delete, `[[`), and Shift+Tab on ragged indentation | fixtures, run |
| `footnote-insert.mjs` | `editor:insert-footnote`'s number and where the definition lands, read from disk with the fixture written before launch — one case per run (`FN_CASE`), because the command only lands once per launch when the probe reaches the file with `openFile` | fixtures, run |
| `ui-rules.mjs` | the UI rows: the empty tab, the lucide icon per callout type, the callout fold chevron, the setext underline, the done-task decoration, the view-header title, the property pills, clearing every property, `graph:open-local` with no file, the local graph's camera, a half-typed query, Ctrl+E in a field, the explorer's context menus and the modal backdrop | fixtures, run |
| `attachment-dropdown.mjs` | Settings → Files and links: the attachment-location dropdown's option labels, the string each stores, the dependent rows, and what each stored value resolves to — on its own because the Settings modal has hung this endpoint before | fixtures, run |
| `search-collation.mjs` | the basename tie in the search pane's sort: several same-basename files at different depths, under every sort order, plus the quick switcher's own order | fixtures, run |
| `code-copy.mjs` | block 6: hover, real click, clipboard read back; trailing newlines, unicode and CRLF; the icon's revert delay; Tab; the editor's `code-block-flair`; an embedded block; the frontmatter pre | fixtures, run |
| `heading-fold.mjs` | block 7: the subfolder key, what `{0,0}` is, fold-more/fold-less, source mode, `foldIndent: false`, the reading-view clicks, the edge headings, the link jump, and the cold restart | fixtures, run, after-restart |
| `explorer-dnd.mjs` | block 5: real drags through `Input.setInterceptDrags`, the drag payload, the drop-zone classes, collisions, folders, multi-select, and F2's inline rename committed with Enter | fixtures, run |
| `graph-writeback.mjs` | block 9: every control section expanded, every toggle clicked and slider dragged, the write timing polled, the zoom clamps, unknown keys, an external rewrite, and the local graph's own options | fixtures, run |
| `attachment-paste.mjs` | block 4: a real clipboard image through `nativeImage`, folder creation, `./`, Markdown links, reading view and a canvas, the disk timeline and undo depth, and a real file drop (`FIXTURES` names a directory of source files) | fixtures, run |
| `canvas-menus.mjs` | the canvas's menus for #48: the card menu, the floating row per kind of selection, the right-click list per verified target (card, Markdown and image file cards, URL card, group label and body, edge on and off its label, background, several objects), the Set color, Line direction and Align submenus, Align and Arrange applied so the geometry is recorded, the quick settings and where they persist, Read-only, and what Convert to file writes | fixtures, run |
| `raw-html-remote.mjs` | blocks 8 and 10: real anchor clicks (guarded so the frame cannot navigate), which tags survive, the iframe and CSS-background fetches, raw HTML in every container, and the remote-image layout — runs its own HTTP server (`HTTP_PORT`) | fixtures, run |
| `bases.mjs` | Bases: the `.base` format and its write-back — whether opening or closing writes, what one real edit writes, key order at every level, the YAML emitter's quoting and indentation, comments, trailing newline, CRLF and BOM, where a new key lands, the unpublished layout keys of all four layouts (driven through "Configure view"), `bases:new-file`, `bases:insert` and `bases:copy-table`, unknown keys at every level, the five ways a base can be broken, and an external rewrite | fixtures, run |
| `bases-expr.mjs` | Bases: the expression language — precedence and associativity read off the parse tree (the private parser is turned into an oracle by patching `mockContext.evaluateFormula`), the lambda form of `list.filter`/`map`/`reduce`, literals and escapes, coercion, null propagation, durations, every method reachable from a value, the global functions, `this`, and an invalid statement in a filter, a formula and a summary | fixtures, run |
| `pdf.mjs` | the PDF view: where pdf.js's `PDFViewer` hangs off the leaf, the leaf's `workspace.json` record against the `pdfjs.history` localStorage entry that really carries page, zoom, rotation and sidebar, the toolbar, sidebar, find bar and Display options menu with their styles in both themes, the zoom ladder, keys, the page-number rule, `#page=`/`#height=` on links and embeds, every embed of `Embeds.md` in reading view and Live Preview, the canvas file card, menus, the three copy strings, the panes, `broken.pdf`, and restart fidelity; the `b` steps are the clean pass after the first pass polluted the history with a rotation (`SHOTS` names the folder) | fixtures, run, after-restart |
| `vault-switcher.mjs` | the left sidebar's vault profile and the switcher it opens: the footer's parts, the menu's rows against a registry holding two vaults that share a folder name and one whose folder is gone, the check on the open vault, the order the rows come in, and what the row under the separator opens; writes `vault-switcher-menu.png` to `SHOTS` | fixtures, run |
| `vault-manager.mjs` | the vault-manager window "Manage vaults..." raises (`starter.html`, a CDP target of its own with no `app` in it): the list's rows and their order against the registry on disk, the ellipsis menu, the panel on the right, and what `app:open-vault` and `app:switch-vault` each do; writes `vault-manager.png` and `vault-manager-row-menu.png` to `SHOTS` | fixtures, run |
| `autopair.mjs` | auto-pairing (predates the runner: start Obsidian yourself with `--remote-debugging-port=9222` on a scratch vault and run it directly) | — |

`results/` keeps the raw JSON of the runs the specification cites, named by
date. A run is evidence; the interpretation of it lives in
`docs/silica/Specification.md`, and the two should be read together.

Most scripts take `ONLY_STEPS=s03,s04` to re-run a step or two, which is how a
late step gets fixed without paying for the whole block again.

## cdp.mjs

No dependencies — Node 24 has a built-in `WebSocket`. `connect()` waits for a
page target whose `app.workspace.layoutReady` is true (the endpoint answers
before the vault window boots, and the first page target can be the launcher)
and returns:

| | |
| --- | --- |
| `evaluate(body)` | run a function body in the renderer, `app` in scope, returns its value |
| `typeText`, `typeChar`, `pressKey(name, {ctrl,shift,alt,meta})` | real keystrokes |
| `rect`, `clickElement`, `rightClickElement`, `dblClickElement`, `hover`, `clickAt`, `drag`, `mouseMove` | real mouse |
| `dragAndDrop`, `dragOverOnly` | a real HTML5 drag, through `Input.setInterceptDrags` plus `Input.dispatchDragEvent` — `dispatchMouseEvent` never raises `dragstart`, which is why the synthetic passes could not establish a drop. Returns the intercepted payload; `dragOverOnly` hands back a `finish(x, y)` so the hover state over a target can be read before the drop, or `dragCancel`ed |
| `on`, `once` | protocol *events* by method name, which drag interception needs |
| `openMenuItems()` | every open `.menu` as labels in order, `---` for separators |
| `selectInfo`, `selectSet` | a native `<select>`'s options, and setting it with a `change` event |
| `readClipboard`, `writeClipboard` | the system clipboard via Electron |
| `vaultPath()` | which vault this window has open — check it, never assume |
| `quit()` | close the window cleanly and wait for the process to go |

## Traps

- A vault id in `obsidian.json` must be **16 hex characters**; a malformed one
  is ignored and no vault window opens. `run-block.sh` generates one.
- **Reusing an id across folders inherits that id's IndexedDB cache**: a
  search on the new vault returned files from the old one until the re-index
  caught up. Fresh id per scratch vault.
- The version question: the window title and update log say **1.14.4**, the
  user agent says `obsidian/1.13.7`. App 1.14.4 on installer build 1.13.7.
- A previous instance lingers for seconds after its window closes. The kill
  pattern must admit the `--remote-debugging-port` flag between the binary
  and the asar, or it never matches a CDP launch; the runner waits for both
  the process and the endpoint to be gone, then checks `vaultPath()` after
  connecting and relaunches once on a mismatch.
- After a click, also ask whether a `.modal-container` appeared. "Copy search
  results" opens a modal; three scripts watched the clipboard for a write
  that never comes until the modal's own button is pressed.
- Obsidian's CSP blocks the renderer's own `fetch` to localhost; read a probe
  server's log from the host.
- `cp` is aliased to `cp -i` on this machine; the runner uses `command cp -f`.
- **Seed a note through the vault, not through the editor.** `editor.setValue`
  leaves the file and the editor disagreeing, and a command that touches the
  file reloads the editor from disk — which puts the previous case's bytes back
  underneath the command's own edit. `app.vault.modify` plus a settle, then the
  caret, then a `seedOk` check that the editor really holds the seed.
- **A second markdown leaf is poison.** `executeCommandById` acts on the
  *active* leaf; a probe reading `getLeavesOfType('markdown')[0]` then sees an
  untouched document and records every command as a no-op that still reports
  it ran. Reuse one leaf and `setActiveLeaf` it.
- A file-change event landing after `setCursor` moves the caret under the
  command. Settle before placing the caret, not after.
- Clicking an explorer row opens the file and hands focus to CodeMirror, so F2
  and Ctrl+A go to the note. Enter the tree with
  `file-explorer:reveal-active-file`.
- The reading view only builds the sections near the viewport: scroll the whole
  note and merge, or a long fixture reports a fraction of its elements.
- A control below the window (the graph's Forces sliders) reports a rect a
  drag cannot reach. `scrollIntoView` first and check the rect is on screen.
- **Escape clears the canvas selection.** Pressing it to dismiss a menu before
  opening the next one deselects everything, so every "with several selected"
  case recorded afterwards is really a single-card case. Remove the `.menu`
  nodes instead (`canvas-menus.mjs`'s `dismiss`), and read the selection back
  either side of the click.
- A menu's rows are nested in 1.14: `.menu > .menu-scroll > .menu-group >
  .menu-item[data-section]`. Walking `.menu > *` sees two wrappers and nothing
  else, and a row without a `.menu-item-title` (a colour swatch row) is
  dropped by a title-only read. Keep each menu's `outerHTML` as well, so a
  dropped row can be recovered without re-running.
- Separators come in pairs at a group boundary, one painted and one
  `display: none`; record `offsetHeight` per row or the capture reads as two
  separators in a row.
- localStorage is shared across every vault the app has opened, so filter any
  dump to the run's own app id before saving it as evidence.
- **`ONLY_STEPS` must not filter the `fixtures` phase.** `run-block.sh` makes a
  fresh vault every time, so a re-run with `ONLY_STEPS` whose `step()` guard
  also applies to fixtures starts on an *empty* vault and every open misses.
  `bases.mjs` guards it with `phase === 'run'`.
- The vault's own index lags `layoutReady` by a second or two:
  `getAbstractFileByPath` can return null for a file the fixtures phase
  certainly wrote. Poll for the file instead of throwing on the first miss.
- **A second Bases leaf is poison the way a second markdown leaf is.** Every
  `.bases-toolbar-*` selector matches the *first* leaf in the document, which
  may be a stale hidden one, so the click lands in the wrong view and the step
  records nothing. `bases.mjs`'s `openBase` detaches every bases and markdown
  leaf first.
- **The Bases toolbar does not open `.menu-item` lists.** Each control opens a
  `.menu.bases-toolbar-menu` whose body is a panel — `.bases-toolbar-section`s
  of comboboxes, native `<select>`s, range sliders and `.suggestion-item` rows,
  or a `.view-config-menu` form. A probe that reads `.menu-item` titles sees an
  empty menu and reports "no such row". A combobox's options open in a
  `.suggestion-container` that is *not* inside the `.menu`.
- A command that writes the clipboard itself (`bases:copy-table`) fails with
  Obsidian's own "Unable to copy to your clipboard" notice unless the view has
  real focus: click a row first, then run the command.

See [[Development#Driving Obsidian over CDP, 2026-10-18]].
