---
title: Settings
description: The Settings window in Silica — which tabs it shows, where every value comes from in your .obsidian folder, which controls write back to your vault, and which settings actually take effect.
---

# Settings

`Ctrl + ,` (`app:open-settings`) opens Settings. Every value in it is read from your vault's
`.obsidian` folder, so what you see is your real configuration rather than a set of Silica
defaults.

**Settings writes.** Every control Obsidian itself offers as one — the toggles, the seven
dropdowns, the two sliders and the two folder fields — writes its key when you change it.
A write merges into the file's own JSON, so only the key you changed moves and keys Silica
does not know about are left where they are.

A few rows are deliberately read-only, because the control Obsidian offers is one Silica
does not have yet: the three font families, spellcheck languages, excluded files, accent
color, the theme list, and the vault name, vault folder, config folder and language rows,
which are not settings of the vault at all. Change those in Obsidian or in the JSON.

## The tabs

| Tab | Read from |
|---|---|
| General | `app.json` |
| Appearance | `appearance.json` |
| Interface | `app.json`, `appearance.json` |
| Editor | `app.json` |
| Files and links | `app.json` |
| [Hotkeys](/docs/hotkeys/) | `hotkeys.json`, over the command registry |
| Core plugins | `core-plugins.json` |
| Community plugins | `community-plugins.json` and each plugin's `manifest.json` |
| Quick switcher | `switcher.json` |
| Command palette | `command-palette.json` |

The last two are plugin tabs, and appear the way Obsidian shows a core plugin's own options
tab.

Each row is drawn as Obsidian draws it: a name, a description, and a control showing the
current state. The layout and the wording follow Obsidian 1.14.4.

## Which settings actually do something

Showing a setting and honoring it are different things, and the distinction matters. These
are the ones that change how Silica behaves:

| Setting | Key | Effect |
|---|---|---|
| Default view for new tabs | `defaultViewMode` | Whether a note opens in reading view or editing |
| Live Preview | `livePreview` | Which editing mode a note opens in |
| Strict line breaks | `strictLineBreaks` | Whether a single newline is a line break |
| Readable line length | `readableLineLength` | Soft wrap width while editing |
| Indent using tabs / Tab size | `useTab`, `tabSize` | What `Tab` inserts in a list |
| Smart list indentation | `smartIndentList` | Whether Enter continues a list |
| New link format | `newLinkFormat` | The path shape of a rewritten link |
| Use `[[Wikilinks]]` | `useMarkdownLinks` | The form of links Silica creates |
| Automatically update internal links | `alwaysUpdateLinks` | Whether a rename rewrites links without asking |
| Default location for new notes | `newFileLocation`, `newFileFolderPath` | Where a new note lands |
| Deleted files | `trashOption` | System trash, `.trash/`, or outright |
| Confirm file deletion | `promptDelete` | Whether deleting asks |
| Detect all file extensions | `showUnsupportedFiles` | Whether unknown kinds appear in Files |
| Base theme, accent color | `theme`, `accentColor` | The window's colors |
| Fonts and font size | `interfaceFontFamily`, `textFontFamily`, `monospaceFontFamily`, `baseFontSize` | Typography |

So are auto-pairing of brackets and of Markdown (`autoPairBrackets`, `autoPairMarkdown`),
which take effect on the next keystroke without reopening the note.

Settings that are **shown but not yet acted on** include spellcheck, Vim mode, inline title,
fold heading and fold indent, line numbers, indent guides, right-to-left, window frame style,
and `cssTheme`. They are read from your configuration and displayed truthfully, and changing
one still writes the key for Obsidian to honor; the behavior behind them is not implemented
here. Each such row says so on its own line — "not in Silica yet", or "on restart" for the
theme and the fonts — so you can tell which is which without leaving the window.

## Core and community plugins

The **Core plugins** tab shows which of Obsidian's core plugins your vault has enabled,
reading both the modern map shape and the legacy array shape of `core-plugins.json`. Silica
implements the core plugins as part of the application rather than as separate modules, so
the tab is informational: switching one off in Obsidian does not currently remove the
corresponding pane from Silica.

The **Community plugins** tab lists your installed plugins, read from
`community-plugins.json` and each plugin's manifest — name, version, author, description.

> **Note** Community plugins are **listed, not run**. They are JavaScript written against an
> Electron application; Silica is a native binary and cannot execute them. A native plugin
> interface of Silica's own is the last planned phase. See [Obsidian
> compatibility](/docs/reference/compatibility/).

## Not shown

Some settings are absent rather than disabled, because the specification does not yet record
the `app.json` key behind them — and inventing a key would be worse than omitting the row.
These include: the default file to open, deleting attachments with their note, Mermaid
support, quick font-size adjustment, zoom level, and URI callbacks. Obsidian's Keychain tab,
and the options tabs of core plugins other than the quick switcher and the command palette,
are also not shown.

## Unknown keys are safe

Every config reader carries unrecognised keys through untouched, and a write merges the one
changed key into the file's own JSON rather than re-serialising the file. A setting written
by a newer Obsidian than the specification covers is read, kept, and still there afterwards.
That is the property that had to hold before Settings was allowed to write at all, and a
test walks every control to check it.

## Next

- [Hotkeys](/docs/hotkeys/)
- [Vault files and formats](/docs/reference/formats/)
