---
type: lab-note
date: 2026-09-21
locality: lab
samples: 6
method: Archimedes, film-wrapped
dre: 2.8
tags:
  - lab
---

# 2026-09-21 Lab density runs

Bulk density by the film-wrap Archimedes method on the samples from [[2026-03-14 Sunset Crater traverse]] and [[2026-06-02 Etna lapilli sampling]], and vesicularity from [[Vesiculation#Measuring it]] with a DRE of 2.8 g/cm³ for all of them.

## Method

1. Dry the clast at 60 °C overnight and weigh it in air, m_a.
2. Wrap it in cling film to seal the vesicles; weigh again, m_w.
3. Weigh the wrapped clast suspended in water, m_s.
4. Bulk volume is (m_w − m_s) / ρ_water less the film's own volume, which is negligible above 50 g.
5. ρ_bulk = m_a / V.

## Results

| ID | Rock | m_a (g) | V (cm³) | ρ_bulk | Φ |
| --- | --- | --: | --: | --: | --: |
| SC-01 | [[Silica]], black | 212.0 | 175.2 | 1.21 | 57 % |
| SC-02 | [[Silica]], red | 185.0 | 134.1 | 1.38 | 51 % |
| SC-03 | [[Basalt]] clinker (top) | 140.3 | 76.3 | 1.84 | 34 % |
| SC-03 | [[Basalt]] core (bottom) | 199.7 | 73.4 | 2.72 | 3 % |
| ET-01 | [[Silica]] lapilli, 16–32 mm | 118.4 | 126.0 | 0.94 | 66 % |
| ET-02 | [[Silica]] bomb, interior | 310.0 | 258.3 | 1.20 | 57 % |

> [!note] Reading the table
> The Etna lapilli are the most vesicular, at 66 %: tall fountains, more time in the air. The red Sunset Crater silica is denser than the black, which fits oxidation happening in a hotter, longer-cooling rim. The two halves of SC-03 are the same lava flow at two depths, 34 % against 3 %.

## Plot

![[vesicularity-chart.svg|560]]

## Reference densities

The published values these runs are compared against, live out of the vault
rather than copied in — the rock notes' own `density`, and the dense-rock
equivalent the base computes from it:

![[Rocks.base]]

## Notes

- ET-01 is below 1.0 g/cm³ and still sinks: the vesicles are connected, so it fills with water through the film's pinholes within a minute. Re-run with wax.
- SC-02 lost a corner in the water bath; mass corrected to the recovered pieces.
- The DRE of 2.8 is an assumption. A pycnometer run on powdered SC-01 is the next step.

## To do

- [ ] Wax-seal ET-01 and repeat
- [ ] Pycnometer DRE on SC-01 and ET-01
- [ ] Add the two [[Reticulite]] reference values from the literature to the plot
