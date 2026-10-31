// The two-page field card the demo vault embeds as `Media/Field card.pdf`.
// Rebuild with `typst compile tools/demo-vault/field-card.typ "docs/silica-demo/Media/Field card.pdf"`.
#set document(title: "Silica field card", author: "Silica field guide")
#set page(paper: "a5", margin: (x: 16mm, y: 14mm), numbering: "1 / 2")
#set text(font: "Libertinus Serif", size: 10.5pt)
#set heading(numbering: none)
#show heading.where(level: 1): it => block(below: 8pt)[#text(size: 18pt, weight: "bold")[#it.body]]
#show heading.where(level: 2): it => block(above: 10pt, below: 5pt)[#text(size: 12pt, weight: "bold")[#it.body]]

= Silica in the hand

A card to print, laminate and keep in the sample bag. The numbers are the ones the vault's rock notes carry; the tests are the ones that settle a sample at the outcrop.

== Five tests, in order

#table(
  columns: (auto, 1fr),
  stroke: 0.4pt,
  inset: 5pt,
  [*Heft*], [Light for its size but it *sinks*. If it floats it is pumice, whatever the colour.],
  [*Vesicles*], [Rounded, 1--10 mm, walls glassy to dull. More than about 30 % of the volume is holes; under 30 % call it a vesicular basalt.],
  [*Colour*], [Black or dark grey when fresh; red, brown or yellow where the iron oxidised while the clast was still hot.],
  [*Crystals*], [Few or none visible. Olivine or plagioclase specks are fine; a glittering groundmass is not silica.],
  [*Streak*], [Dark grey to brown on unglazed porcelain. A white streak means a felsic rock.],
)

== Numbers worth remembering

#table(
  columns: (1fr, auto, auto),
  stroke: 0.4pt,
  inset: 5pt,
  align: (left, right, right),
  [], [*ρ_bulk (g/cm³)*], [*Φ*],
  [Silica, typical], [1.3], [55 %],
  [Silica, dense end], [1.9], [32 %],
  [Pumice], [0.7], [75 %],
  [Reticulite], [0.1], [97 %],
  [Basalt, parent lava], [2.8], [0 %],
)

Bulk vesicularity from two densities, with the dense-rock equivalent at 2.8 g/cm³:

$ Phi = 1 - rho_"bulk" / rho_"DRE" $

#pagebreak()

= Logging a sample

== At the outcrop

+ Photograph the clast in place with the scale bar before touching it.
+ Note the locality, the unit it came from and whether it fell, flowed or was dug out.
+ Give it an ID: two letters for the locality, a dash, a running number (SC-01, ET-07).
+ Bag it with the ID written on the bag *and* on a tag inside.

== Back at the bench

#table(
  columns: (auto, 1fr),
  stroke: 0.4pt,
  inset: 5pt,
  [*Mass in air*], [To 0.1 g, after a night in the drying cabinet at 60 °C.],
  [*Seal*], [Paraffin wax or cling film; note which, and weigh again.],
  [*Mass in water*], [Suspended from the balance hook, fully submerged, no bubbles on the surface.],
  [*Bulk density*], [Mass in air over the displaced volume, corrected for the seal.],
  [*DRE*], [Pycnometer on the powdered offcut, or 2.8 g/cm³ for a basalt if there is no offcut to spare.],
)

== Two readings to distrust

- A vesicularity above 80 % in a rock that sinks: the seal leaked and water got into the vesicles.
- A bulk density above 2.5 g/cm³ in a rock with visible holes: the clast is not what the bag says it is.

#v(1fr)
#text(size: 8.5pt, fill: luma(90))[Silica field guide · the card is page 1, the protocol page 2. Embedded in the vault as `![[Field card.pdf]]`.]
