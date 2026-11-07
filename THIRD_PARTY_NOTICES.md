# Third-party notices

The repository's own license is in `LICENSE` (GPL-3.0-only). The bundled
IBM Plex Mono font is licensed under the SIL Open Font License in
`assets/fonts/OFL.txt`. Both notices are unchanged.

## Native HEIF/AVIF codecs

These components are used only by the optional `heif` feature of
`image-to-binary-processing` and are built by `tools/native-codecs/build.sh`.
Default builds do not include or link them. The Rust WASM package does not
include them either; the separately built browser decoder does, as described
in the next section.

| Component | Version | License | Source | License text |
| --- | --- | --- | --- | --- |
| libheif | 1.23.7 | LGPL-3.0 (library); sample apps and wrappers are MIT and are not built | https://github.com/strukturag/libheif/releases/download/v1.23.7/libheif-1.23.7.tar.gz | `tools/native-codecs/licenses/libheif/COPYING` |
| libde265 | 1.1.3 | LGPL-3.0 (library); sample apps are MIT and are not built | https://github.com/strukturag/libde265/releases/download/v1.1.3/libde265-1.1.3.tar.gz | `tools/native-codecs/licenses/libde265/COPYING` |
| libaom | 3.15.3 | BSD-2-Clause, plus the AOM Patent License 1.0 | https://storage.googleapis.com/aom-releases/libaom-3.15.3.tar.gz | `tools/native-codecs/licenses/libaom/LICENSE` and `tools/native-codecs/licenses/libaom/PATENTS` |

Archive SHA-256 values are pinned in `tools/native-codecs/versions.env`.
The libaom third-party license files that ship in its source tree are copied
under `tools/native-codecs/licenses/libaom/third_party/`.

Rust bindings used by the feature, obtained from crates.io and not vendored:
`libheif-rs` 3.0.0 and `libheif-sys` 5.3.1+1.23.1, both MIT.

### LGPL-3.0 components in the `heif` build

libheif and libde265 (LGPL-3.0) are statically linked into builds that enable
the `heif` feature. The application source and the pinned build scripts in
`tools/native-codecs` are available under GPL-3.0-only, which lets a recipient
rebuild the application against a modified copy of these libraries. Whether
that satisfies LGPL-3.0 section 4 for a particular distribution has not been
decided here and must be reviewed before any distribution.

### HEVC patent licensing

The software licenses listed above do not resolve patent licensing for HEVC
(H.265) decoding, which libde265 and libheif's HEVC path perform for HEIF/HEIC
input. Distributing any build that includes these codecs requires its own
legal review. This notice makes no legal conclusion beyond that statement.

## Browser HEIF/AVIF decoder

`tools/wasm-codecs/build.sh` builds a decode-only Emscripten decoder from the
same pinned versions in `tools/native-codecs/versions.env` and links it into
`.local/wasm-codecs/dist/heif_decode.mjs` and `heif_decode.wasm`. When both
files exist, the web build emits the decoder as hashed assets and loads it only
in a dedicated decoder worker for HEIC, HEIF and AVIF input, created per decode
and terminated afterwards. Builds without those files use a stub and ship none
of the components below.

| Component | Version | License | Source | License text |
| --- | --- | --- | --- | --- |
| libheif | 1.23.7 | LGPL-3.0 (library) | https://github.com/strukturag/libheif/releases/download/v1.23.7/libheif-1.23.7.tar.gz | `tools/native-codecs/licenses/libheif/COPYING` |
| libde265 | 1.1.3 | LGPL-3.0 (library) | https://github.com/strukturag/libde265/releases/download/v1.1.3/libde265-1.1.3.tar.gz | `tools/native-codecs/licenses/libde265/COPYING` |
| libaom | 3.15.3 | BSD-2-Clause, plus the AOM Patent License 1.0 | https://storage.googleapis.com/aom-releases/libaom-3.15.3.tar.gz | `tools/native-codecs/licenses/libaom/LICENSE` and `tools/native-codecs/licenses/libaom/PATENTS` |
| Emscripten SDK | 6.0.13 | Build tool; its generated glue (`heif_decode.mjs`) is part of the decoder assets | https://github.com/emscripten-core/emsdk | Not analyzed in this notice |

`build.sh` removes libheif's Emscripten-only override that turns off HEVC
deblocking and SAO in its libde265 plugin, so the browser HEIC output matches the
native build (AVIF never used that path). The patch is applied to the verified
libheif source copy in the build tree; the archive hash is unchanged.

The glue source `tools/wasm-codecs/heif_decode.c` is part of this repository
and is covered by the repository license. The LGPL-3.0 and HEVC patent questions
recorded above for the native build apply equally to the browser decoder assets.
They have not been decided for that distribution. This notice draws no legal
conclusion about them.
