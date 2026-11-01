<p align="center">
  <img src="docs/img/silica-logo.png" alt="Silica" width="260" height="260">
</p>

<h1 align="center">Silica</h1>

<p align="center"><strong>Your vault. Your files. No import, no conversion.</strong></p>

<p align="center">
  <img alt="Rust" src="https://img.shields.io/badge/100%25-Rust-E8590C?style=flat-square&logo=rust&logoColor=white">
  <img alt="Linux, Windows, macOS" src="https://img.shields.io/badge/Linux%20%7C%20Windows%20%7C%20macOS-2b2b2b?style=flat-square">
  <img alt="MIT" src="https://img.shields.io/badge/license-MIT-2b2b2b?style=flat-square">
  <img alt="v0.2.3" src="https://img.shields.io/badge/status-early%20alpha%20·%20v0.2.3-f2545b?style=flat-square">
</p>

Silica is a native knowledge base written in Rust on [GPUI](https://gpui.rs), built to open [Obsidian](https://obsidian.md) vaults unchanged: same folder, same `.obsidian` configuration, same Markdown dialect, same `.canvas` and `.base` files. There is no import step, no conversion and no second copy of anything. It is an independent implementation and is not affiliated with Obsidian.

## Status

Reader and writer done, all five phases landed (phase 5: plugins). `silica-markdown` parses CommonMark (all 652 spec examples) plus the GitHub extensions Obsidian uses into one tree with byte spans, and renders reading-view HTML that matches Obsidian on 1007 of 1009 corpus notes. The app opens a real vault with a three-pane shell: Files, reading view, Backlinks, Outgoing links, Tags, All properties and Outline; Search with grouping, sort, copy results, and vault-wide search & replace; the quick switcher, graph, command palette and a Settings modal that writes. Notes open in Live Preview or Source mode with find and replace, multiple cursors, auto-pairing, collapsible headings and lists, and autosave; `[[`, `![[` and `#` autocomplete against the vault's files, aliases, headings, tags and block references; clipboard images and dropped files automatically save to the vault's attachment folder and insert embeds; frontmatter is an editable, typed properties table that rewrites only the lines it changes. A `.canvas` opens in a canvas view; a `.pdf` opens in a viewer with toolbar, sidebar and find bar; Mermaid diagrams render directly via a pure-Rust engine; audio and video embeds play in the reading view, remote images load over HTTP, Ctrl+hovering a link previews the target inline, and file and folder rows drag between folders to move them; `$x$` and `$$x$$` typeset inline in reading view and Live Preview. The main area splits into tab groups, and `workspace.json` records the tree the way Obsidian does. Everything is bound through Obsidian's command ids, so a vault's `hotkeys.json` applies unchanged.

Obsidian plugins: installed plugins in `.obsidian/plugins/` are classified into native, compatibility, or refused-with-reason tiers. Compatibility plugins run in a per-plugin QuickJS host with a shimmed `app` (vault read/write, metadataCache, commands, ribbon, settings tabs, `loadData`/`saveData`); their commands appear in the palette and their settings tabs in Settings > Community plugins. Plugins that need Electron, Node, CodeMirror or the DOM are refused with the reason shown, never half-loaded. A `silica.json` marker flags a native plugin targeting Silica itself.

> **Keep a backup of any vault you open with it.** Reading is well covered; editing is newer code against files you cannot get back. Until the round-trip gate is met, let Silica read a vault that matters and let Obsidian write it.

## Benchmarks

Reference machine: Lenovo Legion 5 — CachyOS 7.1.5-1-cachyos, AMD Ryzen 5 4600H with Radeon Graphics, 12 threads. Your numbers will differ; the shape should not. Measured with `crates/silica-bench` on a synthetic vault: 20,000 notes, 2,000 attachments, 35.3 MB of Markdown.

**Vault indexing** (measured on this machine, `silica-bench` on a synthetic 20,000-note / 35.3 MB vault, 12 threads detected; run 1 is cold):

| run | Scan | Read | Index | Resolve | Total |
| --- | --- | --- | --- | --- | --- |
| 1 (cold) | 45 ms | 16 ms | 81 ms | 47 ms | **190 ms** |
| 2 | 44 ms | 12 ms | 80 ms | 40 ms | **175 ms** |
| 3 | 44 ms | 11 ms | 71 ms | 39 ms | **166 ms** |

Gate: 190 ms cold against a 2.00 s budget — PASS (best run 166 ms). Indexed 191,558 links (191,417 resolved, 9,734 unresolved), 9,593 embeds, 51,125 tags, 48,458 headings, 9,677 block ids, 20,000 frontmatter blocks.

Stage definitions: walk the folder, read the files, parse and index each note from the full parser tree, resolve every link against every file.

**Editor** (50,001-line note, 4.3 MB, 246,726 spans, Source mode, soft wrap, 600 self-directed steps over 8.7 s):

| Measure | Result |
| --- | --- |
| Frames painted in 600 steps | 396 |
| Shaping visible lines, per frame | 2.50 ms mean, 21.55 ms max |
| Full reparse + decoration pass (background) | 212.6 ms mean, 254.8 ms max |
| Edit to refreshed decorations | 196.2 ms mean, 196.2 ms max |

## Compatibility

What works today, measured against Obsidian:

| Area | Coverage | Notes |
| --- | --- | --- |
| CommonMark parser | 652/652 spec examples | One tree, byte spans |
| Corpus render match | 1007/1009 notes | HTML matches Obsidian 1.13.7; 2 pre-existing corpus quirks |
| Vault formats | `.md`, `.canvas`, `.base`, `.pdf`, `.obsidian/` | Same files Obsidian writes; no conversion |
| Command ids | 244 ids + default hotkeys | Vault `hotkeys.json` applies unchanged |
| Link rewriting | Rename, move, copy, delete | Rewrites `[[wikilinks]]` as Obsidian does |
| Plugins | Classification + compat host + native trait | Installed plugins classified as native/compat/refused; compat plugins run in a per-plugin QuickJS host with shimmed `app`; their commands show in the palette, settings tabs in Settings > Community plugins. The native `Plugin` trait is defined and native plugins register via `register_native`, with commands exposed to the palette and runnable; post-processor execution in the render pass is the next slice. |

## Limitations

Factual gaps versus upstream Obsidian:

**Rendering** — Math typeset with a KaTeX-compatible engine rather than MathJax (MathJax-only commands show source in red; no `\require` extension loader); HTML in notes parsed and shown as literal text; remote images `![](https://…)` load via the HTTP client with disk caching; PDF viewer has toolbar/sidebar/find bar and `![[x.pdf]]` embeds, but text cannot be selected or copied and there is no export; audio and video recognized as file kinds with no player.

**Views** — Bases YAML format round-trips but there is no base view (canvas has one); `![[Board.canvas]]` opens in its own view rather than embedded in a note; no popout windows; tabs and split groups exist but stacking does not; one vault per window (run Silica once per vault); local graph is a full view, not a side pane; graph depth beyond 1, arrows, timelapse and screenshot export not supported.

**Editing** — No file recovery (Obsidian keeps snapshots in IndexedDB outside the vault); reading-view properties table is editable in reading view; Vim mode read from `vimMode` but not implemented; line numbers, indent guides, inline title and right-to-left read in Settings but not acted on; no spellcheck; no `Ctrl` + hover page previews; `graph.json` forces and filters read but not written back; drag and drop in Files not supported (use the `file-explorer:move-file` command); inline title editing via `F2`; system trash on Windows and macOS falls back to the vault's `.trash/` (Linux uses freedesktop trash); base theme, accent color and fonts are honored; CSS community theme loaded from `.obsidian/themes/<name>.css` when `cssTheme` is set in `appearance.json`; CSS snippets from `.obsidian/snippets/<name>.css` toggled in Settings > Appearance; font families, spellcheck languages, excluded files, accent color and theme list in Settings do not write; no right-click menu on search results or side-pane rows (Files view only).

**Markdown edge cases** — Two of the 1,009 corpus notes do not match Obsidian's output: nested delimiter runs such as `****a****`, `==== four ==x===y==`, and a wikilink inside Markdown link text (Obsidian splits the link on it). Degenerate cases, tracked rather than hidden.

**Not ever** — Obsidian Sync, Publish and the commercial services are Obsidian's products, not file formats; Silica has nothing to do with them — any file-level sync (git, Syncthing, Dropbox, or Obsidian Sync itself) works because files are all Silica needs.

## Deliberately different

**Frontmatter links are rewritten in place.** Obsidian re-serializes the whole frontmatter block on rename (which can turn `aliases: [Bee]` into a block list and change bytes you did not touch); Silica replaces only the link text.

**The command palette lists only runnable commands.** The Hotkeys tab still lists all 244 ids so your bindings remain visible.

**Default font is Liberation Sans.** The pinned GPUI cannot select weights from a variable font, so Liberation Sans ships static Bold faces. Your `appearance.json` fonts override it.

## Crates

| Crate | Responsibility |
| --- | --- |
| `silica` | GPUI application: window, workspace shell, views |
| `silica-vault` | A vault is a folder: scanning, file kinds, `.obsidian` config reads |
| `silica-markdown` | Obsidian Flavored Markdown parsed into one spanned tree, HTML renderer |
| `silica-cache` | Metadata cache (`CachedMetadata`) and link resolution |
| `silica-suggest` | Autocompletion triggers, candidates, accepted-link writes |
| `silica-templates` | Template placeholders and date formats |
| `silica-formats` | On-disk formats: file kinds, `app.json`, JSON Canvas 1.0, Bases YAML |
| `silica-bases` | The Bases expression language |
| `silica-commands` | 244 command ids and default hotkeys |
| `silica-search` | Search query grammar and matcher |
| `silica-text` | Editor text model: buffer, selections, undo, edit plans |
| `silica-canvas` | Canvas model: viewport, edges, selection, undo |
| `silica-pdf` | PDF rendering: pages, outline, title, page text |
| `silica-math` | TeX math typesetting (KaTeX-compatible) |
| `silica-control` | Control socket protocol (`--control`) and `silica-mcp` |

## Running it

```sh
cargo run --release -p silica -- "/path/to/vault"
cargo run --release -p silica -- docs/silica-demo     # the bundled demo vault
cargo run --release -p silica -- "/path/to/vault" --control /run/user/1000/silica.sock
cargo run --release -p silica -- "/path/to/vault" --frame native   # desktop's own title bar
cargo test --workspace
```

`--control` exposes the app on a Unix socket speaking newline-delimited JSON: open a note, run a command id, send keystrokes or clicks, read back the window's whole state. `silica-mcp` wraps that protocol as a Model Context Protocol server over stdio, so an agent can launch Silica, drive it and screenshot it.

## License

MIT. See [LICENSE](LICENSE).
