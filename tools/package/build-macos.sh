#!/usr/bin/env bash
# Builds an unsigned local macOS arm64 .app and .dmg under .local/packages/macos-arm64.
#
# Usage: tools/package/build-macos.sh
# Requires: Apple Silicon host, aarch64-apple-darwin target, cargo-tauri at .local/tauri-cli/bin.
# Never signs, notarizes, uploads or publishes. Exit status 69 means the toolchain is missing.
set -euo pipefail

# shellcheck source=tools/package/common.sh
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/common.sh"

refuse_signing_environment
refuse_signing_configuration

TARGET_LABEL="macos-arm64"
RUST_TARGET="aarch64-apple-darwin"
BINARY_NAME="image-to-binary-desktop"
APP_NAME="Image to Binary.app"
EXPECTED_IDENTIFIER="dev.imagetobinary.desktop"

[ "$(uname -s)" = "Darwin" ] || unavailable "$TARGET_LABEL" "host is $(uname -s), not macOS"
[ "$(uname -m)" = "arm64" ] || unavailable "$TARGET_LABEL" "host is $(uname -m), not arm64"
command -v codesign >/dev/null 2>&1 || unavailable "$TARGET_LABEL" "codesign is not available"
command -v hdiutil >/dev/null 2>&1 || unavailable "$TARGET_LABEL" "hdiutil is not available"
require_rust_target "$RUST_TARGET" "$TARGET_LABEL"
require_tauri_cli "$TARGET_LABEL"

VERSION="$(crate_version)"
[ -n "$VERSION" ] || die "could not read the version from $DESKTOP_DIR/Cargo.toml"
OUTPUT_DIR="$PACKAGE_OUTPUT_ROOT/$TARGET_LABEL"
BUNDLE_ROOT="$DESKTOP_DIR/target/release/bundle"
require_empty_output "$OUTPUT_DIR"

echo "package: building $TARGET_LABEL version $VERSION (default features, no signing, no codec glue)"
build_frontend
generate_notices "$RUST_TARGET"
# Tauri passes --skip-jenkins only when CI=true. That skips the Finder layout AppleScript, which
# times out without an interactive Finder session. The layout is cosmetic; the DMG keeps the app and
# the Applications link.
CI=true run_tauri_build "app,dmg"

BUILT_APP="$BUNDLE_ROOT/macos/$APP_NAME"
[ -d "$BUILT_APP" ] || die "expected app bundle at $BUILT_APP"
BUILT_DMG="$(find "$BUNDLE_ROOT/dmg" -mindepth 1 -maxdepth 1 -name '*.dmg' -print | head -n 1)"
[ -n "$BUILT_DMG" ] || die "no .dmg was produced under $BUNDLE_ROOT/dmg"

BINARY="$BUILT_APP/Contents/MacOS/$BINARY_NAME"
[ -x "$BINARY" ] || die "missing executable $BINARY"
check_no_smoke_markers "$BINARY"
check_no_codec_glue "$BINARY"

PLIST="$BUILT_APP/Contents/Info.plist"
plist_value() {
    /usr/libexec/PlistBuddy -c "Print :$1" "$PLIST"
}
[ "$(plist_value CFBundleIdentifier)" = "$EXPECTED_IDENTIFIER" ] || die "unexpected CFBundleIdentifier"
[ "$(plist_value CFBundleShortVersionString)" = "$VERSION" ] || die "bundle version does not match Cargo.toml $VERSION"
[ "$(plist_value LSMinimumSystemVersion)" = "11.0" ] || die "unexpected LSMinimumSystemVersion"
MINOS="$(otool -l "$BINARY" | awk '/LC_BUILD_VERSION/{f=1} f && /minos/{print $2; exit}')"
[ "$MINOS" = "11.0" ] || die "binary minos is $MINOS, expected 11.0"
for notice in licenses/LICENSE licenses/THIRD_PARTY_NOTICES.md licenses/OFL.txt; do
    [ -f "$BUILT_APP/Contents/Resources/$notice" ] || die "missing bundled notice Contents/Resources/$notice"
done
NOTICES="$BUILT_APP/Contents/Resources/licenses/THIRD_PARTY_LICENSES.txt"
[ -s "$NOTICES" ] || die "missing or empty bundled notices Contents/Resources/licenses/THIRD_PARTY_LICENSES.txt"

mkdir -p "$OUTPUT_DIR"
rm -f "$OUTPUT_DIR/SHA256SUMS.txt"
place_artifact "$BUILT_APP" "$OUTPUT_DIR/$APP_NAME"
place_artifact "$BUILT_DMG" "$OUTPUT_DIR/$(basename "$BUILT_DMG")"
write_manifest "$OUTPUT_DIR"

echo "package: $OUTPUT_DIR/$APP_NAME"
echo "package: $OUTPUT_DIR/$(basename "$BUILT_DMG")"
