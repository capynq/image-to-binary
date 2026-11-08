# Format coverage matrix

Scope: native decode adapters in `crates/processing` (the `image`-crate path for
JPEG, PNG, WebP, BMP, TIFF, and GIF, and the `libheif` path for HEIC and AVIF).
Evidence comes from `crates/processing/tests/format_matrix.rs`, which the
default and `heif` builds both run. The browser column and the browser property
matrix below are filled from the Chromium 154 probe in subphase 4.3.2
(`docs/reports/playwright/format-matrix-browser.mjs`, evidence in the ignored
`docs/reports/playwright/format-matrix-browser-chromium.json`). Firefox and WebKit
were not run, by user decision on 2026-11-07.

Legend:

- `verified: name` means a test with that name ran and passed on this host.
- `not applicable` means the format or decoder path cannot carry the property.
- `not yet verified` means no test covers the cell yet.
- `unavailable on this host` means the environment cannot run the check.

Host for this run: macOS arm64 (`aarch64-apple-darwin`). Browser host: Chromium 154.0.8037.0
(headless, the already-downloaded Playwright Chromium). Firefox and WebKit: not run.

| Format | Grayscale | RGB | RGBA / alpha | Orientation | Multi-page / frame policy | High-bit-depth normalisation | 50 MP limit | Lowered browser limit (16 MP) | Native macOS arm64 | Browser | Native Windows | Native Linux |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| JPEG | verified: `grayscale_pixels_decode_to_equal_rgb_channels_where_the_encoder_keeps_grey` | verified: `rgb_pixels_decode_to_opaque_rgba_for_every_common_codec` (lossy tolerance) | not applicable (encoder rejects alpha, asserted in `alpha_survives_for_every_codec_that_carries_it`) | verified: `exif_orientation_one_to_eight_moves_pixels_for_codecs_that_store_it` | not applicable (single image) | not applicable (8-bit baseline path) | verified: limit check only (`fifty_megapixel_header_passes_the_limit_at_the_boundary_and_is_rejected_one_above`); real 50 MP decode not exercised | verified: limit check only at 16 MP (`lowered_sixteen_megapixel_browser_limit_rejects_a_crafted_header_above_it`); real 16 MP decode not exercised | verified (`format_matrix`, macOS arm64) | verified: `a[jpeg-rgb].checks.exportByteEqualNative` (all JPEG cells: browser property matrix) | unavailable on this host | unavailable on this host |
| PNG | verified: `grayscale_pixels_decode_to_equal_rgb_channels_where_the_encoder_keeps_grey` | verified: `rgb_pixels_decode_to_opaque_rgba_for_every_common_codec` | verified: `alpha_survives_for_every_codec_that_carries_it` | verified: `exif_orientation_one_to_eight_moves_pixels_for_codecs_that_store_it` | verified: `animated_png_decodes_its_first_frame_and_the_second_frame_is_a_sensitive_control` | verified: `sixteen_bit_png_normalises_to_eight_bit_by_the_asserted_value` | verified: `fifty_megapixel_png_decodes_fully_at_the_boundary` (real 50 MP decode, every pixel checked) + `fifty_megapixel_header_passes_the_limit_at_the_boundary_and_is_rejected_one_above` | verified: limit check only at 16 MP (`lowered_sixteen_megapixel_browser_limit_rejects_a_crafted_header_above_it`); real 16 MP decode not exercised | verified (`format_matrix`, macOS arm64) | verified: `a[png-rgb].checks.exportByteEqualNative` (all PNG cells: browser property matrix) | unavailable on this host | unavailable on this host |
| WebP | verified: `grayscale_pixels_decode_to_equal_rgb_channels_where_the_encoder_keeps_grey` (WebP stores grey as equal RGB) | verified: `rgb_pixels_decode_to_opaque_rgba_for_every_common_codec` | verified: `alpha_survives_for_every_codec_that_carries_it` | verified: `exif_orientation_one_to_eight_moves_pixels_for_codecs_that_store_it` | not yet verified (finding: alpha-blended animated frames decode one level low, see notes; no-blend frames verified by `animated_webp_without_blending_decodes_its_first_frame_exactly`) | not applicable (8-bit format) | verified: limit check only (`fifty_megapixel_header_passes_the_limit_at_the_boundary_and_is_rejected_one_above`); real 50 MP decode not exercised | verified: limit check only at 16 MP (`lowered_sixteen_megapixel_browser_limit_rejects_a_crafted_header_above_it`); real 16 MP decode not exercised | verified (`format_matrix`, macOS arm64) | not yet verified (Chromium 154; alpha-blended animated WebP first frame is a finding, see notes) | unavailable on this host | unavailable on this host |
| BMP | verified: `grayscale_pixels_decode_to_equal_rgb_channels_where_the_encoder_keeps_grey` | verified: `rgb_pixels_decode_to_opaque_rgba_for_every_common_codec` | verified: `alpha_survives_for_every_codec_that_carries_it` | not applicable (no EXIF orientation; `bmp_and_gif_have_no_exif_orientation_and_keep_stored_dimensions`) | not applicable (single image) | not applicable (8-bit channel path) | verified: limit check only (`fifty_megapixel_header_passes_the_limit_at_the_boundary_and_is_rejected_one_above`); real 50 MP decode not exercised | verified: limit check only at 16 MP (`lowered_sixteen_megapixel_browser_limit_rejects_a_crafted_header_above_it`); real 16 MP decode not exercised | verified (`format_matrix`, macOS arm64) | verified: `a[bmp-rgb].checks.exportByteEqualNative` (all BMP cells: browser property matrix) | unavailable on this host | unavailable on this host |
| TIFF | verified: `grayscale_pixels_decode_to_equal_rgb_channels_where_the_encoder_keeps_grey` | verified: `rgb_pixels_decode_to_opaque_rgba_for_every_common_codec` | verified: `alpha_survives_for_every_codec_that_carries_it` | verified: `exif_orientation_one_to_eight_moves_pixels_for_codecs_that_store_it` | verified: `tiff_multi_page_source_decodes_the_first_page_only` | verified: `sixteen_bit_tiff_normalises_to_eight_bit_by_the_asserted_value` | verified: limit check only (`fifty_megapixel_header_passes_the_limit_at_the_boundary_and_is_rejected_one_above`); real 50 MP decode not exercised | verified: limit check only at 16 MP (`lowered_sixteen_megapixel_browser_limit_rejects_a_crafted_header_above_it`); real 16 MP decode not exercised | verified (`format_matrix`, macOS arm64) | verified: `a[tiff-rgb].checks.exportByteEqualNative` (all TIFF cells: browser property matrix) | unavailable on this host | unavailable on this host |
| GIF | verified: `grayscale_pixels_decode_to_equal_rgb_channels_where_the_encoder_keeps_grey` | verified: `rgb_pixels_decode_to_opaque_rgba_for_every_common_codec` | verified: `alpha_survives_for_every_codec_that_carries_it` (binary alpha) | not applicable (no EXIF orientation; `bmp_and_gif_have_no_exif_orientation_and_keep_stored_dimensions`) | verified: `gif_decodes_the_first_frame_while_a_second_frame_is_present` (still `decode()` path); animated frames, delays, loop count and limits: see Notes, GIF animation | not applicable (palette, 8-bit) | verified: limit check only (`fifty_megapixel_header_passes_the_limit_at_the_boundary_and_is_rejected_one_above`); real 50 MP decode not exercised | verified: limit check only at 16 MP (`lowered_sixteen_megapixel_browser_limit_rejects_a_crafted_header_above_it`); real 16 MP decode not exercised | verified (`format_matrix`, macOS arm64) | verified: `a[gif-rgb].checks.exportByteEqualNative` (all GIF cells: browser property matrix) | unavailable on this host | unavailable on this host |
| HEIC | verified: `grey_heic_and_avif_fixtures_decode_to_equal_rgb_channels` (4:2:0 with neutral chroma; true monochrome unavailable, see notes) | verified: `plain_heic_and_avif_decode_to_the_gradient_corners` | verified: `heic_and_avif_alpha_channels_survive_decoding` | verified: `heic_irot_ninety_degrees_anticlockwise_rotates_the_gradient` + `heic_imir_mirrors_top_to_bottom_on_axis_zero_and_left_to_right_on_axis_one` | verified: `heif_decodes_the_primary_item_even_when_it_is_not_the_first_item` | unavailable on this host (no 10-bit HEIC writer; see note) | verified: limit check only (`fifty_megapixel_ispe_declaration_is_rejected_before_pixel_decode` + `fifty_megapixel_ispe_boundary_passes_the_limit`); real 50 MP decode not exercised | verified: `lowered_sixteen_megapixel_limit_accepts_the_sample_and_rejects_a_larger_declaration` | verified (`format_matrix`, macOS arm64) | verified: `reusedHeif.sectionA.fixtures["sample.heic"].rgbaExactEqual` (all HEIC cells: browser property matrix) | unavailable on this host | unavailable on this host |
| AVIF | verified: `grey_heic_and_avif_fixtures_decode_to_equal_rgb_channels` (4:2:0 with neutral chroma; true monochrome unavailable, see notes) | verified: `plain_heic_and_avif_decode_to_the_gradient_corners` | verified: `heic_and_avif_alpha_channels_survive_decoding` | verified: `avif_irot_two_seventy_degrees_anticlockwise_rotates_the_gradient` | verified: `heif_decodes_the_primary_item_even_when_it_is_not_the_first_item` | verified: `ten_bit_avif_source_normalises_to_eight_bit_corners` | verified: limit check only (`fifty_megapixel_ispe_declaration_is_rejected_before_pixel_decode` + `fifty_megapixel_ispe_boundary_passes_the_limit`); real 50 MP decode not exercised | verified: `lowered_sixteen_megapixel_limit_accepts_the_sample_and_rejects_a_larger_declaration` | verified (`format_matrix`, macOS arm64) | verified: `reusedHeif.sectionA.fixtures["sample.avif"].rgbaExactEqual` (all AVIF cells: browser property matrix) | unavailable on this host | unavailable on this host |

## Browser property matrix (Chromium 154)

Evidence is `docs/reports/playwright/format-matrix-browser-chromium.json`. `a[ID]`,
`b[ID]` and `c["ID"]` are the probe's section rows at `sections.a.data.rows`,
`sections.b.data.rows` and `sections.c.data.rows`. `reusedHeif` is copied from
`docs/reports/playwright/heif-acceptance-chromium.json` (subphase 4.2, sections a and e),
not re-run. Every browser import goes through the file input of the production bundle
(`npx vite build` into `.local/heif-probe-dist`). Each preview case compares the classic
PNG export at the UI defaults (cell 12, font 16, threshold 128) byte for byte with the
native export, and the decoded RGBA read back from the original preview with the native
RGBA. A negative control is the output a wrong interpretation would give, and the probe
checks that the browser output is not that control.

| Format | Grayscale | RGB | RGBA / alpha | Orientation | Multi-page / frame policy | High-bit-depth normalisation | 50 MP limit | Lowered browser limit (16 MP) |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| JPEG | verified: `a[jpeg-grey].checks.exportByteEqualNative`, `a[jpeg-grey].checks.pixelsMatchNative` | verified: `a[jpeg-rgb].checks.exportByteEqualNative`, `a[jpeg-rgb].checks.pixelsMatchNative` | not applicable | verified: `a[jpeg-orientation-1..8].checks.exportByteEqualNative`, `a[jpeg-orientation-1..8].checks.pixelsMatchNative`, `a[jpeg-orientation-2..8].checks.controlDetectedByPixels` | not applicable | not applicable | verified: limit check only (`b[jpeg-limit-50mp-accept].checks.browserMatchesNativeDefault`, `b[jpeg-limit-50mp-reject].checks.browserRejectsWithLimitMessage`) | not applicable (browser keeps the 50 MP limit by decision; lowering is allowed, not required; the core's lowered-limit support is proven natively) |
| PNG | verified: `a[png-grey].checks.exportByteEqualNative`, `a[png-grey].checks.pixelsMatchNative` | verified: `a[png-rgb].checks.exportByteEqualNative`, `a[png-rgb].checks.pixelsMatchNative` | verified: `a[png-alpha].checks.pixelsMatchNative`, `a[png-alpha].checks.controlDetectedByPixels` (export does not show alpha, see notes) | verified: `a[png-orientation-1..8].checks.exportByteEqualNative`, `a[png-orientation-1..8].checks.pixelsMatchNative`, `a[png-orientation-2..8].checks.controlDetectedByPixels` | verified: `a[png-apng-first-frame].checks.exportByteEqualNative`, `a[png-apng-first-frame].checks.pixelsMatchNative`, `a[png-apng-first-frame].checks.controlDetectedByPixels` (second frame is the control) | verified: `a[png-16bit].checks.exportByteEqualNative`, `a[png-16bit].checks.pixelsMatchNative`, `a[png-16bit].checks.controlDetectedByPixels` (export does not show it, see notes) | verified: limit check only (`b[png-limit-50mp-accept].checks.browserMatchesNativeDefault`, `b[png-limit-50mp-reject].checks.browserRejectsWithLimitMessage`) | not applicable (browser keeps the 50 MP limit by decision; lowering is allowed, not required; the core's lowered-limit support is proven natively) |
| WebP | verified: `a[webp-grey].checks.exportByteEqualNative`, `a[webp-grey].checks.pixelsMatchNative` | verified: `a[webp-rgb].checks.exportByteEqualNative`, `a[webp-rgb].checks.pixelsMatchNative` | verified: `a[webp-alpha].checks.pixelsMatchNative`, `a[webp-alpha].checks.controlDetectedByPixels` | verified: `a[webp-orientation-1..8].checks.exportByteEqualNative`, `a[webp-orientation-1..8].checks.pixelsMatchNative`, `a[webp-orientation-2..8].checks.controlDetectedByPixels` | not yet verified (finding: alpha-blended frames are one level low in the browser, as natively; no-blend frames verified: `a[webp-animated-no-blend].checks.exportByteEqualNative`, `a[webp-animated-no-blend].checks.pixelsMatchNative`, `a[webp-animated-no-blend].checks.controlDetectedByPixels`) | not applicable | verified: limit check only (`b[webp-limit-50mp-accept].checks.browserMatchesNativeDefault`, `b[webp-limit-50mp-reject].checks.browserRejectsWithLimitMessage`) | not applicable (browser keeps the 50 MP limit by decision; lowering is allowed, not required; the core's lowered-limit support is proven natively) |
| BMP | verified: `a[bmp-grey].checks.exportByteEqualNative`, `a[bmp-grey].checks.pixelsMatchNative` | verified: `a[bmp-rgb].checks.exportByteEqualNative`, `a[bmp-rgb].checks.pixelsMatchNative` | verified: `a[bmp-alpha].checks.pixelsMatchNative`, `a[bmp-alpha].checks.controlDetectedByPixels` | not applicable | not applicable | not applicable | verified: limit check only (`b[bmp-limit-50mp-accept].checks.browserMatchesNativeDefault`, `b[bmp-limit-50mp-reject].checks.browserRejectsWithLimitMessage`) | not applicable (browser keeps the 50 MP limit by decision; lowering is allowed, not required; the core's lowered-limit support is proven natively) |
| TIFF | verified: `a[tiff-grey].checks.exportByteEqualNative`, `a[tiff-grey].checks.pixelsMatchNative` | verified: `a[tiff-rgb].checks.exportByteEqualNative`, `a[tiff-rgb].checks.pixelsMatchNative` | verified: `a[tiff-alpha].checks.pixelsMatchNative`, `a[tiff-alpha].checks.controlDetectedByPixels` | verified: `a[tiff-orientation-1..8].checks.exportByteEqualNative`, `a[tiff-orientation-1..8].checks.pixelsMatchNative`, `a[tiff-orientation-2..8].checks.controlDetectedByPixels` | verified: `a[tiff-two-page].checks.exportByteEqualNative`, `a[tiff-two-page].checks.pixelsMatchNative`, `a[tiff-two-page].checks.controlDetectedByPixels` (second page is the control) | verified: `a[tiff-16bit].checks.exportByteEqualNative`, `a[tiff-16bit].checks.pixelsMatchNative`, `a[tiff-16bit].checks.controlDetectedByPixels` (export does not show it, see notes) | verified: limit check only (`b[tiff-limit-50mp-accept].checks.browserMatchesNativeDefault`, `b[tiff-limit-50mp-reject].checks.browserRejectsWithLimitMessage`; 24x16 crafted base, see notes) | not applicable (browser keeps the 50 MP limit by decision; lowering is allowed, not required; the core's lowered-limit support is proven natively) |
| GIF | verified: `a[gif-grey].checks.exportByteEqualNative`, `a[gif-grey].checks.pixelsMatchNative` | verified: `a[gif-rgb].checks.exportByteEqualNative`, `a[gif-rgb].checks.pixelsMatchNative` | verified: `a[gif-alpha].checks.pixelsMatchNative`, `a[gif-alpha].checks.controlDetectedByPixels` (binary alpha) | not applicable | verified: `a[gif-two-frame].checks.exportByteEqualNative`, `a[gif-two-frame].checks.pixelsMatchNative`, `a[gif-two-frame].checks.controlDetectedByPixels` (second frame is the control; this frame-0 probe predates Phase 9 and was not re-run, so animated playback is covered by the tests in Notes, GIF animation) | not applicable | verified: limit check only (`b[gif-limit-50mp-accept].checks.browserMatchesNativeDefault`, `b[gif-limit-50mp-reject].checks.browserRejectsWithLimitMessage`) | not applicable (browser keeps the 50 MP limit by decision; lowering is allowed, not required; the core's lowered-limit support is proven natively) |
| HEIC | verified: `reusedHeif.sectionA.fixtures["sample_grey.heic"].rgbaExactEqual`, `reusedHeif.sectionA.fixtures["sample_grey.heic"].pngExactEqualNative` | verified: `reusedHeif.sectionA.fixtures["sample.heic"].rgbaExactEqual`, `reusedHeif.sectionA.fixtures["sample.heic"].pngExactEqualNative` | verified: `reusedHeif.sectionA.fixtures["sample_alpha.heic"].rgbaExactEqual`, `reusedHeif.sectionA.fixtures["sample_alpha.heic"].pngExactEqualNative` | verified: `reusedHeif.sectionA.fixtures["sample_irot90.heic"].rgbaExactEqual`, `reusedHeif.sectionA.fixtures["sample_imir_axis0.heic"].rgbaExactEqual`, `reusedHeif.sectionA.fixtures["sample_imir_axis1.heic"].rgbaExactEqual` | verified: primary item, `reusedHeif.sectionA.fixtures["sample_two_images.heic"].rgbaExactEqual`, `reusedHeif.sectionA.fixtures["sample_two_images.heic"].pngExactEqualNative` | unavailable on this host (no 10-bit HEIC writer, see native notes) | verified: limit check only (`c["sample.heic-50mp-accept"].checks.browserMatchesNativeDefault`, `c["sample.heic-50mp-reject"].checks.browserRejectsWithLimitMessage`, `reusedHeif.sectionE.over50Heic.typedLimitError`; a real 49 MP HEIC previews, `reusedHeif.sectionE.big49Heic.outcome`) | not applicable (browser keeps the 50 MP limit by decision; lowering is allowed, not required; the core's lowered-limit support is proven natively) |
| AVIF | verified: `reusedHeif.sectionA.fixtures["sample_grey.avif"].rgbaExactEqual`, `reusedHeif.sectionA.fixtures["sample_grey.avif"].pngExactEqualNative` | verified: `reusedHeif.sectionA.fixtures["sample.avif"].rgbaExactEqual`, `reusedHeif.sectionA.fixtures["sample.avif"].pngExactEqualNative` | verified: `reusedHeif.sectionA.fixtures["sample_alpha.avif"].rgbaExactEqual`, `reusedHeif.sectionA.fixtures["sample_alpha.avif"].pngExactEqualNative` | verified: `reusedHeif.sectionA.fixtures["sample_irot270.avif"].rgbaExactEqual`, `reusedHeif.sectionA.fixtures["sample_irot270.avif"].pngExactEqualNative` | verified: primary item, `reusedHeif.sectionA.fixtures["sample_two_images.avif"].rgbaExactEqual`, `reusedHeif.sectionA.fixtures["sample_two_images.avif"].pngExactEqualNative` | verified: 10-bit, `reusedHeif.sectionA.fixtures["sample_10bit.avif"].rgbaExactEqual`, `reusedHeif.sectionA.fixtures["sample_10bit.avif"].pngExactEqualNative` | verified: limit check only (`c["sample.avif-50mp-accept"].checks.browserMatchesNativeDefault`, `c["sample.avif-50mp-reject"].checks.browserRejectsWithLimitMessage`; a real 49 MP AVIF previews, `reusedHeif.sectionE.big49Avif.outcome`) | not applicable (browser keeps the 50 MP limit by decision; lowering is allowed, not required; the core's lowered-limit support is proven natively) |

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

Property cells (8 rows by 8 property columns): 53 verified, 9 not applicable, 1 not yet verified (WebP multi-frame policy, see notes), 1 unavailable on this host (HEIC 10-bit).

Platform cells (8 rows by 4 platform columns): 8 verified (macOS arm64), 7 verified and 1 not yet
verified (browser, Chromium 154), 16 unavailable on this host (Windows and Linux).

The animated PNG and WebP tests added in the 4.3.2 multi-frame follow-up are counted above. The browser work does not change native counts.

Browser property cells (8 rows by 8 property columns, Chromium 154 only, see the browser
property matrix): 45 verified, 17 not applicable, 1 not yet verified (the WebP multi-frame policy, a finding for alpha-blended frames), 1 unavailable on this host (HEIC 10-bit).
The 16 MP column is not applicable by browser decision (the browser keeps the 50 MP limit;
lowering is allowed, not required). Seven browser platform cells are verified and one (WebP)
is not yet verified.

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

Browser notes (Chromium 154 only):

- Decision (user, via the root agent): the browser keeps the 50 MP limit and does not apply
  the 16 MP lowered limit. `docs/PLAN.md` allows browser limits to be lower, not required.
  The 16 MP browser cells are not applicable. The raw `lowered16LimitApplied` fields in
  the probe JSON are unchanged and read false for the 16 MP reject rows; that is the
  decision, not a failing check. The core's lowered-limit support is proven natively by
  `format_matrix.rs`. The browser measured a real 49 MP HEIC and AVIF decode in subphase 4.2
  (`reusedHeif.sectionE`).
- TIFF limit inputs: the first run built the crafted TIFF headers on the 120x96 base, which
  the decoder rejects as inconsistent sizes before the limit check. The example now builds
  the TIFF limit cases on the 24x16 base that `format_matrix.rs` uses. Native outcomes for
  the TIFF rows match the native test: 50 MP accept `io_error`, 50 MP reject
  `resource_limit_exceeded`, 16 MP accept `io_error`, 16 MP reject `io_error` at the
  default limit and `resource_limit_exceeded` at 16 MP. The browser matches on each side.
- Limit cells prove a limit check only. The probe never decodes or renders a 50 MP or
  16 MP pixel image. A JPEG or GIF with a crafted 10000x5000 header is previewed as a
  truncated 50 MP source, and matches native. The real 49 MP HEIC and AVIF previews are
  from subphase 4.2 (`reusedHeif.sectionE`).
- The classic PNG export at the UI defaults does not distinguish alpha from opaque, or
  the native 16-bit rounding from a high-byte reading (`controlExportDiffers` is false in
  `.local/browser-matrix/manifest.json` for those rows). Those rows are proved by the
  decoded RGBA instead, read back from the original preview. The original preview is a
  canvas PNG, which premultiplies semi-transparent pixels, so only alpha and opaque RGB
  are compared for those rows.
- The native decoder rounds 16-bit samples to 8-bit (0xFF00 becomes 254), so the 16-bit
  negative control is the high-byte reading (0xFF00 becomes 255).
- Firefox and WebKit: not run, by user decision on 2026-11-07. They are not reported as
  passed.
- GIF animation (Phase 9): a GIF with two or more frames imports every frame as a
  full canvas. The frame count (at most 1000) and the 1 GiB decoded budget (every
  frame plus one working canvas) are checked before each frame is accepted. A GIF
  over either limit imports its first frame as a still; other failures, including
  malformed data and the 256 MiB encoded limit, are reported as errors. Delays
  of 0 or 1 centisecond normalise to 10 centiseconds. The loop count comes from the
  NETSCAPE2.0 extension, and an absent extension plays once. Tests:
  `crates/processing/src/decode/animation.rs` (`multi_frame_gif_yields_every_frame_on_the_canvas`,
  `partial_frame_composites_with_offset_and_transparency`,
  `delays_round_to_centiseconds_and_normalise_short_values`,
  `stored_zero_and_one_centisecond_frames_normalise_to_ten`,
  `loop_count_distinguishes_finite_infinite_and_absent`,
  `frame_count_limit_accepts_one_thousand_and_rejects_one_thousand_one`,
  `byte_budget_counts_every_frame_plus_one_working_canvas`,
  `single_frame_gif_and_png_return_none`, `oversized_and_truncated_inputs_fail`);
  `crates/adapters/src/session/animation_tests.rs` (`a_thousand_frames_import_as_an_animation_and_one_thousand_one_falls_back_to_the_first`,
  `the_decoded_byte_budget_falls_back_to_the_first_frame`,
  `a_gif_export_writes_every_frame_with_its_timing_and_loop`,
  `cancelling_a_gif_export_mid_run_is_terminal`); `crates/wasm/tests/node/animation.test.mjs`
  (Node WASM tests, not Chromium: `the shaded GIF imports as three frames with normalized delays and an endless loop`,
  `a GIF over the frame limit falls back to its first frame`,
  `the shaded animation exports as a GIF with one image per frame`); `crates/export/src/gif.rs`
  (`monochrome_round_trip_keeps_frame_count_delays_and_exact_endpoints`,
  `loop_extension_is_written_for_each_repeat_case`,
  `source_colour_round_trip_is_deterministic_and_keeps_timing`,
  `dimensions_above_the_gif_limit_are_typed_errors`, `output_limit_is_a_typed_resource_error`);
  desktop processing tests in `crates/desktop/src/processing/mod.rs`
  (`an_animated_gif_imports_with_frame_timing_and_loop_count`,
  `a_gif_export_writes_every_frame_and_refuses_a_still_or_a_frame`); and the desktop
  smoke run, `check_animation` in `crates/desktop/src/smoke/e2e.rs`
  (`e2e_animated_playback_controls_show_two_frames`, `e2e_animated_every_frame_renders`,
  `e2e_animated_import_is_shown`). The smoke run does not export GIF; GIF export on
  desktop is covered by the unit tests only.
- Multi-frame: the animated PNG (`png-apng-first-frame`) decodes to its default image, which is the first frame, exactly, natively and in Chromium. The second-frame control differs in export and pixels. The animated WebP is built in the test from image's lossless VP8L output in the extended container (VP8X, ANIM, ANMF). No host tool or committed fixture is needed, so there is no fixture provenance file.
- Finding (WebP alpha blending): `image-webp` composites an alpha-blended ANMF frame and returns it one level low on each opaque channel (230 becomes 229). The same frame stored with the do-not-blend flag decodes exactly, and the lossless still frame is exact. The native test `animated_webp_with_alpha_blending_is_within_one_level_of_its_first_frame` pins this, and the browser matches the native decode exactly (`a[webp-animated-alpha-blend].checks.pixelsMatchNative`). A browser match therefore means matching the native decode, not the source frame. No library code was changed.
