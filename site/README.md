# silica.cc

The informational site and the manual, built to static files. It moved here
from `ragtux.com/sites/silica` on 2026-10-20, when silica.cc replaced
`silica.ragtux.com`. Nothing redirects from the old name: it was never
announced anywhere, and its DNS record is already gone.

```
npm install     # one dependency: marked
npm run build   # -> dist/silica
npm run serve   # build, then serve it on http://localhost:4321/
npm run watch   # the same, rebuilding on every change
```

The build exits 1 if any internal link or in-page anchor does not resolve, so
a cross-reference into the manual cannot rot unnoticed.

## Why the builder is vendored

`build.mjs`, `lib/` and `shared/` are a copy of the builder in the
`ragtux.com` repository, which also builds `ragtux.com` and
`lapstack.ragtux.com`. They are kept **byte-identical on purpose**, so a fix
made on either side can be copied across whole rather than merged, with two
deliberate exceptions:

- `package.json` names this copy and defaults its scripts to the one site.
- `lib/highlight.mjs` highlights `silica` as a shell command, where the other
  copy knows `lapstack`.

The alternative — flattening `sites/silica/` to the repository root and
editing `build.mjs` to match — would have bought a shorter path at the cost of
a permanent diff against the upstream builder. Hence the slightly redundant
`site/sites/silica/`.

## Layout

```
build.mjs              routes, assets, search index, sitemap, link check
lib/markdown.mjs       front matter, marked renderer (anchors, callouts, TOC)
lib/highlight.mjs      dependency-free highlighter (shell, rust, json, lua, toml)
lib/template.mjs       the page shell: top bar, sidebar, on-this-page, pager, footer
shared/                copied into /assets — site.css, site.js, Fira fonts
sites/silica/site.json host, nav, sidebar order, footer, the CTA button
sites/silica/content/  the pages; .md is rendered, .html is passed through
sites/silica/assets/   logo.svg, favicon.svg, silica.css, platform.js and Inter, over shared/
sites/silica/assets/img/app/  the README's recordings, copied by `just site-media`
dist/silica/           the output to deploy (gitignored)
```

A page at `content/docs/app/files.md` is served at `/docs/app/files/`. Front
matter takes `title`, `description`, `layout` (`doc` or `page`) and `noindex`;
pages under `/docs` default to the `doc` layout. The sidebar order lives in
`site.json`, and the prev/next pager follows it, so a new page has to be added
there to be reachable.

## The look

`shared/site.css` is the vendored builder's stylesheet and is not edited here.
Everything that makes silica.cc look like Silica rather than like ragtux.com is
in `sites/silica/assets/silica.css`, which the page loads after it: the type,
the neutral ramp, and the geometry.

The short version of what it does — the long version is the comment at the top
of the file:

- **Inter**, the same faces the application bundles, so the manual and the app
  read as one product. Fira Mono stays, and carries every micro-label: card
  kickers, rail headings, badges, table headers, the code-block language.
- **A warm neutral ramp** — near-black rather than blue-grey in the dark theme,
  paper rather than white in the light one. The accent hues are untouched; they
  are the ones fixed in `docs/silica/Silica.md`.
- **Chrome removed.** Cards drop their fill and their box for a single hairline
  above them, which is also what makes the grid safe to wrap: a row of four
  cards in three columns leaves no half-empty panel behind. Section rules run
  the full width of the window, radii drop to 3 px, and the structure is the
  lines and the whitespace.
- **One loud thing per page.** The primary action is ink on paper, inverted,
  rather than an orange button, so the orange left on the page is the links and
  the kickers.

## The type

`assets/fonts/inter-{400,500,600}.woff2` and `inter-400-italic.woff2` are
subset from the TTFs the app bundles, `crates/silica/assets/fonts` in this
repository, to Latin plus the punctuation, arrows and marks the content uses —
about 26 KB a face. 700 maps to 600 on purpose: Inter's SemiBold is already the
weight the headings want. Inter is under the SIL Open Font License, and
`assets/fonts/LICENSE-Inter.txt` ships beside the faces, as Fira's does in
`shared/fonts/`.

To cut them again after changing the subset:

```
nix-shell -p 'python3.withPackages (ps: [ ps.fonttools ps.brotli ps.zopfli ])'
U="U+0000-00FF,U+0131,U+0152-0153,U+02BB-02BC,U+02C6,U+02DA,U+02DC,U+2000-206F,\
U+2074,U+20AC,U+2122,U+2190-2199,U+21B5,U+2212,U+2215,U+2248,U+2260,U+2264-2265,\
U+2502,U+2505,U+25A0-25A1,U+25B6,U+25CF,U+2605,U+2610-2611,U+2630,U+2713-2714,\
U+2717,U+2726,U+FEFF,U+FFFD"
pyftsubset ../crates/silica/assets/fonts/Inter-Regular.ttf --unicodes="$U" \
  --layout-features=kern,liga,calt,tnum,ccmp,mark,mkmk \
  --flavor=woff2 --output-file=sites/silica/assets/fonts/inter-400.woff2
```

## The recordings

`assets/img/app/` is a copy of `docs/img/app/` in this repository — the GIFs and screenshots
the README is illustrated with, made by `tools/readme-recording` over the control socket. The
builder publishes what is under `assets/`, so they have to live here too; `just site-media`
from the repository root re-cuts the copy, the way the logo's copies are re-cut from the
master. Change a recording at the source, not here.

They are 3.5 MB together and every one is `loading="lazy"` with its intrinsic `width` and
`height` in the markup, so a page costs its text until the reader scrolls to the figure, and
nothing reflows when the bytes land. `figure.shot` in `silica.css` is what styles them.

## The one script

`assets/platform.js` (listed in `site.json`'s `scripts`) points the download buttons at the
visitor's platform: it rewrites the `[data-dl-main]` button's href and label, and marks the
matching `.dl-grid` card `[data-you]`. Every address in the markup is already a working direct
download and Linux is what ships in the HTML, so with JavaScript off the pages stand as
written. Android and iOS deliberately match nothing — there is no build for either.

## The marks

`assets/logo.svg` is the full mark, `docs/img/silica_v3_fixed.svg` in this
repository with Inkscape's editor state stripped. `assets/favicon.svg` is
generated from the same file: its outline and six largest vesicles, flat
`#E8590C` on the dark rounded square, because the full mark's gradients and
all fourteen holes are mud at 16 px.

## Deploying

`dist/silica` is plain static files. `404.html`, `robots.txt`, `sitemap.xml`
and `search-index.json` are generated; search is client-side against that
index, and there is no server. silica.cc is served by the Hostinger VPS
(NixOS, `hosts/vps` in `~/bstore2/raga/nix`): nginx from `/var/www/silica.cc`,
a certificate from Let's Encrypt, DNS-only — no proxy in front, as the privacy
page says. From the nix repository, `just deploy-silica` builds and rsyncs
this site, and `just rebuild-vps` applies the server configuration.
