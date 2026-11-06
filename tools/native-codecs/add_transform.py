"""Adds an irot and/or imir transform to the primary item of a HEIF/AVIF file.

Usage: python3 -I add_transform.py INPUT OUTPUT [--rotate {0,90,180,270}]
       [--mirror-axis {0,1}]

The rotation is anticlockwise, as in ISO/IEC 23008-12. The mirror axis is the
raw imir axis field; libheif maps axis 0 to a top-bottom exchange and axis 1 to
a left-right exchange, which the decoder tests verify. Existing irot or imir
associations of the primary item are removed, because libheif rejects more than
one transform per item; the removed boxes stay in ipco unassociated. Each added
property is marked essential and associated with the primary item in ipma.
Offsets of items stored after the edited meta box are shifted by the size
change, so the output decodes to the same pixels with the transform applied.
"""

import argparse
import struct
import sys
from dataclasses import dataclass

FULL_BOX_HEADER = 4
ESSENTIAL = 0x80


class PatchError(Exception):
    pass


@dataclass(frozen=True)
class Box:
    kind: bytes
    start: int
    size: int
    header: int

    @property
    def end(self):
        return self.start + self.size

    @property
    def body_start(self):
        return self.start + self.header


def read_boxes(data, start, end):
    boxes = []
    position = start
    while position < end:
        if end - position < 8:
            raise PatchError(f"truncated box header at {position}")
        size, kind = struct.unpack_from(">I4s", data, position)
        header = 8
        if size == 1:
            if end - position < 16:
                raise PatchError(f"truncated largesize header at {position}")
            size = struct.unpack_from(">Q", data, position + 8)[0]
            header = 16
        elif size == 0:
            size = end - position
        if size < header or position + size > end:
            raise PatchError(f"box {kind!r} at {position} overruns its parent")
        boxes.append(Box(kind, position, size, header))
        position += size
    return boxes


def only(boxes, kind):
    matches = [box for box in boxes if box.kind == kind]
    if len(matches) != 1:
        raise PatchError(f"expected exactly one {kind!r} box, found {len(matches)}")
    return matches[0]


def make_box(kind, payload):
    size = 8 + len(payload)
    if size > 0xFFFFFFFF:
        raise PatchError("rebuilt box exceeds 32-bit size")
    return struct.pack(">I4s", size, kind) + payload


def raw(data, box):
    return bytes(data[box.start : box.end])


def body(data, box):
    return bytes(data[box.body_start : box.end])


def need(data, position, size, what):
    if position < 0 or position + size > len(data):
        raise PatchError(f"truncated input: {what} at {position} needs {size} bytes")


def byte_at(data, position, what):
    need(data, position, 1, what)
    return data[position]


def unpack(fmt, data, position, what):
    need(data, position, struct.calcsize(fmt), what)
    return struct.unpack_from(fmt, data, position)[0]


def primary_item_id(data, meta_children):
    pitm = only(meta_children, b"pitm")
    payload = body(data, pitm)
    version = byte_at(payload, 0, "pitm version")
    if version == 0:
        return unpack(">H", payload, FULL_BOX_HEADER, "pitm item id")
    return unpack(">I", payload, FULL_BOX_HEADER, "pitm item id")


def parse_ipma(payload):
    version = byte_at(payload, 0, "ipma version")
    flags = int.from_bytes(payload[1:4], "big")
    wide = bool(flags & 1)
    count = unpack(">I", payload, 4, "ipma entry count")
    position = 8
    entries = []
    for _ in range(count):
        if version < 1:
            item_id = unpack(">H", payload, position, "ipma item id")
            position += 2
        else:
            item_id = unpack(">I", payload, position, "ipma item id")
            position += 4
        association_count = byte_at(payload, position, "ipma association count")
        position += 1
        associations = []
        for _ in range(association_count):
            if wide:
                value = unpack(">H", payload, position, "ipma association")
                position += 2
            else:
                value = byte_at(payload, position, "ipma association")
                position += 1
            associations.append(value)
        entries.append([item_id, associations])
    return version, wide, entries


def serialize_ipma(version, wide, entries):
    out = bytearray(bytes([version]) + (1 if wide else 0).to_bytes(3, "big"))
    out += struct.pack(">I", len(entries))
    for item_id, associations in entries:
        if version < 1:
            out += struct.pack(">H", item_id)
        else:
            out += struct.pack(">I", item_id)
        out.append(len(associations))
        for value in associations:
            if wide:
                out += struct.pack(">H", value)
            else:
                out.append(value)
    return bytes(out)


def transform_properties(rotate, mirror_axis):
    properties = []
    if rotate:
        if rotate not in (90, 180, 270):
            raise PatchError("rotation must be 0, 90, 180, or 270")
        properties.append((b"irot", bytes([(rotate // 90) & 0x3])))
    if mirror_axis is not None:
        if mirror_axis not in (0, 1):
            raise PatchError("mirror axis must be 0 or 1")
        properties.append((b"imir", bytes([mirror_axis & 0x1])))
    if not properties:
        raise PatchError("at least one of --rotate or --mirror-axis is required")
    return properties


def patch_ipma_for_primary(ipma_payload, primary, first_index, count, transform_indices):
    version, wide, entries = parse_ipma(ipma_payload)
    limit = 0x7FFF if wide else 0x7F
    if first_index + count - 1 > limit:
        raise PatchError("property index does not fit the ipma association width")
    for entry in entries:
        if entry[0] == primary:
            index_mask = 0x7FFF if wide else 0x7F
            kept = [value for value in entry[1] if (value & index_mask) not in transform_indices]
            kept.extend(
                ESSENTIAL | index for index in range(first_index, first_index + count)
            )
            entry[1] = kept
            return serialize_ipma(version, wide, entries)
    raise PatchError("the primary item has no ipma entry")


def shift_item_offsets(out, meta_start, meta_end, old_meta_end, delta):
    meta_children = read_boxes(out, meta_start + 12, meta_end)
    iloc = only(meta_children, b"iloc")
    position = iloc.body_start + FULL_BOX_HEADER
    version = byte_at(out, iloc.body_start, "iloc version")
    sizes = byte_at(out, position, "iloc offset and length sizes")
    offset_size, length_size = sizes >> 4, sizes & 0xF
    position += 1
    sizes = byte_at(out, position, "iloc base and index sizes")
    base_size, index_size = sizes >> 4, (sizes & 0xF) if version in (1, 2) else 0
    position += 1
    if version < 2:
        item_count = unpack(">H", out, position, "iloc item count")
        position += 2
    else:
        item_count = unpack(">I", out, position, "iloc item count")
        position += 4
    for _ in range(item_count):
        if version < 2:
            position += 2
        else:
            position += 4
        construction = 0
        if version in (1, 2):
            construction = unpack(">H", out, position, "iloc construction method") & 0xF
            position += 2
        position += 2
        base_position = position
        base = read_uint(out, position, base_size)
        position += base_size
        extent_count = unpack(">H", out, position, "iloc extent count")
        position += 2
        extents = []
        for _ in range(extent_count):
            position += index_size
            extents.append((position, read_uint(out, position, offset_size)))
            position += offset_size + length_size
        if construction != 0:
            continue
        above = [base + value >= old_meta_end for _, value in extents if value or base]
        if not above:
            continue
        if any(above) and not all(above):
            raise PatchError("an item's extents straddle the edited meta box")
        if not any(above):
            continue
        if base_size:
            write_uint(out, base_position, base_size, base + delta)
        else:
            for extent_position, value in extents:
                write_uint(out, extent_position, offset_size, value + delta)


def read_uint(data, position, size):
    if size == 0:
        return 0
    need(data, position, size, "integer field")
    return int.from_bytes(data[position : position + size], "big")


def write_uint(data, position, size, value):
    if value >> (8 * size):
        raise PatchError("shifted offset does not fit its field")
    data[position : position + size] = value.to_bytes(size, "big")


def add_transform(data, rotate, mirror_axis):
    data = bytes(data)
    top = read_boxes(data, 0, len(data))
    meta = only(top, b"meta")
    if not any(box.kind == b"mdat" for box in top):
        raise PatchError("expected an mdat box after meta; the input is truncated")
    if meta.header != 8:
        raise PatchError("largesize meta boxes are not supported")
    meta_body = meta.body_start + FULL_BOX_HEADER
    meta_children = read_boxes(data, meta_body, meta.end)
    primary = primary_item_id(data, meta_children)
    iprp = only(meta_children, b"iprp")
    iprp_children = read_boxes(data, iprp.body_start, iprp.end)
    ipco = only(iprp_children, b"ipco")
    ipma = only(iprp_children, b"ipma")

    existing_boxes = read_boxes(data, ipco.body_start, ipco.end)
    existing = len(existing_boxes)
    transform_indices = {
        index
        for index, box in enumerate(existing_boxes, start=1)
        if box.kind in (b"irot", b"imir")
    }
    properties = transform_properties(rotate, mirror_axis)
    new_payload = body(data, ipco) + b"".join(
        make_box(kind, payload) for kind, payload in properties
    )
    new_ipco = make_box(b"ipco", new_payload)
    new_ipma = make_box(
        b"ipma",
        patch_ipma_for_primary(
            body(data, ipma), primary, existing + 1, len(properties), transform_indices
        ),
    )

    rebuilt_iprp_children = []
    for child in iprp_children:
        if child.kind == b"ipco":
            rebuilt_iprp_children.append(new_ipco)
        elif child.kind == b"ipma":
            rebuilt_iprp_children.append(new_ipma)
        else:
            rebuilt_iprp_children.append(raw(data, child))
    new_iprp = make_box(b"iprp", b"".join(rebuilt_iprp_children))

    rebuilt_meta_children = []
    for child in meta_children:
        rebuilt_meta_children.append(new_iprp if child.kind == b"iprp" else raw(data, child))
    new_meta = make_box(
        b"meta",
        data[meta.body_start : meta_body] + b"".join(rebuilt_meta_children),
    )

    out = bytearray(data[: meta.start] + new_meta + data[meta.end :])
    delta = len(new_meta) - meta.size
    shift_item_offsets(out, meta.start, meta.start + len(new_meta), meta.end, delta)
    return bytes(out)


def parse_iloc(data, box):
    payload = body(data, box)
    version = byte_at(payload, 0, "iloc version")
    sizes = byte_at(payload, 4, "iloc offset and length sizes")
    offset_size, length_size = sizes >> 4, sizes & 0xF
    sizes = byte_at(payload, 5, "iloc base and index sizes")
    base_size = sizes >> 4
    index_size = (sizes & 0xF) if version in (1, 2) else 0
    position = 6
    if version < 2:
        count = unpack(">H", payload, position, "iloc item count")
        position += 2
    else:
        count = unpack(">I", payload, position, "iloc item count")
        position += 4
    items = []
    for _ in range(count):
        if version < 2:
            item_id = unpack(">H", payload, position, "iloc item id")
            position += 2
        else:
            item_id = unpack(">I", payload, position, "iloc item id")
            position += 4
        construction = 0
        if version in (1, 2):
            construction = unpack(">H", payload, position, "iloc construction method") & 0xF
            position += 2
        data_reference = unpack(">H", payload, position, "iloc data reference index")
        position += 2
        base = read_uint(payload, position, base_size)
        position += base_size
        extent_count = unpack(">H", payload, position, "iloc extent count")
        position += 2
        extents = []
        for _ in range(extent_count):
            index = read_uint(payload, position, index_size)
            position += index_size
            offset = read_uint(payload, position, offset_size)
            position += offset_size
            length = read_uint(payload, position, length_size)
            position += length_size
            extents.append((index, offset, length))
        items.append(
            {
                "id": item_id,
                "construction": construction,
                "data_reference": data_reference,
                "base": base,
                "extents": extents,
            }
        )
    layout = {
        "version": version,
        "offset_size": offset_size,
        "length_size": length_size,
        "base_size": base_size,
        "index_size": index_size,
    }
    return layout, items


def uint_bytes(value, size):
    if size == 0:
        if value:
            raise PatchError("value does not fit a zero-width field")
        return b""
    if value >> (8 * size):
        raise PatchError("value does not fit its field")
    return value.to_bytes(size, "big")


def serialize_iloc(layout, items):
    version = layout["version"]
    out = bytearray(bytes([version]) + b"\0\0\0")
    out.append((layout["offset_size"] << 4) | layout["length_size"])
    out.append((layout["base_size"] << 4) | (layout["index_size"] if version in (1, 2) else 0))
    if version < 2:
        out += struct.pack(">H", len(items))
    else:
        out += struct.pack(">I", len(items))
    for item in items:
        if version < 2:
            out += struct.pack(">H", item["id"])
        else:
            out += struct.pack(">I", item["id"])
        if version in (1, 2):
            out += struct.pack(">H", item["construction"] & 0xF)
        out += struct.pack(">H", item["data_reference"])
        out += uint_bytes(item["base"], layout["base_size"])
        out += struct.pack(">H", len(item["extents"]))
        for index, offset, length in item["extents"]:
            if version in (1, 2) and layout["index_size"]:
                out += uint_bytes(index, layout["index_size"])
            out += uint_bytes(offset, layout["offset_size"])
            out += uint_bytes(length, layout["length_size"])
    return bytes(out)


def combine_primary(first, second, primary):
    if primary not in (1, 2):
        raise PatchError("primary must be item 1 or item 2")
    top_first = read_boxes(first, 0, len(first))
    top_second = read_boxes(second, 0, len(second))
    meta_first = only(top_first, b"meta")
    meta_second = only(top_second, b"meta")
    mdat_second = only(top_second, b"mdat")
    if meta_first.header != 8 or meta_second.header != 8:
        raise PatchError("largesize meta boxes are not supported")
    kids_first = read_boxes(first, meta_first.body_start + FULL_BOX_HEADER, meta_first.end)
    kids_second = read_boxes(second, meta_second.body_start + FULL_BOX_HEADER, meta_second.end)
    allowed = {b"hdlr", b"dinf", b"pitm", b"iinf", b"iprp", b"iloc"}
    if {box.kind for box in kids_first} - allowed or {box.kind for box in kids_second} - allowed:
        raise PatchError("unsupported meta child; expected hdlr, dinf, pitm, iinf, iprp, iloc")
    if primary_item_id(first, kids_first) != 1 or primary_item_id(second, kids_second) != 1:
        raise PatchError("both inputs must be single-image files whose primary item is 1")

    iinf_first = only(kids_first, b"iinf")
    iinf_second = only(kids_second, b"iinf")
    iinf_versions = (
        byte_at(first, iinf_first.body_start, "iinf version"),
        byte_at(second, iinf_second.body_start, "iinf version"),
    )
    if iinf_versions != (0, 0):
        raise PatchError("only version 0 iinf boxes are supported")
    infe_children_first = read_boxes(first, iinf_first.body_start + 6, iinf_first.end)
    infe_second = read_boxes(second, iinf_second.body_start + 6, iinf_second.end)
    if (
        len(infe_second) != 1
        or infe_second[0].kind != b"infe"
        or byte_at(second, infe_second[0].body_start, "infe version") != 2
    ):
        raise PatchError("the second input must have one version 2 infe entry")
    infe_bytes = bytearray(raw(second, infe_second[0]))
    need(infe_bytes, 12, 2, "infe item id")
    struct.pack_into(">H", infe_bytes, 12, 2)
    iinf_payload = (
        body(first, iinf_first)[:4]
        + struct.pack(">H", len(infe_children_first) + 1)
        + b"".join(raw(first, box) for box in infe_children_first)
        + bytes(infe_bytes)
    )
    new_iinf = make_box(b"iinf", iinf_payload)

    iprp_first = only(kids_first, b"iprp")
    iprp_second = only(kids_second, b"iprp")
    prop_first = read_boxes(first, iprp_first.body_start, iprp_first.end)
    prop_second = read_boxes(second, iprp_second.body_start, iprp_second.end)
    ipco_first = only(prop_first, b"ipco")
    ipco_second = only(prop_second, b"ipco")
    ipma_first = only(prop_first, b"ipma")
    ipma_second = only(prop_second, b"ipma")
    shift = len(read_boxes(first, ipco_first.body_start, ipco_first.end))
    version_first, wide_first, entries_first = parse_ipma(body(first, ipma_first))
    version_second, wide_second, entries_second = parse_ipma(body(second, ipma_second))
    if wide_first != wide_second:
        raise PatchError("ipma association widths differ")
    index_mask = 0x7FFF if wide_first else 0x7F
    moved = []
    for _, associations in entries_second:
        values = []
        for value in associations:
            index = value & index_mask
            if index + shift > index_mask:
                raise PatchError("combined property index does not fit")
            values.append((value & ~index_mask) | (index + shift))
        moved.append([2, values])
    new_ipco = make_box(b"ipco", body(first, ipco_first) + body(second, ipco_second))
    new_ipma = make_box(
        b"ipma", serialize_ipma(version_first, wide_first, entries_first + moved)
    )
    new_iprp = make_box(b"iprp", new_ipco + new_ipma)

    layout_first, items_first = parse_iloc(first, only(kids_first, b"iloc"))
    layout_second, items_second = parse_iloc(second, only(kids_second, b"iloc"))
    if len(items_second) != 1 or items_second[0]["construction"] != 0 or items_second[0]["base"] != 0:
        raise PatchError("the second input must have one file-offset item with no base")
    for item in items_first:
        if item["construction"] != 0 or item["base"] != 0:
            raise PatchError("only file-offset items are supported in the first input")

    hdlr = raw(first, only(kids_first, b"hdlr"))
    dinf = raw(first, only(kids_first, b"dinf"))

    def build(delta, second_start):
        moved_first = []
        for item in items_first:
            extents = [
                (index, offset + delta if offset >= meta_first.end else offset, length)
                for index, offset, length in item["extents"]
            ]
            moved_first.append(dict(item, extents=extents))
        moved_second = []
        for item in items_second:
            extents = []
            for index, offset, length in item["extents"]:
                if not (mdat_second.start <= offset and offset + length <= mdat_second.end):
                    raise PatchError("second input item data lies outside its mdat box")
                extents.append((index, offset - mdat_second.start + second_start, length))
            moved_second.append(dict(item, id=2, extents=extents))
        iloc_bytes = serialize_iloc(layout_first, moved_first + moved_second)
        payload = (
            first[meta_first.body_start : meta_first.body_start + FULL_BOX_HEADER]
            + hdlr
            + dinf
            + make_box(b"pitm", bytes(FULL_BOX_HEADER) + struct.pack(">H", primary))
            + new_iinf
            + new_iprp
            + make_box(b"iloc", iloc_bytes)
        )
        return make_box(b"meta", payload)

    provisional = build(0, 0)
    tail_length = len(first) - meta_first.end
    delta = len(provisional) - meta_first.size
    second_start = meta_first.start + len(provisional) + tail_length
    new_meta = build(delta, second_start)
    if len(new_meta) != len(provisional):
        raise PatchError("meta length changed between offset passes")
    return (
        bytes(first[: meta_first.start])
        + new_meta
        + bytes(first[meta_first.end :])
        + bytes(second[mdat_second.start : mdat_second.end])
    )


def main(argv):
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("input")
    parser.add_argument("output")
    parser.add_argument("--rotate", type=int, default=0, choices=[0, 90, 180, 270])
    parser.add_argument("--mirror-axis", type=int, choices=[0, 1])
    parser.add_argument("--combine-with")
    parser.add_argument("--primary", type=int, default=1, choices=[1, 2])
    args = parser.parse_args(argv)
    with open(args.input, "rb") as source:
        data = source.read()
    try:
        if args.combine_with:
            if args.rotate or args.mirror_axis is not None:
                raise PatchError("combining is not combined with a transform")
            with open(args.combine_with, "rb") as other:
                patched = combine_primary(data, other.read(), args.primary)
        else:
            patched = add_transform(data, args.rotate, args.mirror_axis)
    except PatchError as error:
        print(f"add_transform.py: error: {error}", file=sys.stderr)
        return 1
    with open(args.output, "wb") as target:
        target.write(patched)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
