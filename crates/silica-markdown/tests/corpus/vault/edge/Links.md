# Links

Plain [[Blocks]] link, alias [[Blocks|the blocks note]], heading [[Blocks#Headings]], nested heading [[Blocks#Headings#Deeper]], block [[Blocks#^para1]], self heading [[#Section two]], missing [[Nowhere To Be Found]].

Path link [[edge/Inlines]], extension [[Inlines.md]], folder path [[Areas/Missing]], spaces [[  Blocks  |  spaced  ]], pipe in alias [[Blocks|a|b]], empty [[]] and lone [[ brackets.

Embeds: ![[Inlines]] and ![[Blocks#Headings]] and ![[img-1.png]] and ![[img-1.png|200]] and ![[img-1.png|200x100]] and ![[missing.png]].

Markdown links: [vault note](Blocks.md), [with space](Areas/Some%20Note.md), [heading](Blocks.md#headings), [external](https://example.com/path?q=1&r=2 "Title"), [relative](../Daily/x.md), [empty]().

Autolinks: <https://example.com/a>, <mailto:x@y.z>, <x@y.z>, bare https://example.com/b?c=d. and www.example.org/e, trailing (https://example.com/f) and https://example.com/g).

Reference links: [ref link][ref], [collapsed][], [shortcut], ![ref image][img].

[ref]: https://example.com/ref "Ref title"
[collapsed]: /collapsed
[shortcut]: /shortcut
[img]: /image.png

Footnotes[^1] and[^note] and inline^[an inline footnote with [[Blocks]] inside] and undefined[^nope].

[^1]: First footnote.
[^note]: Second footnote with *emphasis* and a [[Blocks|link]].

    Indented continuation of the footnote.

## Section two

Links inside other constructs: **[[Blocks|bold link]]**, *[[Blocks]]*, ==[[Blocks]]==, `[[not a link]]`, ~~[[Blocks]]~~, > quoted below.

> Quoted [[Blocks]] and #quoted-tag

| Column | Link |
| --- | --- |
| alias | [[Blocks\|aliased]] |
| plain | [[Blocks]] |
| code | `a\|b` |
| escaped | a \| b |
