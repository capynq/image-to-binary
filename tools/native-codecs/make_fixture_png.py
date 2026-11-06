"""Writes a deterministic 64x48 gradient PNG (no third-party content).

Usage: python3 -I make_fixture_png.py OUT.png [--alpha | --grey | --inverted]

The default is RGB. --alpha adds an alpha channel falling from 255 at the left
edge to 127 at the right edge. --grey writes an 8-bit grayscale PNG (colour type
0). --inverted writes an RGB gradient whose red and green run opposite to the
default, so the same geometry has visibly different content.
"""
import struct
import sys
import zlib

WIDTH = 64
HEIGHT = 48


def pixel(x, y, alpha, inverted):
    red = x * 255 // (WIDTH - 1)
    green = y * 255 // (HEIGHT - 1)
    if inverted:
        red = 255 - red
        green = 255 - green
    rgb = (red, green, 128)
    if not alpha:
        return rgb
    return rgb + (255 - (x * 128 // (WIDTH - 1)),)


def grey(x, y):
    return (x * 255 // (WIDTH - 1) + y * 255 // (HEIGHT - 1)) // 2


def chunk(tag, data):
    body = tag + data
    return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body) & 0xFFFFFFFF)


def main(argv):
    out_path = argv[0]
    options = set(argv[1:])
    alpha = "--alpha" in options
    is_grey = "--grey" in options
    inverted = "--inverted" in options
    if is_grey:
        colour_type = 0
    elif alpha:
        colour_type = 6
    else:
        colour_type = 2
    rows = bytearray()
    for y in range(HEIGHT):
        rows.append(0)
        for x in range(WIDTH):
            if is_grey:
                rows.append(grey(x, y))
            else:
                rows.extend(pixel(x, y, alpha, inverted))
    png = b"\x89PNG\r\n\x1a\n"
    png += chunk(b"IHDR", struct.pack(">IIBBBBB", WIDTH, HEIGHT, 8, colour_type, 0, 0, 0))
    png += chunk(b"IDAT", zlib.compress(bytes(rows), 9))
    png += chunk(b"IEND", b"")
    with open(out_path, "wb") as f:
        f.write(png)


if __name__ == "__main__":
    main(sys.argv[1:])
