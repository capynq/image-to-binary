#!/usr/bin/env bash
# Regenerates the procedural HEIC/AVIF fixtures in crates/processing/tests/fixtures/heif.
# Requires macOS sips (writes public.heic and public.avif) and ffmpeg with libsvtav1
# (10-bit AVIF). Output is lossy; decoded pixels are checked with tolerances.
# Set NATIVE_FIXTURES_DIR to write elsewhere, for example to verify regeneration.
# Set FFMPEG to choose an ffmpeg binary (default: ffmpeg on PATH).
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT_DIR="$REPO_ROOT/tools/native-codecs"
OUT="${NATIVE_FIXTURES_DIR:-$REPO_ROOT/crates/processing/tests/fixtures/heif}"
FFMPEG="${FFMPEG:-ffmpeg}"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
mkdir -p "$OUT"

png="$WORK/sample.png"
alpha_png="$WORK/sample_alpha.png"
grey_png="$WORK/sample_grey.png"
inverted_png="$WORK/sample_inverted.png"
python3 -I "$SCRIPT_DIR/make_fixture_png.py" "$png"
python3 -I "$SCRIPT_DIR/make_fixture_png.py" "$alpha_png" --alpha
python3 -I "$SCRIPT_DIR/make_fixture_png.py" "$grey_png" --grey
python3 -I "$SCRIPT_DIR/make_fixture_png.py" "$inverted_png" --inverted

sips -s format heic "$png" --out "$OUT/sample.heic" >/dev/null
sips -s format avif "$png" --out "$OUT/sample.avif" >/dev/null
sips -s format heic "$alpha_png" --out "$OUT/sample_alpha.heic" >/dev/null
sips -s format avif "$alpha_png" --out "$OUT/sample_alpha.avif" >/dev/null

python3 -I "$SCRIPT_DIR/add_transform.py" "$OUT/sample.heic" "$OUT/sample_irot90.heic" --rotate 90
python3 -I "$SCRIPT_DIR/add_transform.py" "$OUT/sample.heic" "$OUT/sample_imir_axis0.heic" --mirror-axis 0
python3 -I "$SCRIPT_DIR/add_transform.py" "$OUT/sample.heic" "$OUT/sample_imir_axis1.heic" --mirror-axis 1
python3 -I "$SCRIPT_DIR/add_transform.py" "$OUT/sample.avif" "$OUT/sample_irot270.avif" --rotate 270

sips -s format heic "$grey_png" --out "$OUT/sample_grey.heic" >/dev/null
"$FFMPEG" -hide_banner -loglevel error -y -i "$grey_png" -frames:v 1 \
    -pix_fmt gray -c:v libsvtav1 -crf 20 "$OUT/sample_grey.avif"

sips -s format heic "$inverted_png" --out "$WORK/sample_inverted.heic" >/dev/null
sips -s format avif "$inverted_png" --out "$WORK/sample_inverted.avif" >/dev/null
python3 -I "$SCRIPT_DIR/add_transform.py" "$OUT/sample.heic" "$OUT/sample_two_images.heic" \
    --combine-with "$WORK/sample_inverted.heic" --primary 2
python3 -I "$SCRIPT_DIR/add_transform.py" "$OUT/sample.avif" "$OUT/sample_two_images.avif" \
    --combine-with "$WORK/sample_inverted.avif" --primary 2

"$FFMPEG" -hide_banner -loglevel error -y -i "$png" -frames:v 1 \
    -pix_fmt yuv420p10le -c:v libsvtav1 -crf 20 "$OUT/sample_10bit.avif"

echo "$OUT/sample.heic"
echo "$OUT/sample.avif"
echo "$OUT/sample_alpha.heic"
echo "$OUT/sample_alpha.avif"
echo "$OUT/sample_irot90.heic"
echo "$OUT/sample_imir_axis0.heic"
echo "$OUT/sample_imir_axis1.heic"
echo "$OUT/sample_irot270.avif"
echo "$OUT/sample_10bit.avif"
echo "$OUT/sample_grey.heic"
echo "$OUT/sample_grey.avif"
echo "$OUT/sample_two_images.heic"
echo "$OUT/sample_two_images.avif"
