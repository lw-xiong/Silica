---
title: Install and run
description: Getting a Silica binary for Linux, Windows or macOS, building it from source, the command-line flags it accepts, and the extra step NixOS needs.
---

# Install and run

There is no installer. A release is one archive per platform holding one binary; unpack it,
put it wherever you keep such things, and run it with the path to a vault.

## Get a binary

Releases are published on [GitHub](https://github.com/ragtux/silica/releases) (Linux x86-64,
Windows x86-64, macOS on Apple silicon) and on
[git.ragtux.com](https://git.ragtux.com/ragtux/silica/releases) (Linux x86-64). Both come
from the same `v*` tag and the same commit; the homelab runner only has a Linux container,
which is the whole reason the two lists differ.

```shell
tar xf silica-x86_64-unknown-linux-gnu.tar.gz
sha256sum -c SHA256SUMS      # against the file in the same release
./silica ~/notes
```

> **Warning** Nothing is code signed or notarized. Linux is the platform Silica is developed on
> and the one that gets the hours; since 0.3.1 the Windows and macOS builds are run on real
> hardware too, each drawing its own platform's window. On macOS, Gatekeeper
> will refuse the first launch — open it once with
> *right-click › Open*, or run `xattr -d com.apple.quarantine silica`. On Windows,
> SmartScreen warns — *More info › Run anyway*.

## Requirements

Silica's interface is [GPUI](https://gpui.rs), which renders on the GPU.

| Platform | Needs |
|---|---|
| Linux | A Vulkan-capable driver. Wayland and X11 both work; libxcb and libxkbcommon are linked, and Wayland, Vulkan and GL are loaded at run time |
| Windows | Windows 10 or 11 with a Direct3D 12 GPU |
| macOS | Apple silicon, macOS with Metal |

Memory follows the size of your vault, not the size of the note you are reading: the
metadata cache holds every note's links, headings, tags and sections. A 20,000-note vault is
comfortable on an ordinary machine — see [performance](/docs/reference/performance/) for
measured numbers.

## Build from source

The toolchain is pinned by `rust-toolchain.toml`, and the dependency set is pinned by
`Cargo.lock`. Build with `--locked` and let the lock file decide:

```shell
git clone https://github.com/ragtux/silica
cd silica
cargo build --release --locked -p silica
cargo test --workspace            # library suites, including the conformance ratchets
./target/release/silica ~/notes
```

> **Note** `gpui` and `gpui_platform` are git dependencies pinned to a Zed release commit,
> because the crates.io release is stale and crashes on NVIDIA. `libc` is pinned too, for an
> old `xattr` in gpui's tree. Both pins are explained in the root `Cargo.toml`. This is why
> `--locked` matters: resolving the dependencies freshly can get you a build that does not
> run.

A release build is the one to use. In a debug build the editor reparses the note on every
keystroke in the foreground, which is fine for a test and unpleasant for typing.

## On NixOS

GPUI loads its X11, Wayland and Vulkan libraries at run time, so a `nix-shell` supplying
them is the straightforward way to both build and run:

```shell
cd silica
nix-shell                          # shell.nix at the repository root
cargo run --release -p silica -- ~/notes
```

The same applies to a downloaded binary: it needs those libraries on the loader path like
any other dynamically linked program, which on NixOS means a shell that provides them (or
`nix-ld`, or patching the binary) rather than running it bare.

## Command-line flags

The only argument you need is the vault. A bare second argument is taken as a note name, so
`silica ~/notes Welcome` works like `--note Welcome`.

```shell
silica <vault> [--note "Name#Heading"] [flags]
```

| Flag | Effect |
|---|---|
| `--note <name>` | Open this note at startup. Takes Obsidian's link syntax: `Name`, `Name#Heading`, `Name#^block-id` |
| `--source` | Open the note in Source mode |
| `--reading` | Open the note in reading view |
| `--search <query>` | Open with the Search tab filled and run. Takes the full [query grammar](/docs/reference/search-syntax/) |
| `--switcher <text>` | Open with the quick switcher up and this typed into it |
| `--palette <text>` | Open with the command palette up and this typed into it |
| `--pane <name>` | Which right-dock tab is showing: `backlinks`, `outgoing`, `outline`, `tags` |
| `--graph [local]` | Open on the graph view; `local` for the local graph of the open note |
| `--settings [tab]` | Open with Settings up, on a tab: `general`, `appearance`, `editor`, `hotkeys`, … |
| `--find <text>` | Open with the find panel up and this matched |
| `--find-all <text>` | As `--find`, then select every match (a cursor per match) |
| `--rename <name>` | Rename the opened note at startup, rewriting links. **Writes to the vault** |
| `--prompt <kind>` | Open a file-operation prompt: `rename`, `move`, `delete`, `folder` |
| `--synthetic [n]` | Ignore the vault's notes and generate one of `n` lines (default 50,000) |
| `--stress [n]` | Drive `n` self-directed edits on the open note, then report timings (default 600) |

Most of these exist so a screenshot or a benchmark can be taken without clicking through the
interface, and they are useful for exactly that. There is no `--help`: an unrecognized
argument is taken as the vault path, or as a note name if a vault was already given.

> **Warning** `--rename` and `--stress` both write to the vault you pass them. Use a scratch
> copy.

## Next

- [Open a vault](/docs/open-a-vault/) — what happens on first launch, and what gets read.
- [Coming from Obsidian](/docs/from-obsidian/) — if you have a vault already.
