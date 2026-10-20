"""Build icon.icns from icon-macos.svg, rendering every size from the vector source.

Needs rsvg-convert (librsvg):
    python3 crates/agt-desktop/icons/build_icns.py
"""
import struct
import subprocess
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
# ICNS entry types that hold PNG data, by pixel size: 16, 32, 128, 256 and 512 points
# at 1x and 2x, as macOS expects in an application icon.
ENTRIES = [
    ("icp4", 16), ("ic11", 32),
    ("icp5", 32), ("ic12", 64),
    ("ic07", 128), ("ic13", 256),
    ("ic08", 256), ("ic14", 512),
    ("ic09", 512), ("ic10", 1024),
]


def main():
    chunks = []
    with tempfile.TemporaryDirectory() as tmp:
        for kind, size in ENTRIES:
            png = Path(tmp) / f"{size}.png"
            if not png.exists():
                subprocess.run(
                    ["rsvg-convert", "-w", str(size), "-h", str(size),
                     str(HERE / "icon-macos.svg"), "-o", str(png)],
                    check=True,
                )
            data = png.read_bytes()
            chunks.append(kind.encode() + struct.pack(">I", 8 + len(data)) + data)
    body = b"".join(chunks)
    (HERE / "icon.icns").write_bytes(b"icns" + struct.pack(">I", 8 + len(body)) + body)
    print(f"wrote {HERE / 'icon.icns'}")


if __name__ == "__main__":
    main()
