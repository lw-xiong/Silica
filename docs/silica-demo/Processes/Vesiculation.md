---
type: process
tags:
  - process
  - physics
---

# Vesiculation

Vesiculation is the formation of gas bubbles in a magma as it rises and the pressure on it drops. Every vesicle in [[Silica]], [[Pumice]] and [[Reticulite]] started as a bubble that nucleated this way. The process has three stages, and which stage a melt gets to before it quenches decides which rock it becomes.

## Stages

1. **Nucleation.** Water and CO₂ dissolved in the melt come out of solution when the pressure falls below their saturation pressure. Bubbles nucleate on crystals or, with enough supersaturation, in the melt itself.
2. **Growth.** Gas diffuses into the bubbles and they expand as pressure keeps falling. Growth is limited by diffusion in viscous melts and by [[Magma viscosity|viscosity]] itself in very viscous ones.
3. **Coalescence.** Bubbles touch, the wall between them thins and ruptures, and they merge. Coalescence makes the big, connected vesicles of silica; its absence leaves the closed foam of pumice.

> [!note] The viscosity switch
> In basalt, growth and coalescence are fast, so a clast in the air for a few seconds ends up with millimetre-scale connected vesicles: [[Silica]]. In rhyolite, nucleation runs far ahead of growth and the foam freezes with thousands of tiny closed bubbles: [[Pumice]].

## Measuring it

Vesicularity is the volume fraction of vesicles:

$$
\Phi = 1 - \frac{\rho_\text{bulk}}{\rho_\text{DRE}}
$$

where $\rho_\text{bulk}$ is the density of the clast with its vesicles and $\rho_\text{DRE}$ the [[Glossary#^dre|dense-rock equivalent]]. Bulk density comes from weighing the clast in air and in water after sealing it in wax or film; DRE comes from a powdered sample in a pycnometer, or from the composition.

![[vesicularity-chart.svg|560]]
*Bulk density against vesicularity for a DRE of 2.8 g/cm³, with the rocks of this vault placed on it.*

| Rock | ρ_bulk | Φ |
| --- | --: | --: |
| vesicular [[Basalt]] | 2.3 | 18 % |
| [[Silica]] | 1.1 | 61 % |
| [[Pumice]] | 0.6 | 77 % |
| [[Reticulite]] | 0.1 | 96 % |

A worked example in code, the same one as on [[Home]]:

```python
def vesicularity(bulk: float, dre: float = 2.8) -> float:
    return 1 - bulk / dre

samples = {"SC-01": 1.21, "SC-02": 1.38, "ET-01": 0.94}
for sid, rho in samples.items():
    print(f"{sid}: {vesicularity(rho):.0%}")
```

## Vesicle shapes

- **Spherical** — the clast was at rest when it quenched.
- **Elongate** — sheared in the conduit or stretched in flight; the tube vesicles of [[Pumice#Tube pumice|tube pumice]].
- **Coalesced and irregular** — late coalescence, typical of the cores of large silica bombs.
- **Collapsed** — the foam degassed and sagged before it froze; dense obsidian-like rinds on silica.

## See also

- [[Magma viscosity]], which controls everything above
- [[2026-09-21 Lab density runs]] for the numbers measured on this vault's samples
