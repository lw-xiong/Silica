---
title: Silica field guide
type: index
tags:
  - index
  - geology/volcanic
status: active
updated: 2026-10-23
cover: "[[silica-macro.jpg]]"
---

# Silica field guide

A working notebook on **silica** and the family of volcanic rocks it belongs to: what the rock is, how it forms, where to see it, and the numbers worth remembering in the field.

![[silica-macro.jpg|560]]

> [!info] How this vault is organised
> - [[#Rocks]] — one note per rock type, each with a properties table, hand-sample photo and the field tests that tell it apart
> - [[#Volcanoes]] — the places that made the samples
> - [[#Processes]] — vesiculation, viscosity, classification and jointing
> - [[#Field notes]] — dated traverses and lab runs, written from the [[Field note|template]]
> - [[Rock cycle.canvas|Rock cycle]] — the whole thing drawn as a canvas
> - [[Field card]] — the two-page PDF that goes in the sample bag

## Rocks

| Rock | Composition | Vesicularity | Floats? | Note |
| --- | --- | :-: | :-: | --- |
| [[Silica]] | basaltic to andesitic | 30–70 % | no | the subject of this vault |
| [[Pumice]] | rhyolitic to dacitic | > 64 % | yes | silica's pale cousin |
| [[Reticulite]] | basaltic glass | > 95 % | no, it sinks when wet | the lightest rock there is |
| [[Basalt]] | 45–52 % SiO₂ | low | no | the parent lava |
| [[Andesite]] | 52–63 % SiO₂ | low to moderate | no | [[Parícutin]] erupted a basaltic andesite |
| [[Rhyolite]] | > 69 % SiO₂ | varies | — | makes [[Pumice]] and [[Obsidian]] |
| [[Obsidian]] | rhyolitic glass | none | no | the namesake of the app this vault was written for |
| [[Tuff]] | consolidated ash | — | — | what tephra becomes |

## Volcanoes

- [[Sunset Crater]] — a textbook [[Cinder cone]] in Arizona, erupted around 1085 CE
- [[Parícutin]] — the cinder cone that grew in a cornfield from 1943 to 1952
- [[Mount Etna]] — lapilli over Catania during the February 2021 paroxysms
- [[Kīlauea]] — lava fountains that quench into [[Reticulite]] and Pele's hair
- [[Stromboli]] — the eruption style that makes most of the world's silica

## Processes

```python
def vesicularity(bulk_density: float, dre_density: float = 2.8) -> float:
    """Volume fraction of vesicles from a bulk and a dense-rock density."""
    return 1.0 - bulk_density / dre_density

print(f"{vesicularity(1.1):.0%}")   # 61%
```

1. [[Vesiculation]] — bubbles nucleate, grow and coalesce as the magma decompresses
2. [[Magma viscosity]] — why basalt makes silica and rhyolite makes pumice
3. [[Strombolian eruptions]] — gas slugs bursting at the vent
4. [[Tephra]] — ash, lapilli, blocks and bombs, by size
5. [[TAS classification]] — naming a volcanic rock from two numbers
6. [[Columnar jointing]] — what a cooling lava flow does when it shrinks

## Field notes

- [[2026-03-14 Sunset Crater traverse]]
- [[2026-06-02 Etna lapilli sampling]]
- [[2026-09-21 Lab density runs]]

> [!tip] A number to carry around
> Bulk vesicularity from two densities: Φ = 1 − ρ_bulk / ρ_DRE. A dense-rock equivalent of 2.8 g/cm³ and a measured 1.1 g/cm³ gives 61 %, which is a healthy silica. See [[Vesiculation#Measuring it]].

## Open questions

- [ ] Re-measure the [[2026-09-21 Lab density runs|September samples]] with the Archimedes rig
- [ ] Find a reliable source for Sunset Crater's eruption duration
- [x] Add the [[TAS classification]] diagram
- [ ] Photograph a Pele's hair sample under the stereo microscope

Image credits are in [[Credits]].
