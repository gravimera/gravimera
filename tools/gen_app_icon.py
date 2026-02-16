#!/usr/bin/env python3
"""
Generate Gravimera's app icons (bright, cute 3D primitives).

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
from pathlib import Path


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


def resample_square_image(src_bytes: bytes, src_size: int, dst_size: int) -> bytes:
    assert len(src_bytes) == src_size * src_size * 4
    if dst_size == src_size:
        return src_bytes

    if dst_size < src_size and src_size % dst_size == 0:
        factor = src_size // dst_size
        n = factor * factor
        src = src_bytes
        out = bytearray(dst_size * dst_size * 4)
        for dy in range(dst_size):
            sy0 = dy * factor
            for dx in range(dst_size):
                sx0 = dx * factor
                sum_a = 0
                sum_rp = 0
                sum_gp = 0
                sum_bp = 0
                for oy in range(factor):
                    row = ((sy0 + oy) * src_size + sx0) * 4
                    for ox in range(factor):
                        i = row + ox * 4
                        a = src[i + 3]
                        sum_a += a
                        sum_rp += src[i] * a
                        sum_gp += src[i + 1] * a
                        sum_bp += src[i + 2] * a
                j = (dy * dst_size + dx) * 4
                out_a = (sum_a + n // 2) // n
                out[j + 3] = out_a
                if sum_a == 0:
                    out[j] = 0
                    out[j + 1] = 0
                    out[j + 2] = 0
                else:
                    out[j] = (sum_rp + sum_a // 2) // sum_a
                    out[j + 1] = (sum_gp + sum_a // 2) // sum_a
                    out[j + 2] = (sum_bp + sum_a // 2) // sum_a
        return bytes(out)

    # Bilinear (works for both upscale + arbitrary downscale).
    src = src_bytes
    out = bytearray(dst_size * dst_size * 4)
    scale = src_size / dst_size

    for dy in range(dst_size):
        fy = (dy + 0.5) * scale - 0.5
        y0 = int(math.floor(fy))
        wy = fy - y0
        if y0 < 0:
            y0 = 0
            y1 = 0
            wy = 0.0
        elif y0 >= src_size - 1:
            y0 = src_size - 1
            y1 = y0
            wy = 0.0
        else:
            y1 = y0 + 1

        for dx in range(dst_size):
            fx = (dx + 0.5) * scale - 0.5
            x0 = int(math.floor(fx))
            wx = fx - x0
            if x0 < 0:
                x0 = 0
                x1 = 0
                wx = 0.0
            elif x0 >= src_size - 1:
                x0 = src_size - 1
                x1 = x0
                wx = 0.0
            else:
                x1 = x0 + 1

            i00 = (y0 * src_size + x0) * 4
            i10 = (y0 * src_size + x1) * 4
            i01 = (y1 * src_size + x0) * 4
            i11 = (y1 * src_size + x1) * 4

            j = (dy * dst_size + dx) * 4
            w00 = (1.0 - wx) * (1.0 - wy)
            w10 = wx * (1.0 - wy)
            w01 = (1.0 - wx) * wy
            w11 = wx * wy

            a00 = src[i00 + 3]
            a10 = src[i10 + 3]
            a01 = src[i01 + 3]
            a11 = src[i11 + 3]
            a = a00 * w00 + a10 * w10 + a01 * w01 + a11 * w11

            if a <= 1e-6:
                out[j] = 0
                out[j + 1] = 0
                out[j + 2] = 0
                out[j + 3] = 0
                continue

            rp = (src[i00] * a00) * w00 + (src[i10] * a10) * w10 + (src[i01] * a01) * w01 + (src[i11] * a11) * w11
            gp = (src[i00 + 1] * a00) * w00 + (src[i10 + 1] * a10) * w10 + (src[i01 + 1] * a01) * w01 + (src[i11 + 1] * a11) * w11
            bp = (src[i00 + 2] * a00) * w00 + (src[i10 + 2] * a10) * w10 + (src[i01 + 2] * a01) * w01 + (src[i11 + 2] * a11) * w11

            out[j] = int(rp / a + 0.5)
            out[j + 1] = int(gp / a + 0.5)
            out[j + 2] = int(bp / a + 0.5)
            out[j + 3] = int(a + 0.5)

    return bytes(out)


def render_base_pixels() -> tuple[int, bytes]:
    base = 1024

    def clamp01(v: float) -> float:
        return 0.0 if v < 0.0 else (1.0 if v > 1.0 else v)

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

    def norm3(v: tuple[float, float, float]) -> tuple[float, float, float]:
        l2 = dot3(v, v)
        if l2 <= 1e-12:
            return (0.0, 0.0, 0.0)
        inv = 1.0 / math.sqrt(l2)
        return (v[0] * inv, v[1] * inv, v[2] * inv)

    def rot_x(p: tuple[float, float, float], a: float) -> tuple[float, float, float]:
        ca, sa = math.cos(a), math.sin(a)
        return (p[0], p[1] * ca - p[2] * sa, p[1] * sa + p[2] * ca)

    def rot_y(p: tuple[float, float, float], a: float) -> tuple[float, float, float]:
        ca, sa = math.cos(a), math.sin(a)
        return (p[0] * ca + p[2] * sa, p[1], -p[0] * sa + p[2] * ca)

    # Bright, cute palette.
    outline_rgb = (46, 20, 72)  # deep purple (used for edge darkening)
    outline_col = (outline_rgb[0] / 255.0, outline_rgb[1] / 255.0, outline_rgb[2] / 255.0)
    cube_col = (0.08, 0.94, 0.86)  # mint block
    sphere_col = (1.00, 0.35, 0.72)  # "AI orb"

    # Camera.
    cam_pos = (0.0, 0.95, 3.25)
    cam_target = (0.10, 0.05, 0.0)
    cam_fwd = norm3(sub3(cam_target, cam_pos))
    cam_right = norm3(cross3(cam_fwd, (0.0, 1.0, 0.0)))
    cam_up = cross3(cam_right, cam_fwd)
    fov = 0.92

    light_dir = norm3((-0.65, 0.78, 0.28))

    # Scene: a mint "block" + a floating pink orb (Gravimera = build + magic/AI).
    cube_center = (-0.10, -0.52, 0.35)
    cube_half = (0.98, 0.98, 0.98)
    cube_rot_y = -0.68
    cube_rot_x = 0.36

    sphere_center = (0.90, 0.78, -0.30)
    sphere_r = 0.60

    eps = 1e-4

    def cube_to_local_point(p: tuple[float, float, float]) -> tuple[float, float, float]:
        q = sub3(p, cube_center)
        q = rot_y(q, -cube_rot_y)
        q = rot_x(q, -cube_rot_x)
        return q

    def cube_to_local_dir(d: tuple[float, float, float]) -> tuple[float, float, float]:
        q = rot_y(d, -cube_rot_y)
        q = rot_x(q, -cube_rot_x)
        return q

    def cube_to_world_dir(d: tuple[float, float, float]) -> tuple[float, float, float]:
        q = rot_x(d, cube_rot_x)
        q = rot_y(q, cube_rot_y)
        return q

    def intersect_sphere(
        ro: tuple[float, float, float],
        rd: tuple[float, float, float],
        center: tuple[float, float, float],
        r: float,
    ) -> float | None:
        oc = sub3(ro, center)
        b = dot3(oc, rd)
        c = dot3(oc, oc) - r * r
        h = b * b - c
        if h <= 0.0:
            return None
        s = math.sqrt(h)
        t = -b - s
        if t > eps:
            return t
        t = -b + s
        if t > eps:
            return t
        return None

    def intersect_aabb_local(
        ro: tuple[float, float, float],
        rd: tuple[float, float, float],
        half: tuple[float, float, float],
    ) -> float | None:
        tmin = -1.0e20
        tmax = 1.0e20
        for o, d, h in ((ro[0], rd[0], half[0]), (ro[1], rd[1], half[1]), (ro[2], rd[2], half[2])):
            if abs(d) < 1e-9:
                if o < -h or o > h:
                    return None
                continue
            t1 = (-h - o) / d
            t2 = (h - o) / d
            if t1 > t2:
                t1, t2 = t2, t1
            if t1 > tmin:
                tmin = t1
            if t2 < tmax:
                tmax = t2
            if tmin > tmax:
                return None
        if tmax <= eps:
            return None
        return tmin if tmin > eps else tmax

    def intersect_cube(
        ro: tuple[float, float, float],
        rd: tuple[float, float, float],
    ) -> tuple[float, tuple[float, float, float], tuple[float, float, float]] | None:
        ro_l = cube_to_local_point(ro)
        rd_l = cube_to_local_dir(rd)
        t = intersect_aabb_local(ro_l, rd_l, cube_half)
        if t is None:
            return None
        hp = add3(ro_l, mul3(rd_l, t))
        ax = abs(abs(hp[0]) - cube_half[0])
        ay = abs(abs(hp[1]) - cube_half[1])
        az = abs(abs(hp[2]) - cube_half[2])
        if ax <= ay and ax <= az:
            n_l = (1.0 if hp[0] > 0.0 else -1.0, 0.0, 0.0)
        elif ay <= az:
            n_l = (0.0, 1.0 if hp[1] > 0.0 else -1.0, 0.0)
        else:
            n_l = (0.0, 0.0, 1.0 if hp[2] > 0.0 else -1.0)
        return t, hp, n_l

    def intersect_scene(
        ro: tuple[float, float, float],
        rd: tuple[float, float, float],
        t_max: float = 1.0e20,
    ) -> float | None:
        t_min = None

        t = intersect_sphere(ro, rd, sphere_center, sphere_r)
        if t is not None and t < t_max:
            t_min = t

        hit = intersect_cube(ro, rd)
        if hit is not None:
            t2 = hit[0]
            if t2 < t_max and (t_min is None or t2 < t_min):
                t_min = t2

        return t_min

    def apply_cute_face(
        col: tuple[float, float, float],
        cube_hit_local: tuple[float, float, float],
        cube_normal_local: tuple[float, float, float],
    ) -> tuple[float, float, float]:
        # Only on the local +Z face.
        if cube_normal_local[2] < 0.9:
            return col
        if abs(cube_hit_local[2] - cube_half[2]) > 0.02:
            return col

        u = (cube_hit_local[0] / cube_half[0] + 1.0) * 0.5  # 0..1
        v = (cube_hit_local[1] / cube_half[1] + 1.0) * 0.5  # 0..1
        if not (0.0 <= u <= 1.0 and 0.0 <= v <= 1.0):
            return col
        if u < 0.14 or u > 0.86 or v < 0.18 or v > 0.86:
            return col

        ink = (0.14, 0.08, 0.24)

        def ellipse(px: float, py: float, cx: float, cy: float, rx: float, ry: float) -> bool:
            dx = (px - cx) / rx
            dy = (py - cy) / ry
            return dx * dx + dy * dy <= 1.0

        for cx in (0.38, 0.62):
            if ellipse(u, v, cx, 0.66, 0.07, 0.10):
                if ellipse(u, v, cx - 0.020, 0.70, 0.018, 0.018):
                    return (1.0, 1.0, 1.0)
                return ink

        du = u - 0.50
        dv = v - 0.40
        curve = 0.10 * (du * du)
        if abs(dv - curve) < 0.016 and abs(du) < 0.16 and v < 0.48:
            return ink

        return col

    def to_u8(v: float) -> int:
        v = clamp01(v)
        # Cheap-ish gamma for a brighter, friendlier look.
        v = math.sqrt(v)
        return int(v * 255 + 0.5)

    out = bytearray(base * base * 4)  # starts transparent
    mat = bytearray(base * base)  # 0=bg, 1=cube, 2=sphere

    sx_vals = [((x + 0.5) / base) * 2.0 - 1.0 for x in range(base)]
    sy_vals = [1.0 - ((y + 0.5) / base) * 2.0 for y in range(base)]

    ambient = 0.28
    diff_k = 0.92
    spec_k = 0.55
    shininess = 90.0
    rim_k = 0.10

    for y in range(base):
        sy = sy_vals[y] * fov
        for x in range(base):
            sx = sx_vals[x] * fov
            rd = norm3(
                (
                    cam_fwd[0] + cam_right[0] * sx + cam_up[0] * sy,
                    cam_fwd[1] + cam_right[1] * sx + cam_up[1] * sy,
                    cam_fwd[2] + cam_right[2] * sx + cam_up[2] * sy,
                )
            )

            t_best = None
            hit_kind = 0
            hit_local = (0.0, 0.0, 0.0)
            hit_n_local = (0.0, 0.0, 0.0)
            hit_n_world = (0.0, 0.0, 0.0)

            t_s = intersect_sphere(cam_pos, rd, sphere_center, sphere_r)
            if t_s is not None:
                t_best = t_s
                hit_kind = 2

            cube_hit = intersect_cube(cam_pos, rd)
            if cube_hit is not None:
                t_c, hp_l, n_l = cube_hit
                if t_best is None or t_c < t_best:
                    t_best = t_c
                    hit_kind = 1
                    hit_local = hp_l
                    hit_n_local = n_l
                    hit_n_world = norm3(cube_to_world_dir(n_l))

            if t_best is None:
                continue

            hit_p = add3(cam_pos, mul3(rd, t_best))
            if hit_kind == 2:
                hit_n_world = norm3(sub3(hit_p, sphere_center))
            elif hit_kind == 1:
                # Cube normal already computed above.
                hit_n_world = hit_n_world

            base_col = cube_col if hit_kind == 1 else sphere_col

            ndl = max(0.0, dot3(hit_n_world, light_dir))
            shade = ambient + diff_k * ndl

            # Simple hard shadow for depth.
            shadow_ray_o = add3(hit_p, mul3(hit_n_world, 0.003))
            t_shadow = intersect_scene(shadow_ray_o, light_dir, t_max=10.0)
            if t_shadow is not None:
                shade *= 0.62

            view_dir = (-rd[0], -rd[1], -rd[2])
            half_v = norm3(add3(light_dir, view_dir))
            kind_spec = spec_k * (1.35 if hit_kind == 2 else 1.0)
            kind_rim = rim_k * (1.20 if hit_kind == 2 else 1.0)
            kind_shiny = shininess * (1.15 if hit_kind == 2 else 1.0)
            spec = pow(max(0.0, dot3(hit_n_world, half_v)), kind_shiny) * kind_spec
            rim = pow(1.0 - max(0.0, dot3(hit_n_world, view_dir)), 2.0) * kind_rim

            col = (
                base_col[0] * shade + spec + rim,
                base_col[1] * shade + spec + rim,
                base_col[2] * shade + spec + rim,
            )

            if hit_kind == 1:
                # Add crisp cube edge lines to make the primitive read at small sizes.
                edge_w = cube_half[0] * 0.11
                if abs(hit_n_local[0]) > 0.9:
                    edge = max(
                        abs(hit_local[1]) - (cube_half[1] - edge_w),
                        abs(hit_local[2]) - (cube_half[2] - edge_w),
                    )
                elif abs(hit_n_local[1]) > 0.9:
                    edge = max(
                        abs(hit_local[0]) - (cube_half[0] - edge_w),
                        abs(hit_local[2]) - (cube_half[2] - edge_w),
                    )
                else:
                    edge = max(
                        abs(hit_local[0]) - (cube_half[0] - edge_w),
                        abs(hit_local[1]) - (cube_half[1] - edge_w),
                    )
                if edge > 0.0:
                    w = clamp01(edge / edge_w) * 0.70
                    col = (
                        col[0] * (1.0 - w) + outline_col[0] * w,
                        col[1] * (1.0 - w) + outline_col[1] * w,
                        col[2] * (1.0 - w) + outline_col[2] * w,
                    )
                col = apply_cute_face(col, hit_local, hit_n_local)
            else:
                # Slight emissive lift for the "orb" so it pops on any background.
                col = (col[0] + 0.06, col[1] + 0.02, col[2] + 0.08)

            i = y * base + x
            j = i * 4
            out[j] = to_u8(col[0])
            out[j + 1] = to_u8(col[1])
            out[j + 2] = to_u8(col[2])
            out[j + 3] = 255
            mat[i] = hit_kind

    # Darken silhouette pixels for readability on any desktop background.
    k = 92  # blend factor in 0..255 (about 36%)
    inv = 255 - k
    for y in range(1, base - 1):
        row = y * base
        for x in range(1, base - 1):
            i = row + x
            if mat[i] == 0:
                continue
            if (
                mat[i - 1] != mat[i]
                or mat[i + 1] != mat[i]
                or mat[i - base] != mat[i]
                or mat[i + base] != mat[i]
            ):
                j = i * 4
                out[j] = (out[j] * inv + outline_rgb[0] * k) // 255
                out[j + 1] = (out[j + 1] * inv + outline_rgb[1] * k) // 255
                out[j + 2] = (out[j + 2] * inv + outline_rgb[2] * k) // 255

    return base, bytes(out)


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

    icon_1024 = resample_square_image(base_bytes, base_size, 1024)
    icon_64 = resample_square_image(base_bytes, base_size, 64)

    write_png(Path("assets/icon.png"), 1024, 1024, icon_1024)
    write_png(Path("assets/icon_64.png"), 64, 64, icon_64)

    ico_sizes = [16, 32, 64, 128, 256]
    ico_pngs: dict[int, bytes] = {}
    for size in ico_sizes:
        out = resample_square_image(base_bytes, base_size, size)
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
            write_png(
                tmp_512,
                512,
                512,
                resample_square_image(base_bytes, base_size, 512),
            )
            icns_pngs[size] = tmp_512.read_bytes()
            continue
        raise RuntimeError(f"Missing ICNS PNG for size {size}")

    write_icns(Path("assets/icon.icns"), icns_pngs)

    for tmp in Path("target").glob("tmp_icon_*.png"):
        tmp.unlink()

    print("Wrote assets/icon.png assets/icon_64.png assets/icon.ico assets/icon.icns")


if __name__ == "__main__":
    main()

