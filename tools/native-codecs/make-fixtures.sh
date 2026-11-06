#!/usr/bin/env bash
# Regenerates the procedural HEIC/AVIF fixtures in crates/processing/tests/fixtures/heif.
# Requires macOS sips (writes public.heic and public.avif). Output is lossy; the
# decoded pixel at (0,0) is expected to be near (0,0,128), not exact.
# Set NATIVE_FIXTURES_DIR to write elsewhere, for example to verify regeneration.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT_DIR="$REPO_ROOT/tools/native-codecs"
OUT="${NATIVE_FIXTURES_DIR:-$REPO_ROOT/crates/processing/tests/fixtures/heif}"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
mkdir -p "$OUT"

png="$WORK/sample.png"
python3 -I "$SCRIPT_DIR/make_fixture_png.py" "$png"
sips -s format heic "$png" --out "$OUT/sample.heic" >/dev/null
sips -s format avif "$png" --out "$OUT/sample.avif" >/dev/null
echo "$OUT/sample.heic"
echo "$OUT/sample.avif"
