# Silica

Silica is a native Rust/GPUI clone of Obsidian's desktop app, targeting byte-level vault compatibility. 

## Ground rules

- The vault folder is the only source of truth. Never write a file Silica did not change; preserve
  line endings, BOMs, and unknown JSON/YAML keys (`#[serde(flatten)] extra` everywhere).
- One Markdown parser (`silica-markdown`) feeds the editor, reading view, cache, and link rewriting.
  Do not add a second tokenizer for any of those.
- Formats and behaviors are specified in `docs/silica/Specification.md`. When the spec and Obsidian disagree, test
  against a real vault and fix the spec.
- Command ids, config keys, and file names must match Obsidian's exactly (`silica-commands/data/commands.txt`,
  `silica-formats/src/config.rs`). Add new ones only with the observed string.
- Clean room: implement from the spec and public docs. Do not paste code, CSS, or icons from
  Obsidian's bundle.

## Layout

```
crates/silica           GPUI app (bin): workspace shell, Files view, reading view, source editor
crates/silica-vault     folder scanning, .obsidian config
crates/silica-markdown  OFM primitives
crates/silica-cache     CachedMetadata, LinkResolver
crates/silica-formats   FileKind, config types, canvas, base
crates/silica-commands  command registry, default hotkeys
crates/silica-search    search query grammar and matcher, fuzzy search
crates/silica-text      editor text model: buffer, selections, undo, edit plans (no gpui)
crates/silica-suggest   autocompletion: triggers, candidates, the link an accepted row writes (no gpui)
crates/silica-canvas    canvas model: viewport math, edge geometry, hit tests, selection, edits with undo (no gpui)
crates/silica-bases     the Bases expression language: statements parsed into Obsidian's own tree (no gpui)
crates/silica-pdf       PDF rendering on hayro: pages to RGBA, outline, title, page text, the #page= subpath (no gpui)
crates/silica-math      TeX math typeset on RaTeX (KaTeX-compatible): a drawing list in em units plus the KaTeX fonts (no gpui)
crates/silica-bench     spike benchmarks: synthetic vault generator, index timer (bin)
docs/silica             Obsidian vault: spec and design notes (dogfooded by Silica)
docs/silica-demo        Obsidian vault: the geology field guide the README recordings are made on
tools/demo-vault        the typst source of the PDF that vault embeds
docs/img                brand art; silica_v3_fixed.svg is the logo every other copy is cut from
docs/img/app            README screenshots and recordings, made by tools/readme-recording over the control socket
site                    silica.cc: the informational site and the manual (static, node)
```

## Commands

On NixOS run these inside `nix-shell` (`shell.nix` supplies gpui's X11, Wayland, and Vulkan libraries).
The `justfile` wraps every command below (`just` lists them); on macOS it points `CARGO_TARGET_DIR` at
`~/.cache/cargo-target/silica`, because a repo on a quarantined network share makes Gatekeeper prompt
for every build script written there.
gpui is a git dependency pinned to a Zed release commit (see the note in `Cargo.toml`); the crates.io
release is stale and crashes on NVIDIA. Bump the pin on its own, one release at a time, and run the
app on a real GPU before committing.

```
cargo test --workspace            # all library tests (fast)
SILICA_ROUNDTRIP_VAULT=<vault> cargo test -p silica-text --test writer_roundtrip   # Writer gate round trips, over a real vault too
cargo check -p silica             # app, pulls gpui (slow first time)
cargo run --release -p silica -- <vault> [--note "Name#Heading"]   # open a vault (release: the editor reparses on every edit)
cargo run --release -p silica -- <vault> --frame native   # the desktop's title bar instead of the hidden frame (SILICA_FRAME=native does the same)
cargo run --release -p silica -- <vault> --synthetic 50000 --stress   # editor spike: self-driven edits, prints timings
cargo run --release -p silica -- <vault> --note X --source --type '[[al' [--accept enter]  # open the autocompletion popup (and press its keys) for a screenshot
cargo run --release -p silica-bench -- generate <dir>   # synthetic 20k-note vault
cargo run --release -p silica-bench -- index <vault>    # time scan/read/index/resolve, check the 2 s gate
cargo fmt --all && cargo clippy --workspace --all-targets

cd site && npm install && npm run build   # silica.cc -> site/dist/silica (exits 1 on a dead link)
cd site && npm run serve                  # the same, served on http://localhost:4321/
```

## Conventions

- Vault-relative paths use `/` and keep original case; comparisons are case-insensitive.
- Positions are zero-based line/column plus byte offset, like Obsidian's `Pos`.
- Tests that need a vault build one in a `tempfile::tempdir()`; never depend on the developer's vault.
- `crates/silica-markdown/tests/corpus` holds Obsidian's own rendering of a 1000-note vault; the
  corpus test and the CommonMark suite are ratchets. When Obsidian and the parser disagree, add a
  probe note to `tests/corpus/vault/edge`, re-render with the plugin in that vault, and match the
  observed behavior (list it in `docs/silica/Specification.md`).
- `docs/silica/` is an Obsidian vault: write notes with wikilinks and frontmatter, and keep
  `.obsidian/` committed except `workspace.json`.
- The logo lives in three places — `docs/img/silica_v3_fixed.svg` (the master), the app's
  `crates/silica/assets/logo.svg` and the site's `site/sites/silica/assets/`. Change the master
  and re-cut the copies; never edit one of the copies on its own.
- The app icons are a fourth cut of that master, and `just icons` (`tools/app-icons/cut.py`) is
  the only thing that writes them: `crates/silica/resources/windows/silica.ico`, which
  `crates/silica/build.rs` embeds as resource 1 of `silica.exe`;
  `crates/silica/resources/macos/silica.icns`, which `tools/macos-app/bundle.sh` puts in
  `Silica.app`; and `crates/silica/resources/linux/icons`, the hicolor theme
  `tools/linux-desktop/install.sh` installs beside `silica.desktop`. Re-cut them when the
  master changes. The entry's `StartupWMClass` must stay equal to the `app_id` in
  `WindowOptions`, or the Linux task bar loses the icon.
- `site/` vendors the builder from the `ragtux.com` repository byte for byte, bar the two
  differences `site/README.md` records. Port fixes across whole rather than merging.
