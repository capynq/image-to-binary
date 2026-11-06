# HEIC and AVIF fixture provenance

Every file in this directory is generated from a procedural 64x48 gradient (or its grayscale or inverted variant) with
no third-party content. The source PNGs are written by
`tools/native-codecs/make_fixture_png.py` and are not stored in the repository.
Regenerate everything with `tools/native-codecs/make-fixtures.sh`; it writes
into this directory, or into `NATIVE_FIXTURES_DIR` when that is set.

On the host recorded below, regenerating in a scratch directory reproduced all
13 committed fixtures byte for byte (checked with `cmp`). That result holds only
on matching `sips` and `ffmpeg` (with `libsvtav1`) versions. A different system
encoder build can change the encoded bytes without changing the decoded pixels,
so a byte difference alone is not a regression until the tool versions are
compared. The decoder tests check decoded pixels with the tolerances below, not
the encoded bytes.

## Base samples

| File | Bytes | How it was made |
| --- | --- | --- |
| `sample.heic` | 653 | `sips -s format heic` on the RGB gradient PNG (lossy system encoder). |
| `sample.avif` | 614 | `sips -s format avif` on the RGB gradient PNG (lossy system encoder). |

The base samples already contain an identity `irot` (angle 0) associated with
the primary item, which `sips` writes. Transform fixtures replace that
association; see below.

## Alpha samples

`make_fixture_png.py --alpha` writes an RGBA gradient whose alpha falls from 255
at the left edge to 127 at the right edge.

| File | Bytes | How it was made |
| --- | --- | --- |
| `sample_alpha.heic` | 979 | `sips -s format heic` on the RGBA gradient PNG. |
| `sample_alpha.avif` | 837 | `sips -s format avif` on the RGBA gradient PNG. |

## 10-bit AVIF

| File | Bytes | How it was made |
| --- | --- | --- |
| `sample_10bit.avif` | 421 | `ffmpeg -pix_fmt yuv420p10le -c:v libsvtav1 -crf 20` on the RGB gradient PNG. The encoder reports 10-bit input and the libheif-rs test asserts `luma_bits_per_pixel() == 10`. |

No 10-bit HEIC fixture exists. macOS `sips` writes 8-bit HEIC, and the host
ffmpeg has `libx265` but no HEIF muxer, so no host tool produces a 10-bit HEIC
container. That gap is reported as unavailable rather than faked.

## Transform fixtures

Neither `sips` nor ffmpeg writes `irot` or `imir` properties. Pillow writes
EXIF orientation only. The transform fixtures are therefore produced by
`tools/native-codecs/add_transform.py`, a stdlib-only patcher. It inserts an
`irot` and/or `imir` property into `ipco`, associates it with the primary item
as an essential property in `ipma`, removes any existing irot/imir association
of that item, and shifts `iloc` offsets for the change in meta size. The image
payload is not re-encoded, so the transform fixtures decode to the exact pixels
of their base samples, moved by the transform.

| File | Bytes | How it was made | Pixel effect (libheif) |
| --- | --- | --- | --- |
| `sample_irot90.heic` | 662 | `add_transform.py sample.heic --rotate 90` | Anticlockwise 90 degrees; 48x64 output. |
| `sample_imir_axis0.heic` | 662 | `add_transform.py sample.heic --mirror-axis 0` | Top and bottom rows exchanged. |
| `sample_imir_axis1.heic` | 662 | `add_transform.py sample.heic --mirror-axis 1` | Left and right columns exchanged. |
| `sample_irot270.avif` | 623 | `add_transform.py sample.avif --rotate 270` | Anticlockwise 270 degrees; 48x64 output. |

The mirror axis is the raw `imir` field. libheif maps axis 0 to a top-bottom
exchange and axis 1 to a left-right exchange; the tests assert those mappings
against the gradient corners. libheif rejects more than one transform per item,
which is why the patcher removes the identity `irot` before adding a transform.

## Grayscale fixtures

`make_fixture_png.py --grey` writes an 8-bit grayscale PNG (colour type 0) whose
value is the mean of the horizontal and vertical ramps. The results are not
monochrome bitstreams: `sips` and libsvtav1 (`-pix_fmt gray`) write 4:2:0 with
neutral chroma, and libheif reports 8 chroma bits for each.

| File | Bytes | How it was made |
| --- | --- | --- |
| `sample_grey.heic` | 5068 | `sips -s format heic` on the grayscale PNG. |
| `sample_grey.avif` | 471 | `ffmpeg -pix_fmt gray -c:v libsvtav1 -crf 20` on the grayscale PNG. |

## Two-image fixtures

`sample_two_images.heic` and `sample_two_images.avif` each contain two items.
Item 1 is the gradient base sample; item 2 is the inverted gradient from
`make_fixture_png.py --inverted`. `pitm` names item 2, so the primary item is
not the first item. They are built by
`add_transform.py sample.<ext> sample_two_images.<ext> --combine-with <inverted> --primary 2`,
which appends the second file's `infe`, `ipco` properties (with `ipma`
associations shifted past the first file's properties), `iloc` entry (offset
moved into the appended mdat), and mdat payload, then rewrites `pitm`.

| File | Bytes | How it was made |
| --- | --- | --- |
| `sample_two_images.heic` | 1107 | `add_transform.py` combine as above, base and inverted HEIC from `sips`. |
| `sample_two_images.avif` | 1031 | `add_transform.py` combine as above, base and inverted AVIF from `sips`. |

The test patches `pitm` back to item 1 in memory; the decoded pixels then equal
the plain base sample, while the file as written decodes the inverted item.

## Expected values

The tests in `crates/processing/tests/format_matrix.rs` use these tolerances:

| Check | Tolerance |
| --- | --- |
| HEIC and AVIF corner and alpha pixels, including the 10-bit AVIF corners and the grey fixture corners | 12 per channel, alpha included (`NATIVE_TOLERANCE`) |
| Grey HEIC and AVIF fixtures, every pixel | channels within 6 of each other |
| Transform fixtures (`irot` and `imir`), compared with the transformed decoded base sample | 4 per channel (`TRANSFORM_TOLERANCE`) |
| Primary-item test, item 1 patched in memory, compared with the plain base sample | exact (0) |

The first decoded pixel is expected near HEIC `[0, 2, 121, 255]` and AVIF
`[1, 0, 129, 255]`, within the 12-per-channel tolerance above.

## Tool versions used on this host

Recorded for the regeneration check (macOS arm64, `aarch64-apple-darwin`):

| Tool | Version |
| --- | --- |
| macOS | 27.0.1 (build 26A434) |
| `sips` | sips-316 |
| `ffmpeg` | 9.0.2_1 (Homebrew), configured with `--enable-libsvtav1` |
| Python | 3.14.8 (`add_transform.py` and `make_fixture_png.py` use only the standard library, run with `-I`) |
