#!/usr/bin/env python3
"""Builds the derived icon files from assets/icons/icon-*.png.

- assets/icons/app.ico      Windows executable / installer icon
- assets/icons/app-64.rgba  raw RGBA, embedded as window icon
- assets/icons/tray-32.rgba / tray-64.rgba  raw RGBA for the tray
Requires Pillow.
"""
from pathlib import Path
from PIL import Image

icons = Path(__file__).resolve().parent.parent / "assets" / "icons"
src = {n: Image.open(icons / f"icon-{n}.png").convert("RGBA") for n in (16, 24, 32, 48, 64, 128, 256)}

src[256].save(icons / "app.ico", sizes=[(n, n) for n in (16, 24, 32, 48, 64, 128, 256)])
(icons / "app-64.rgba").write_bytes(src[64].tobytes())
(icons / "tray-32.rgba").write_bytes(src[32].tobytes())
(icons / "tray-64.rgba").write_bytes(src[64].tobytes())
print("ok")
