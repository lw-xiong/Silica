#!/usr/bin/env python3
"""Writes rotated.pdf and blank.pdf: minimal one-page PDFs with a correct xref table.

rotated.pdf: MediaBox 200 x 100 with /Rotate 90, so it displays as 100 x 200; a black
rectangle fills the unrotated page's bottom-left quadrant (x 0..100, y 0..50), which the
clockwise rotation puts in the displayed page's top-left quarter (x 0..50, y 0..100).
blank.pdf: an A4 page with no content stream at all.
"""
from pathlib import Path


def write(path, objects):
    out = bytearray(b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n")
    offsets = []
    for number, body in enumerate(objects, 1):
        offsets.append(len(out))
        out += f"{number} 0 obj\n".encode() + body + b"\nendobj\n"
    xref = len(out)
    out += f"xref\n0 {len(objects) + 1}\n".encode()
    out += b"0000000000 65535 f \n"
    for offset in offsets:
        out += f"{offset:010d} 00000 n \n".encode()
    out += f"trailer\n<< /Size {len(objects) + 1} /Root 1 0 R >>\n".encode()
    out += f"startxref\n{xref}\n%%EOF\n".encode()
    Path(path).write_bytes(out)


content = b"0 g\n0 0 100 50 re\nf\n"
write(
    "rotated.pdf",
    [
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 100] /Rotate 90 /Contents 4 0 R >>",
        b"<< /Length %d >>\nstream\n" % len(content) + content + b"endstream",
    ],
)
write(
    "blank.pdf",
    [
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595.276 841.89] >>",
    ],
)
