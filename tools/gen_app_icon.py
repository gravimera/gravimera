#!/usr/bin/env python3
"""
Generate Gravimera's pixel-style app icons.

Outputs:
  - assets/icon.png      (1024x1024 PNG)
  - assets/icon_64.png   (64x64 PNG, good for window icons)
  - assets/icon.ico      (Windows ICO with embedded PNG sizes)
  - assets/icon.icns     (macOS ICNS; requires `iconutil`)

This script is deterministic and uses a tiny built-in PNG encoder (no Pillow).
"""

from __future__ import annotations

import math
import shutil
import struct
import subprocess
import zlib
from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class Rgba:
    r: int
    g: int
    b: int
    a: int = 255

    def bytes(self) -> bytes:
        return bytes((self.r, self.g, self.b, self.a))


def write_png(path: Path, width: int, height: int, rgba_bytes: bytes) -> None:
    assert len(rgba_bytes) == width * height * 4

    def chunk(tag: bytes, data: bytes) -> bytes:
        return (
            struct.pack(">I", len(data))
            + tag
            + data
            + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)
        )

    raw = b"".join(
        b"\x00" + rgba_bytes[y * width * 4 : (y + 1) * width * 4] for y in range(height)
    )
    compressed = zlib.compress(raw, level=9)
    ihdr = struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0)

    path.parent.mkdir(parents=True, exist_ok=True)
    with open(path, "wb") as f:
        f.write(b"\x89PNG\r\n\x1a\n")
        f.write(chunk(b"IHDR", ihdr))
        f.write(chunk(b"IDAT", compressed))
        f.write(chunk(b"IEND", b""))


def scale_square_image(src_bytes: bytes, src_size: int, dst_size: int) -> bytes:
    assert len(src_bytes) == src_size * src_size * 4
    if dst_size == src_size:
        return src_bytes
    if dst_size > src_size:
        factor = dst_size // src_size
        assert src_size * factor == dst_size
        out = bytearray(dst_size * dst_size * 4)
        for sy in range(src_size):
            for sx in range(src_size):
                i = (sy * src_size + sx) * 4
                px = src_bytes[i : i + 4]
                for oy in range(factor):
                    dy = sy * factor + oy
                    row_start = (dy * dst_size + sx * factor) * 4
                    for ox in range(factor):
                        j = row_start + ox * 4
                        out[j : j + 4] = px
        return bytes(out)
    factor = src_size // dst_size
    assert dst_size * factor == src_size
    out = bytearray(dst_size * dst_size * 4)
    sample = factor // 2
    for dy in range(dst_size):
        sy = dy * factor + sample
        for dx in range(dst_size):
            sx = dx * factor + sample
            i = (sy * src_size + sx) * 4
            j = (dy * dst_size + dx) * 4
            out[j : j + 4] = src_bytes[i : i + 4]
    return bytes(out)


def flatten(grid: list[list[Rgba]]) -> bytes:
    out = bytearray()
    for row in grid:
        for px in row:
            out += px.bytes()
    return bytes(out)


def render_base_pixels() -> tuple[int, bytes]:
    base = 32

    def c(r: int, g: int, b: int) -> Rgba:
        return Rgba(r, g, b, 255)

    bg0 = c(10, 12, 24)
    bg1 = c(14, 18, 34)
    bg2 = c(18, 26, 48)
    border = c(4, 5, 10)
    star1 = c(220, 232, 255)
    star2 = c(170, 190, 230)

    stone0 = c(62, 66, 78)
    stone1 = c(96, 102, 118)
    stone2 = c(134, 140, 160)
    stone3 = c(188, 194, 208)
    stone_outline = c(20, 22, 30)

    teal_dark = c(14, 118, 112)
    teal = c(34, 190, 172)
    teal_light = c(84, 232, 214)
    teal_outline = c(8, 44, 46)

    pixels: list[list[Rgba]] = [[bg0 for _ in range(base)] for _ in range(base)]

    for y in range(base):
        for x in range(base):
            t = (x + y) / (2 * (base - 1))
            if t < 0.33:
                pixels[y][x] = bg2
            elif t < 0.66:
                pixels[y][x] = bg1
            else:
                pixels[y][x] = bg0

    for x in range(base):
        pixels[0][x] = border
        pixels[base - 1][x] = border
    for y in range(base):
        pixels[y][0] = border
        pixels[y][base - 1] = border

    stars = [
        (4, 5, 0),
        (10, 3, 1),
        (26, 6, 0),
        (28, 12, 1),
        (6, 22, 1),
        (13, 26, 0),
        (22, 25, 1),
        (2, 15, 0),
    ]
    for x, y, kind in stars:
        if 0 < x < base - 1 and 0 < y < base - 1:
            pixels[y][x] = star1 if kind == 0 else star2
            if kind == 0:
                for dx, dy in [(1, 0), (-1, 0), (0, 1), (0, -1)]:
                    xx, yy = x + dx, y + dy
                    if 0 < xx < base - 1 and 0 < yy < base - 1:
                        pixels[yy][xx] = star2

    cx, cy = 16, 17
    r = 11
    lx, ly, lz = -0.6, -0.8, 0.4
    ln = math.sqrt(lx * lx + ly * ly + lz * lz)
    lx, ly, lz = lx / ln, ly / ln, lz / ln

    inside: list[list[bool]] = [[False for _ in range(base)] for _ in range(base)]
    for y in range(1, base - 1):
        for x in range(1, base - 1):
            dx = (x - cx) / r
            dy = (y - cy) / r
            d2 = dx * dx + dy * dy
            if d2 <= 1.0:
                inside[y][x] = True
                nz = math.sqrt(max(0.0, 1.0 - d2))
                dot = max(0.0, dx * lx + dy * ly + nz * lz)
                dot = dot * (0.65 + 0.35 * nz)
                if dot > 0.74:
                    pixels[y][x] = stone3
                elif dot > 0.50:
                    pixels[y][x] = stone2
                elif dot > 0.26:
                    pixels[y][x] = stone1
                else:
                    pixels[y][x] = stone0

    for ccx, ccy, cr in [(13, 15, 2), (20, 20, 3), (11, 22, 2)]:
        for y in range(ccy - cr - 1, ccy + cr + 2):
            for x in range(ccx - cr - 1, ccx + cr + 2):
                if not (0 <= x < base and 0 <= y < base):
                    continue
                if not inside[y][x]:
                    continue
                dd = (x - ccx) * (x - ccx) + (y - ccy) * (y - ccy)
                if dd <= cr * cr:
                    pixels[y][x] = stone0
                if dd == cr * cr:
                    pixels[y][x] = stone1
        hx, hy = ccx - 1, ccy - 1
        if 0 <= hx < base and 0 <= hy < base and inside[hy][hx]:
            pixels[hy][hx] = stone2

    a = 6.8
    b = 6.8
    thick = 2.1
    letter: list[list[bool]] = [[False for _ in range(base)] for _ in range(base)]
    for y in range(1, base - 1):
        for x in range(1, base - 1):
            if not inside[y][x]:
                continue
            dx = x - cx
            dy = y - cy
            if abs(dx) > 9 or abs(dy) > 9:
                continue
            outer = (dx / a) ** 2 + (dy / b) ** 2 <= 1.0
            inner_a = max(1.0, a - thick)
            inner_b = max(1.0, b - thick)
            inner = (dx / inner_a) ** 2 + (dy / inner_b) ** 2 <= 1.0
            ring = outer and not inner
            if ring and dx >= 2 and dy <= -2:
                ring = False
            fill = ring
            if -1 <= dy <= 0 and dx >= 0 and (dx / a) ** 2 + (dy / b) ** 2 <= 1.0:
                fill = True
            if fill:
                letter[y][x] = True

    for y in range(1, base - 1):
        for x in range(1, base - 1):
            if not letter[y][x]:
                continue
            for oy in (-1, 0, 1):
                for ox in (-1, 0, 1):
                    if ox == 0 and oy == 0:
                        continue
                    xx, yy = x + ox, y + oy
                    if (
                        0 <= xx < base
                        and 0 <= yy < base
                        and inside[yy][xx]
                        and not letter[yy][xx]
                    ):
                        pixels[yy][xx] = teal_outline

    for y in range(1, base - 1):
        for x in range(1, base - 1):
            if not letter[y][x]:
                continue
            dx = x - cx
            dy = y - cy
            if dx + dy < 0:
                pixels[y][x] = teal_light
            elif dx - dy > 2:
                pixels[y][x] = teal_dark
            else:
                pixels[y][x] = teal

    for y in range(1, base - 1):
        for x in range(1, base - 1):
            if not inside[y][x]:
                continue
            if (
                not inside[y][x + 1]
                or not inside[y][x - 1]
                or not inside[y + 1][x]
                or not inside[y - 1][x]
            ):
                pixels[y][x] = stone_outline

    for x, y in [(12, 9), (14, 8), (16, 8), (10, 11)]:
        if 0 <= x < base and 0 <= y < base and inside[y][x]:
            pixels[y][x] = stone3

    return base, flatten(pixels)


def write_ico(path: Path, size_to_png_bytes: dict[int, bytes]) -> None:
    sizes = sorted(size_to_png_bytes.keys())
    header = struct.pack("<HHH", 0, 1, len(sizes))
    entries: list[bytes] = []
    offset = 6 + 16 * len(sizes)
    payloads: list[bytes] = []

    for size in sizes:
        data = size_to_png_bytes[size]
        w = 0 if size == 256 else size
        h = 0 if size == 256 else size
        entries.append(struct.pack("<BBBBHHII", w, h, 0, 0, 1, 32, len(data), offset))
        offset += len(data)
        payloads.append(data)

    path.parent.mkdir(parents=True, exist_ok=True)
    with open(path, "wb") as f:
        f.write(header)
        for entry in entries:
            f.write(entry)
        for payload in payloads:
            f.write(payload)


def main() -> None:
    base_size, base_bytes = render_base_pixels()

    icon_1024 = scale_square_image(base_bytes, base_size, 1024)
    icon_64 = scale_square_image(base_bytes, base_size, 64)

    write_png(Path("assets/icon.png"), 1024, 1024, icon_1024)
    write_png(Path("assets/icon_64.png"), 64, 64, icon_64)

    ico_sizes = [16, 32, 64, 128, 256]
    ico_pngs: dict[int, bytes] = {}
    for size in ico_sizes:
        out = scale_square_image(base_bytes, base_size, size)
        tmp = Path(f"target/tmp_icon_{size}.png")
        write_png(tmp, size, size, out)
        ico_pngs[size] = tmp.read_bytes()

    write_ico(Path("assets/icon.ico"), ico_pngs)

    iconutil = shutil.which("iconutil")
    if not iconutil:
        raise SystemExit("`iconutil` not found; cannot build assets/icon.icns")

    iconset = Path("target/gravimera.iconset")
    if iconset.exists():
        shutil.rmtree(iconset)
    iconset.mkdir(parents=True, exist_ok=True)

    def copy_png(src: Path, dst_name: str) -> None:
        (iconset / dst_name).write_bytes(src.read_bytes())

    # Reuse the already written sizes where possible.
    copy_png(Path("target/tmp_icon_16.png"), "icon_16x16.png")
    copy_png(Path("target/tmp_icon_32.png"), "icon_16x16@2x.png")
    copy_png(Path("target/tmp_icon_32.png"), "icon_32x32.png")
    copy_png(Path("assets/icon_64.png"), "icon_32x32@2x.png")

    copy_png(Path("target/tmp_icon_128.png"), "icon_128x128.png")
    copy_png(Path("target/tmp_icon_256.png"), "icon_128x128@2x.png")
    copy_png(Path("target/tmp_icon_256.png"), "icon_256x256.png")

    tmp_512 = Path("target/tmp_icon_512.png")
    write_png(
        tmp_512,
        512,
        512,
        scale_square_image(base_bytes, base_size, 512),
    )
    copy_png(tmp_512, "icon_256x256@2x.png")
    copy_png(tmp_512, "icon_512x512.png")

    copy_png(Path("assets/icon.png"), "icon_512x512@2x.png")

    subprocess.check_call([iconutil, "-c", "icns", str(iconset), "-o", "assets/icon.icns"])

    shutil.rmtree(iconset)
    for tmp in Path("target").glob("tmp_icon_*.png"):
        tmp.unlink()

    print("Wrote assets/icon.png assets/icon_64.png assets/icon.ico assets/icon.icns")


if __name__ == "__main__":
    main()

