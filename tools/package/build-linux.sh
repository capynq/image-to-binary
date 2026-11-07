#!/usr/bin/env bash
# Builds unsigned local Linux x86_64 .deb and AppImage packages under .local/packages/linux-x86_64.
#
# Usage: tools/package/build-linux.sh
# Requires: x86_64 Linux host, x86_64-unknown-linux-gnu target, cargo-tauri at .local/tauri-cli/bin,
# and the WebKitGTK 4.1 development packages that Tauri links against.
# The AppImage bundler fetches linuxdeploy at build time; this script does not pre-fetch it.
# Never signs, notarizes, uploads or publishes. Exit status 69 means the toolchain or host is missing.
set -euo pipefail

# shellcheck source=tools/package/common.sh
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/common.sh"

refuse_signing_environment
refuse_signing_configuration

TARGET_LABEL="linux-x86_64"
RUST_TARGET="x86_64-unknown-linux-gnu"

[ "$(uname -s)" = "Linux" ] || unavailable "$TARGET_LABEL" "host is $(uname -s), not Linux; run this script on Linux"
[ "$(uname -m)" = "x86_64" ] || unavailable "$TARGET_LABEL" "host is $(uname -m), not x86_64"
command -v pkg-config >/dev/null 2>&1 || unavailable "$TARGET_LABEL" "pkg-config is not on PATH"
pkg-config --exists webkit2gtk-4.1 \
    || unavailable "$TARGET_LABEL" "webkit2gtk-4.1 development files are missing (pkg-config)"
require_rust_target "$RUST_TARGET" "$TARGET_LABEL"
require_tauri_cli "$TARGET_LABEL"

VERSION="$(crate_version)"
[ -n "$VERSION" ] || die "could not read the version from $DESKTOP_DIR/Cargo.toml"
OUTPUT_DIR="$PACKAGE_OUTPUT_ROOT/$TARGET_LABEL"
BUNDLE_ROOT="$DESKTOP_DIR/target/release/bundle"
require_empty_output "$OUTPUT_DIR"

echo "package: building $TARGET_LABEL version $VERSION (default features, no signing)"
build_frontend
generate_notices "$RUST_TARGET"
run_tauri_build "deb,appimage"

DEB_PACKAGE="$(find "$BUNDLE_ROOT/deb" -mindepth 1 -maxdepth 1 -name '*.deb' -print | head -n 1)"
APPIMAGE_PACKAGE="$(find "$BUNDLE_ROOT/appimage" -mindepth 1 -maxdepth 1 -name '*.AppImage' -print | head -n 1)"
[ -n "$DEB_PACKAGE" ] || die "no .deb was produced under $BUNDLE_ROOT/deb"
[ -n "$APPIMAGE_PACKAGE" ] || die "no AppImage was produced under $BUNDLE_ROOT/appimage"

RELEASE_EXECUTABLE="$DESKTOP_DIR/target/release/image-to-binary-desktop"
[ -f "$RELEASE_EXECUTABLE" ] || die "missing release executable $RELEASE_EXECUTABLE"
check_no_smoke_markers "$RELEASE_EXECUTABLE"
check_no_codec_glue "$RELEASE_EXECUTABLE"

mkdir -p "$OUTPUT_DIR"
rm -f "$OUTPUT_DIR/SHA256SUMS.txt"
place_artifact "$DEB_PACKAGE" "$OUTPUT_DIR/$(basename "$DEB_PACKAGE")"
place_artifact "$APPIMAGE_PACKAGE" "$OUTPUT_DIR/$(basename "$APPIMAGE_PACKAGE")"
write_manifest "$OUTPUT_DIR"

echo "package: $OUTPUT_DIR/$(basename "$DEB_PACKAGE")"
echo "package: $OUTPUT_DIR/$(basename "$APPIMAGE_PACKAGE")"
