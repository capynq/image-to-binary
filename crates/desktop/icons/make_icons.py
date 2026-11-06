#!/usr/bin/env python3
"""Generates the original procedural Image to Binary icon set.

The artwork is drawn from signed-distance shapes with the Python standard library
only; no third-party art is used. Run as `python3 -I make_icons.py` from any directory.
"""

import math
import os
import struct
import zlib

OUTPUT_DIR = os.path.dirname(os.path.abspath(__file__))

GRID_BITS = (
    1, 0, 1, 1,
    0, 1, 1, 0,
    1, 1, 0, 1,
    0, 0, 1, 0,
)
GRID_SIZE = 4
GRID_LEFT = 0.24
GRID_PITCH = 0.17
DOT_RADIUS = 0.058

PLATE_TOP = (0x2B, 0x31, 0x3C)
PLATE_BOTTOM = (0x12, 0x16, 0x1D)
PLATE_HALF_EXTENT = 0.46
PLATE_CORNER = 0.22

COBALT = (0x3D, 0x6B, 0xFF)
COBALT_HIGHLIGHT = (0xB4, 0xC8, 0xFF)
GRAPHITE_DOT = (0x4A, 0x52, 0x60)

ICO_SIZES = (16, 32, 48, 256)
ICNS_ENTRIES = ((b"ic07", 128), (b"ic08", 256), (b"ic09", 512))
PNG_FILES = (("32x32.png", 32), ("128x128.png", 128), ("128x128@2x.png", 256), ("icon.png", 512))


def rounded_box_distance(px, py, half, corner):
    qx = abs(px) - (half - corner)
    qy = abs(py) - (half - corner)
    outside = math.hypot(max(qx, 0.0), max(qy, 0.0))
    inside = min(max(qx, qy), 0.0)
    return outside + inside - corner


def circle_distance(px, py, cx, cy, radius):
    return math.hypot(px - cx, py - cy) - radius


def coverage(distance, pixel):
    return min(max(0.5 - distance / pixel, 0.0), 1.0)


def mix(base, top, amount):
    return tuple(b + (t - b) * amount for b, t in zip(base, top))


def over(dst, src, alpha):
    out_alpha = alpha + dst[3] * (1.0 - alpha)
    if out_alpha <= 0.0:
        return (0.0, 0.0, 0.0, 0.0)
    rgb = tuple((src[i] * alpha + dst[i] * dst[3] * (1.0 - alpha)) / out_alpha for i in range(3))
    return (rgb[0], rgb[1], rgb[2], out_alpha)


def render(size):
    pixel = 1.0 / size
    rows = bytearray()
    for row in range(size):
        rows.append(0)
        for col in range(size):
            px = (col + 0.5) / size - 0.5
            py = (row + 0.5) / size - 0.5
            plate_alpha = coverage(rounded_box_distance(px, py, PLATE_HALF_EXTENT, PLATE_CORNER), pixel)
            if plate_alpha <= 0.0:
                rows.extend((0, 0, 0, 0))
                continue
            vertical = (py + 0.5)
            pixel_color = mix(PLATE_TOP, PLATE_BOTTOM, vertical)
            color = (pixel_color[0], pixel_color[1], pixel_color[2], 1.0)
            color = over((0.0, 0.0, 0.0, 0.0), color, plate_alpha)
            for index, bit in enumerate(GRID_BITS):
                cx = GRID_LEFT + (index % GRID_SIZE) * GRID_PITCH - 0.5
                cy = GRID_LEFT + (index // GRID_SIZE) * GRID_PITCH - 0.5
                dot_alpha = coverage(circle_distance(px, py, cx, cy, DOT_RADIUS), pixel)
                if dot_alpha <= 0.0:
                    continue
                if bit:
                    highlight = max(0.0, 1.0 - math.hypot(px - (cx - 0.02), py - (cy - 0.02)) / DOT_RADIUS)
                    dot_color = mix(COBALT, COBALT_HIGHLIGHT, 0.6 * highlight)
                else:
                    dot_color = GRAPHITE_DOT
                color = over(color, dot_color, dot_alpha)
            rows.extend(
                (
                    int(round(color[0])) & 0xFF,
                    int(round(color[1])) & 0xFF,
                    int(round(color[2])) & 0xFF,
                    int(round(color[3] * 255.0)) & 0xFF,
                )
            )
    return bytes(rows)


def supersampled(size):
    if size < 128:
        return render_supersampled(size, 4)
    return render(size)


def render_supersampled(size, factor):
    large = render(size * factor)
    stride = size * factor * 4
    out = bytearray()
    for row in range(size):
        out.append(0)
        for col in range(size):
            acc = [0, 0, 0, 0]
            for dy in range(factor):
                base = (row * factor + dy) * stride + col * factor * 4
                for dx in range(factor):
                    offset = base + dx * 4
                    alpha = large[offset + 3]
                    acc[0] += large[offset] * alpha
                    acc[1] += large[offset + 1] * alpha
                    acc[2] += large[offset + 2] * alpha
                    acc[3] += alpha
            count = factor * factor
            if acc[3] == 0:
                out.extend((0, 0, 0, 0))
            else:
                out.extend(
                    (
                        acc[0] // acc[3],
                        acc[1] // acc[3],
                        acc[2] // acc[3],
                        acc[3] // count,
                    )
                )
    return bytes(out)


def png_bytes(size):
    raw = supersampled(size)
    body = bytearray()
    body.extend(b"\x89PNG\r\n\x1a\n")
    body.extend(chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0)))
    body.extend(chunk(b"IDAT", zlib.compress(raw, 9)))
    body.extend(chunk(b"IEND", b""))
    return bytes(body)


def chunk(kind, data):
    crc = zlib.crc32(kind + data) & 0xFFFFFFFF
    return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", crc)


def write_ico(path):
    images = [(size, png_bytes(size)) for size in ICO_SIZES]
    header = struct.pack("<HHH", 0, 1, len(images))
    offset = 6 + 16 * len(images)
    directory = bytearray()
    payload = bytearray()
    for size, data in images:
        dimension = 0 if size >= 256 else size
        directory.extend(struct.pack("<BBBBHHII", dimension, dimension, 0, 0, 1, 32, len(data), offset))
        payload.extend(data)
        offset += len(data)
    with open(path, "wb") as handle:
        handle.write(header + bytes(directory) + bytes(payload))


def write_icns(path):
    elements = bytearray()
    for kind, size in ICNS_ENTRIES:
        data = png_bytes(size)
        elements.extend(kind + struct.pack(">I", len(data) + 8) + data)
    with open(path, "wb") as handle:
        handle.write(b"icns" + struct.pack(">I", len(elements) + 8) + bytes(elements))


def main():
    for name, size in PNG_FILES:
        with open(os.path.join(OUTPUT_DIR, name), "wb") as handle:
            handle.write(png_bytes(size))
    write_ico(os.path.join(OUTPUT_DIR, "icon.ico"))
    write_icns(os.path.join(OUTPUT_DIR, "icon.icns"))


if __name__ == "__main__":
    main()
