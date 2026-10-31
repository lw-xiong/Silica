---
tags:
  - silica/dev
  - silica/design
created: 2026-10-25
---
# GPUI Kit

[gpui-kit](https://github.com/longbridge/gpui-kit) is Longbridge's component framework for GPUI, the successor to their `gpui-component` crate. Silica does not use it. This note records what it offers, why the shell was hand-rolled instead, and the conditions under which adopting part of it would be a good move.

## What it is

Three layered crates, one dependency for an application:

- `gpui-base` — behaviour without presentation: focus management, keyboard conventions, virtual scrolling, text-field editing, dock splitting. Meant to be skinned by the application.
- `gpui-component` — the same behaviour with Longbridge's design system on top: 75+ components covering forms, navigation, overlays, data display, feedback and layout. Includes a dock with draggable tabs and resizable panels, virtual lists and tables, menus, popovers, tooltips, dialogs.
- `gpui-kit` — re-exports GPUI, both crates above and the bundled assets.

It also ships a tree-sitter code editor with LSP support, native Markdown and HTML rendering, syntax highlighting and charts. Apache licensed.

It pins GPUI through `gpui-pre` snapshot packages on crates.io (`=0.3.9` at the time of writing), not a git revision of Zed.

## Why Silica did not start on it

Silica's success criterion is "indistinguishable from Obsidian, byte-compatible vault", and that cuts against a component library in three places.

**The shell is measured, not designed.** The Specification ("Shell typography and chrome", see [[Specification]] for where it lives) lists the values Obsidian computes for its own shell — 40 px header rows, a 44 px ribbon, 13 px small UI text, a 10 px tab curve, a 6.7 percent hover wash — and `crates/silica/src/theme.rs` and `panes.rs` are written to them. `gpui-component` arrives with its own tab shapes, input chrome, focus rings and spacing. Time saved writing widgets would go to un-styling them instead, and the un-styling never ends because the kit's defaults move with each release.

**One Markdown parser.** The ground rule in `CLAUDE.md` is that `silica-markdown` alone feeds the editor, reading view, cache and link rewriting. The kit's two largest components — the Markdown renderer and the code editor — each carry their own tokenizer and would never render OFM (wikilinks, embeds, callouts, block references) against the 1000-note corpus the way Obsidian does. They are the components that look most useful and the ones Silica is forbidden to use.

**The GPUI pin.** [[Development#Environment]] explains why `gpui` is a git dependency on a Zed release commit: the crates.io release crashed on NVIDIA. The pin is bumped one Zed release at a time and run on a real GPU before it is committed. Adopting the kit means taking its `gpui-pre` snapshot, on its schedule, and giving that control up.

A smaller point: every kit component that holds its own state (dock layout, table columns, split sizes) is one more place that is not `workspace.json`, and the vault folder is the only source of truth.

The cost of the decision is real and bounded: roughly 27,000 lines of shell chrome in `crates/silica/src` — `text_input.rs`, `panes.rs`, the modals, the settings tree, much of `workspace.rs` — plus the GPUI traps recorded in [[Development]] that a mature component layer would have absorbed.

## When it becomes a good move

The case for the kit grows as the shell stabilises and the remaining work shifts from "match Obsidian" to "stay correct while GPUI moves". Specifically:

1. **`gpui-base` can be taken without `gpui-component`.** The split the project advertises — behaviour in the foundation, presentation in the application — is exactly the one Silica needs. If the unstyled crate can be depended on alone, the leaf primitives that have no Obsidian-specific shape are candidates: the virtual list, the scrollbar, the resizable split, tooltips, and the clipboard and IME plumbing inside a text field. Each would be skinned from `theme.rs` and replace a hand-rolled element one at a time; none touches the parser or the vault.

2. **Its GPUI pin can be made to match ours.** Either the `gpui-pre` snapshot it tracks reaches a revision that runs clean on NVIDIA, or the kit builds against a git `gpui` through a `[patch]` entry. Until one of those holds, the kit cannot be used at all, whatever its merits. Check this first.

3. **Zed's GPUI changes faster than Silica can follow.** Every bump of the pin currently means re-learning the dispatch order, the `list` reset rule, the notify-during-paint rule, against our own elements. A component layer that tracks GPUI upstream for a living takes that churn on. The more pins Silica has absorbed by hand, the stronger this argument gets.

4. **A new surface has no Obsidian measurement to match.** Anything Silica adds that Obsidian does not have — a diagnostics panel, a control-socket inspector, a benchmark viewer — can use `gpui-component` as it comes, because there is nothing to be pixel-faithful to.

What should never move: the editor (`editor.rs` and `silica-text`), the reading view, anything that renders Markdown, and the dock and tab layout as long as `workspace.json` round-trips through it.

## How to evaluate it

- Add `gpui-base` to a scratch branch with the `[patch]` for our `gpui` revision and see whether it compiles at all.
- Port `text_input.rs` first: it is the most self-contained, has the most hand-written edge cases (the chords Obsidian swallows, the 30 px magnifier inset), and has tests. If the port keeps the tests green and the screenshots in `docs/img/app` identical, the virtual list is next.
- Measure the binary and the release build time before and after; the kit pulls in tree-sitter and a Markdown stack even if those modules go unused.
- Run on the NVIDIA machine and on `w11` before committing, the same as a pin bump.

See also [[Development]] for the pin and the build, [[UI parity backlog]] for which shell pieces are still being matched against Obsidian and so are not ready to move.
