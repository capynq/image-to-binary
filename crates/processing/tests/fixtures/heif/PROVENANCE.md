# HEIC and AVIF fixture provenance

`sample.heic` and `sample.avif` are generated from a procedural 64x48 RGB
gradient with no third-party content. The source PNG is written by
`tools/native-codecs/make_fixture_png.py` and is not stored in the repository.

Regenerate with `tools/native-codecs/make-fixtures.sh`, which runs:

```sh
python3 -I tools/native-codecs/make_fixture_png.py "$WORK/sample.png"
sips -s format heic "$WORK/sample.png" --out crates/processing/tests/fixtures/heif/sample.heic
sips -s format avif "$WORK/sample.png" --out crates/processing/tests/fixtures/heif/sample.avif
```

Both files were produced by macOS `sips` (system encoders, lossy). The tests
accept a small per-channel tolerance and expect the first decoded pixel near
HEIC `[0, 2, 121, 255]` and AVIF `[1, 0, 129, 255]`.
