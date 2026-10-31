# PDF fixtures

Generated once and committed; the tests read them as bytes.

- `pages.pdf` (16.6 KB) comes from `pages.typ`: `typst compile pages.typ pages.pdf`
  (typst 0.14.2). Five pages, A4 portrait except page 3 which is landscape, an outline
  from the headings, the title "Pages fixture", and a subset of typst's default font
  (Libertinus Serif) embedded.
- `rotated.pdf` (0.5 KB) and `blank.pdf` (0.4 KB) come from `python3 handmade.py`, which
  writes them by hand with a computed xref table. `rotated.pdf` is a 200 x 100 page with
  `/Rotate 90` and a black rectangle in the unrotated page's bottom-left quadrant;
  `blank.pdf` is an A4 page with no content stream.

Regenerate with the commands above from this directory.
