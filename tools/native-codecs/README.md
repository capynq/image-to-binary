# Native HEIF/AVIF codecs

This directory builds the pinned, decode-only native codecs used by the
optional `heif` feature of `image-to-binary-processing`:

- `versions.env`: pinned libheif, libde265, and libaom versions and SHA-256 archive hashes.
- `requirements.txt`: build tools (cmake, ninja, pkgconf) installed into `.local/tools`.
- `build.sh`: downloads, verifies, and builds the static libraries under `.local/native-codecs/<triple>/`. Re-running is a no-op when versions, hashes, and flags are unchanged.
- `cargo-with-codecs.sh`: runs cargo with `PKG_CONFIG_LIBDIR` and `SYSTEM_DEPS_LINK=static` for the host target, and fails if the prefix has not been built.
- `make-fixtures.sh` and `make_fixture_png.py`: regenerate the HEIC/AVIF test fixtures in `crates/processing/tests/fixtures/heif/` (macOS `sips` required).
- `licenses/`: license texts copied from the verified source archives. See `THIRD_PARTY_NOTICES.md`.

## Prerequisites

- Rust 1.88 via the project's `rust-toolchain.toml`.
- Build tools on `PATH` or in the project venv:
  `python3 -m venv .local/tools && .local/tools/bin/python -m pip install -r tools/native-codecs/requirements.txt`
- `clang`, `curl`, and `tar`. On macOS, the Xcode command line tools provide all three.
- x86_64 hosts also need `nasm` or `yasm`. Without it, libaom silently uses slow C code, so `build.sh` refuses to continue.

## Usage

```sh
tools/native-codecs/build.sh
tools/native-codecs/cargo-with-codecs.sh test -p image-to-binary-processing --features heif --locked
```

Default builds (`cargo build`, `cargo test --workspace`) and the
`wasm32-unknown-unknown` check need none of this. Without the `heif` feature,
HEIC and AVIF input returns `DecodeError::CodecUnavailable`, which the WASM
binding reports as `codec_unavailable`.

## Target status

| Target | Status |
| --- | --- |
| `aarch64-apple-darwin` (macOS arm64) | Verified: build, `heif` tests, and clean rebuild. |
| `x86_64-apple-darwin` | Not verified. Build branch exists in `build.sh`; needs nasm. |
| `x86_64-unknown-linux-gnu` | Not verified. Build branch exists; needs nasm. |
| `aarch64-unknown-linux-gnu` | Not verified. Build branch exists. |
| Windows | Not supported by these bash scripts and not verified. |

Cross-compiling is refused by `build.sh`; run it on the matching host.

## Per-platform notes

- macOS: `MACOSX_DEPLOYMENT_TARGET` is 11.0 in `versions.env`. The fixture
  generator uses `sips`, which is macOS-only.
- Linux: install `pkg-config` or rely on the venv `pkgconf`. The cargo wrapper
  sets `PKG_CONFIG_LIBDIR`, so system pkg-config files are not used.

## Missing-codec behavior

If the `heif` feature is off, or libheif reports no matching decoder
(`PluginLoadingError` / `NoMatchingDecoderInstalled`), HEIC and AVIF input
returns `DecodeError::CodecUnavailable(format)`. This is distinct from
`UnsupportedFormat`, so a later lazy decoder can be selected for it.
