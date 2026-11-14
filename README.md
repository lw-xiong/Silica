<p align="center">
  <img src="docs/img/silica-logo.png" alt="Silica" width="260" height="260">
</p>

<h1 align="center">Silica</h1>

<p align="center"><strong>Your vault. Your files. No import, no conversion.</strong></p>

<p align="center">
  <img alt="Rust" src="https://img.shields.io/badge/100%25-Rust-E8590C?style=flat-square&logo=rust&logoColor=white">
  <img alt="Linux, Windows, macOS" src="https://img.shields.io/badge/Linux%20%7C%20Windows%20%7C%20macOS-2b2b2b?style=flat-square">
  <img alt="MIT" src="https://img.shields.io/badge/license-MIT-2b2b2b?style=flat-square">
  <img alt="v0.9.18" src="https://img.shields.io/badge/status-early%20alpha%20v0.9.18-f2545b?style=flat-square">
</p>

Silica is a native desktop knowledge base written in Rust on GPUI. It opens an Obsidian vault exactly as it sits on disk: the same `.obsidian` configuration, the same Markdown dialect, the same `.canvas` and `.base` files. Nothing is imported or converted, and the vault folder is the only source of truth. Silica is an independent, clean-room implementation and is not affiliated with Obsidian.

## Status

Reader and writer are substantially complete. All five development phases have landed, including phase 5 (plugins). `silica-markdown` parses CommonMark (all 652 spec examples) plus the GitHub extensions Obsidian uses into a single spanned tree with byte spans, and renders reading-view HTML that matches Obsidian on all 1,009 corpus notes.

The app opens a real vault in a three-pane shell: Files, reading view, and side panes for Backlinks, Outgoing links, Tags, All properties, Outline, and live Graph view. The reading view also lists a note's backlinks at its bottom (↗ per linking note, click to open), toggleable with `backlink:toggle-backlinks-in-document`. Search supports grouping, sorting, copying results, vault-wide search & replace, and term-index candidate pre-filtering (~10 ms across 20,000 notes). The quick switcher, graph, command palette, and a Settings modal that writes are all present. Notes open in Live Preview or Source mode with find and replace, multiple cursors, auto-pairing, collapsible headings and lists, and autosave. Fenced code highlights in 80+ languages, and Live Preview renders task boxes, raw `<table>` blocks, and inline HTML elements the way the reading view does — a cursor anywhere in an element brings its source back, and what the overlay cannot draw (a script, an unclosed tag, a lone `<br>`) stays as source. The update-links prompt's **Always** answer rewrites links from then on without asking. Wikilinks (`[[`, `![[`) and tags (`#`) autocomplete against the vault's files, aliases, headings, tags, and block references. Clipboard images and dropped files save to the vault's attachment folder and insert embeds. Frontmatter is an editable, typed properties table that rewrites only the lines it changes.

A `.canvas` opens in a canvas view; a `.pdf` opens in a native viewer with toolbar, sidebar, find bar, and full-text search indexing; Mermaid diagrams render through a pure-Rust engine; audio and video embeds play in the reading view; native voice notes record via microphone (`cpal`/`hound`) directly into the attachments directory; remote images load over HTTP; Ctrl+hovering a link previews the target inline; and file and folder rows drag between folders to move them. `$x$` and `$$x$$` typeset inline in reading view and Live Preview. The right dock's Flashcards tab reviews `#flashcard` cards from the vault on an SM-2 schedule (store at `<vault>/.silica-flashcards.json`), and its AI chat tab asks a local OpenAI-compatible endpoint about your notes, configured per send with `SILICA_AI_URL` / `SILICA_AI_MODEL` / `SILICA_AI_KEY` — the key stays in memory and is never logged or written to disk. The palette also gained the caret navigation commands (`editor:move-caret-*`, `editor:go-start/end`), word count for the open note, **Copy Obsidian URL** for the current file, line-number and readable-width toggles, `footnotes:open` and `file-recovery:open` for the right-dock panes, and `backlink:toggle-backlinks-in-document` for the note-bottom Backlinks section, `editor:focus`, `workspace:toggle-pin`, `app:show-release-notes`, `open-with-default-app:open`, `graph:animate`, `app:show-tab-switcher`, `app:show-debug-info`, and `workspace:export-pdf`. The main area splits into tab groups, and `workspace.json` records and restores named window layouts the way Obsidian does. Everything is bound through Obsidian's command ids, so a vault's `hotkeys.json` applies unchanged.

Installed plugins in `.obsidian/plugins/` are classified as native, compatibility, or refused-with-reason. Compatibility plugins run in a per-plugin QuickJS host with a shimmed `app`; their commands appear in the palette and their settings tabs in Settings > Community plugins. A `silica.json` marker flags a native plugin targeting Silica itself.

Every item in Obsidian's file explorer context menu now runs. **Merge entire file with...** is a three-way merge against the note on disk, with conflicts written as standard `<<<<<<<`/`=======`/`>>>>>>>` blocks rather than guessed away. **Open version history** lists the snapshots Silica records on every save — the last 30 per note, under `.obsidian/.silica-versions/` — and a restore lands in the buffer unsaved. **Open in new window** opens a popout, re-activating an existing one instead of duplicating it. **Show in system explorer** selects the file through D-Bus `FileManager1.ShowItems`, falling back to opening the containing folder. **Make a copy** on a folder copies the whole tree.

Reading view took the same pass: the properties block is editable there, text selects and copies, the inline title renames its file with link rewriting, and image embeds render inline mid-sentence instead of breaking the line. The view header's bookmark button bookmarks the open note through the same prompt as the palette. The file tree renames the same way: F2 or the row menu's **Rename...** turns the row's own label into the field — basename selected, click lands the caret, Enter rewrites links, and a name the filesystem refuses leaves the field open and says nothing.

`silica:export-site` publishes the vault as a static site in one command: every note renders to HTML through the same parser, links become relative URLs that work from disk with no server, attachments come along, and `index.html` is the note called README, index or Home.

> Keep a backup of any vault you open with it. Reading is well covered; editing is newer code against files you cannot get back. Until the round-trip gate is met, let Silica read a vault that matters and let Obsidian write it.

## Benchmarks

Measured on a Lenovo Legion 5 (CachyOS, AMD Ryzen 5 4600H, 12 threads) with `crates/silica-bench` on a synthetic vault of 20,000 notes, 2,000 attachments, and 35.3 MB of Markdown:

| Metric | Measured | Target / Gate |
| --- | --- | --- |
| **Vault index** (scan + read + index + resolve) | **736 ms** (762 ms cold) | < 2.00 s budget (**PASS**) |
| **Search** (20k notes, term index + parallel match) | **~10 ms** | Zero perceptible screen delay |
| **Markdown parse** (20k notes) | **394 ms** (~20 µs/note) | Zero perceptible screen delay |
| **HTML export** (20k notes) | **3.34 s** (~5,992 notes/sec) | Standalone static site export |
| **Memory** (RSS, 20k notes loaded) | **~48 MB** | Lightweight footprint |

All operations execute with zero perceptible latency — searches return as you type, index builds complete in under a second, and exports render thousands of notes per second.

Run benchmarks yourself with `silica-bench`:

```sh
# Generate a synthetic 20,000-note vault
cargo run --release -p silica-bench -- generate /tmp/test-vault --notes 20000

# Run all benchmarks sequentially
cargo run --release -p silica-bench -- all /tmp/test-vault

# Or run individual subcommands
cargo run --release -p silica-bench -- index /tmp/test-vault
cargo run --release -p silica-bench -- search /tmp/test-vault "needle"
cargo run --release -p silica-bench -- parse /tmp/test-vault
cargo run --release -p silica-bench -- export /tmp/test-vault /tmp/export-out
```

## Crates

| Crate | Responsibility |
| --- | --- |
| `silica` | GPUI application: window, workspace shell, Live Preview, reading view |
| `silica-vault` | A vault is a folder: scanning, file kinds, `.obsidian` config reads |
| `silica-markdown` | Obsidian Flavored Markdown parsed into one spanned tree, HTML renderer |
| `silica-cache` | Metadata cache and link resolution |
| `silica-search` | Search query grammar, term index pre-filter, and parallel matcher |
| `silica-text` | Editor text model: buffer, selections, undo, edit plans |
| `silica-suggest` | Autocompletion triggers, candidates, accepted-link writes |
| `silica-templates` | Template placeholders and date formats |
| `silica-formats` | On-disk formats: file kinds, `app.json`, JSON Canvas 1.0, Bases YAML |
| `silica-bases` | The Bases expression language |
| `silica-commands` | 284 command ids and default hotkeys |
| `silica-canvas` | Canvas model: viewport, edges, selection, undo |
| `silica-pdf` | PDF rendering (pages, outline, find bar) and full-text indexing |
| `silica-math` | TeX math typesetting (KaTeX-compatible pure-Rust engine) |
| `silica-highlight` | Prism.js tokenizer for fenced code blocks; theme token painting |
| `silica-plugins` | QuickJS runtime host, shimmed Obsidian API, and plugin discovery |
| `silica-net` | HTTP client abstraction and remote attachment / media fetcher |
| `silica-control` | Control socket protocol (`--control`) for IPC automation |
| `silica-mcp` | Model Context Protocol server driving Silica via control socket |
| `silica-llm-mcp` | Local LLM MCP server for digest, generate, and models |
| `silica-bench` | Benchmark suite and synthetic vault generator |

## Running it

```sh
cargo run --release -p silica -- "/path/to/vault"
cargo run --release -p silica -- docs/silica-demo   # bundled demo vault
cargo run --release -p silica -- "/path/to/vault" --control /run/user/1000/silica.sock
cargo test --workspace
```

`--control` exposes the app on a Unix socket speaking newline-delimited JSON: open a note, run a command id, send keystrokes or clicks, read back the window's whole state.

Two Model Context Protocol servers ship with the workspace:

- `silica-mcp` launches Silica and drives it through the control socket, so an agent can open notes, run commands, and screenshot the app. Register it as `{"mcpServers": {"silica": {"command": "target/release/silica-mcp"}}}`.
- `silica-llm-mcp` talks to a local OpenAI-compatible model server over
  plain HTTP or TLS (`https://` URLs are encrypted via the OS trust store).
  Its `llm_digest` tool reads a file here and returns only the model's
  summary, so large logs never enter the caller's context.
  `llm_generate` and `llm_models` round it out. Configure it with
  `SILICA_LLM_URL` (default `http://127.0.0.1:11434/v1`); `SILICA_LLM_DIGEST_ROOT`
  bounds the files `llm_digest` may read and defaults to a `digest` folder
  next to the server's working directory (there is no unrestricted mode).

## Graph View and Side Pane

Silica's graph works both ways: as a full-screen view (`Ctrl+G`, `graph:open`) and as a side pane in the right dock (`silica:toggle-graph-pane`). When the graph is already open full screen, the dock tab shows a placeholder; when opened from the dock, it renders the live `GraphView` beside the note. The graph respects the core plugin's enabled state like any other pane.

## Limitations

Factual gaps versus Obsidian, tracked rather than hidden:

- Math uses a KaTeX-compatible engine rather than MathJax; genuinely unsupported TeX still shows source in red. MathJax is a JavaScript engine and cannot run in Silica's native Rust/GPUI process; the KaTeX port covers the commands Obsidian uses most.
- HTML is drawn where it can be drawn faithfully and left as source where it cannot. `<sup>` and `<sub>` render with raised/lowered baseline and reduced size; genuinely unsupported HTML tags stay as source.
- Local video plays with sound (ffmpeg decodes the soundtrack through rodio) and seeking works via the skip-back/skip-forward buttons (±10 s) on the player bar; remote media opens in the system player.

Obsidian Sync, Publish, and the commercial services are Obsidian's products, not file formats; Silica has nothing to do with them. Any file-level sync (git, Syncthing, Dropbox) works because files are all Silica needs.

Version history is Silica's own store — snapshots taken on every save, the last 30 per note under `.obsidian/.silica-versions/`. It is deliberately separate from Obsidian Sync: a vault written only by Obsidian has no Silica history until Silica saves it once, and Silica never reads or writes Obsidian Sync's proprietary format. This is a design decision, not a gap.

## Deliberately different

- Frontmatter links are rewritten in place: Silica replaces only the link text instead of re-serializing the whole block.
- The command palette lists only runnable commands. The Hotkeys tab still lists all 284 ids so your bindings remain visible.
- Default font is Liberation Sans; `appearance.json` fonts override it.
- Copying a folder leaves links inside the copies pointing at their originals. Rewriting `[[Plan]]` to `[[Plan 1]]` inside `Notes 1/` would make the copy describe files it does not own, so the copy is a snapshot rather than a second vault.
- Merges and version restores land in the buffer unsaved, so either is reviewed before it overwrites anything.

## License

MIT. See [LICENSE](LICENSE).
