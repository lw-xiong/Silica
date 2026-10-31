---
title: Performance
description: Measured numbers for Silica — indexing a 20,000-note vault, editing a 50,000-line note, what each stage costs, and where the remaining bottlenecks are.
---

# Performance

Silica is a native application, and the reason to be one is that the numbers are different.
This page is the measurements, with the method, rather than adjectives.

**Reference machine:** NixOS, 128-thread host, NVIDIA GPU. Your numbers will differ; the
shape of them should not.

## Indexing a vault

The gate the project set itself: a 20,000-file vault indexed cold in under two seconds.

Measured on a synthetic vault from `silica-bench generate` — 20,000 notes, 2,000
attachments, 35 MB of Markdown — with a warm page cache:

| Threads | Scan | Read | Index | Resolve | Total |
|---|---|---|---|---|---|
| 128 | 75 ms | 19 ms | 11 ms | 34 ms | **139 ms** |
| 1 | 76 ms | 116 ms | 563 ms | 140 ms | **895 ms** |

The four stages are: walking the folder, reading the files, parsing and indexing each note,
and resolving every link in the vault against every file.

What the table says:

- **Folder scanning is the largest fixed cost**, and it barely moves with thread count —
  it is the filesystem, not Silica. It is the stage to watch as the indexer grows.
- **Parsing is not the bottleneck.** Indexing builds the metadata cache from the full parser
  tree, not from a line scanner, and costs 563 ms for 35 MB on a *single* thread — about
  60 MB/s — and 11 ms with the thread pool.
- **Single-threaded, the gate still passes with margin.** 895 ms warm on one thread, against
  a 2-second budget, is the number that matters for a modest machine.

> **Note** These are warm-cache numbers. Cold, the read stage grows by whatever your disk
> costs; the gate was checked cold and passed with margin.

### Reproducing it

```shell
cargo run --release -p silica-bench -- generate /tmp/vault
cargo run --release -p silica-bench -- index /tmp/vault
```

## The editor

The gate: a 50,000-line note that scrolls and types inside a frame budget.

Measured on a generated note of **50,001 lines** — 4.3 MB, 244,000 decorated spans, which
is far denser in syntax than real prose — in Source mode with soft wrap on, driven through
600 self-directed edit, selection, undo and navigation steps:

| Measure | Result |
|---|---|
| Frames painted during 600 steps over 7 s | 607 |
| Shaping the visible lines, per frame | **0.17 ms** mean, 1.95 ms max |
| Full reparse plus decoration pass (background) | 222 ms mean |
| Edit to refreshed decorations | 244 ms |

In Live Preview, on an ordinary 530-line note, shaping costs **0.07 ms** per frame.

What the table says:

- **The frame path is the gate, and it passes by a wide margin.** Only the visible lines are
  ever shaped, so scrolling and typing cost the same in a 50,000-line note as in a short one.
- **Decoration latency scales with note length**, because the whole note is reparsed after
  each edit. 244 ms on this note; a real 50,000-line note of ordinary prose is about half
  its span count. Typing never waits for it — the parse runs on a background thread, one in
  flight and one queued, and the new decorations swap in when ready.
- **Incremental reparse** of just the edited block is the planned follow-up, and is what
  would bring that 244 ms down.

The editor's own timings are in the status bar while you edit — parse time, span count and
shaping time — so you can see this on your own note rather than taking the table's word for
it.

### Reproducing it

```shell
cargo run --release -p silica -- docs/silica --synthetic 50000
cargo run --release -p silica -- docs/silica --synthetic 50000 --stress
```

> **Warning** Use a scratch vault: `--stress` writes.

## Why large notes are cheap

Two decisions do most of the work:

**The scroll position is a line plus an offset into it**, not an absolute pixel. With soft
wrap, knowing the pixel position of line 40,000 would mean measuring the wrapped height of
every line before it. Anchoring to a line instead means wrapped-row heights are only
measured near the viewport — so Page Up, Page Down, revealing the cursor and clamping at the
end of the note all walk rows from that anchor, and no operation needs a full layout pass.

**The reading view lays out one item per top-level block, lazily measured.** Only the blocks
on screen are laid out, which is why a long note opens immediately.

## Memory

Memory follows the vault, not the note. The metadata cache holds every note's links, embeds,
tags, headings, sections, list items, blocks and frontmatter, plus the resolved-link graph in
both directions. Note *text* is held for search. There is no separate database, so this is
the whole cost.

## A caveat on the GPU

GPUI renders on the GPU, and the numbers above are all from the reference Linux machine. The
Windows and macOS builds do run on real hardware, but they have not been profiled there, so
their frame timings are unrecorded rather than slow.

## Next

- [How Silica works](/docs/reference/architecture/) — why it is built this way.
- [Troubleshooting](/docs/reference/troubleshooting/)
