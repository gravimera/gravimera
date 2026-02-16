#!/usr/bin/env python3
"""
Generate Gravimera's pixel-style app icons.

Outputs:
  - assets/icon.png      (1024x1024 PNG)
  - assets/icon_64.png   (64x64 PNG, good for window icons)
  - assets/icon.ico      (Windows ICO with embedded PNG sizes)
  - assets/icon.icns     (macOS ICNS; pure Python writer)

This script is deterministic and uses a tiny built-in PNG encoder (no Pillow).
"""

from __future__ import annotations

import math
import struct
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
    base = 64

    def clamp01(v: float) -> float:
        return 0.0 if v < 0.0 else (1.0 if v > 1.0 else v)

    def lerp(a: float, b: float, t: float) -> float:
        return a + (b - a) * t

    def lerp3(a: tuple[float, float, float], b: tuple[float, float, float], t: float) -> tuple[float, float, float]:
        return (lerp(a[0], b[0], t), lerp(a[1], b[1], t), lerp(a[2], b[2], t))

    def add3(a: tuple[float, float, float], b: tuple[float, float, float]) -> tuple[float, float, float]:
        return (a[0] + b[0], a[1] + b[1], a[2] + b[2])

    def sub3(a: tuple[float, float, float], b: tuple[float, float, float]) -> tuple[float, float, float]:
        return (a[0] - b[0], a[1] - b[1], a[2] - b[2])

    def mul3(a: tuple[float, float, float], s: float) -> tuple[float, float, float]:
        return (a[0] * s, a[1] * s, a[2] * s)

    def dot3(a: tuple[float, float, float], b: tuple[float, float, float]) -> float:
        return a[0] * b[0] + a[1] * b[1] + a[2] * b[2]

    def cross3(a: tuple[float, float, float], b: tuple[float, float, float]) -> tuple[float, float, float]:
        return (
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        )

    def len3(v: tuple[float, float, float]) -> float:
        return math.sqrt(dot3(v, v))

    def norm3(v: tuple[float, float, float]) -> tuple[float, float, float]:
        l = len3(v)
        if l <= 1e-9:
            return (0.0, 0.0, 0.0)
        return (v[0] / l, v[1] / l, v[2] / l)

    def rot_x(p: tuple[float, float, float], a: float) -> tuple[float, float, float]:
        ca, sa = math.cos(a), math.sin(a)
        return (p[0], p[1] * ca - p[2] * sa, p[1] * sa + p[2] * ca)

    def rot_y(p: tuple[float, float, float], a: float) -> tuple[float, float, float]:
        ca, sa = math.cos(a), math.sin(a)
        return (p[0] * ca + p[2] * sa, p[1], -p[0] * sa + p[2] * ca)

    def sd_sphere(p: tuple[float, float, float], r: float) -> float:
        return len3(p) - r

    def sd_box(p: tuple[float, float, float], b: tuple[float, float, float]) -> float:
        qx = abs(p[0]) - b[0]
        qy = abs(p[1]) - b[1]
        qz = abs(p[2]) - b[2]
        ox = max(qx, 0.0)
        oy = max(qy, 0.0)
        oz = max(qz, 0.0)
        outside = math.sqrt(ox * ox + oy * oy + oz * oz)
        inside = min(max(qx, max(qy, qz)), 0.0)
        return outside + inside

    def sd_capped_cylinder_y(p: tuple[float, float, float], r: float, h: float) -> float:
        dx = math.sqrt(p[0] * p[0] + p[2] * p[2]) - r
        dy = abs(p[1]) - h
        ox = max(dx, 0.0)
        oy = max(dy, 0.0)
        outside = math.sqrt(ox * ox + oy * oy)
        inside = min(max(dx, dy), 0.0)
        return outside + inside

    # Cute, bright palette (sRGB-ish, stored as linear-ish floats for simple math).
    bg_a = (0.47, 0.90, 1.00)  # cyan
    bg_b = (1.00, 0.49, 0.88)  # pink
    bg_glow = (1.00, 0.96, 0.72)  # soft yellow
    outline = (0.18, 0.10, 0.30)  # deep purple
    cube_col = (0.10, 0.92, 0.80)  # mint/teal
    sphere_col = (1.00, 0.38, 0.72)  # bubblegum pink
    cyl_col = (1.00, 0.85, 0.30)  # sunny yellow

    pixels: list[list[Rgba]] = [[Rgba(0, 0, 0, 255) for _ in range(base)] for _ in range(base)]
    mat: list[list[int]] = [[0 for _ in range(base)] for _ in range(base)]

    def to_rgba(c: tuple[float, float, float]) -> Rgba:
        r = int(clamp01(c[0]) * 255 + 0.5)
        g = int(clamp01(c[1]) * 255 + 0.5)
        b = int(clamp01(c[2]) * 255 + 0.5)
        return Rgba(r, g, b, 255)

    # Background: bright gradient + warm glow + gentle vignette.
    for y in range(base):
        for x in range(base):
            u = (x + 0.5) / base
            v = (y + 0.5) / base

            t = clamp01(0.58 * u + 0.42 * (1.0 - v))
            c = lerp3(bg_a, bg_b, t)

            dx = u - 0.52
            dy = v - 0.55
            glow = math.exp(-(dx * dx + dy * dy) / 0.035)
            c = lerp3(c, bg_glow, glow * 0.70)

            # Vignette so the center pops at small sizes.
            ex = (u - 0.5) / 0.55
            ey = (v - 0.5) / 0.55
            edge = clamp01(math.sqrt(ex * ex + ey * ey))
            c = mul3(c, 1.0 - 0.14 * (edge * edge))

            pixels[y][x] = to_rgba(c)

    # Little sparkles (kept sparse so the icon doesn't get noisy at 16x16).
    sparkle = (1.0, 1.0, 1.0)
    sparkle2 = (0.95, 0.98, 1.0)
    sparkles = [
        (10, 14),
        (18, 8),
        (46, 12),
        (54, 22),
        (14, 46),
        (50, 44),
    ]
    for sx, sy in sparkles:
        if 1 <= sx < base - 1 and 1 <= sy < base - 1:
            pixels[sy][sx] = to_rgba(sparkle)
            for dx, dy in [(1, 0), (-1, 0), (0, 1), (0, -1)]:
                pixels[sy + dy][sx + dx] = to_rgba(sparkle2)

    # Soft shadow under the primitives to anchor them.
    for y in range(base):
        for x in range(base):
            u = (x + 0.5) / base
            v = (y + 0.5) / base
            dx = (u - 0.52) / 0.24
            dy = (v - 0.83) / 0.08
            d = dx * dx + dy * dy
            if d <= 1.0:
                w = (1.0 - d) ** 2
                cur = pixels[y][x]
                c = (cur.r / 255.0, cur.g / 255.0, cur.b / 255.0)
                c = mul3(c, 1.0 - 0.22 * w)
                pixels[y][x] = to_rgba(c)

    # 3D-ish primitives via a tiny SDF ray marcher (cute toy look).
    light_dir = norm3((-0.62, 0.78, 0.35))
    cam_pos = (0.0, 0.38, 3.25)
    cam_target = (0.0, 0.05, 0.0)
    cam_fwd = norm3(sub3(cam_target, cam_pos))
    cam_right = norm3(cross3(cam_fwd, (0.0, 1.0, 0.0)))
    cam_up = cross3(cam_right, cam_fwd)
    fov = 1.05

    cube_center = (0.0, -0.55, 0.35)
    cube_half = (0.70, 0.70, 0.70)
    cube_rot_y = -0.55
    cube_rot_x = 0.25

    sphere_center = (-1.00, 0.38, -0.18)
    sphere_r = 0.58

    cyl_center = (1.05, 0.35, -0.18)
    cyl_r = 0.40
    cyl_h = 0.62

    def cube_local(p: tuple[float, float, float]) -> tuple[float, float, float]:
        q = sub3(p, cube_center)
        q = rot_y(q, -cube_rot_y)
        q = rot_x(q, -cube_rot_x)
        return q

    def scene(p: tuple[float, float, float]) -> tuple[float, int]:
        # 1=cube, 2=sphere, 3=cylinder
        q = cube_local(p)
        d1 = sd_box(q, cube_half)
        d2 = sd_sphere(sub3(p, sphere_center), sphere_r)
        d3 = sd_capped_cylinder_y(sub3(p, cyl_center), cyl_r, cyl_h)

        d = d1
        m = 1
        if d2 < d:
            d, m = d2, 2
        if d3 < d:
            d, m = d3, 3
        return d, m

    def scene_dist(p: tuple[float, float, float]) -> float:
        return min(
            sd_box(cube_local(p), cube_half),
            sd_sphere(sub3(p, sphere_center), sphere_r),
            sd_capped_cylinder_y(sub3(p, cyl_center), cyl_r, cyl_h),
        )

    def shade_quantize(v: float) -> float:
        if v < 0.45:
            return 0.42
        if v < 0.63:
            return 0.58
        if v < 0.80:
            return 0.76
        return 0.92

    def soft_shadow(p: tuple[float, float, float], n: tuple[float, float, float]) -> float:
        # A few cheap steps are enough for an icon; keeps it cute/toony.
        t = 0.035
        for _ in range(18):
            h = scene_dist(add3(p, mul3(light_dir, t)))
            if h < 0.004:
                return 0.55
            t += h
            if t > 3.0:
                break
        return 1.0

    def apply_cute_face(
        base_c: tuple[float, float, float],
        p_world: tuple[float, float, float],
        n_world: tuple[float, float, float],
        mat_id: int,
    ) -> tuple[float, float, float]:
        if mat_id != 1:
            return base_c

        p = cube_local(p_world)

        # Only draw the face on the "front" local +Z face.
        if abs(p[2] - cube_half[2]) > 0.065:
            return base_c

        u = (p[0] / cube_half[0] + 1.0) * 0.5  # 0..1
        v = (p[1] / cube_half[1] + 1.0) * 0.5  # 0..1 (bottom..top)
        if not (0.0 <= u <= 1.0 and 0.0 <= v <= 1.0):
            return base_c

        # Keep face features away from the outline.
        if u < 0.10 or u > 0.90 or v < 0.12 or v > 0.90:
            return base_c

        # Eyes.
        eye_white = (0.98, 0.99, 1.00)
        pupil = (0.14, 0.08, 0.24)
        blush = (1.00, 0.63, 0.80)

        def ellipse(px: float, py: float, cx: float, cy: float, rx: float, ry: float) -> bool:
            dx = (px - cx) / rx
            dy = (py - cy) / ry
            return dx * dx + dy * dy <= 1.0

        # Slightly larger eyes reads better at small sizes.
        for cx in (0.36, 0.64):
            if ellipse(u, v, cx, 0.66, 0.08, 0.11):
                # Iris/pupil
                if ellipse(u, v, cx + 0.012, 0.64, 0.035, 0.055):
                    return pupil
                # Highlight
                if ellipse(u, v, cx - 0.020, 0.70, 0.018, 0.022):
                    return (1.0, 1.0, 1.0)
                return eye_white

        # Blush dots.
        if ellipse(u, v, 0.22, 0.50, 0.055, 0.040) or ellipse(u, v, 0.78, 0.50, 0.055, 0.040):
            return lerp3(base_c, blush, 0.65)

        # Small smile.
        du = u - 0.50
        dv = v - 0.38
        curve = 0.10 * (du * du)  # gentle upward curve
        if abs(dv - curve) < 0.018 and abs(du) < 0.16 and v < 0.46:
            return pupil

        # Tiny "nose" dot helps the face read at 16x16.
        if (du * du + (v - 0.45) * (v - 0.45)) < 0.0065:
            return lerp3(base_c, pupil, 0.35)

        return base_c

    for y in range(base):
        for x in range(base):
            sx = ((x + 0.5) / base) * 2.0 - 1.0
            sy = 1.0 - ((y + 0.5) / base) * 2.0
            rd = norm3(
                add3(
                    add3(mul3(cam_right, sx * fov), mul3(cam_up, sy * fov)),
                    cam_fwd,
                )
            )

            t = 0.0
            hit_m = 0
            hit_p = (0.0, 0.0, 0.0)
            for _ in range(84):
                p = add3(cam_pos, mul3(rd, t))
                d, m = scene(p)
                if d < 0.002:
                    hit_m = m
                    hit_p = p
                    break
                t += d
                if t > 7.0:
                    break

            if hit_m == 0:
                continue

            e = 0.0025
            nx = scene_dist(add3(hit_p, (e, 0.0, 0.0))) - scene_dist(add3(hit_p, (-e, 0.0, 0.0)))
            ny = scene_dist(add3(hit_p, (0.0, e, 0.0))) - scene_dist(add3(hit_p, (0.0, -e, 0.0)))
            nz = scene_dist(add3(hit_p, (0.0, 0.0, e))) - scene_dist(add3(hit_p, (0.0, 0.0, -e)))
            n = norm3((nx, ny, nz))

            base_c = cube_col if hit_m == 1 else (sphere_col if hit_m == 2 else cyl_col)

            ndl = max(0.0, dot3(n, light_dir))
            ambient = 0.34
            raw = ambient + 0.92 * ndl
            raw *= soft_shadow(hit_p, n)
            shade = shade_quantize(raw)

            view_dir = mul3(rd, -1.0)
            half_v = norm3(add3(light_dir, view_dir))
            spec = pow(max(0.0, dot3(n, half_v)), 80.0) * 0.55
            rim = pow(1.0 - max(0.0, dot3(n, view_dir)), 2.0) * 0.10

            c = add3(mul3(base_c, shade), (spec + rim, spec + rim, spec + rim))
            c = apply_cute_face(c, hit_p, n, hit_m)
            pixels[y][x] = to_rgba(c)
            mat[y][x] = hit_m

    # Outline pass for readability at 16x16.
    for y in range(1, base - 1):
        for x in range(1, base - 1):
            m = mat[y][x]
            if m == 0:
                continue
            if (
                mat[y][x - 1] != m
                or mat[y][x + 1] != m
                or mat[y - 1][x] != m
                or mat[y + 1][x] != m
            ):
                cur = pixels[y][x]
                c = (cur.r / 255.0, cur.g / 255.0, cur.b / 255.0)
                c = lerp3(c, outline, 0.55)
                pixels[y][x] = to_rgba(c)

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


def write_icns(path: Path, size_to_png_bytes: dict[int, bytes]) -> None:
    # https://en.wikipedia.org/wiki/Apple_Icon_Image_format
    # Modern ICNS stores PNG payloads in type-tagged chunks.
    tag_for_size: dict[int, bytes] = {
        16: b"icp4",
        32: b"icp5",
        64: b"icp6",
        128: b"ic07",
        256: b"ic08",
        512: b"ic09",
        1024: b"ic10",
    }

    chunks: list[bytes] = []
    for size in sorted(size_to_png_bytes.keys()):
        tag = tag_for_size.get(size)
        if not tag:
            continue
        data = size_to_png_bytes[size]
        chunks.append(tag + struct.pack(">I", 8 + len(data)) + data)

    payload = b"".join(chunks)
    header = b"icns" + struct.pack(">I", 8 + len(payload))

    path.parent.mkdir(parents=True, exist_ok=True)
    with open(path, "wb") as f:
        f.write(header)
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

    icns_sizes = [16, 32, 64, 128, 256, 512, 1024]
    icns_pngs: dict[int, bytes] = {}
    for size in icns_sizes:
        if size in ico_pngs:
            icns_pngs[size] = ico_pngs[size]
            continue
        if size == 64:
            icns_pngs[size] = Path("assets/icon_64.png").read_bytes()
            continue
        if size == 1024:
            icns_pngs[size] = Path("assets/icon.png").read_bytes()
            continue
        if size == 512:
            tmp_512 = Path("target/tmp_icon_512.png")
            write_png(tmp_512, 512, 512, scale_square_image(base_bytes, base_size, 512))
            icns_pngs[size] = tmp_512.read_bytes()
            continue
        raise RuntimeError(f"Missing ICNS PNG for size {size}")

    write_icns(Path("assets/icon.icns"), icns_pngs)

    for tmp in Path("target").glob("tmp_icon_*.png"):
        tmp.unlink()

    print("Wrote assets/icon.png assets/icon_64.png assets/icon.ico assets/icon.icns")


if __name__ == "__main__":
    main()

