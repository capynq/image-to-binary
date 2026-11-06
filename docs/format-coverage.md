# Format coverage matrix

Scope: native decode adapters in `crates/processing` (the `image`-crate path for
JPEG, PNG, WebP, BMP, TIFF, and GIF, and the `libheif` path for HEIC and AVIF).
Evidence comes from `crates/processing/tests/format_matrix.rs`, which the
default and `heif` builds both run. The browser column is covered by the lazy
decoder work in subphase 4.2 and the browser acceptance runs, not by this file.

Legend:

- `verified: name` means a test with that name ran and passed on this host.
- `not applicable` means the format or decoder path cannot carry the property.
- `not yet verified` means no test covers the cell yet.
- `unavailable on this host` means the environment cannot run the check.

Host for this run: macOS arm64 (`aarch64-apple-darwin`). Browser cells remain
`not yet verified (browser, until the next subphase)`.

| Format | Grayscale | RGB | RGBA / alpha | Orientation | Multi-page / frame policy | High-bit-depth normalisation | 50 MP limit | Lowered browser limit (16 MP) | Native macOS arm64 | Browser | Native Windows | Native Linux |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| JPEG | verified: `grayscale_pixels_decode_to_equal_rgb_channels_where_the_encoder_keeps_grey` | verified: `rgb_pixels_decode_to_opaque_rgba_for_every_common_codec` (lossy tolerance) | not applicable (encoder rejects alpha, asserted in `alpha_survives_for_every_codec_that_carries_it`) | verified: `exif_orientation_one_to_eight_moves_pixels_for_codecs_that_store_it` | not applicable (single image) | not applicable (8-bit baseline path) | verified: limit check only (`fifty_megapixel_header_passes_the_limit_at_the_boundary_and_is_rejected_one_above`); real 50 MP decode not exercised | verified: limit check only at 16 MP (`lowered_sixteen_megapixel_browser_limit_rejects_a_crafted_header_above_it`); real 16 MP decode not exercised | verified (`format_matrix`, macOS arm64) | not yet verified (browser, until the next subphase) | unavailable on this host | unavailable on this host |
| PNG | verified: `grayscale_pixels_decode_to_equal_rgb_channels_where_the_encoder_keeps_grey` | verified: `rgb_pixels_decode_to_opaque_rgba_for_every_common_codec` | verified: `alpha_survives_for_every_codec_that_carries_it` | verified: `exif_orientation_one_to_eight_moves_pixels_for_codecs_that_store_it` | not yet verified (animated PNG first frame not tested) | verified: `sixteen_bit_png_normalises_to_eight_bit_by_the_asserted_value` | verified: `fifty_megapixel_png_decodes_fully_at_the_boundary` (real 50 MP decode, every pixel checked) + `fifty_megapixel_header_passes_the_limit_at_the_boundary_and_is_rejected_one_above` | verified: limit check only at 16 MP (`lowered_sixteen_megapixel_browser_limit_rejects_a_crafted_header_above_it`); real 16 MP decode not exercised | verified (`format_matrix`, macOS arm64) | not yet verified (browser, until the next subphase) | unavailable on this host | unavailable on this host |
| WebP | verified: `grayscale_pixels_decode_to_equal_rgb_channels_where_the_encoder_keeps_grey` (WebP stores grey as equal RGB) | verified: `rgb_pixels_decode_to_opaque_rgba_for_every_common_codec` | verified: `alpha_survives_for_every_codec_that_carries_it` | verified: `exif_orientation_one_to_eight_moves_pixels_for_codecs_that_store_it` | not yet verified (animated WebP first frame not tested) | not applicable (8-bit format) | verified: limit check only (`fifty_megapixel_header_passes_the_limit_at_the_boundary_and_is_rejected_one_above`); real 50 MP decode not exercised | verified: limit check only at 16 MP (`lowered_sixteen_megapixel_browser_limit_rejects_a_crafted_header_above_it`); real 16 MP decode not exercised | verified (`format_matrix`, macOS arm64) | not yet verified (browser, until the next subphase) | unavailable on this host | unavailable on this host |
| BMP | verified: `grayscale_pixels_decode_to_equal_rgb_channels_where_the_encoder_keeps_grey` | verified: `rgb_pixels_decode_to_opaque_rgba_for_every_common_codec` | verified: `alpha_survives_for_every_codec_that_carries_it` | not applicable (no EXIF orientation; `bmp_and_gif_have_no_exif_orientation_and_keep_stored_dimensions`) | not applicable (single image) | not applicable (8-bit channel path) | verified: limit check only (`fifty_megapixel_header_passes_the_limit_at_the_boundary_and_is_rejected_one_above`); real 50 MP decode not exercised | verified: limit check only at 16 MP (`lowered_sixteen_megapixel_browser_limit_rejects_a_crafted_header_above_it`); real 16 MP decode not exercised | verified (`format_matrix`, macOS arm64) | not yet verified (browser, until the next subphase) | unavailable on this host | unavailable on this host |
| TIFF | verified: `grayscale_pixels_decode_to_equal_rgb_channels_where_the_encoder_keeps_grey` | verified: `rgb_pixels_decode_to_opaque_rgba_for_every_common_codec` | verified: `alpha_survives_for_every_codec_that_carries_it` | verified: `exif_orientation_one_to_eight_moves_pixels_for_codecs_that_store_it` | verified: `tiff_multi_page_source_decodes_the_first_page_only` | verified: `sixteen_bit_tiff_normalises_to_eight_bit_by_the_asserted_value` | verified: limit check only (`fifty_megapixel_header_passes_the_limit_at_the_boundary_and_is_rejected_one_above`); real 50 MP decode not exercised | verified: limit check only at 16 MP (`lowered_sixteen_megapixel_browser_limit_rejects_a_crafted_header_above_it`); real 16 MP decode not exercised | verified (`format_matrix`, macOS arm64) | not yet verified (browser, until the next subphase) | unavailable on this host | unavailable on this host |
| GIF | verified: `grayscale_pixels_decode_to_equal_rgb_channels_where_the_encoder_keeps_grey` | verified: `rgb_pixels_decode_to_opaque_rgba_for_every_common_codec` | verified: `alpha_survives_for_every_codec_that_carries_it` (binary alpha) | not applicable (no EXIF orientation; `bmp_and_gif_have_no_exif_orientation_and_keep_stored_dimensions`) | verified: `gif_decodes_the_first_frame_while_a_second_frame_is_present` | not applicable (palette, 8-bit) | verified: limit check only (`fifty_megapixel_header_passes_the_limit_at_the_boundary_and_is_rejected_one_above`); real 50 MP decode not exercised | verified: limit check only at 16 MP (`lowered_sixteen_megapixel_browser_limit_rejects_a_crafted_header_above_it`); real 16 MP decode not exercised | verified (`format_matrix`, macOS arm64) | not yet verified (browser, until the next subphase) | unavailable on this host | unavailable on this host |
| HEIC | verified: `grey_heic_and_avif_fixtures_decode_to_equal_rgb_channels` (4:2:0 with neutral chroma; true monochrome unavailable, see notes) | verified: `plain_heic_and_avif_decode_to_the_gradient_corners` | verified: `heic_and_avif_alpha_channels_survive_decoding` | verified: `heic_irot_ninety_degrees_anticlockwise_rotates_the_gradient` + `heic_imir_mirrors_top_to_bottom_on_axis_zero_and_left_to_right_on_axis_one` | verified: `heif_decodes_the_primary_item_even_when_it_is_not_the_first_item` | unavailable on this host (no 10-bit HEIC writer; see note) | verified: limit check only (`fifty_megapixel_ispe_declaration_is_rejected_before_pixel_decode` + `fifty_megapixel_ispe_boundary_passes_the_limit`); real 50 MP decode not exercised | verified: `lowered_sixteen_megapixel_limit_accepts_the_sample_and_rejects_a_larger_declaration` | verified (`format_matrix`, macOS arm64) | not yet verified (browser, until the next subphase) | unavailable on this host | unavailable on this host |
| AVIF | verified: `grey_heic_and_avif_fixtures_decode_to_equal_rgb_channels` (4:2:0 with neutral chroma; true monochrome unavailable, see notes) | verified: `plain_heic_and_avif_decode_to_the_gradient_corners` | verified: `heic_and_avif_alpha_channels_survive_decoding` | verified: `avif_irot_two_seventy_degrees_anticlockwise_rotates_the_gradient` | verified: `heif_decodes_the_primary_item_even_when_it_is_not_the_first_item` | verified: `ten_bit_avif_source_normalises_to_eight_bit_corners` | verified: limit check only (`fifty_megapixel_ispe_declaration_is_rejected_before_pixel_decode` + `fifty_megapixel_ispe_boundary_passes_the_limit`); real 50 MP decode not exercised | verified: `lowered_sixteen_megapixel_limit_accepts_the_sample_and_rejects_a_larger_declaration` | verified (`format_matrix`, macOS arm64) | not yet verified (browser, until the next subphase) | unavailable on this host | unavailable on this host |

Limit tests common to every path (`pixel_limit_accepts_exactly_the_stored_pixel_count_and_rejects_one_fewer`,
`encoded_byte_limit_accepts_exact_length_and_rejects_one_byte_less`,
`exact_pixel_limit_accepts_the_sample_and_one_fewer_rejects`,
`encoded_byte_limit_accepts_exact_length_and_rejects_one_byte_less_for_heif`)
pass for the common codecs and for HEIC and AVIF. They check that the pixel and
encoded-byte limits are inclusive at their boundary.

What the matrix does and does not prove about width and height:

- The per-axis decoder limit (`crates/processing/src/decode.rs`, the
  `max_image_width` and `max_image_height` assignments) is the only check that
  distinguishes width from height. `per_axis_decoder_limit_accepts_non_square_images_in_both_orientations`
  detects a swap of those two fields. So do 14 other common-codec tests, each of
  which decodes a non-square image (the 24x16 fixtures, a 2x1 PNG or TIFF, or a
  10000x5000 header or PNG).
- Output dimensions after orientation are checked by
  `exif_orientation_one_to_eight_moves_pixels_for_codecs_that_store_it` (orientations 5 to 8
  swap width and height) and, for HEIC and AVIF, by
  `heic_irot_ninety_degrees_anticlockwise_rotates_the_gradient` and
  `avif_irot_two_seventy_degrees_anticlockwise_rotates_the_gradient` (48x64 output).
- The product pixel limit (`validate` in `crates/core/src/geometry.rs`) and the HEIC
  handle-dimension product read (`crates/processing/src/decode/heif.rs`) are symmetric
  in width and height. A swap there is not observable by any test in this matrix.
- `orientation_six_working_copy_counts_toward_the_decoded_byte_budget` and
  `transformed_heic_byte_budget_is_exactly_twice_the_rgba_output` prove byte
  accounting of the working copy. They do not prove axis orientation.
- `orientation_six_output_passes_an_exact_product_pixel_limit` and
  `rotated_heic_ispe_product_is_checked_against_the_pixel_limit` prove product
  checks only.

Mutation evidence, run in scratch copies and not in the repository: swapping the
two per-axis assignments in `decode.rs` fails 15 of the 17 default-build
`format_matrix` tests, including `per_axis_decoder_limit_accepts_non_square_images_in_both_orientations`.
The two that still pass use a 1x1 GIF frame and a square 4000x4000 input. Swapping the
HEIC handle width and height in `heif.rs` leaves all 33 `heif`-build
`format_matrix` tests passing.

Known limit: libheif's internal ceiling is fixed at twice the
50 MP cap, so a file whose `ispe` understates its size can use up to that much
inside libheif before the post-decode dimension check rejects it; this is best
effort, like other codec internals.

## Counts

Property cells (8 rows by 8 property columns): 52 verified, 9 not applicable,
2 not yet verified (PNG and WebP multi-frame policy), 1 unavailable on this host
(HEIC 10-bit).

Platform cells (8 rows by 4 platform columns): 8 verified (macOS arm64), 8 not yet
verified (browser), 16 unavailable on this host (Windows and Linux).

## Notes

- Grayscale HEIC and AVIF fixtures are not monochrome (4:0:0) bitstreams. sips
  writes 3-channel HEIC, and libsvtav1 via ffmpeg (`-pix_fmt gray` and `gray10le`)
  writes 4:2:0 with neutral chroma; `av1C` monochrome bit is clear and libheif
  reports 8 chroma bits (`grey_fixtures_keep_chroma_planes_so_they_are_not_monochrome`).
  True monochrome decode is unavailable on this host.
- The primary-item fixtures (`sample_two_images.*`) combine two single-image
  files with `add_transform.py --combine-with`; item 2 (inverted gradient) is
  primary. The test patches `pitm` to item 1 to show the decoded pixels change.
- HEIC high-bit-depth is unavailable here. macOS `sips` writes 8-bit HEIC, and
  the host ffmpeg has `libx265` but no HEIF muxer. Only the AVIF 10-bit path is
  verified.
- Transform fixtures were built with `tools/native-codecs/add_transform.py`
  because no host tool writes `irot` or `imir`. Mirror-axis semantics follow
  libheif: axis 0 is a top-bottom exchange and axis 1 a left-right exchange.
- Tolerances, per channel: JPEG cells use 16 (`LOSSY_TOLERANCE`); the other
  common codecs compare exactly. HEIC and AVIF corner and alpha checks use 12
  (`NATIVE_TOLERANCE`), and grey HEIC and AVIF also require channels within 6 of
  each other. Transform cells use 4 (`TRANSFORM_TOLERANCE`) against the transformed
  decoded base sample, for chroma resampling. The primary-item check is exact.
  `crates/processing/tests/fixtures/heif/PROVENANCE.md` has the full table.
- 50 MP coverage is not uniform. Only the PNG cell decodes a real 50 MP image:
  grayscale, 10000x5000, every pixel checked. For the other codecs, the 50 MP cell
  proves only the limit check and the codec's own later outcome on a crafted
  header: JPEG and GIF complete with the declared size, PNG and WebP fail with
  malformed data, BMP and TIFF fail with unexpected end of input, and HEIC and AVIF
  fail with libheif's size-mismatch error. No 50 MP HEIC or AVIF file is committed,
  so a real 50 MP HEIC or AVIF decode is not exercised.
- A defect was fixed in `crates/processing/src/decode/heif.rs`: libheif's
  security ceiling was set to the caller's pixel limit, but libheif checks it
  against its padded internal decode size. An image of exactly the limit
  (for example 64x48 under a 3072-pixel limit) was rejected as a resource-limit
  error. The ceiling is now twice the product cap; the caller budget is still
  enforced from the handle dimensions before any pixel decode.
- Decoder limits are best effort for libheif's internal allocations, as
  documented in `decode.rs` and `heif.rs`.
