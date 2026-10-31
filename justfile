# Silica task runner. `just` lists the recipes; `just <recipe>` runs one.
#
# On NixOS run the gpui recipes (check, build, run, ...) inside `nix-shell`,
# which supplies the X11, Wayland and Vulkan libraries: `nix-shell --run 'just run <vault>'`.
# On macOS a repo on a network share is mounted with the quarantine flag, and
# every build script cargo writes there triggers a Gatekeeper prompt, so the
# target directory defaults to a local cache unless CARGO_TARGET_DIR is set.

export CARGO_TARGET_DIR := env("CARGO_TARGET_DIR", if os() == "macos" { home_directory() / ".cache/cargo-target/silica" } else { justfile_directory() / "target" })

docs_vault := justfile_directory() / "docs/silica"

# List the recipes
default:
    @just --list --unsorted

# ---- Tests ---------------------------------------------------------------

# All tests in the workspace, the gpui app included
test:
    cargo test --workspace

# The library tests only: every crate but the gpui app (fast, no Metal or Vulkan)
test-lib:
    cargo test --workspace --exclude silica

# The Writer gate: round-trip the test corpus and a real vault through the text model
roundtrip vault:
    SILICA_ROUNDTRIP_VAULT="{{vault}}" cargo test -p silica-text --test writer_roundtrip

# ---- Build ---------------------------------------------------------------

# Type-check the app crate (pulls gpui; slow the first time)
check:
    cargo check -p silica

# Release build of every binary: silica, silica-mcp, silica-llm-mcp, silica-bench
build:
    cargo build --release --workspace
    @echo "binaries in $CARGO_TARGET_DIR/release"

# Release build of the MCP server alone, for the entry in .mcp.json
mcp:
    cargo build --release -p silica-mcp
    @echo "$CARGO_TARGET_DIR/release/silica-mcp"

# Release build of the local-model MCP server, the other entry in .mcp.json
llm-mcp:
    cargo build --release -p silica-llm-mcp
    @echo "$CARGO_TARGET_DIR/release/silica-llm-mcp"

# Candidate OFM probes for one AREA (see probe-areas): generate, render, triage
probe AREA ROUNDS="3":
    tools/probe-gen/probe_gen.py generate --area {{AREA}} --rounds {{ROUNDS}}
    tools/probe-gen/probe_gen.py render
    tools/probe-gen/probe_gen.py triage

# The syntax areas `probe` can generate for
probe-areas:
    tools/probe-gen/probe_gen.py areas

# Remove the build output
clean:
    cargo clean

# ---- Packaging -----------------------------------------------------------

# Re-cut the app icons from the logo master (docs/img/silica_v3_fixed.svg)
icons:
    nix-shell -p imagemagick --run tools/app-icons/cut.py

# Wrap a release build in Silica.app, the only place macOS looks for an icon (run on a Mac)
macos-app:
    cargo build --release -p silica
    tools/macos-app/bundle.sh "$CARGO_TARGET_DIR/release/silica" "$CARGO_TARGET_DIR/release"

# Put a menu entry and a task bar icon for the release build in ~/.local/share (Linux)
desktop-install:
    cargo build --release -p silica
    tools/linux-desktop/install.sh --binary "$CARGO_TARGET_DIR/release/silica"

# Take the menu entry and the icons back out again
desktop-uninstall:
    tools/linux-desktop/install.sh --uninstall

# ---- Run -----------------------------------------------------------------

# Open a vault (release: the editor reparses on every edit). Extra args go to the app.
run vault *args:
    cargo run --release -p silica -- "{{vault}}" {{args}}

# Open a vault with the desktop's own title bar instead of the hidden frame
run-native vault *args:
    cargo run --release -p silica -- "{{vault}}" --frame native {{args}}

# Open a vault at a note, optionally at a heading: `just note <vault> "Name#Heading"`
note vault note *args:
    cargo run --release -p silica -- "{{vault}}" --note "{{note}}" {{args}}

# Open the autocompletion popup in a note's source view for a screenshot, e.g. `just popup <vault> Welcome '[[al'`
popup vault note typed *args:
    cargo run --release -p silica -- "{{vault}}" --note "{{note}}" --source --type "{{typed}}" {{args}}

# Dogfood: open the docs/silica vault, the spec and design notes
docs *args:
    cargo run --release -p silica -- "{{docs_vault}}" {{args}}

# Editor spike: self-driven edits over a synthetic 50k-note vault, prints timings
stress vault:
    cargo run --release -p silica -- "{{vault}}" --synthetic 50000 --stress

# ---- Benchmarks ----------------------------------------------------------

# Generate a synthetic 20k-note vault into a directory
generate dir:
    cargo run --release -p silica-bench -- generate "{{dir}}"

# Time scan, read, index and resolve over a vault and check the 2 s gate
index vault:
    cargo run --release -p silica-bench -- index "{{vault}}"

# ---- Lint ----------------------------------------------------------------

# Format the workspace
fmt:
    cargo fmt --all

# Lint the workspace, tests included
clippy:
    cargo clippy --workspace --all-targets

# The CI lint job, step for step: formatting, the library tests, clippy without the app
ci:
    cargo fmt --all -- --check
    cargo test --workspace --exclude silica
    cargo clippy --workspace --exclude silica --all-targets

# ---- Site ----------------------------------------------------------------

# Build silica.cc into site/dist/silica (exits 1 on a dead link)
site:
    cd site && npm install && npm run build

# Build silica.cc and serve it on http://localhost:4321/
site-serve:
    cd site && npm install && npm run serve

# Serve silica.cc and rebuild on change
site-watch:
    cd site && npm install && npm run watch

# Re-cut the site's copy of the README recordings from docs/img/app.
# The builder only publishes what is under sites/silica/assets, so the GIFs and
# screenshots live there as well as in docs/img/app; this is the copy step, the
# way the logo's three copies are re-cut from the master.
site-media:
    cp -f docs/img/app/*.gif docs/img/app/*.png site/sites/silica/assets/img/app/
    @echo "site/sites/silica/assets/img/app refreshed from docs/img/app"
