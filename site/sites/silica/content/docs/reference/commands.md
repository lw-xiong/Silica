---
title: Commands and hotkeys
description: The commands Silica registers: the 256 Obsidian command ids, the 295 with an observed desktop name, the 41 with a default hotkey, and 292 ids Silica can actually run today (234 Obsidian + 58 `silica:`-prefixed), with every id and default key listed.
---

# Commands and hotkeys

Silica registers **313 command ids**: the **256** exact strings Obsidian 1.12.7 registers, plus
**60** `silica:`-prefixed ones. Of those, **295 have an observed desktop name**. **41** carry a
default hotkey. **292** do something in Silica today.

Those four numbers are different things and the difference is the point:

- An **id** is what your `hotkeys.json` keys on. Registering all of them is why your
  bindings survive opening the vault in Silica.
- A **name** is what the command palette and the Hotkeys tab display.
- A **default hotkey** is Obsidian's binding, applied unless your file overrides it.
- **Runnable** means Silica implements the behavior. The [command
  palette](/docs/app/switcher/) offers only these; the [Hotkeys tab](/docs/hotkeys/) lists
  all 313, so you can see your bindings for everything else.

Some of the 292 are runnable only in the right place, which is also how Obsidian treats
them: **74** need an editor on screen with the keyboard in it, so they leave the palette in
a reading leaf, and the **19 table commands** that act on an existing table are offered
only while the cursor is in one. `graph:open-local` is not offered with no file open. The palette, the hotkey and the
control socket all ask the same question, so a command that is listed is a command that
runs.

Ids come from Obsidian 1.12.7; names and default hotkeys were captured from Obsidian 1.14.4
with a throwaway plugin that dumped `app.commands` and the settings tabs.

## Commands with a default hotkey

`Ctrl` is Cmd on macOS, except where Obsidian itself registers a literal `Ctrl`, namely the
tab commands below, which are `Ctrl` everywhere.

| Command | Id | Default key | In Silica |
|---|---|---|---|
| Add file property | `markdown:add-metadata-property` | `Ctrl + ;` | Yes |
| Close current tab | `workspace:close` | `Ctrl + W` | Yes |
| Close window | `workspace:close-window` | `Ctrl + Shift + W` | Yes |
| Command palette: Open command palette | `command-palette:open` | `Ctrl + P` | Yes |
| Create new note | `file-explorer:new-file` | `Ctrl + N` | Yes |
| Create note to the right | `file-explorer:new-file-in-new-pane` | `Ctrl + Shift + N` | Yes |
| Delete paragraph | `editor:delete-paragraph` | `Ctrl + D` | Yes |
| Follow link under cursor | `editor:follow-link` | `Alt + Enter` | Yes |
| Go to last tab | `workspace:goto-last-tab` | `Ctrl + 9` | Yes |
| Go to next tab | `workspace:next-tab` | `Ctrl + Tab`, `Ctrl + Page Down` | Yes |
| Go to previous tab | `workspace:previous-tab` | `Ctrl + Shift + Tab`, `Ctrl + Page Up` | Yes |
| Go to tab #1 … #8 | `workspace:goto-tab-1` … `-8` | `Ctrl + 1` … `Ctrl + 8` | Yes |
| Graph view: Open graph view | `graph:open` | `Ctrl + G` | Yes |
| Insert Markdown link | `editor:insert-link` | `Ctrl + K` | Yes |
| Navigate back | `app:go-back` | `Ctrl + Alt + ←` | Yes |
| Navigate forward | `app:go-forward` | `Ctrl + Alt + →` | Yes |
| New tab | `workspace:new-tab` | `Ctrl + T` | Yes |
| Open help | `app:open-help` | `F1` | Yes |
| Open link under cursor in new tab | `editor:open-link-in-new-leaf` | `Ctrl + Enter` | Yes |
| Open link under cursor in new window | `editor:open-link-in-new-window` | `Ctrl + Alt + Shift + Enter` | Yes |
| Open link under cursor to the right | `editor:open-link-in-new-split` | `Ctrl + Alt + Enter` | Yes |
| Open settings | `app:open-settings` | `Ctrl + ,` | Yes |
| Quick switcher: Open quick switcher | `switcher:open` | `Ctrl + O` | Yes |
| Rename file | `workspace:edit-file-title` | `F2` | Yes |
| Save current file | `editor:save-file` | `Ctrl + S` | Yes |
| Search & replace in current file | `editor:open-search-replace` | `Ctrl + H` | Yes |
| Search current file | `editor:open-search` | `Ctrl + F` | Yes |
| Search: Search in all files | `global-search:open` | `Ctrl + Shift + F` | Yes |
| Toggle bold | `editor:toggle-bold` | `Ctrl + B` | Yes |
| Toggle checkbox status | `editor:toggle-checklist-status` | `Ctrl + L` | Yes |
| Toggle comment | `editor:toggle-comments` | `Ctrl + /` | Yes |
| Toggle italic | `editor:toggle-italics` | `Ctrl + I` | Yes |
| Toggle reading view | `markdown:toggle-preview` | `Ctrl + E` | Yes |
| Undo close tab | `workspace:undo-close-pane` | `Ctrl + Shift + T` | Yes |

> **Note** `workspace:close-window` closes the whole window (the app exits when the last one
> closes), and `editor:open-link-in-new-window` opens the link in a new tab of this window:
> a real popout window does not exist here.

## Runnable commands with no default key

All 251 of them: the runnable commands that carry no Obsidian default key, including every
`silica:`-prefixed command (Silica never had a binding for those). Bind one in
`hotkeys.json` if you want a key. Several are worth binding on day one:
`editor:toggle-source`, `graph:open-local`, and the two split commands.

| Command | Id |
|---|---|
| Add alias | `markdown:add-alias` |
| Add cursor above | `editor:add-cursor-above` |
| Add cursor below | `editor:add-cursor-below` |
| Add embed | `editor:insert-embed` |
| Add internal link | `editor:insert-wikilink` |
| Add tag | `editor:insert-tag` |
| AI chat: Ask about your notes | `silica:ai-chat` |
| AI: Summarize current note | `silica:ai-summarize-note` |
| app:show-tab-switcher | `app:show-tab-switcher` |
| Attach file | `silica:attach-file` |
| Audio recorder: Start recording audio | `audio-recorder:start` |
| Audio recorder: Stop recording audio | `audio-recorder:stop` |
| Back up the vault to a local folder | `silica:vault-backup` |
| Backlinks: Count the notes linking here | `silica:backlink-count` |
| Backlinks: Open backlinks for the current note | `backlink:open-backlinks` |
| Backlinks: Show backlinks | `backlink:open` |
| Backlinks: Toggle backlinks in document | `backlink:toggle-backlinks-in-document` |
| Bases: Add item | `bases:add-item` |
| Bases: Add view | `bases:add-view` |
| Bases: Copy table to clipboard | `bases:copy-table` |
| Bases: Create new base | `bases:new-file` |
| Bases: Insert new base | `bases:insert` |
| Bases: Switch view... | `bases:change-view` |
| Bookmarks: Bookmark all tabs... | `bookmarks:bookmark-all-tabs` |
| Bookmarks: Bookmark block under cursor... | `bookmarks:bookmark-current-section` |
| Bookmarks: Bookmark current search... | `bookmarks:bookmark-current-search` |
| Bookmarks: Bookmark heading under cursor... | `bookmarks:bookmark-current-heading` |
| Bookmarks: Bookmark the current note | `silica:bookmarks` |
| Bookmarks: Bookmark... | `bookmarks:bookmark-current-view` |
| Bookmarks: Remove bookmark for the current file | `bookmarks:unbookmark-current-view` |
| Bookmarks: Show bookmarks | `bookmarks:open` |
| Canvas: Convert to file... | `canvas:convert-to-file` |
| Canvas: Create new canvas | `canvas:new-file` |
| Canvas: Export as image | `canvas:export-as-image` |
| Canvas: Jump to group | `canvas:jump-to-group` |
| Canvas: Reset zoom | `silica:canvas-zoom-reset` |
| Canvas: Zoom in | `silica:canvas-zoom-in` |
| Canvas: Zoom out | `silica:canvas-zoom-out` |
| Canvas: Zoom to fit | `silica:canvas-zoom-fit` |
| Change theme... | `theme:switch` |
| Change vault... | `app:switch-vault` |
| Clear file properties | `markdown:clear-metadata-properties` |
| Clear formatting | `editor:clear-formatting` |
| Close all other tabs | `workspace:close-others` |
| Close current tab | `workspace:close-tab` |
| Close others in tab group | `workspace:close-others-tab-group` |
| Close this tab group | `workspace:close-tab-group` |
| Copy block reference: Copy a link to the block under the cursor | `silica:copy-block-ref` |
| Copy current file path from system root | `workspace:copy-full-path` |
| Copy current file path from vault folder | `workspace:copy-path` |
| Copy current note path | `silica:copy-path` |
| Copy embed: Copy the current note as an embed | `silica:copy-embed` |
| Copy Obsidian URL for current file | `workspace:copy-url` |
| Copy wikilink to current note | `silica:copy-wikilink` |
| Create new note in current tab | `file-explorer:new-file-in-current-tab` |
| Cycle bullet/checkbox | `editor:cycle-list-checklist` |
| Daily notes: Open next daily note | `daily-notes:goto-next` |
| Daily notes: Open previous daily note | `daily-notes:goto-prev` |
| Daily notes: Open today's daily note | `daily-notes` |
| Daily notes: Open today's note | `silica:daily-notes` |
| Decrease font size | `app:decrease-font-size` |
| Delete current file | `app:delete-file` |
| Delete workspace layout | `silica:workspace-delete` |
| Download attachments for current file | `editor:download-attachments` |
| Duplicate line: Duplicate the current line | `silica:duplicate-line` |
| editor:copy | `editor:copy` |
| editor:cut | `editor:cut` |
| editor:go-end | `editor:go-end` |
| editor:go-start | `editor:go-start` |
| editor:move-caret-down | `editor:move-caret-down` |
| editor:move-caret-left | `editor:move-caret-left` |
| editor:move-caret-right | `editor:move-caret-right` |
| editor:move-caret-up | `editor:move-caret-up` |
| editor:paste | `editor:paste` |
| editor:redo | `editor:redo` |
| editor:table-col-duplicate | `editor:table-col-duplicate` |
| editor:table-row-duplicate | `editor:table-row-duplicate` |
| editor:undo | `editor:undo` |
| Export active note to PDF | `silica:export-pdf` |
| Export to PDF... | `workspace:export-pdf` |
| Export vault as site | `silica:export-site` |
| File explorer: Collapse all folders | `file-explorer:collapse-all` |
| File explorer: Expand all folders | `file-explorer:expand-all` |
| File recovery: Open local history | `file-recovery:open` |
| File recovery: Recover a note from a snapshot | `silica:file-recovery` |
| File recovery: Restore a snapshot | `silica:file-recovery-restore` |
| file-explorer:rename-file | `file-explorer:rename-file` |
| Files: Create new folder | `file-explorer:new-folder` |
| Files: Reveal current file in navigation | `file-explorer:reveal-active-file` |
| Files: Show file explorer | `file-explorer:open` |
| Flashcards: grade Again | `silica:flashcard-again` |
| Flashcards: grade Easy | `silica:flashcard-easy` |
| Flashcards: grade Good | `silica:flashcard-good` |
| Flashcards: grade Hard | `silica:flashcard-hard` |
| Flashcards: Review due cards | `silica:flashcards` |
| Focus on last note | `editor:focus` |
| Focus on tab group above | `editor:focus-top` |
| Focus on tab group below | `editor:focus-bottom` |
| Focus on tab group to the left | `editor:focus-left` |
| Focus on tab group to the right | `editor:focus-right` |
| Fold all headings and lists | `editor:fold-all` |
| Fold less | `editor:fold-less` |
| Fold more | `editor:fold-more` |
| Footnotes view: Show footnotes | `footnotes:open` |
| Footnotes view: Show footnotes | `silica:footnotes` |
| Graph view: Open local graph | `graph:open-local` |
| Graph view: Start graph timelapse animation | `graph:animate` |
| Graph: Reset view | `silica:graph-reset-view` |
| Graph: Zoom in | `silica:graph-zoom-in` |
| Graph: Zoom out | `silica:graph-zoom-out` |
| Increase font size | `app:increase-font-size` |
| Indent | `editor:indent` |
| Indent list item | `editor:indent-list` |
| Insert attachment | `editor:attach-file` |
| Insert callout | `editor:insert-callout` |
| Insert code block | `editor:insert-codeblock` |
| Insert footnote | `editor:insert-footnote` |
| Insert horizontal rule | `editor:insert-horizontal-rule` |
| Insert image | `silica:insert-image` |
| Insert math block | `editor:insert-mathblock` |
| Insert table | `editor:insert-table` |
| Kalender | `silica:calendar` |
| Load workspace layout | `silica:workspace-load` |
| Make a copy of the current file | `file-explorer:duplicate-file` |
| Manage vaults | `app:open-vault` |
| Move current file to another folder | `file-explorer:move-file` |
| Move current tab to new window | `workspace:move-to-new-window` |
| Move line down | `editor:swap-line-down` |
| Move line up | `editor:swap-line-up` |
| New window | `workspace:new-window` |
| Note composer: Extract current selection... | `note-composer:split-file` |
| Note composer: Extract this heading... | `note-composer:extract-heading` |
| Note composer: Merge current file with another file... | `note-composer:merge-file` |
| Note composer: Merge the current note | `silica:note-composer-merge` |
| Note info: Show the current note's file facts | `silica:note-info` |
| Open current tab in new window | `workspace:open-in-new-window` |
| Open file from outside the vault... | `app:open-file` |
| Open in default app | `open-with-default-app:open` |
| Open sandbox vault | `app:open-sandbox-vault` |
| Open the template gallery | `silica:template-gallery` |
| Open vault... | `app:open-another-vault` |
| Outdent | `editor:outdent` |
| Outgoing links: Count this note's links | `silica:outgoing-link-count` |
| Outgoing links: Open outgoing links for the current file | `outgoing-links:open-for-current` |
| Outgoing links: Show outgoing links | `outgoing-links:open` |
| Outline: Collapse or expand all headings | `silica:outline-toggle-folds` |
| Outline: Open outline of the current file | `outline:open-for-current` |
| Outline: Show outline | `outline:open` |
| Outline: Toggle follow-cursor | `silica:outline-toggle-follow-cursor` |
| Pin the active tab | `silica:pin-tab` |
| Properties view: Show all properties | `properties:open` |
| Properties view: Show file properties | `properties:open-local` |
| Publish: Export every note to HTML | `silica:publish-vault` |
| Publish: Publish current file | `publish:publish-file` |
| Quick Capture: New note | `quick-capture:open` |
| Random note: Open a random note | `silica:random-note` |
| Random note: Open random note | `random-note` |
| Reload app without saving | `app:reload` |
| Reminder: add for the task at the cursor | `silica:reminder-add` |
| Remove attachment | `silica:remove-attachment` |
| Remove heading | `editor:set-heading-0` |
| Rename this heading... | `editor:rename-heading` |
| Reset zoom | `window:reset-zoom` |
| Reveal the current note in the file manager | `silica:reveal-current-file` |
| Save workspace layout | `silica:workspace-save` |
| Set as heading 1 | `editor:set-heading-1` |
| Set as heading 2 | `editor:set-heading-2` |
| Set as heading 3 | `editor:set-heading-3` |
| Set as heading 4 | `editor:set-heading-4` |
| Set as heading 5 | `editor:set-heading-5` |
| Set as heading 6 | `editor:set-heading-6` |
| Show context menu under cursor | `editor:context-menu` |
| Show debug info | `app:show-debug-info` |
| Show in system explorer | `open-with-default-app:show` |
| Show release notes | `app:show-release-notes` |
| Show trash | `workspace:show-trash` |
| Show vault folder in file manager | `silica:show-vault-folder` |
| Sort lines: Sort the selected lines | `silica:sort-lines` |
| Split down | `workspace:split-horizontal` |
| Split right | `workspace:split-vertical` |
| Table: Add column to the left | `editor:table-col-before` |
| Table: Add column to the right | `editor:table-col-after` |
| Table: Add row above | `editor:table-row-before` |
| Table: Add row below | `editor:table-row-after` |
| Table: Align center | `editor:table-col-align-center` |
| Table: Align left | `editor:table-col-align-left` |
| Table: Align right | `editor:table-col-align-right` |
| Table: Delete column | `editor:table-col-delete` |
| Table: Delete row | `editor:table-row-delete` |
| Table: Duplicate column | `editor:table-col-copy` |
| Table: Duplicate row | `editor:table-row-copy` |
| Table: Move column left | `editor:table-col-left` |
| Table: Move column right | `editor:table-col-right` |
| Table: Move row down | `editor:table-row-down` |
| Table: Move row up | `editor:table-row-up` |
| Tag pane: Collect every tag in the vault | `silica:tag-pane` |
| Tags view: Show tags | `tag-pane:open` |
| Task Dashboard: Open | `task-dashboard:open` |
| Tasks: Show or hide completed tasks | `silica:tasks-toggle-done` |
| Templates: Insert a template | `silica:insert-template` |
| Templates: Insert current date | `insert-current-date` |
| Templates: Insert current time | `insert-current-time` |
| Templates: Insert template | `insert-template` |
| Templates: Insert template | `templates:insert-template` |
| Toggle blockquote | `editor:toggle-blockquote` |
| Toggle blue highlight | `editor:toggle-highlight-blue` |
| Toggle bullet list | `editor:toggle-bullet-list` |
| Toggle case: Swap the case of the selection or word | `silica:toggle-case` |
| Toggle code | `editor:toggle-code` |
| Toggle comment | `editor:toggle-comment` |
| Toggle default mode for new tabs | `app:toggle-default-new-pane-mode` |
| Toggle fold on the current line | `editor:toggle-fold` |
| Toggle fold properties in current file | `editor:toggle-fold-properties` |
| Toggle green highlight | `editor:toggle-highlight-green` |
| Toggle heading | `editor:set-heading` |
| Toggle highlight | `editor:toggle-highlight` |
| Toggle inline math | `editor:toggle-inline-math` |
| Toggle left sidebar | `app:toggle-left-sidebar` |
| Toggle light/dark mode | `theme:toggle-light-dark` |
| Toggle line numbers | `editor:toggle-line-numbers` |
| Toggle Live Preview/Source mode | `editor:toggle-source` |
| Toggle node labels on/off | `graph:toggle-labels` |
| Toggle numbered list | `editor:toggle-numbered-list` |
| Toggle orange highlight | `editor:toggle-highlight-orange` |
| Toggle pin | `workspace:toggle-pin` |
| Toggle purple highlight | `editor:toggle-highlight-purple` |
| Toggle readable line length | `editor:toggle-readable-line-length` |
| Toggle red highlight | `editor:toggle-highlight-red` |
| Toggle ribbon | `app:toggle-ribbon` |
| Toggle right sidebar | `app:toggle-right-sidebar` |
| Toggle spellcheck | `editor:toggle-spellcheck` |
| Toggle stacked tabs | `workspace:toggle-stacked-tabs` |
| Toggle strikethrough | `editor:toggle-strikethrough` |
| Toggle the graph side pane | `silica:toggle-graph-pane` |
| Toggle yellow highlight | `editor:toggle-highlight-yellow` |
| Undo close tab | `workspace:undo-close-tab` |
| Unfold all headings and lists | `editor:unfold-all` |
| Unindent list item | `editor:unindent-list` |
| Unique note creator: Add unique internal link | `insert-unique-link` |
| Unique note creator: Create a timestamped note | `silica:zk-prefixer` |
| Unique note creator: Create new unique note | `zk-prefixer` |
| Vault doctor: Write a health report | `silica:vault-doctor` |
| Verify the command palette | `silica:palette-verify` |
| Voice note: Record audio and insert it into the active note | `silica:voice-note` |
| Word count: Show the current note's counts | `silica:word-count` |
| Workspaces: Load workspace layout | `workspaces:load` |
| Workspaces: Manage workspace layouts | `workspaces:open-modal` |
| Workspaces: Save and load another layout | `workspaces:save-and-load` |
| Workspaces: Save layout | `workspaces:save` |
| Zoom in | `window:zoom-in` |
| Zoom out | `window:zoom-out` |

## Keys that are not commands

A few keys belong to a widget rather than to the command registry, and are not rebindable:

| Key | Where | Does |
|---|---|---|
| `Ctrl + Q` | Anywhere | Quits |
| `Escape` | A modal or the find panel | Closes it |
| `Enter`, `↑`, `↓` | The switcher or palette | Choose and move |
| `Enter`, `↑`, `↓` | The find panel | Next and previous match |
| `Alt + Enter` | The find panel | *Find all*, a cursor at every match |
| `Alt + click` | The editor | Add or remove a cursor |
| `Ctrl + click` | Source mode | Follow a link |
| `Ctrl + 0` | The graph | Refit the view |

## The full id list

All 316 ids (256 Obsidian + 60 `silica:`-prefixed) live in `crates/silica-commands/data/commands.txt` in the repository, and the
names in `data/names.txt` beside it. Settings' [Hotkeys tab](/docs/hotkeys/) shows the same
list with your own bindings applied, which is usually the faster way to look one up.

## Silica-only commands

60 `silica:`-prefixed ids exist only in Silica — no Obsidian equivalent. They cover the
app-level actions the spec calls out: `silica:copy-wikilink` / `silica:copy-path` /
`silica:copy-embed` / `silica:copy-block-ref`, `silica:reveal-current-file`,
`silica:canvas-zoom-in|out|reset|fit`, `silica:graph-zoom-in|out|reset-view`,
`silica:tasks-toggle-done`, and the AI/chat pane actions. None carry a default hotkey;
bind them in Settings' Hotkeys tab.

## Next

- [Hotkeys](/docs/hotkeys/): how `hotkeys.json` is applied.
- [Obsidian compatibility](/docs/reference/compatibility/)
