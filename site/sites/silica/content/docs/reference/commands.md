---
title: Commands and hotkeys
description: The commands Silica registers — the 244 Obsidian command ids, the 41 with a default hotkey, and the 127 Silica can actually run today, with every id and default key listed.
---

# Commands and hotkeys

Silica registers **244 command ids** — the exact strings Obsidian registers, so your
`hotkeys.json` matches them — of which **227 have an observed desktop name**. **41** carry a
default hotkey. **127** do something in Silica today.

Those four numbers are different things and the difference is the point:

- An **id** is what your `hotkeys.json` keys on. Registering all of them is why your
  bindings survive opening the vault in Silica.
- A **name** is what the command palette and the Hotkeys tab display.
- A **default hotkey** is Obsidian's binding, applied unless your file overrides it.
- **Runnable** means Silica implements the behavior. The [command
  palette](/docs/app/switcher/) offers only these; the [Hotkeys tab](/docs/hotkeys/) lists
  all 244, so you can see your bindings for everything else.

Some of the 127 are runnable only in the right place, which is also how Obsidian treats
them: **37** need an editor on screen with the keyboard in it, so they leave the palette in
a reading leaf, and the **fifteen table commands** that act on an existing table are offered
only while the cursor is in one. `graph:open-local` is not offered with no file open. The palette, the hotkey and the
control socket all ask the same question, so a command that is listed is a command that
runs.

Ids come from Obsidian 1.12.7; names and default hotkeys were captured from Obsidian 1.14.4
with a throwaway plugin that dumped `app.commands` and the settings tabs.

## Commands with a default hotkey

`Ctrl` is Cmd on macOS, except where Obsidian itself registers a literal `Ctrl` — the tab
commands below, which are `Ctrl` everywhere.

| Command | Id | Default key | In Silica |
|---|---|---|---|
| Add file property | `markdown:add-metadata-property` | `Ctrl + ;` | Yes |
| Close current tab | `workspace:close` | `Ctrl + W` | Yes |
| Close window | `workspace:close-window` | `Ctrl + Shift + W` | — |
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
| Open link under cursor in new window | `editor:open-link-in-new-window` | `Ctrl + Alt + Shift + Enter` | — |
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

> **Note** The two dashes are the two window commands: a popout window and a second window
> do not exist here. Their ids are registered and your bindings for them are kept.

## Runnable commands with no default key

These do something, and Obsidian ships no binding for them — so if you want a key, bind one
in `hotkeys.json`. Several are worth binding on day one: `editor:toggle-source`,
`graph:open-local`, and the two split commands.

| Command | Id |
|---|---|
| Add cursor above | `editor:add-cursor-above` |
| Add cursor below | `editor:add-cursor-below` |
| Add embed | `editor:insert-embed` |
| Add internal link | `editor:insert-wikilink` |
| Backlinks: Open backlinks for the current note | `backlink:open-backlinks` |
| Backlinks: Show backlinks | `backlink:open` |
| Bookmarks: Bookmark all tabs... | `bookmarks:bookmark-all-tabs` |
| Bookmarks: Bookmark block under cursor... | `bookmarks:bookmark-current-section` |
| Bookmarks: Bookmark current search... | `bookmarks:bookmark-current-search` |
| Bookmarks: Bookmark heading under cursor... | `bookmarks:bookmark-current-heading` |
| Bookmarks: Bookmark... | `bookmarks:bookmark-current-view` |
| Bookmarks: Remove bookmark for the current file | `bookmarks:unbookmark-current-view` |
| Bookmarks: Show bookmarks | `bookmarks:open` |
| Canvas: Convert to file... | `canvas:convert-to-file` |
| Canvas: Create new canvas | `canvas:new-file` |
| Canvas: Jump to group | `canvas:jump-to-group` |
| Change vault... | `app:switch-vault` |
| Clear file properties | `markdown:clear-metadata-properties` |
| Close all other tabs | `workspace:close-others` |
| Close others in tab group | `workspace:close-others-tab-group` |
| Close this tab group | `workspace:close-tab-group` |
| Create new note in current tab | `file-explorer:new-file-in-current-tab` |
| Daily notes: Open next daily note | `daily-notes:goto-next` |
| Daily notes: Open previous daily note | `daily-notes:goto-prev` |
| Daily notes: Open today's daily note | `daily-notes` |
| Delete current file | `app:delete-file` |
| Files: Create new folder | `file-explorer:new-folder` |
| Files: Reveal current file in navigation | `file-explorer:reveal-active-file` |
| Files: Show file explorer | `file-explorer:open` |
| Focus on tab group above | `editor:focus-top` |
| Focus on tab group below | `editor:focus-bottom` |
| Focus on tab group to the left | `editor:focus-left` |
| Focus on tab group to the right | `editor:focus-right` |
| Graph view: Open local graph | `graph:open-local` |
| Indent list item | `editor:indent-list` |
| Insert table | `editor:insert-table` |
| Make a copy of the current file | `file-explorer:duplicate-file` |
| Manage vaults | `app:open-vault` |
| Move current file to another folder | `file-explorer:move-file` |
| Move line down | `editor:swap-line-down` |
| Move line up | `editor:swap-line-up` |
| Open sandbox vault | `app:open-sandbox-vault` |
| Open vault... | `app:open-another-vault` |
| Outgoing links: Open outgoing links for the current file | `outgoing-links:open-for-current` |
| Outgoing links: Show outgoing links | `outgoing-links:open` |
| Outline: Open outline of the current file | `outline:open-for-current` |
| Outline: Show outline | `outline:open` |
| Properties view: Show all properties | `properties:open` |
| Remove heading | `editor:set-heading-0` |
| Set as heading 1 | `editor:set-heading-1` |
| Set as heading 2 | `editor:set-heading-2` |
| Set as heading 3 | `editor:set-heading-3` |
| Set as heading 4 | `editor:set-heading-4` |
| Set as heading 5 | `editor:set-heading-5` |
| Set as heading 6 | `editor:set-heading-6` |
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
| Tags view: Show tags | `tag-pane:open` |
| Templates: Insert current date | `insert-current-date` |
| Templates: Insert current time | `insert-current-time` |
| Templates: Insert template | `insert-template` |
| Toggle blockquote | `editor:toggle-blockquote` |
| Toggle bullet list | `editor:toggle-bullet-list` |
| Toggle code | `editor:toggle-code` |
| Toggle fold properties in current file | `editor:toggle-fold-properties` |
| Toggle highlight | `editor:toggle-highlight` |
| Toggle inline math | `editor:toggle-inline-math` |
| Toggle left sidebar | `app:toggle-left-sidebar` |
| Toggle Live Preview/Source mode | `editor:toggle-source` |
| Toggle numbered list | `editor:toggle-numbered-list` |
| Toggle right sidebar | `app:toggle-right-sidebar` |
| Toggle strikethrough | `editor:toggle-strikethrough` |
| Unindent list item | `editor:unindent-list` |

## Keys that are not commands

A few keys belong to a widget rather than to the command registry, and are not rebindable:

| Key | Where | Does |
|---|---|---|
| `Ctrl + Q` | Anywhere | Quits |
| `Escape` | A modal or the find panel | Closes it |
| `Enter`, `↑`, `↓` | The switcher or palette | Choose and move |
| `Enter`, `↑`, `↓` | The find panel | Next and previous match |
| `Alt + Enter` | The find panel | *Find all* — a cursor at every match |
| `Alt + click` | The editor | Add or remove a cursor |
| `Ctrl + click` | Source mode | Follow a link |
| `Ctrl + 0` | The graph | Refit the view |

## The full id list

All 244 ids live in `crates/silica-commands/data/commands.txt` in the repository, and the
names in `data/names.txt` beside it. Settings' [Hotkeys tab](/docs/hotkeys/) shows the same
list with your own bindings applied, which is usually the faster way to look one up.

## Next

- [Hotkeys](/docs/hotkeys/) — how `hotkeys.json` is applied.
- [Obsidian compatibility](/docs/reference/compatibility/)
