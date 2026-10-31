#!/usr/bin/env python3
"""Cut the executables' icons from the logo master.

`crates/silica/assets/logo.png` is the master every other copy of the logo comes from, and
this is the cut each desktop reads: `crates/silica/resources/windows/silica.ico`, which
`build.rs` embeds as resource 1 of `silica.exe`; `crates/silica/resources/macos/silica.icns`,
which `tools/macos-app/bundle.sh` puts in `Silica.app`; and the hicolor theme under
`crates/silica/resources/linux/icons`, which `tools/linux-desktop/install.sh` copies into the
XDG data directory beside `silica.desktop`. Run it after changing the master:

    tools/app-icons/cut.py
"""

import base64
import io
import struct
import sys
from pathlib import Path
from PIL import Image

ROOT = Path(__file__).resolve().parents[2]
MASTER_PNG = ROOT / "crates/silica/assets/logo.png"
ICO = ROOT / "crates/silica/resources/windows/silica.ico"
ICNS = ROOT / "crates/silica/resources/macos/silica.icns"
HICOLOR = ROOT / "crates/silica/resources/linux/icons/hicolor"

# Windows Explorer / taskbar sizes
ICO_SIZES = [16, 20, 24, 32, 48, 64, 128, 256]

# Modern macOS ICNS chunk set
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

# Linux hicolor theme sizes
HICOLOR_SIZES = [16, 24, 32, 48, 64, 128, 256, 512]
APPLE_BOX = 824 / 1024


def render_png(im: Image.Image, size: int, inset: bool = False) -> bytes:
    target_box = round(size * APPLE_BOX) if inset else size
    resized = im.resize((target_box, target_box), Image.Resampling.LANCZOS)
    if inset:
        canvas = Image.new("RGBA", (size, size), (0, 0, 0, 0))
        offset = ((size - target_box) // 2, (size - target_box) // 2)
        canvas.paste(resized, offset, resized)
        final_img = canvas
    else:
        final_img = resized
    buf = io.BytesIO()
    final_img.save(buf, format="PNG", optimize=True)
    return buf.getvalue()


def write_ico(frames: list[tuple[int, bytes]]) -> bytes:
    """An ICONDIR of PNG-compressed entries: every Windows since Vista reads these."""
    out = io.BytesIO()
    out.write(struct.pack("<HHH", 0, 1, len(frames)))
    offset = 6 + 16 * len(frames)
    for size, png in frames:
        # 0 in width and height bytes means 256
        out.write(
            struct.pack(
                "<BBBBHHII", size % 256, size % 256, 0, 0, 1, 32, len(png), offset
            )
        )
        offset += len(png)
    for _, png in frames:
        out.write(png)
    return out.getvalue()


def write_icns(entries: list[tuple[bytes, bytes]]) -> bytes:
    """The icns container: a typed, length-prefixed chunk per entry, PNG payloads throughout."""
    body = b"".join(
        kind + struct.pack(">I", len(png) + 8) + png for kind, png in entries
    )
    return b"icns" + struct.pack(">I", len(body) + 8) + body


def main():
    if not MASTER_PNG.exists():
        sys.exit(f"missing master image: {MASTER_PNG.relative_to(ROOT)}")

    im = Image.open(MASTER_PNG).convert("RGBA")

    # Windows .ico
    ico_frames = [(s, render_png(im, s, inset=False)) for s in ICO_SIZES]
    ICO.parent.mkdir(parents=True, exist_ok=True)
    ICO.write_bytes(write_ico(ico_frames))
    print(f"{ICO.relative_to(ROOT)}: {len(ICO_SIZES)} sizes, {ICO.stat().st_size} bytes")

    # macOS .icns
    sized_icns = {}
    for kind, size in ICNS_ENTRIES:
        if size not in sized_icns:
            sized_icns[size] = render_png(im, size, inset=True)
    ICNS.parent.mkdir(parents=True, exist_ok=True)
    ICNS.write_bytes(write_icns([(k, sized_icns[s]) for k, s in ICNS_ENTRIES]))
    print(f"{ICNS.relative_to(ROOT)}: {len(ICNS_ENTRIES)} entries, {ICNS.stat().st_size} bytes")

    # Linux hicolor PNGs
    for size in HICOLOR_SIZES:
        png_path = HICOLOR / f"{size}x{size}/apps/silica.png"
        png_path.parent.mkdir(parents=True, exist_ok=True)
        png_path.write_bytes(render_png(im, size, inset=False))

    # Linux hicolor scalable SVG
    scalable_1024 = render_png(im, 1024, inset=False)
    b64 = base64.b64encode(scalable_1024).decode("ascii")
    svg_content = f"""<?xml version="1.0" encoding="UTF-8"?>
<!-- Cut from crates/silica/assets/logo.png by tools/app-icons/cut.py. Do not edit. -->
<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="1024" height="1024" viewBox="0 0 1024 1024">
  <image width="1024" height="1024" href="data:image/png;base64,{b64}"/>
</svg>
"""
    svg_path = HICOLOR / "scalable/apps/silica.svg"
    svg_path.parent.mkdir(parents=True, exist_ok=True)
    svg_path.write_text(svg_content)
    print(
        f"{HICOLOR.relative_to(ROOT)}: {len(HICOLOR_SIZES)} sizes "
        f"({', '.join(str(n) for n in HICOLOR_SIZES)}) and scalable SVG"
    )


if __name__ == "__main__":
    main()
