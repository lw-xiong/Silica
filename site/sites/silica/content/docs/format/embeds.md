---
title: Embedding files
description: Embedding notes, sections, blocks and images with ![[…]] in Silica — the size hints, how deep embeds nest, what the alt text becomes, and which file kinds display.
---

# Embedding files

An embed is a link with a `!` in front of it. Instead of a link to the thing, you get the
thing itself, rendered in place.

```markdown
![[Note]]                  the whole note
![[Note#Heading]]          one section of it
![[Note#^block-id]]        one block of it
![[diagram.png]]           an image
![[diagram.png|400]]       scaled to 400 px wide
![[diagram.png|400x300]]   scaled to fit a box
![alt](path/to/image.png)  Markdown form
```

## Embedding notes

**A whole note** (`![[Note]]`) renders the whole note's content inline.

**A section** (`![[Note#Heading]]`) renders from that heading to the next heading of the
same or a higher level. A `## Method` section ends at the next `##` or `#`, and includes any
`###` subsections along the way.

**A block** (`![[Note#^block-id]]`) renders the single block that id is attached to. See
[block ids](/docs/format/blocks/) for how to make one.

Embeds **nest up to four levels deep**. A note that embeds a note that embeds a note works;
beyond four, the chain stops rather than recursing forever.

Links inside an embedded note are live, and follow to the note they point at — not to the
note doing the embedding.

## Embedding images and other files

| Kind | In reading view |
|---|---|
| Local image | Displayed, with size hints honored |
| Remote image (`https://…`) | **Not loaded yet** |
| Audio, video | Recognized as a file kind; no player yet |
| PDF | Recognized; no viewer yet |
| Canvas | Not drawn inside a note. A `.canvas` opens in a canvas view of its own |
| Base | Recognized and round-tripped on disk; no view yet |

Size hints are Obsidian's: `|400` sets the width, `|400x300` fits a box. A hint is not an
alias — Silica tells them apart the way Obsidian does, described next.

## What the alt text becomes

This is fiddly, and it matches Obsidian exactly:

- **`![alt](path)` with no URL scheme** is an *internal embed*. Its alt text is the raw alt
  up to a `|`.
- **`![alt][ref]`** — a reference-style image — is always a plain `<img>`, never an internal
  embed.
- **`![[x|a|200]]`** takes as its alt the first alias part that is *not* a size hint — here
  `a`. With no such part, the alt is the display text, which for a section embed is
  `Note > Heading`.

## Embeds while editing

In Live Preview, a paragraph that is **nothing but an embed** renders as a widget in place of
its source line, through the same renderer reading view uses — so you see the embedded
content while writing around it. Put your cursor in the line and the source comes back. See
[editing a note](/docs/app/editing/).

## Next

- [Callouts](/docs/format/callouts/)
