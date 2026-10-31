---
title: The graph view
description: Silica's graph of your vault — what the nodes and edges are, the filters and color groups it reads from graph.json, the force simulation, and how to move around in it.
---

# The graph view

`Ctrl + G` (`graph:open`) opens the graph; `Ctrl + G` again returns to the note you were
reading. `graph:open-local` opens the local graph of the open note instead.

## What is in it

| Node | When |
|---|---|
| A note | Always |
| An attachment | With `showAttachments` on |
| An unresolved target | With `hideUnresolved` off — a `[[Name]]` nobody has created |
| A tag | With `showTags` on |
| An orphan | With `showOrphans` on — a note with no links either way |

Edges are links, in either direction, including links in frontmatter.

Every one of those switches comes from your vault's `graph.json`, so the graph opens looking
the way you left it in Obsidian. So do the `search` filter — an arbitrary [search
query](/docs/reference/search-syntax/) restricting which notes appear — and your
**color groups**, each a query and a color, evaluated with the same grammar.

The display and force settings in `graph.json` are read too: node size, link thickness, text
fade threshold, centre force, repel force, link force and link distance, along with the saved
`scale`.

## Moving around

| | |
|---|---|
| Zoom | Mouse wheel |
| Pan | Drag the background |
| Move a node | Drag the node — which reheats the simulation around it |
| Open a note | Click its node |
| Refit the view | `Ctrl + 0` |

The view auto-fits the graph until the first time you zoom or pan, after which it leaves your
framing alone.

Labels appear once you are zoomed in past a threshold — and that threshold rises with the
number of nodes, so a 5,000-note graph does not try to paint 5,000 labels at once.

## The local graph

`graph:open-local` shows the open note and the notes one link away from it, in either
direction. Opening another note re-centres it on that note.

> **Note** Depth is fixed at one. Obsidian's depth slider, and its local graph as a side pane
> beside the note, are both still ahead.

## How the layout works

A force simulation, stepped on a background thread:

- **Repulsion** between all nodes, computed through a quadtree (Barnes–Hut), so the cost is
  not quadratic in node count.
- **Springs** along the links, at the `linkDistance` from your settings.
- **A weak pull to the centre**, so disconnected clusters do not drift off screen.
- **Simulated annealing**: the simulation cools and stops rather than jiggling forever.
  Dragging a node reheats it.

Painting is a single stroked path for all the edges plus circles for the nodes, which is why
a large graph stays smooth.

## Not yet

- Local-graph depth beyond one, and the local graph as a side pane.
- Arrows on the edges.
- The timelapse animation.
- Screenshot export.
- Writing display changes back to `graph.json` — adjust the forces in Obsidian and Silica
  will read them.

## Next

- [Managing notes](/docs/app/manage/)
- [Internal links](/docs/format/links/)
