---
aliases:
  - Home
tags:
  - silica/meta
---
# Silica

Silica is a native Rust/GPUI knowledge base that opens [Obsidian](https://obsidian.md) vaults
unchanged. This folder is itself an Obsidian vault: open `docs/silica` in Obsidian, and once Silica can
browse a vault, open it in Silica too. Anything the two apps disagree on here is a bug or a spec gap.

## Notes

- [[Specification]] — what the app implements and where the full specification lives.
- [[Development]] — machine setup, build commands, pinned dependencies, and benchmark results.
- [[Interaction probes]] — the hands-on Obsidian observations the automated probe could not make.
- [[GPUI Kit]] — the component framework Silica does not use, and when adopting part of it would pay.

## Brand colors

The logo (`bitmap.svg` at the repository root, a silica rock) is filled with the brand orange.
Picked on 2026-10-17 for an ember look that holds on both base themes and stays apart from
Obsidian's purple accent.

| Role | Hex |
| --- | --- |
| Brand, logo fill, large marks | `#E8590C` |
| Text and links on light backgrounds (5.2:1 on white) | `#C2410C` |
| Accent on dark backgrounds, hover states (5.4:1 on the dark theme) | `#F2691F` |

## Rules for this vault

- Write notes the way Obsidian would: wikilinks, YAML frontmatter, `#tags`, callouts.
- Keep `.obsidian/` committed except `workspace.json`, so the vault exercises Silica's config reader.
- Project rules for contributors and agents stay in the repository root `CLAUDE.md`.
