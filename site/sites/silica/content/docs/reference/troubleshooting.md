---
title: Troubleshooting
description: The failures people actually hit with Silica — it will not start, nothing renders bold, a formula shows in red, a link went to the wrong note, a file changed unexpectedly — and what to do about each.
---

# Troubleshooting

## It will not start

**On Linux, nothing happens or it exits immediately.** GPUI loads its Wayland, X11, Vulkan
and GL libraries at run time. If they are not on the loader path, it dies early.

- On NixOS: run it through the repository's `shell.nix` (`nix-shell`, then `cargo run` or the
  binary), or use `nix-ld`. A downloaded binary needs those libraries like any other
  dynamically linked program.
- Elsewhere: install your distribution's Vulkan driver package (`mesa-vulkan-drivers`,
  `vulkan-radeon`, `nvidia-utils`, as applicable) and `libxcb` / `libxkbcommon`.
- Check the terminal output. Start it from a terminal rather than a launcher — GPUI's
  failures are printed, not shown in a dialog.

**On macOS, "cannot be opened because the developer cannot be verified".** It is not
notarized. Open it once with *right-click › Open*, or `xattr -d com.apple.quarantine silica`.

**On Windows, SmartScreen blocks it.** It is not code signed. *More info › Run anyway*.

**It crashes on an NVIDIA GPU.** If you built from source without `--locked`, you may have
resolved a newer `gpui` from crates.io, which is stale and crashes on NVIDIA. Rebuild with
`cargo build --release --locked`.

> **Note** The Windows and macOS builds are run on real hardware, but for a fraction of the
> hours Linux gets. If one of them fails to start, that is worth an
> [issue](https://github.com/ragtux/silica/issues) — include the terminal output.

## Nothing renders bold

The pinned GPUI cannot select weights out of a variable font. With a single-file variable
font — Noto Sans's one `NotoSans.ttf` for every weight is the usual culprit — every weight
shapes as regular.

Set a font with real static Bold faces in `appearance.json`:

```json
{
  "textFontFamily": "Liberation Sans",
  "interfaceFontFamily": "Liberation Sans"
}
```

Liberation Sans is the default for exactly this reason. Any family with separate Regular and
Bold files works.

## A formula shows its source in red

The formula did not parse. Silica typesets math with a KaTeX-compatible engine, and Obsidian
with MathJax; the two agree on nearly all of what people write, but a MathJax-only command
(`\require`, a `\newcommand` defined in another formula, a few AMS extensions) is a parse
error here, and a parse error is shown the way MathJax shows one — the source, in red. A
formula that is red in Obsidian too simply has a typo. See [tables, code and
math](/docs/format/code-math/).

HTML in a note shows as text, and remote images do not load.

## A `.base` file does not open

Also current behavior: the Bases YAML format is implemented and round-trips on disk, but
there is no base view yet, so the file is listed in Files and the main area says the kind is
not supported. See [what differs](/docs/reference/differences/).

A `.canvas` does open, in a canvas view of its own. If one does not, that is a bug worth
[reporting](https://github.com/ragtux/silica/issues) with the file.

## A link opened the wrong note

Almost always two notes sharing a basename. Obsidian's resolution order is not "nearest
first", and Silica matches it:

1. A file at the **vault root** wins.
2. Then a file in the **linking note's own folder**.
3. Then the **shortest path**.
4. Then path order.

So `[[Zed]]` in `Long/Way/Down/C.md` goes to `Zed.md` at the root, not to the sibling. To
force a specific file, write more of the path: `[[Long/Way/Down/Zed]]`.

If Silica and Obsidian disagree about where a link goes, that *is* a bug — please
[report it](https://github.com/ragtux/silica/issues) with the two paths and the link text.

## A note looks different from Obsidian

Check [what differs](/docs/reference/differences/) first — math, HTML, remote images and
Mermaid are all known.

If it is none of those, it is a parser bug and a valuable one. The useful report is the
smallest note that shows it, plus what Obsidian renders. The project keeps a 1,009-note
corpus of Obsidian's own output as a test ratchet, and a reproducing note gets added to it.

## I cannot change a setting

Most rows do write — see [Settings](/docs/settings/). The ones that do not are the controls
Silica has no equivalent for yet: the three font families, spellcheck languages, excluded
files, accent color, the theme list, and the vault name, vault folder, config folder and
language rows, which are not settings of the vault at all. Change those in Obsidian, or edit
the JSON in `.obsidian/` and reopen the vault.

A row that writes but changes nothing you can see is a different thing, and the row says so
itself: "not in Silica yet" means the key is written for Obsidian to honor, and "on restart"
means the theme or the fonts are read once at startup.

## A file changed and I did not change it

Take this seriously and please report it. Silica is supposed to write only the note you
edited and the link bytes of notes whose target you renamed —
[the full list](/docs/reference/formats/) of what it writes is short.

The best way to see what happened, and the reason to keep a vault in git: `git diff`. A diff
showing a change you did not make is a bug report that needs almost no words.

Two things that are *expected* and can look surprising:

- **A rename rewrote links in many notes.** That is `alwaysUpdateLinks`, on by default. The
  rules are in [managing notes](/docs/app/manage/). Turn it off in Obsidian and Silica will
  ask first.
- **Line endings or a BOM look different in a tool.** They should not be: Silica restores
  both as it found them. If a tool reports otherwise, that is a bug — say so.

## Editing feels laggy in a huge note

Check the status bar while you type: it shows the parse time and the span count. Typing
itself never waits on the parser (it runs on a background thread), but the *decorations* —
syntax highlighting, Live Preview's hidden markers — refresh only after a reparse, which on
a 50,000-line note is about a quarter of a second.

Also: make sure you are running a **release** build. A debug build reparses in the
foreground on every keystroke.

Incremental reparse of just the edited block is the planned fix. See
[performance](/docs/reference/performance/).

## Search found nothing I expected

Precedence. The implicit AND binds tighter than `OR`, so `a b OR c` means
`(a AND b) OR c`. Parenthesise what you meant, and use the *explain* output to see how your
query was read. The whole grammar is in [search syntax](/docs/reference/search-syntax/).

Also note that `tag:#project` matches child tags like `#project/silica` — a parent matches
its children.

## My community plugins are listed but do nothing

They are listed from `community-plugins.json` and their manifests, and they are not executed.
Obsidian plugins are JavaScript against an Electron app; Silica is a native binary. This is
permanent — see [tier 4](/docs/reference/compatibility/).

## Reporting a bug

[GitHub issues](https://github.com/ragtux/silica/issues) or
[git.ragtux.com](https://git.ragtux.com/ragtux/silica/issues). What helps:

- Your platform, and whether you built from source or downloaded a release.
- The smallest note or vault that shows it. A three-line note beats a description.
- What Obsidian does with the same file, if the bug is a difference.
- Terminal output, if it crashed or would not start.
- `git diff`, if a file changed unexpectedly.

## Next

- [What differs from Obsidian](/docs/reference/differences/)
- [Glossary](/docs/reference/glossary/)
