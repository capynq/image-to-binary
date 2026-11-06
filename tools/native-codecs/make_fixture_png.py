"""Writes a deterministic 64x48 RGB gradient PNG (no third-party content)."""
import struct
import sys
import zlib

WIDTH = 64
HEIGHT = 48


def pixel(x, y):
    return (x * 255 // (WIDTH - 1), y * 255 // (HEIGHT - 1), 128)


def chunk(tag, data):
    body = tag + data
    return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body) & 0xFFFFFFFF)


def main(out_path):
    rows = bytearray()
    for y in range(HEIGHT):
        rows.append(0)
        for x in range(WIDTH):
            rows.extend(pixel(x, y))
    png = b"\x89PNG\r\n\x1a\n"
    png += chunk(b"IHDR", struct.pack(">IIBBBBB", WIDTH, HEIGHT, 8, 2, 0, 0, 0))
    png += chunk(b"IDAT", zlib.compress(bytes(rows), 9))
    png += chunk(b"IEND", b"")
    with open(out_path, "wb") as f:
        f.write(png)


if __name__ == "__main__":
    main(sys.argv[1])
