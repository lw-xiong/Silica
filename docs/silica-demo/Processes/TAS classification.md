---
type: process
aliases:
  - TAS diagram
tags:
  - process
  - classification
reference: Le Bas et al. 1986
---

# TAS classification

Total alkali versus silica. Two numbers from a whole-rock analysis, Na₂O + K₂O on the vertical axis and SiO₂ on the horizontal, place a volcanic rock in a named field. It is the standard scheme for rocks too fine-grained to classify by their minerals, which is nearly all of them.

![[tas-diagram.svg|640]]
*The TAS diagram with the rocks of this vault plotted on it.*

## The fields

| Field | SiO₂ | Alkalis | In this vault |
| --- | --- | --- | --- |
| basalt | 45–52 % | low | [[Basalt]], [[Silica]] |
| basaltic andesite | 52–57 % | low | [[Parícutin]]'s lava, [[Andesite#Basaltic andesite]] |
| andesite | 57–63 % | low | [[Andesite]] |
| dacite | 63–69 % | low | the [[Pumice]] of Mount St. Helens |
| rhyolite | > 69 % | any | [[Rhyolite]], [[Obsidian]] |
| trachybasalt | 45–52 % | higher | [[Mount Etna]]'s lavas |
| phonolite, trachyte | — | highest | not represented |

## How to use it

1. Recalculate the analysis to 100 % on a volatile-free basis.
2. Plot SiO₂ against Na₂O + K₂O, both in weight percent.
3. Read the field. If the point falls on a boundary, use the mineralogy or the name the literature uses for that volcano.

> [!warning] Texture is not composition
> [[Silica]], [[Pumice]], [[Obsidian]] and [[Tuff]] are not TAS names. They describe texture; the TAS field says what the rock is made of. A silica is "a vesicular basalt", a pumice is "a vesicular rhyolite", and so on.

## Example

[[Parícutin]]'s 1943 lava: SiO₂ 55.0 %, Na₂O 3.9 %, K₂O 0.9 % → alkalis 4.8 % → basaltic andesite, just inside the field. Its 1952 lava at 60.3 % plots in andesite.

```json
{
  "sample": "PAR-1943",
  "SiO2": 55.0,
  "Na2O": 3.9,
  "K2O": 0.9,
  "field": "basaltic andesite"
}
```
