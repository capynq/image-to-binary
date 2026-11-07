#!/usr/bin/env bash
# Verifies a local macOS package: signature state (unsealed, ad-hoc, no Developer ID), read-only .dmg
# contents, and optionally a launch-window-quit cycle of a scratch copy. Never installs, never touches
# /Applications, and quits only the scratch copy by its path.
#
# Usage: tools/package/verify-macos.sh [--launch] [package-dir]
#   package-dir defaults to .local/packages/macos-arm64.
set -euo pipefail

# shellcheck source=tools/package/common.sh
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/common.sh"

LAUNCH=0
if [ "${1:-}" = "--launch" ]; then
    LAUNCH=1
    shift
fi
PACKAGE="${1:-$PACKAGE_OUTPUT_ROOT/macos-arm64}"
APP_NAME="Image to Binary.app"
BINARY_NAME="image-to-binary-desktop"
EXPECTED_IDENTIFIER="dev.imagetobinary.desktop"
APP="$PACKAGE/$APP_NAME"
DMG="$(find "$PACKAGE" -mindepth 1 -maxdepth 1 -name '*.dmg' -print | head -n 1)"
[ -d "$APP" ] || die "missing $APP"
[ -n "$DMG" ] || die "missing .dmg in $PACKAGE"

SCRATCH="$(mktemp -d "$REPO_ROOT/.local/package-verify.XXXXXX")"
MOUNT_POINT="$SCRATCH/dmg-mount"
MOUNTED=0
PID=""

cleanup() {
    if [ -n "$PID" ] && kill -0 "$PID" 2>/dev/null; then
        kill "$PID" 2>/dev/null || true
        echo "cleanup: killed scratch pid $PID" >&2
    fi
    if [ "$MOUNTED" = "1" ]; then
        diskutil eject "$MOUNT_POINT" >/dev/null 2>&1 || diskutil eject force "$MOUNT_POINT" >/dev/null 2>&1 || true
    fi
    find "$SCRATCH" -depth -delete 2>/dev/null || true
    rmdir "$SCRATCH" 2>/dev/null || true
}
trap cleanup EXIT

echo "== signature state (unsealed, ad-hoc, no Developer ID)"
for target in "$APP" "$APP/Contents/MacOS/$BINARY_NAME"; do
    details="$(codesign -dvvv "$target" 2>&1 || true)"
    printf '%s\n' "$details" | grep -E "Signature|Authority|TeamIdentifier|Sealed Resources|Info.plist" || true
    if printf '%s\n' "$details" | grep -q "Authority=Developer ID"; then
        die "$target carries a Developer ID signature"
    fi
    printf '%s\n' "$details" | grep -q "Signature=adhoc" || die "$target is not ad-hoc signed"
    printf '%s\n' "$details" | grep -q "TeamIdentifier=not set" || die "$target has a team identifier"
done
printf '%s\n' "$(codesign -dvvv "$APP" 2>&1)" | grep -q "Sealed Resources=none" \
    || die "the app bundle is sealed; expected the unsealed local build"
echo "state: unsealed, ad-hoc signed, local use only"

echo "== dmg mounts read-only and contains the app"
mkdir "$MOUNT_POINT"
diskutil image attach --readOnly --mountOptions nobrowse --mountPoint "$MOUNT_POINT" "$DMG" >/dev/null
MOUNTED=1
mount | grep -F "$MOUNT_POINT" | grep -q "read-only" || die "dmg is not mounted read-only"
if touch "$MOUNT_POINT/.write-probe" 2>/dev/null; then
    die "dmg accepted a write"
fi
[ -d "$MOUNT_POINT/$APP_NAME" ] || die "dmg does not contain $APP_NAME"
echo "dmg: read-only mount holds: $(ls "$MOUNT_POINT" | tr '\n' ' ')"
diskutil eject "$MOUNT_POINT" >/dev/null
MOUNTED=0

if [ "$LAUNCH" = "0" ]; then
    echo "== launch skipped (pass --launch to run the scratch copy)"
    exit 0
fi

echo "== launch, window and quit (scratch copy only)"
LAUNCH_DIR="$SCRATCH/launch"
mkdir "$LAUNCH_DIR"
LAUNCH_APP="$LAUNCH_DIR/$APP_NAME"
ditto "$APP" "$LAUNCH_APP"
LAUNCH_BINARY="$LAUNCH_APP/Contents/MacOS/$BINARY_NAME"

WINDOW_SCRIPT="$SCRATCH/windows.js"
cat > "$WINDOW_SCRIPT" <<'JXA'
ObjC.import('CoreGraphics');
function run(argv) {
    const pid = Number(argv[0]);
    const windows = ObjC.deepUnwrap(ObjC.castRefToObject($.CGWindowListCopyWindowInfo(1, 0)));
    const mine = windows.filter(w => w.kCGWindowOwnerPID === pid && w.kCGWindowLayer === 0 && w.kCGWindowBounds.Width > 0);
    return JSON.stringify(mine.map(w => ({ width: w.kCGWindowBounds.Width, height: w.kCGWindowBounds.Height })));
}
JXA

open -n "$LAUNCH_APP"
for _ in $(seq 1 30); do
    PID="$(pgrep -f "$LAUNCH_BINARY" | head -n 1 || true)"
    [ -n "$PID" ] && break
    sleep 1
done
[ -n "$PID" ] || die "the scratch app did not start"
echo "started pid $PID"

WINDOWS=""
for _ in $(seq 1 30); do
    WINDOWS="$(osascript -l JavaScript "$WINDOW_SCRIPT" "$PID" 2>/dev/null || true)"
    case "$WINDOWS" in
        *'"width"'*) break ;;
    esac
    sleep 1
done
case "$WINDOWS" in
    *'"width"'*) echo "window bounds: $WINDOWS" ;;
    *) die "no on-screen window appeared for pid $PID" ;;
esac

osascript -e "tell application \"$LAUNCH_APP\" to quit" >/dev/null 2>&1 || true
for _ in $(seq 1 20); do
    if ! kill -0 "$PID" 2>/dev/null; then
        break
    fi
    sleep 1
done
if kill -0 "$PID" 2>/dev/null; then
    die "pid $PID did not quit after the quit request"
fi
PID=""
if pgrep -f "$LAUNCH_BINARY" >/dev/null 2>&1; then
    die "a scratch app process is still running after quit"
fi
echo "quit: scratch process exited cleanly; no scratch process remains"
