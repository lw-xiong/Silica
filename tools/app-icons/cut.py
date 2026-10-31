#!/usr/bin/env python3
"""Cut the executables' icons from the logo master.

`docs/img/silica_v3_fixed.svg` is the master every other copy of the logo comes from, and
this is the cut each desktop reads: `crates/silica/resources/windows/silica.ico`, which
`build.rs` embeds as resource 1 of `silica.exe`; `crates/silica/resources/macos/silica.icns`,
which `tools/macos-app/bundle.sh` puts in `Silica.app`; and the hicolor theme under
`crates/silica/resources/linux/icons`, which `tools/linux-desktop/install.sh` copies into the
XDG data directory beside `silica.desktop`. Run it after changing the master, the way the three
SVG copies are re-cut:

    nix-shell -p imagemagick --run tools/app-icons/cut.py

Two sources, because one drawing cannot cover 16 px and 1024 px:

  * at 48 px and up the master itself, centred on a dark rounded square with the same corner
    radius the site's favicon uses, so the mark keeps its outline, gradients and all fourteen
    vesicles;
  * at 32 px and below `site/sites/silica/assets/favicon.svg`, which is that same master
    already flattened to a silhouette and its six largest holes for exactly this reason - the
    full mark turns to mud by 16 px.

The macOS entries are additionally inset to Apple's 824/1024 content box, which is what keeps a
Dock icon the same visual size as its neighbours; the Windows and hicolor entries are full-bleed,
which is what Explorer and the GNOME and KDE shells draw. Linux also gets the scalable SVG, which
is the plate with the master nested inside it rather than anything redrawn.
"""

import io
import re
import shutil
import struct
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
MASTER = ROOT / "docs/img/silica_v3_fixed.svg"
FLAT = ROOT / "site/sites/silica/assets/favicon.svg"
ICO = ROOT / "crates/silica/resources/windows/silica.ico"
ICNS = ROOT / "crates/silica/resources/macos/silica.icns"
HICOLOR = ROOT / "crates/silica/resources/linux/icons/hicolor"

# The plate behind the mark: the favicon's colour and its 7/32 corner radius, on a 1024 grid.
PLATE = b"""<svg xmlns="http://www.w3.org/2000/svg" width="1024" height="1024">
  <rect width="1024" height="1024" rx="224" fill="#1e1e22"/>
</svg>"""
MARK_WIDTH = 800  # of 1024: the mark's own margin inside the plate
FLAT_BELOW = 48  # under this many pixels the flattened favicon is drawn instead
APPLE_BOX = 824 / 1024  # macOS content box; the rest of the canvas is transparent margin

# What Windows asks the shell for: the four list and tile sizes, then 256 for the preview pane.
ICO_SIZES = [16, 20, 24, 32, 48, 64, 128, 256]
# The modern icns set: `icp4`/`icp5` and `ic07`-`ic10` are the 1x icons, `ic11`-`ic14` the
# retina ones. 256 and 512 appear twice because 256 @1x and 128 @2x are different entries of
# the same pixel size, and the Finder reads both.
ICNS_ENTRIES = [
    (b"icp4", 16),
    (b"icp5", 32),
    (b"ic11", 32),
    (b"ic12", 64),
    (b"ic07", 128),
    (b"ic13", 256),
    (b"ic08", 256),
    (b"ic14", 512),
    (b"ic09", 512),
    (b"ic10", 1024),
]
# The hicolor sizes a Linux desktop looks for, biggest last. 512 is what GNOME's shell and the
# KDE task switcher reach for; the scalable SVG sits beside them for anything larger.
HICOLOR_SIZES = [16, 24, 32, 48, 64, 128, 256, 512]


def magick(args, stdin=None):
    """Run ImageMagick and hand back what it wrote to stdout."""
    out = subprocess.run(
        ["magick", *args], input=stdin, stdout=subprocess.PIPE, check=True
    )
    return out.stdout


def full_plate():
    """The master on its plate, 1024 px square, as PNG bytes."""
    # Everything goes through files: ImageMagick hands SVG to an external renderer (librsvg
    # or Inkscape), neither of which reads a pipe, and a composite wants two inputs where
    # only one of them could be stdin anyway.
    with tempfile.TemporaryDirectory() as tmp:
        plate = Path(tmp) / "plate.svg"
        plate.write_bytes(PLATE)
        layers = []
        for name, args in (
            ("plate.png", ["-background", "none", str(plate)]),
            (
                "mark.png",
                ["-background", "none", "-density", "1200", str(MASTER),
                 "-trim", "+repage", "-resize", f"{MARK_WIDTH}x{MARK_WIDTH}"],
            ),
        ):
            path = Path(tmp) / name
            magick([*args, f"png32:{path}"])
            layers.append(str(path))
        return magick([*layers, "-gravity", "center", "-composite", "png32:-"])


def render(size, plate, inset=False):
    """One square PNG of `size` pixels, from whichever drawing reads at that size."""
    box = round(size * APPLE_BOX) if inset else size
    if size < FLAT_BELOW:
        art = magick(
            ["-background", "none", str(FLAT), "-resize", f"{box}x{box}", "png32:-"]
        )
    else:
        art = magick(
            ["-background", "none", "png32:-", "-filter", "Lanczos",
             "-resize", f"{box}x{box}", "png32:-"],
            stdin=plate,
        )
    if not inset:
        return art
    return magick(
        ["-background", "none", "png32:-", "-gravity", "center",
         "-extent", f"{size}x{size}", "png32:-"],
        stdin=art,
    )


def scalable_svg():
    """The plate with the master nested inside it, for `hicolor/scalable`.

    An SVG may contain another SVG with its own viewBox and viewport, so the master goes in
    whole - gradients, ids and all - rather than being redrawn or traced. Only its outer tag's
    `width` and `height` change, into the box the raster cuts centre the mark in.
    """
    master = MASTER.read_text()
    open_tag = re.search(r"<svg\b[^>]*>", master, re.S)
    if open_tag is None:
        sys.exit(f"no <svg> element in {MASTER.relative_to(ROOT)}")

    view_box = re.search(r'viewBox="([^"]+)"', open_tag.group(0))
    if view_box is None:
        sys.exit(f"no viewBox on {MASTER.relative_to(ROOT)}'s <svg>")
    _, _, vw, vh = (float(n) for n in view_box.group(1).split())

    # The same box the raster sizes put the mark in: MARK_WIDTH wide, centred on the plate.
    width = MARK_WIDTH
    height = width * vh / vw
    x = (1024 - width) / 2
    y = (1024 - height) / 2

    inner = re.sub(r'\s+(?:width|height|x|y)="[^"]*"', "", open_tag.group(0))
    # Dropping those attributes leaves Inkscape's one-per-line formatting with holes in it.
    inner = re.sub(r"\n\s*(?=\n)", "", inner)
    inner = inner[:-1].rstrip() + (
        f'\n   x="{x:g}" y="{y:g}" width="{width:g}" height="{height:g}">'
    )
    # Everything before the opening tag is the XML declaration and Inkscape's comment.
    body = inner + master[open_tag.end():]

    return (
        '<?xml version="1.0" encoding="UTF-8"?>\n'
        '<!-- Cut from docs/img/silica_v3_fixed.svg by tools/app-icons/cut.py. Do not edit. -->\n'
        '<svg xmlns="http://www.w3.org/2000/svg" width="1024" height="1024"'
        ' viewBox="0 0 1024 1024">\n'
        '  <rect width="1024" height="1024" rx="224" fill="#1e1e22"/>\n'
        f"{body}\n"
        "</svg>\n"
    )


def write_ico(frames):
    """An ICONDIR of PNG-compressed entries: every Windows since Vista reads these."""
    out = io.BytesIO()
    out.write(struct.pack("<HHH", 0, 1, len(frames)))
    offset = 6 + 16 * len(frames)
    for size, png in frames:
        # 0 in the width and height bytes means 256; everything else fits in a byte.
        out.write(
            struct.pack(
                "<BBBBHHII", size % 256, size % 256, 0, 0, 1, 32, len(png), offset
            )
        )
        offset += len(png)
    for _, png in frames:
        out.write(png)
    return out.getvalue()


def write_icns(entries):
    """The icns container: a typed, length-prefixed chunk per entry, PNG payloads throughout."""
    body = b"".join(
        kind + struct.pack(">I", len(png) + 8) + png for kind, png in entries
    )
    return b"icns" + struct.pack(">I", len(body) + 8) + body


def main():
    if shutil.which("magick") is None:
        sys.exit("ImageMagick is not on PATH; try: nix-shell -p imagemagick --run tools/app-icons/cut.py")
    for source in (MASTER, FLAT):
        if not source.exists():
            sys.exit(f"missing {source.relative_to(ROOT)}")

    plate = full_plate()

    ICO.write_bytes(write_ico([(s, render(s, plate)) for s in ICO_SIZES]))
    print(f"{ICO.relative_to(ROOT)}: {len(ICO_SIZES)} sizes, {ICO.stat().st_size} bytes")

    sized = {}
    for kind, size in ICNS_ENTRIES:
        if size not in sized:
            sized[size] = render(size, plate, inset=True)
    ICNS.write_bytes(write_icns([(kind, sized[size]) for kind, size in ICNS_ENTRIES]))
    print(f"{ICNS.relative_to(ROOT)}: {len(ICNS_ENTRIES)} entries, {ICNS.stat().st_size} bytes")

    # hicolor wants one file per size in its own directory, and the theme is installed by
    # copying the tree, so the layout on disk is the layout the spec asks for.
    for size in HICOLOR_SIZES:
        png = HICOLOR / f"{size}x{size}/apps/silica.png"
        png.parent.mkdir(parents=True, exist_ok=True)
        png.write_bytes(render(size, plate))
    svg = HICOLOR / "scalable/apps/silica.svg"
    svg.parent.mkdir(parents=True, exist_ok=True)
    svg.write_text(scalable_svg())
    print(
        f"{HICOLOR.relative_to(ROOT)}: {len(HICOLOR_SIZES)} sizes"
        f" ({', '.join(str(n) for n in HICOLOR_SIZES)}) and scalable"
    )


if __name__ == "__main__":
    main()
