---
tags:
  - silica/dev
  - silica/design
created: 2026-11-01
---
# Plugins (phase 5)

The goal of phase 5 is to run the plugins a vault already has. The honest version of that goal
is *as much of them as can run without giving up the native renderer*, and this note is why the
line falls where it does and what sits on each side of it.

Silica opens a vault unchanged and draws it with GPUI: no Chromium, no DOM, no CodeMirror, no
Node. Obsidian's plugin API is TypeScript over Electron and assumes all four. There is no
version of "run Obsidian plugins unchanged" that does not ship Chromium and reimplement
Obsidian's renderer around it — that is the one trade this project exists to refuse. What is
left, and what is worth building, is the large set of plugins that never touch any of it.

## What a plugin is, on the two sides

Obsidian hands a plugin an `App`: `vault` (read and write files), `metadataCache` (links,
headings, tags, frontmatter, block ids), `fileManager` (rename with link repair, trash),
`workspace` (open a leaf, notepad views), `commands`, plus the `Plugin` base class whose methods
are mostly registration — `registerMarkdownPostProcessor`, `registerMarkdownCodeBlockProcessor`,
`addCommand`, `addRibbonIcon`, `addSettingTab`, `registerView`. Everything above that is the DOM
and CodeMirror 6, which is where a native app and Obsidian part ways.

The three tiers below are that split made concrete. `silica-plugins` classifies every installed
plugin into one of them before anything is loaded, so the user sees "runs here", "runs here in
the compatibility host" or "does not run here, and why" — never a half-started plugin.

| Tier | What it is | How it runs |
| --- | --- | --- |
| Native | A plugin Silica ships, or one written for Silica | Rust in-process, no bridge, no sandbox |
| Compat | JavaScript that touches `app` and the vault index only | QuickJS, one runtime per plugin, off the UI thread |
| Refused | A plugin that needs Electron, Node, the DOM or CodeMirror | Not loaded; the reason is shown |

## Native plugins (tier 0)

Silica's own core plugins — Daily notes, Templates, Backlinks, Bases, and the rest of
`CORE_PLUGINS` — are this tier. They are Rust modules on the same trait a third-party native
plugin would implement, so a plugin that wants the last of the performance budget has a path
that is not JavaScript. A folder under `plugins/` carrying a `silica.json` marker is read as a
native plugin; there is no bridge and no cost beyond the work it does.

This is also the answer to "not sacrifice performance": the pieces a vault leans on hardest are
the pieces that live here, and nothing in the JS host can slow them down.

## The compatibility host (tier 1)

The value is here. A plugin whose `main.js` never reaches for the DOM is, in effect, a function
from vault data to vault data — a frontmatter tool, a format converter, an importer, a query
engine, a periodic-note namer. Those run against a **shimmed `app`** that maps Obsidian's API
onto the native one already in the tree:

| Obsidian | Silica |
| --- | --- |
| `app.vault.read` / `readBinary` / `cachedRead` | `silica-vault`, with the cache already there |
| `app.vault.modify` / `create` / `delete` / `rename` | the same writers the editor uses |
| `app.metadataCache.getFileCache` / `resolvedLinks` / `getFirstLinkpathDest` | the parse index the reader built |
| `app.fileManager.renameFile` (link repair) | the vault's rename, which repairs links |
| `app.commands.addCommand` / `removeCommand` | the command registry, Obsidian command ids |
| `plugin.addRibbonIcon` / `addStatusBarItem` | the ribbon and status bar, through the same view model |
| `plugin.registerMarkdownPostProcessor` | a post-processor stage in the native parse pass |
| `plugin.registerMarkdownCodeBlockProcessor` | the code-block widget a core plugin already is |
| `plugin.addSettingTab` | a settings tab built from the same declarative rows |
| `plugin.loadData` / `saveData` | `<config>/plugins/<id>/data.json`, unchanged |
| `Notice`, `requestUrl`, `normalizePath`, `moment` | native `Notice`, the HTTP client, the path rules, the moment subset |

The engine is QuickJS through `rquickjs`: small, fast to start, and cheap enough that a vault
with a dozen compat plugins does not notice them. One runtime per plugin gives isolation for
free — a crash or a runaway is one plugin's, not the app's. Hard, hostile-code isolation
(WASM under Wasmtime) is a later switch behind the same host trait; it costs more per call, and
nothing about the design above depends on which engine is underneath.

### The performance contract

The order of business is that plugins are cheap when idle and cannot stall a frame.

- **Nothing plugin-side runs on the paint path.** A post-processor runs inside the parse pass,
  its result cached with the parse; a frame only ever paints the cache.
- **One runtime per plugin, off the UI thread.** Calls are marshalled, not made from the paint
  thread, and a plugin that blocks blocks only its own thread.
- **Loaded on demand, opt-in per vault.** Only ids in `community-plugins.json` load, the same
  list Obsidian reads. A vault with no plugins pays nothing, and the classification pass reads
  `manifest.json` and a bounded prefix of `main.js`, not the whole `plugins/` tree.
- **A budget with a watchdog.** Each runtime gets a step and memory ceiling; exceeding it stops
  that plugin and reports it, rather than freezing the app.
- **`metadataCache` is served from the index, not recomputed.** A plugin asking for a file's
  cache gets the one the reader already built; this is the call plugins make most.

## What cannot run, and why that is fine

A plugin that calls `createEl` to draw a custom view, drives `EditorView` decorations, or
`require("electron")` for the filesystem is refused, with the reason, at classification time.
That includes the plugins people would most like to have — a Kanban board that renders its own
DOM, a live-preview extension stack. Refusing them cleanly is better than a shim that draws
wrong or half-works: the vault is never left in a state the user cannot explain. Where such a
plugin's *value* is a data transform under a DOM view — which is most of them — the transform is
exactly the tier-1 case, and only the drawing is missing.

## Where it lands

1. `silica-plugins` classifies every installed plugin into a tier without loading any of it, and
   the Settings > Community plugins list shows the tier and the refusal reason. *(first slice —
   landed 2026-11-01)*
2. The QuickJS host and the `app` shim, with the registration API: commands, ribbon, settings
   tabs and `loadData`/`saveData` first, because they are the smallest surface with the widest
   reach.
3. Markdown post-processors and code-block processors into the parse pass.
4. `metadataCache` and `vault` reads and writes, and with them the first real compat plugins.
5. The native plugin path (`silica.json`, the `Plugin` trait) for anything worth keeping fast.

See [[Development#Plugins (phase 5)]] for the build and [[Specification]] for what the vault's
`.obsidian/plugins` folder holds on disk.
