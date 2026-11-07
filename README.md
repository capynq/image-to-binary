# Image to Binary

A local image-to-binary-art desktop tool. The current reference application is
Python/Tkinter, while the Rust migration is being implemented in the workspace
described by [docs/PLAN.md](docs/PLAN.md). The Rust library is not yet a desktop
or browser replacement.

## Desktop status

The Tauri shell in `crates/desktop` runs the studio UI natively. Open image,
drop, Load demo, preview, PNG export to a chosen file, cancel, and reset call
the native commands and report the same errors as the browser. On macOS arm64
the `--smoke` run passes 42 of 42 checks: exported PNGs are byte-equal to the
browser references, and the largest frame gap during a 1200 by 1200 export is
18 ms. Real OS dialogs and drag and drop are not automated, and Windows and
Linux have not been run.

### Desktop keyboard, menus and appearance

The native menu bar (macOS arm64 verified) binds these accelerators, where
`CmdOrCtrl` is Cmd on macOS:

- `CmdOrCtrl+O` opens an image; `CmdOrCtrl+E` exports; `CmdOrCtrl+1` shows the
  original and `CmdOrCtrl+2` the artwork; `CmdOrCtrl+R` renders.
- `Escape` cancels a running job. It is handled by the page, not the menu, and
  never fires while focus is in a text field, so it cannot take a typed key.
- Menu accelerators fire while a modifier is held even in text fields, because
  the menu consumes the key before the field sees it. Typing a plain letter in
  the ramp field is never intercepted.
- Windows and Linux shortcut behaviour is unverified. On those systems the
  webview may claim `Ctrl+R` as a reload before the menu sees it.

The header System, Light and Dark control stores the choice under
`image-to-binary-theme-choice`, and `System` is stored explicitly. The earlier
`image-to-binary-theme` key is deliberately not migrated: it was written on every
load, so it cannot tell a real choice from the default.

## Rust migration status

The Rust workspace currently provides normalized RGBA8 decoding for six image
formats (BMP, GIF, JPEG, PNG, TIFF, and WebP), checked resource limits with a
50-megapixel default ceiling, classic and Average source sampling, and a
bounded `fontdue` glyph renderer. Phases 2.4 and 2.5 are complete. The Rust
renderer uses the bundled IBM Plex Mono font, so its glyph rasterization is
intentionally not byte-for-byte compatible with the Python/Pillow reference
font raster.

The new Average preset applies font-aspect correction through rounded source
and output cell geometry. Edge rounding is explicit and bounded to the source
cell; classic geometry remains the compatibility path. Rendered cells use an
opaque green foreground and opaque black background with symmetric cell
clipping, rather than relying on legacy glyph spill outside a cell. Preview
renders are capped at a 1600-pixel longest edge and reuse the export source
grid. Glyph-mask cache accounting is bounded to mask payload capacity and
excludes font, metadata, and external allocations.

The browser app decodes HEIC, HEIF and AVIF through a lazily loaded decoder
worker (phase 4.2). That path exists only in builds made with
`tools/wasm-codecs/build.sh`; other builds report that HEIC and AVIF are not
available. On the 13 repository HEIC/AVIF fixtures, the browser RGBA and exported
PNG bytes match the native decoder exactly in Chromium 154.0.8037.0, WebKit 26.6
and Firefox 156.0, and the Node glue matches on the same fixtures with Node
v24.15.0. These runs used the local probe `docs/reports/playwright/heif-acceptance.mjs`;
their JSON reports are local evidence, not a CI gate. Orientation and fixture
acceptance (phase 4.3) and the native/browser replacement remain later migration
phases. The Python
application remains the supported reference UI until the documented acceptance
gates pass.

Rust checks use the pinned 1.88.0 toolchain and the project-local `.local`
toolchain environment when available. CI runs:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo check --workspace --target wasm32-unknown-unknown --locked
```

Run the release benchmark harness for one measured size or all three:

```sh
cargo run -p image-to-binary-processing --release --example core_bench -- 1
cargo run -p image-to-binary-processing --release --example core_bench -- 12
cargo run -p image-to-binary-processing --release --example core_bench -- 50
cargo run -p image-to-binary-processing --release --example core_bench -- all
```

The harness uses deterministic checker/gradient fixtures and reports retained
payload counts separately from process RSS. Native PNG output streams to a
file; browser chunks retain encoded payload for Blob creation. Partial output
on any export error must be discarded. Fast compression and NoFilter are
benchmark settings, not quality or cross-platform performance guarantees.

## Frontend development

The Rust processing crate includes pixel-budgeted sampling and owned preview
rendering tasks with generation and cancellation checks at call boundaries.
The `crates/wasm` session exposes both to WASM (see "WASM bindings"); neither
makes synchronous setup operations interruptible. The phase 3.1
frontend uses the pinned Node.js 24.15.0 toolchain in
`frontend/.node-version`. Install and run its local checks from `frontend/`:

```sh
npm ci
npm run dev
npm test
npm run typecheck
npm run build
npm run format:check
```

`npm run dev`, `npm run build` and `npm run typecheck` first run
`npm run wasm:build` (`node ../crates/wasm/scripts/build.mjs`). That step needs
the Rust toolchain and the already installed `wasm-bindgen-cli` 0.2.128, installs
nothing, and writes the ignored package in `.local/wasm/pkg`, which the worker
imports. `npm test` never builds WASM and never uses the network.

The phase 3.1 frontend scaffold, the phase 3.2.1 latest-job scheduler and the
phase 3.2.3 worker transport are implemented, and parent phase 3.2 is complete.
The app runs the Rust WASM engine
in a lazily created module worker: "Load demo" imports a deterministic 256×192
RGBA fixture through the worker, and "Render preview" returns real Rust preview
pixels that the main thread encodes to a PNG object URL. Input buffers are
transferred to the worker, but JS to WASM still copies the source once and each
returned preview frame is a copy out of WASM memory, so these memory-copy costs
remain. Cancellation is cooperative between bounded `advance` calls and cannot
interrupt a single synchronous WASM call. Work yields through `MessageChannel`
between calls, and the yielder periodically (every eighth yield) uses a timer
task instead, so timers are not starved by `MessageChannel` yields alone; this
is fairness between yields, not preemption of any single call.

Browser evidence is local and bounded: on one macOS host, isolated Chromium ran
both the development server and the production bundle with the real Rust
worker. It covered worker rendering, input buffer transfer, newest-generation
cancellation, live UI heartbeats during processing, golden parity with the
native reference, and real React StrictMode ownership transitions. Firefox,
Windows and Linux remain unverified. No general performance guarantee is made.
PNG download (3.4) and text export (6.3) remain pending, so Export is disabled.

Phases 3.3.3.1 and 3.3.3.2 connect encoded imports and bounded original-preview
requests through the worker, adapter and controller. File reads are lazy,
encoded buffers are transferred, and decode yields before publishing its
source so queued cancellation can be observed. The controller owns original
and artwork previews independently: settings invalidate artwork only, and
failed replacements retain the accepted source and both previews. The 184
frontend tests cover these contracts.

Phase 3.3.3 browser acceptance passed through the real worker APIs in local
Chromium and WebKit: nine existing native fixtures matched original/artwork
pixels, including rotated JPEG and first-frame/page GIF/TIFF. The probes also
verified transferred buffers, failed/cancelled/superseded replacement retention,
bounded originals, and object-URL cleanup. A 3000x2000 source produced a
1600x1066 original while timer and animation-frame heartbeats continued. These
development-server probes do not claim production-bundle file-import coverage.
Phase 3.3.4 adds the native file picker, whole-window single-file drop,
Original/Artwork selection and metadata. Accepted imports open the bounded
original automatically; failed or cancelled replacements preserve the current
image and view. Local Chromium and WebKit checks cover keyboard controls,
long names, narrow/short windows, both themes and production reflow at device
scale 4. Manual screen-reader testing remains unavailable.

Phase 3.4.5 PNG download acceptance (production bundle, `vite preview` on
127.0.0.1:4173, real Rust worker; probe in `docs/reports/playwright/`):

- Chromium 154, WebKit 26.6 and Firefox 156 (macOS arm64): all six sections ran
  without deadline or timeout. Section a byte-compared 9/9 encoded fixtures and
  5/5 native reference cases against native PNG bytes; repeated exports were
  byte-identical (`encodedExportAgainByteEqual`: 9/9). Sections b (reset revokes
  the export URL), c (cancel, replace-source and change-setting during export
  leave no link or download event, announce "PNG export cancelled.", and a
  following export matches native), and d (20 import/export/reset cycles: live
  object URLs are 0 after every reset, live workers go from 0 to 1 at cycle 1
  and do not grow after it, and all 20 downloads match native) passed in all
  three engines.
- Keyboard (section e): Chromium and Firefox reached Export and Cancel by Tab
  (`exportReachedByTab`, `cancelReachedByTab`), and Tab from Download moved focus
  to Export again (`focusAfterTabFromDownload`). WebKit did not reach Export by
  Tab (`exportReachedByTab: false`, `bodyFocusWhileTabbingToExport: 120`), so its
  section e focused Export programmatically and verified Enter activation, focus
  after Cancel and Export, and the downloaded bytes; Tab reachability was not
  verified there. From the Export step onward, the requestAnimationFrame sampler
  recorded no body-focus frames (`sampledFramesWithBodyFocus: 0`) in each engine.
  During that interaction the footer live region stayed at "Preview ready"
  (`footerLiveValues`), and the export panel announced the lifecycle.
- Responsive (section f): 320, 768, 1200 and 1920 CSS px, each in idle, ready,
  running and failed states: 16 screenshots per browser, no horizontal overflow,
  and every Export/Cancel/Download/Retry control at least 44 px and reachable.
  Running was held at the worker boundary; failed was forced by an unknown source
  id in one export message. Focus was never on the "Skip to workspace" link in
  the ready state (`skipLinkFocused: false`).
- Firefox launched only with `HOME` and `CFFIXED_USER_HOME` pointed at
  `.local/firefox-home`; the earlier profile-folder error did not recur.
- Not verified: manual screen-reader testing (unavailable), native Windows and
  Linux, Safari-specific keyboard modes beyond the WebKit result above, and any
  performance guarantee. The single 1200 px classic export measured 32 ms
  (Chromium), 36 ms (WebKit) and 38 ms (Firefox) from click to download link.
  Text export (6.3) remains pending.

## WASM bindings

`crates/wasm` (`image-to-binary-wasm`) pins `wasm-bindgen =0.2.128` and
`js-sys =0.3.105`; the generating `wasm-bindgen-cli` must be exactly 0.2.128.
It exports one `ImageSession` that owns the normalized source, the sample
cache, the renderer and at most one job, plus `PreviewFrame` and `JobStage`.
It needs Rust 1.88.0 with the `wasm32-unknown-unknown` target and Node 24:

```sh
node crates/wasm/scripts/setup.mjs   # installs wasm-bindgen-cli 0.2.128 into .local/wasm-tools if absent
node crates/wasm/scripts/build.mjs   # release wasm32 build + wasm-bindgen --target web into .local/wasm/pkg
node crates/wasm/scripts/test.mjs    # native reference fixture, then node --test on the generated package
cargo test -p image-to-binary-wasm --locked
```

Only `setup.mjs` installs anything, and only under `.local`; build and test
never install. Explicit `CARGO_HOME`, `RUSTUP_HOME` and `PATH` win, with
`.local` toolchains used as fallbacks. Generated output stays ignored.

Contract and constraints:

- `ImageSession.fromRgba(width, height, pixels)` (or `fromRgbaWithMaxPixels`)
  validates numbers, the 50 MP ceiling, byte budget and exact length before
  copying `pixels` once into WASM memory; JS may drop its array afterwards.
  A 50 MP source needs about 200 MB in JS plus 200 MB in WASM at construction.
- `ImageSession.fromEncoded(bytes)` decodes BMP, GIF, JPEG, PNG, TIFF or WebP
  bytes with the Rust decoder. The getters `sourceWidth`, `sourceHeight` and
  `sourceChannels` return post-orientation metadata (channels is always 4).
  The type and 256 MiB length are checked before the single fallible copy, and
  the caller's array is never retained or mutated. GIF and TIFF use the first
  frame or page. Decoding is synchronous, and codec allocations are best
  effort. Node/WASM tests cover all six codecs, orientation, first page, stable
  error codes and buffer ownership. Worker and adapter wiring is implemented;
  the browser file UI is implemented in 3.3.4.
- `startOriginalPreview(generation)` starts a bounded preview of the original
  image. It uses the same `advance`/`cancel`/`finish` lifecycle as
  `startPreview`, and generations are strictly increasing and shared across
  both modes. The output longest edge is at most 1600 pixels and a source is
  never upscaled. Resampling uses nearest-center mapping and preserves alpha
  and the post-orientation aspect ratio. It reads the session's same private
  `Arc` decoded source and never clones the full source. Output reservation is
  at most 10.24 MB (1600 x 1600 x 4). Calls are synchronous, and a call already
  in progress is not preempted; cancellation and stale checks happen only at
  call boundaries. Worker API browser acceptance passed in 3.3.3.3; the import
  UI acceptance is complete in 3.3.4.
- Numeric arguments must be JS numbers holding safe integers; null, booleans,
  strings, NaN, fractions, negatives and out-of-range values are rejected,
  never coerced. Generations must strictly increase (zero is valid first).
- `startPreview` → `advance(generation, pixelBudget)` → `finish(generation)`.
  Each advance works only in the current stage (`Sampling`, then `Preview`);
  the call finishing a stage also performs non-interruptible setup but no
  next-stage work. `cancel`, stale generations and terminal errors cannot be
  revived; a newer generation can start. `dispose()` is idempotent; `free()`
  invalidates the handle.
- `finish` returns a frame once, only when `Ready`. `takePixels()` returns a
  fresh `Uint8Array` copy (not a view into WASM memory) once. WASM linear
  memory never shrinks, so released memory is reusable but not returned.
  The copy is independent of later memory growth. `takePixels()` consumes the
  pixel buffer, but the `PreviewFrame` wrapper still needs `free()`.
- Errors are thrown `Error` objects named `ImageToBinaryError` with a `code`:
  `invalid_argument`, `invalid_dimensions`, `invalid_settings`,
  `invalid_progress`, `resource_limit_exceeded`, `arithmetic_overflow`,
  `cancelled`, `stale_generation`, `disposed`, `no_job`, `job_incomplete`,
  `frame_consumed`, `encoded_too_large`, `unsupported_format`, `decode_failed`,
  `reservation_failed`, `internal`.

Scope: phase 3.2 and the encoded-source and original-preview bindings in
3.3.1–3.3.2 are complete. Validation includes native tests, strict Clippy,
formatting, the workspace wasm32 check, the target-web build and 59 real Node
binding tests covering codec/orientation fixtures, source/grid validation,
preview parity, cancellation, generations, copy semantics and declarations.
The worker and file adapter/controller consume these bindings, with browser
acceptance complete through the import UI in 3.3.4. Exports (3.4) and the native
shell remain pending. No Firefox, Windows or Linux test coverage, deployment or
performance claim is made.

## Python reference behavior

Open or drop an image, adjust cell and font sizes, inspect the two previews,
and save the result as PNG. Dark samples become green `1` characters and light
samples become `0`, on an opaque black background. Processing and export stay
on your machine; the application does not upload images.

The picker offers JPEG, PNG, and BMP. Processing uses Pillow, grayscale
conversion, nearest-neighbor sampling, and a threshold of 128. Output dimensions
are rounded down to whole cells; the displayed preview is limited to 256 pixels.
ASCII presets, source-colored characters, text exports, HEIC/AVIF coverage,
responsive layouts, and browser support are future work, not current guarantees.

## Baseline setup

The dependency metadata targets CPython 3.11–3.14 on Windows, macOS, and Linux.
Local headless checks passed on Python 3.14.7/macOS; other environments are
configured in CI but have not been verified here. Create a virtual environment
and install the pinned dependencies:

```sh
python3 -m venv .venv
.venv/bin/python -m pip install -r requirements.txt
```

Install the development checks as well when working on the project:

```sh
.venv/bin/python -m pip install -r requirements-dev.txt
```

On Windows, create the environment with `py -3 -m venv .venv` and use
`.venv\Scripts\python.exe` in place of `.venv/bin/python`.
Tkinter is part of the Python installation, but some Linux distributions ship
it separately (for example, install the distribution's `python3-tk` package).
TkinterDnD2 additionally requires a supported Tk runtime. Run the application
with:

```sh
.venv/bin/python main.py
```

The bundled `assets/fonts/IBMPlexMono-Regular.ttf` test font is IBM Plex Mono
from the Google Fonts repository revision
`633f3200539c52ee0aba2dfd7f46921417a81877` and is provided under the SIL Open
Font License; its notice is retained beside the font in `assets/fonts/OFL.txt`.

## Validation

```sh
.venv/bin/python -m pip check
.venv/bin/python -m compileall -q main.py image_processor.py tests
.venv/bin/python -m unittest discover -s tests -v
.venv/bin/python -m ruff format --check main.py image_processor.py tests
.venv/bin/python -m ruff check main.py image_processor.py tests
```

The 14 headless tests use deterministic, generated image fixtures and the bundled
font. UI callback tests execute the original class with fake UI boundaries, so
they do not need a display or substitute a second implementation. Three expected
failures describe bugs still present: tiny-image resizing, reset retaining an
exportable image, and multiple dropped paths becoming one invalid path. An
unexpected success fails the suite, prompting the defect record to be updated.
These checks do not establish GUI or cross-platform visual acceptance.

The formatting and lint commands require the development dependencies. A
headless environment without `_tkinter` can still run the processor checks, but
cannot launch the GUI; install a Tk-enabled Python to verify the desktop app.

[Checks CI](.github/workflows/checks.yml) runs dependency consistency, syntax,
and headless regression checks on Python 3.11–3.14/Linux and Python 3.14 on
Windows/macOS. Official actions are pinned by commit, permissions are read-only,
dependencies are cached, and superseded runs are cancelled. There are no deploy,
release, signing, or artifact-upload steps. Remote CI has not run for these local
changes yet.

## Limitations and troubleshooting

- Processing is synchronous and can freeze the fixed-size window on large
  images. No benchmark or 50-megapixel support claim is made for this baseline.
- Inputs smaller than a cell can fail. Reset does not yet remove the previous
  export. Avoid saving after reset until a new image is processed.
- The cell slider accepts 5–30 while typed values accept 8–30. Drag-and-drop
  handling is naïve for multiple paths; use the picker for ambiguous paths.
- `ModuleNotFoundError: _tkinter` means the Python installation lacks Tk.
  Install a Tk-enabled Python/distribution package and recreate the virtual
  environment. Headless tests still work without Tk. GUI launch could not be
  verified on the current host for this reason.
- A TkinterDnD load error indicates a Tk/runtime/platform mismatch. This is
  separate from image decoding; check your Python and Tk installation first.

## Repository and contribution

`main.py` owns the Tk UI, `image_processor.py` owns baseline rendering,
`tests/` contains regression checks, and `assets/fonts/` supplies the portable
reference font and notice. Dependency pins live in `requirements.txt` and
`pyproject.toml`; keep both consistent when changing them.

Follow the numbered subphases in [the migration plan](docs/PLAN.md). Each heading
has a scope, checks, and a separate proposed commit. Keep Python until the
replacement passes its acceptance gates; retain fixtures and licensing notices.
Local `AGENTS.md`, UX skills, and generated `docs/reports/` are intentionally
ignored. Keep shared documentation and source fixtures version-controlled.

## License

The application is licensed under [GPLv3](LICENSE). The bundled IBM Plex Mono
font has its own [SIL Open Font License](assets/fonts/OFL.txt). Preserve both
notices when redistributing. Future native codec dependencies need a separate
license and packaging review before distribution.
