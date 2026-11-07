#!/usr/bin/env bash
# Builds unsigned local Windows x86_64 NSIS and MSI installers under .local/packages/windows-x86_64.
#
# Usage: tools/package/build-windows.sh   (run from Git Bash or MSYS2 on Windows)
# Requires: Windows host, x86_64-pc-windows-msvc target, cargo-tauri at .local/tauri-cli/bin.
# The MSI bundler downloads WiX Toolset during the build; this script does not pre-install it.
# Never signs, notarizes, uploads or publishes. Exit status 69 means the toolchain or host is missing.
set -euo pipefail

# shellcheck source=tools/package/common.sh
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/common.sh"

refuse_signing_environment
refuse_signing_configuration

TARGET_LABEL="windows-x86_64"
RUST_TARGET="x86_64-pc-windows-msvc"

case "$(uname -s)" in
    MINGW* | MSYS* | CYGWIN*) ;;
    *) unavailable "$TARGET_LABEL" "host is $(uname -s), not Windows; run this script on Windows" ;;
esac
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
run_tauri_build "nsis,msi"

NSIS_INSTALLER="$(find "$BUNDLE_ROOT/nsis" -mindepth 1 -maxdepth 1 -name '*-setup.exe' -print | head -n 1)"
MSI_INSTALLER="$(find "$BUNDLE_ROOT/msi" -mindepth 1 -maxdepth 1 -name '*.msi' -print | head -n 1)"
[ -n "$NSIS_INSTALLER" ] || die "no NSIS installer was produced under $BUNDLE_ROOT/nsis"
[ -n "$MSI_INSTALLER" ] || die "no MSI installer was produced under $BUNDLE_ROOT/msi"

RELEASE_EXECUTABLE="$DESKTOP_DIR/target/release/image-to-binary-desktop.exe"
[ -f "$RELEASE_EXECUTABLE" ] || die "missing release executable $RELEASE_EXECUTABLE"
check_no_smoke_markers "$RELEASE_EXECUTABLE"
check_no_codec_glue "$RELEASE_EXECUTABLE"

mkdir -p "$OUTPUT_DIR"
rm -f "$OUTPUT_DIR/SHA256SUMS.txt"
place_artifact "$NSIS_INSTALLER" "$OUTPUT_DIR/$(basename "$NSIS_INSTALLER")"
place_artifact "$MSI_INSTALLER" "$OUTPUT_DIR/$(basename "$MSI_INSTALLER")"
write_manifest "$OUTPUT_DIR"

echo "package: $OUTPUT_DIR/$(basename "$NSIS_INSTALLER")"
echo "package: $OUTPUT_DIR/$(basename "$MSI_INSTALLER")"
