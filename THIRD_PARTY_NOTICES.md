# Third-party notices

The repository's own license is in `LICENSE` (GPL-3.0-only). The bundled
IBM Plex Mono font is licensed under the SIL Open Font License in
`assets/fonts/OFL.txt`. Both notices are unchanged.

## Native HEIF/AVIF codecs

These components are used only by the optional `heif` feature of
`image-to-binary-processing` and are built by `tools/native-codecs/build.sh`.
Default builds and the WASM build do not include or link them.

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
