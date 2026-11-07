# Image to Binary migration plan

Status: subphases 0.2, 1.1, 1.2, 1.3, 2.1, 2.2, 2.3, 2.4, and 2.5 are
committed. Subphase 3.1 is complete.
Subphases 3.2.1 through 3.2.3, and therefore parent 3.2, are also complete.
See the validation records below.

## Initial prototype baseline

The current application is a Windows-oriented Tkinter/TkinterDnD shell around
Pillow, NumPy, and a synchronous `ImageProcessor`. It converts an image to
grayscale, resizes with nearest-neighbor sampling, applies a threshold of 128,
and paints `0`/`1` glyphs in green on black. The UI has fixed dimensions,
hard-coded `C:/Windows/Fonts/times.ttf`, a limited file filter, synchronous
slider work, and PNG-only output.

Known baseline defects to preserve as regression fixtures or explicitly retire:

- Tiny inputs can resize to a zero width or height and fail.
- Reset clears the labels but leaves the processed image exportable.
- Entry validation accepts 8–30 while the cell-size slider exposes 5–30.
- Every slider event performs synchronous processing on the UI thread.
- The font path only exists on one Windows installation.
- Drag-and-drop path parsing is naïve for quoted paths, spaces, and multiple
  dropped files.

## Architecture and scope

The product remains Python until the replacement is accepted. The target is a
shared Rust core with a React/TypeScript/Vite UI, exposed through a browser Rust
WASM worker and a Tauri native shell for Windows, macOS, and Linux. The core
owns decoding, normalization, sampling, glyph masks, rendering, export, limits,
and deterministic contracts; adapters own platform I/O and scheduling.

The classic binary tool remains compatible: nearest-neighbor resize with the
existing floor-grid geometry, threshold 128, dark pixels mapped to `1`, and
green-on-black output. New rendering presets use average luminance/color and
font-aspect correction; those improvements are not retroactively claimed as
byte-for-byte classic parity. The expanded product also supports ASCII and
block modes, a custom printable ASCII ramp of at most 64 characters,
monochrome foreground/background colors, and source-colored output. PNG and
UTF-8 text are export formats.

Source data is decoded once, orientation-normalized, and represented in an
8-bit primary path with alpha handling. JPEG, PNG, WebP, BMP, TIFF (first page),
and GIF (first frame) are explicit image-crate formats. HEIF/HEIC uses the
primary image and AVIF uses its primary item. HEIF/HEIC and AVIF are v1 formats
with pinned native decode-only
`libheif` plus `libde265` and `libaom`; the embedded libheif build must not
assume codecs are included. The browser uses an Emscripten build with both
codecs enabled, lazy temporary decoder workers, worker termination, and
transferred RGBA buffers.

Resource limits are part of the contract: reject or gracefully lower work for
large inputs (50 MP cap before allocation where possible), with browser limits
allowed to be lower. Native processing stays off the UI thread and uses binary
IPC rather than huge JSON or base64 payloads.

New rendering presets use average luminance/color and font-aspect correction.
Classic mode retains its nearest-neighbor sampling path. Caches are private
and bounded where appropriate: sampled cells, luminance/colors, glyph masks,
and preview artifacts. The preview has a longest edge of 1600 and uses the
same source grid as export, with viewport zoom/pan. There is one active job and
one newest pending job; chunked work yields between synchronous WASM calls, and
cancellation plus stale-generation rejection prevents obsolete results.

Native PNG export streams rows to a file sink. Browser export accumulates
encoded chunks in a Blob and necessarily retains the encoded result for the
download path; this is a bounded-output design, not a claim of constant-memory
browser export.

The UI direction is a graphite/cobalt studio with light and dark themes,
native decorations, a resizable default 1280×800 logical window, and a
provisional, content-tested minimum area of 800×600. It has original/artwork views, split comparison, zoom,
pan, fit, keyboard focus, progress, cancellation, errors, metadata, reset, and
disposal. Layout contracts:

- 320–767 CSS px: one column, controls below, wrapped toolbar, normal vertical
  scrolling.
- 768–1199 px: canvas first with a collapsible inspector.
- 1200 px and above: canvas plus an independently scrolling approximately
  320 px inspector.

Use intrinsic sizing and container queries; resize must preserve state. Canvas
rendering may use a 2D reflow exception, but controls must always reflow.
Touch targets are at least 44 px. Support reduced motion, high contrast, visible
keyboard focus, 200% text, 320 px CSS reflow, and 400% zoom without masked
overflow. Long names and full error text remain accessible.

The GPLv3 notice is preserved. CI validates source, tests, and build outputs
only; it does not deploy, publish artifacts, sign releases, or make release
decisions.

## Gates and non-goals

Each numbered subphase below is one exact commit/review boundary. The commit
must contain only that subphase and its completion-checklist update. Do not
store a self-referential commit SHA in the plan. A subphase is complete only
when its acceptance checks pass and the root agent has reviewed the diff. Stop
for review at each subphase boundary. The user authorized the root agent to
create ordinary local granular commits for scoped migration work after
reporting their scope, superseding routine commit approval. Pushes, history
rewrites, merges, rebases, and other remote or history-changing operations
still require exact current-conversation approval.

No implementation, dependency installation, generated artifact, signing,
release, or remote/history operation is authorized by subphase 0.1. Future
subphases must preserve classic parity fixtures and licensing notices. The
repository-root `/AGENTS.md` is intended to remain ignored as local
guidance, while `docs/PLAN.md` is intended to be tracked as product
documentation after approval. `.agents/skills` is intended to be ignored and
reports belong under intended-to-be-ignored `docs/reports/`; these local
materials are not product source.

Risks requiring explicit evidence include codec licensing and availability,
cross-platform font behavior, WASM heap and transfer costs, 50 MP memory
pressure, deterministic raster differences, cancellation races, native IPC
backpressure, accessibility across browsers, and inability to run one of the
native targets in CI.

## Decisions and engineering conventions

- Keep Python until the Rust/browser/native replacement passes the acceptance
  gates; do not silently remove the fallback.
- Keep contracts typed and minimal. Encapsulate caches and resources privately,
  use composition, RAII, checked arithmetic, and `Result`-based error handling
  in Rust; use strict TypeScript and opaque adapter sessions.
- Spend comments on contracts, invariants, and complex work: use concise
  Rustdoc/TSDoc, avoid ordinary single-line narration, and preserve generated
  comments and license notices.
- Use the references *Rust API Guidelines*, *A Philosophy of Software Design*,
  and *Refactoring* when making API and decomposition choices.
- Preview and export must share the source grid and deterministic settings.
- Benchmark stages are decode/render/export/memory at 1, 12, and 50 MP. The
  aspirational reference-hardware targets are cached preview under 100 ms at
  12 MP and under 250 ms at 50 MP; these are not measured yet and are not
  flaky CI gates.
- Browser checks run in the already-downloaded Playwright Chromium only, by
  user decision on 2026-11-07, to limit load on the development machine.
  Firefox and WebKit are reported as not run, never as passed; earlier
  evidence from them stays in the validation records. Native checks identify
  unavailable OS targets explicitly rather than implying they passed.
- Routing uses fresh-context Luna agents for reads/search/code/docs/tests and
  an independent Luna review. Findings return to the owning agent for fixes and
  retest; the root agent integrates, codes only when delegation is unavailable,
  or makes minor formatting fixes. RTK prefixes are required for shell work and
  Context7 is required for current library/API documentation. Git history and
  remote changes require exact current-conversation approval; report scope and
  wait for that approval before acting.

## Subphases

### 0.1 Record migration plan

Objective: establish this reviewed, tracked plan and the scope boundaries for
the migration.

Prerequisites: current Python prototype inspected; no implementation change.

Ordered steps:

1. Record the baseline behavior, defects, architecture, decisions, gates, and
   all later subphase boundaries in `docs/PLAN.md`.
2. State that only this document changes in this subphase and that future work
   remains unchecked until acceptance.
3. Have the root agent review scope, licensing, and commit-boundary wording.

Acceptance checks: `docs/PLAN.md` is readable Markdown, includes every
subphase 0.1–8.3, and `git status` shows no intended change outside this file.

Proposed commit title: `docs: record image-to-binary migration plan`

Completion checklist:

- [x] Plan drafted.
- [x] Root and independent review complete.
- [x] User review and exact commit approval received.

### 0.2 Add local engineering guidance, ignore rules, and skills

Objective: make local instructions and reusable UI/testing skills available
without treating them as product source.

Prerequisites: 0.1 accepted; exact local guidance reviewed by root.

Ordered steps:

1. Add a local `AGENTS.md` with concise project guidance for Rustdoc/TSDoc,
   contracts, invariants, checked arithmetic, RAII, `Result`, cache/resource
   ownership, strict TypeScript, composition, and testing. Include a
   token-aware comment check that permits contract/invariant/complex-work
   comments while rejecting ordinary narration and preserving generated or
   license comments.
2. Extend ignore rules for `/AGENTS.md`, local `.agents/skills`, generated
   `docs/reports/`, local tooling, and other non-product artifacts without
   ignoring tracked `docs/PLAN.md`, documentation, or fixtures.
3. Reuse existing `frontend-design` instructions. Select exact pinned
   revisions (recorded at this subphase) and install, only with explicit
   approval, the project-local sources at
   `plugins/ui-design/skills/responsive-design`,
   `skills/web-design-guidelines`, and `skills/.curated/playwright`.
4. Use skill-creator to define a custom desktop-UX skill from official Apple,
   Microsoft, GNOME, and Tauri guidance. Preserve source licenses and record
   revisions and destinations.

Acceptance checks: `AGENTS.md` guidance and the token-aware comment check are
discoverable, selected skill revisions and source paths are recorded, all local
skill material is ignored and reproducible, licenses are retained, and no
product dependency is introduced.

Proposed commit title: `chore: add local engineering and UX guidance`

Completion checklist:

- [x] Guidance and ignore rules implemented and reviewed.
- [x] Skill sources, revisions, destinations, and available notices recorded.
- [x] All four local skills pass the bundled validator; ignore checks pass.

Upstream source paths are not installation destinations:

| Repository and pinned revision | Source path | Ignored destination |
| --- | --- | --- |
| `wshobson/agents@156b7a5e7a8b93642628a339ee4039c925b34c7f` | `plugins/ui-design/skills/responsive-design` | `.agents/skills/responsive-design` |
| `vercel-labs/agent-skills@063bee94c3f4df8453406c830b0a7df0f2860278` | `skills/web-design-guidelines` | `.agents/skills/web-design-guidelines` |
| `openai/skills@49f948faa9258a0c61caceaf225e179651397431` | `skills/.curated/playwright` | `.agents/skills/playwright` |

The custom `.agents/skills/desktop-ux` is local; existing frontend-design is
reused from the session catalog. Reinstall remote skills with skill-installer
using the exact repository, source path, `--ref`, and `--dest .agents/skills`.
Responsive-design retains its MIT license; Playwright retains its packaged
Apache license and notice. No license was found in the pinned Vercel skill:
keep it local and resolve redistribution terms before copying its material
into product source. Validation uses ignored `.agents/tools/skill-validation`
with PyYAML 6.0.3, not an application dependency. Use the local Playwright
wrapper or a future pinned project CLI, artifacts under `docs/reports/`, and
never reassign `CODEX_HOME`.

### 1.1 Make Python dependencies and test font reproducible

Objective: make the current implementation runnable and testable on a clean
machine while it remains the reference fallback.

Prerequisites: 0.2; dependency and font licenses reviewed.

Ordered steps:

1. Pin Python dependencies and document supported interpreter/platform versions.
2. Bundle a licensed test font (proposed IBM Plex) with its notice and use it
   in tests instead of the Windows-only path.
3. Add deterministic setup and a minimal command for the existing app/checks.

Acceptance checks: a clean environment can run the baseline tests with the
bundled font; notices are present; no machine-specific path is required.

Proposed commit title: `build: pin Python baseline dependencies`

Completion checklist:

- [x] Dependencies and interpreter documented.
- [x] Bundled font and license checked.
- [x] Baseline command passes on local Python 3.14.7.

### 1.2 Add headless baseline fixtures and known-edge tests

Objective: freeze current classic behavior and expose known defects as named
tests before changing implementation.

Prerequisites: 1.1; fixture and font available.

Ordered steps:

1. Add tiny, threshold-boundary, transparency, orientation, long-name, and
   representative image fixtures.
2. Add headless tests for classic threshold 128, dark-to-1 mapping,
   green-on-black output, and deterministic dimensions.
3. Add explicit tests for zero-size inputs, reset/export lifecycle, entry/slider
   bounds, synchronous UI behavior, Windows font assumptions, and dropped paths.

Acceptance checks: tests are deterministic and document whether each known edge
is preserved as compatibility or marked for a later fix.

Proposed commit title: `test: freeze Python baseline behavior`

Completion checklist:

- [x] Fixtures retained and licensed.
- [x] Classic parity assertions pass.
- [x] Known-edge tests are named and triaged.

### 1.3 Add initial README and Python checks CI

Objective: document the fallback and run its checks automatically.

Prerequisites: 1.2; CI scope approved.

Ordered steps:

1. Add initial README setup, supported inputs, classic behavior, limitations,
   and local test commands.
2. Add pinned `requirements-dev.txt`, configure Ruff's critical lint and format
   checks, and keep any formatter-only legacy diff in this same 1.3 commit.
3. Add CI for formatting, static checks, and Python tests on supported versions.
4. Keep CI validation-only: no release, deploy, artifact publication, or
   signing.

Acceptance checks: local commands and configuration match the README, and the
declared matrix is structurally validated. Remote job results remain a
follow-up before 8.1.

Proposed commit title: `ci: validate Python fallback`

Completion checklist:

- [x] README reviewed against the local commands and implemented fallback.
- [x] Local formatting, static, dependency, syntax, and regression checks pass;
  remote job results remain unverified.
- [x] CI has no release side effects and its six-job matrix is locally validated.

### 2.1 Establish Rust workspace and contracts

Objective: create the core crate boundaries and typed contracts without
changing product behavior yet.

Prerequisites: 1.3; Rust toolchain and licensing choices approved.

Ordered steps:

1. Create a four-crate Rust workspace for core types, processing, export, and
   adapters.
2. Define private checked types for input metadata, dimensions and the 50 MP
   limit, settings and ramps, progress, cancellation, errors, and outputs;
   document invariants with Rustdoc.
3. Keep caches/resources private, use RAII-owned sessions, and define stable
   serialization only where an adapter requires it.
4. Add adapter-owned lifecycle state and cancellation backed by `AtomicBool`;
   a result's generation token alone is insufficient, so the adapter must also
   compare its current generation before accepting an old result.

Acceptance checks: workspace builds and contract tests cover invalid settings,
checked dimensions, errors, and cancellation states without adding image
rendering or codec implementation.

Proposed commit title: `feat: define Rust processing contracts`

Completion checklist:

- [x] Four crate boundaries and contracts reviewed.
- [x] Invalid input tests pass.
- [x] No image rendering or codec implementation added prematurely.

### 2.2 Implement decode, normalization, common codecs, and limits

Objective: implement one source decode and normalized 8-bit representation for
JPEG, PNG, WebP, BMP, TIFF, and first-frame GIF.

Prerequisites: 2.1; image-crate and license review.

Ordered steps:

1. Pin `image` 0.25.10 with only BMP, GIF, JPEG, PNG, TIFF, and WebP features;
   keep AVIF and Rayon out of this phase and use the Rust 1.88.0 toolchain/MSRV.
2. Read headers, validate the recognized format, then decode once into owned
   normalized RGBA8 pixels with EXIF orientation, the primary TIFF page, and
   the first GIF frame.
3. Enforce checked native, normalized-RGBA, and orientation working budgets,
   including the 50 MP guard before pixel allocation; allow supported browser
   adapters to select lower budgets.
4. Add malformed/recognized-format, grayscale/RGB/RGBA/alpha, orientation,
   and common-codec fixtures with actionable errors.

Acceptance checks: all six declared formats decode deterministically; both the
header reader and actual decoder apply allocation ceilings; oversized or
malformed inputs fail gracefully without unbounded allocation. Allocation
limits are best-effort because codec implementations may not support every
limit, and the decoded working budget excludes encoded input bytes.

Proposed commit title: `feat: add normalized image decoding`

Completion checklist:

- [x] Common codec fixtures pass.
- [x] Orientation and alpha policy tested.
- [x] Resource-limit errors tested.

### 2.3 Implement binary sampling, caches, and partial cells

Objective: implement separate classic floor-grid/nearest-neighbor sampling and
new average cell sampling with safe partial-cell handling.

Prerequisites: 2.2; parity fixtures.

Ordered steps:

1. Add an immutable `Arc`-owned source and a sampler module that builds checked
   classic and average source grids.
2. Preserve classic floor-grid geometry, sequential `f64` center truncation,
   nearest-neighbor sampling, threshold 128, and dark-to-1 behavior as a named
   compatibility path; clamp tiny inputs to one cell.
3. Implement average ceil-grid geometry with partial edges, actual edge areas,
   average grayscale and straight RGBA channels, and alpha-ignored luminance.
4. Add a private LRU cache with a 64 MiB default. Charge `Vec` capacity times
   `size_of::<SampleCell>()`; exclude metadata, `Arc` source pixels, external
   references, and temporary coordinate vectors from the cache ceiling.

Acceptance checks: tiny images never produce zero dimensions in the fixed path;
classic fixtures match Pillow-compatible center coordinates and 16-bit ITU
luminance coefficients; average sampling handles partial edges deliberately;
cache ownership and accounting are private and bounded. Threshold is excluded
from cache keys, sources remain immutable per sampler, and any future matte
policy change must invalidate affected cached grids. Rendering and font-aspect
correction remain deferred to 2.4.

Proposed commit title: `feat: add deterministic cell sampling`

Completion checklist:

- [x] Geometry and partial-cell tests pass.
- [x] Classic threshold parity passes.
- [x] Cache ownership and limits reviewed.

### 2.4 Add glyph caches, bounded preview, and deterministic fixtures

Objective: render glyphs efficiently and establish preview determinism.

Prerequisites: 2.3; bundled font and licensing.

Ordered steps:

1. Add fontdue glyph-mask generation with a bounded private cache.
2. Apply font-aspect correction only to new rendering presets and define
   deterministic raster settings; preserve classic font/cell geometry.
3. Bound preview longest edge at 1600 while reusing the export source grid.
4. Add golden fixtures for classic output only. Defer ASCII, block,
   monochrome, and source-colored fixtures to 6.1 and 6.2, when those modes
   are implemented.

Acceptance checks: repeated renders are byte-stable for the same inputs and
settings; preview and export agree on grid geometry; cache limits hold.

Validation record: 28 processing tests (22 unit and 6 independent acceptance
tests) passed, with strict processing clippy and formatting checks passing. The
renderer uses pinned `fontdue` 0.9.4 and the bundled IBM Plex Mono font under
the adjacent SIL Open Font License notice. Preview uses integer nearest-center
mapping from the bounded destination into the export coordinate space, so it
does not allocate a full export frame and keeps the longest edge at 1600 pixels.
Classic output uses opaque black/green RGBA cells with symmetric clipping; byte
parity with the legacy Python/Pillow rasterizer is intentionally not claimed.
Cache limits charge retained glyph-mask payload capacity and exclude font data,
metadata, and caller-owned buffers. WASM compilation is recorded separately by
the root acceptance run passed the WASM compilation check; no 2.5
streaming/export claim is made here.

Rendering contract notes: the bundled IBM Plex Mono font is rasterized through
`fontdue`, so glyph bytes intentionally differ from the Python/Pillow
reference raster even when classic sampling and threshold semantics match.
Classic cells use opaque green foreground and opaque black background with
symmetric clipping at the cell boundary; glyph spill from the legacy renderer
is not part of the new contract. The Average preset uses font-aspect-corrected
source and output cell geometry with checked, rounded edge dimensions. Preview
and export reuse the same deterministic source grid, with preview longest edge
bounded at 1600 pixels. Glyph-mask cache accounting is bounded to mask payload
capacity and excludes font storage, metadata, and external allocations.

Proposed commit title: `feat: cache deterministic glyph rendering`

Completion checklist:

- [x] Golden fixtures pass.
- [x] Preview geometry matches export.
- [x] Font and cache behavior reviewed.

### 2.5 Add streaming PNG export and core benchmarks

Objective: export PNG without unnecessary full-frame duplication and measure
the agreed processing stages.

Prerequisites: 2.4; benchmark fixtures.

Ordered steps:

1. Implement row streaming to a native file sink and a browser chunk contract;
   browser export retains the encoded Blob result as documented above.
2. Add decode/render/export/memory benchmarks at 1, 12, and 50 MP.
3. Record hardware and settings with results; keep aspirational preview targets
   informational rather than CI gates.

Acceptance checks: streamed PNGs decode correctly and benchmark reports are
   reproducible enough to compare; no flaky timing threshold gates CI.

Proposed commit title: `feat: stream PNG output and benchmark core`

Completion checklist:

- [x] Native and chunked export tests pass.
- [x] Three benchmark sizes recorded.
- [x] Memory and target caveats documented.

### 3.1 Scaffold frontend and processing adapters

Objective: establish the React/TypeScript/Vite UI and an adapter interface that
can use a worker or native backend.

Prerequisites: 2.5; frontend guidance from 0.2.

Ordered steps:

1. Add typed frontend state for source, settings, job status, progress, errors,
   metadata, and disposal.
2. Implement an adapter contract with opaque sessions for import, preview,
   export, cancel, and dispose; keep platform-specific details behind it.
3. Add the initial graphite/cobalt shell and light/dark theme tokens.

Acceptance checks: frontend builds, typechecks, and can display a fixture through
an adapter stub without embedding processing logic in components.

Proposed commit title: `feat: scaffold typed frontend adapters`

Completion checklist:

- [x] UI state and adapter contracts reviewed.
- [x] Build and typecheck pass.
- [x] No large binary payload in JSON state.

### 3.2 Add WASM worker scheduling, cancellation, and generations

Objective: run Rust WASM processing off the browser UI thread with predictable
latest-job behavior. This parent phase is complete: its bridge and browser
acceptance checks have passed.

Prerequisites: 3.1; 2.x core contracts.

#### 3.2.1 Add the latest-job scheduler foundation

Ordered steps:

1. Implement the private one-active, newest-pending scheduler.
2. Add cooperative cancellation checkpoints and stale-result release.
3. Cover replacement, executor failure, disposal, and generation races in
   scheduler tests.

Implement a generic TypeScript scheduler that holds one active execution and
one newest pending job, rejects superseded callers promptly, and releases stale
results after execution unwinds. Cooperative checkpoints are an adapter seam;
they do not interrupt a synchronous WASM call.

Acceptance checks: scheduler unit tests cover replacement, cancellation,
disposal, executor failures, stale-result cleanup, and generation identity.
This foundation alone does not provide prompt cancellation of synchronous WASM
work or UI responsiveness.

Proposed commit title: `feat: add latest-job processing scheduler`

Completion checklist:

- [x] Eight scheduler unit tests pass.
- [x] Strict TypeScript typecheck, formatting, and frontend build pass.
- [x] Cancellation, stale-result, and generation cases are covered.

#### 3.2.2 Add the incremental Rust/WASM bridge

This parent phase is split into ordered, independently reviewable foundations:

##### 3.2.2.1 Add pixel-budgeted Rust sampling

1. Add an owned resumable sampler with checked geometry, progress, cancellation,
   and generation checks at each synchronous call boundary.
2. Count source pixel reads as work units, including pauses inside Average
   cells; preserve the classic `f64` coordinate recurrence without eager arrays.
3. Route the synchronous sampler through the same primitive and admit only
   completed grids to the existing bounded cache.

Acceptance checks: chunked sampling matches synchronous classic and Average
results; budget, partial-cell, cancellation, stale-generation, and cache identity
cases pass. This step establishes Rust resumability only; no WASM binding or
worker is provided.

Proposed commit title: `feat: add pixel-budgeted Rust sampling`

Completion checklist:

- [x] Bounded sampling and cancellation contract tests pass.
- [x] Existing cache identity and sampling parity tests pass.
- [x] No WASM worker or UI integration is claimed.

##### 3.2.2.2 Add incremental preview rendering

1. Refactor preview rendering into an owned resumable task with bounded output
   work and generation checks.
2. Keep preview and export on the same deterministic source grid and settings.
3. Verify cancellation drops partial raster state.

Acceptance checks: chunked preview matches synchronous output and cancellation
or stale generations cannot return partial frames.

Proposed commit title: `feat: make preview rendering incremental`

Completion checklist:

- [x] Chunked preview matches synchronous output and an inverse-mapping oracle.
- [x] Cancellation and stale generations discard partial frames.

##### 3.2.2.3 Add actual WASM bindings and build

1. Expose stable task construction, progress, advance, cancel, and finish APIs.
2. Build the processing crate for `wasm32-unknown-unknown` without adding UI
   or worker transport in this step.
3. Verify binding-level lifecycle and generation behavior.

Acceptance checks: WASM builds and binding tests pass; worker transport remains
in 3.2.3.

Proposed commit title: `feat: expose incremental processing to wasm`

Completion checklist:

- [x] Processing WASM builds and binding lifecycle tests pass.
- [x] Cancellation and generation behavior are verified through bindings.

The Rust tasks in 3.2.2.1 and 3.2.2.2 added no bindings, transferred buffers,
or worker transport; 3.2.2.3 added the real normalized-RGBA bindings. Worker
transport remains in 3.2.3.
Keep decoder-worker lifecycle out of these subphases; termination belongs to
the lazy codec workers introduced in 4.2.

Acceptance checks for the complete parent phase: WASM bridge tests prove
cancellation at chunk boundaries and preserve the shared deterministic source
grid and settings.

Proposed commit title: `feat: add incremental WASM processing bridge`

Completion checklist:

- [x] WASM builds and bridge tests pass.
- [x] Cancellation is covered at cooperative chunk boundaries.
- [x] Deterministic source grid and settings are preserved.

#### 3.2.3 Add worker transport and browser proof

Ordered steps:

1. Connect scheduler jobs to a processing worker.
2. Verify binary transfer, resource cleanup, and stale-result handling.
3. Verify UI responsiveness in the browser; codec-worker termination is
   checked in 4.2.

Acceptance checks: processing-worker disposal releases resources, stale
results never replace newer settings, and browser memory transfer behavior is
reviewed. Full phase 3.2 acceptance requires this subphase and 3.2.2.

Proposed commit title: `feat: connect processing scheduler to worker`

Completion checklist:

- [x] Processing worker lifecycle and stale-result tests pass.
- [x] Browser memory transfer behavior is reviewed.
- [x] UI responsiveness is verified in the browser.

### 3.3 Implement browser import and preview

Objective: provide browser import, metadata, original/artwork views, and basic
preview controls on the shared source grid. Expanded split/zoom/pan canvas work
belongs to 7.2.

Prerequisites: 3.2; 2.2–2.4.

Ordered steps:

1. Add file picker and drop handling for supported formats and long names.
2. Display metadata, loading/progress, errors, and cancellation state.
3. Implement the basic original/artwork preview without changing source
   processing settings; defer split, zoom, pan, and fit expansion to 7.2.

Acceptance checks: supported fixtures import once, previews match core golden
fixtures, controls remain usable during processing, and errors are fully
readable and actionable.

Proposed commit title: `feat: add browser import and preview`

Completion checklist:

- [x] Import and metadata tests pass.
- [x] Preview parity passes.
- [x] Loading, error, and cancellation states reviewed.

#### 3.3.1 Add the encoded-image WASM boundary

Objective: let the browser hand encoded bytes of the six existing common
codecs (BMP, GIF, JPEG, PNG, TIFF, WebP) to the existing Rust `decode` and get
an orientation-normalized RGBA8 session or a stable error. No new codecs; extra
HEIC/AVIF stays in 4.2.

Ordered steps:

1. Add a WASM entry point that takes a `Uint8Array` of encoded bytes and calls
   the existing `image_to_binary_processing::decode` with `DecodeLimits`; do
   not add a second decode path.
2. Before copying, validate at runtime that the input is a `Uint8Array` and
   that its encoded length is within the encoded-byte limit. Reject wrong-type
   or oversized input with a coded error without copying.
3. Copy into WASM memory with fallible allocation (`try_reserve_exact`). A
   failed reservation returns a coded reservation-failure error instead of
   trapping. This covers only the encoded-input copy; output-frame allocation
   belongs to 3.3.2. Decoder limits are best effort: do not claim that every
   allocation made inside the decoder is non-trapping.
4. Detect the format only through the Rust decode, after the copy. Do not add
   a duplicated JS format sniff or a JS format-detection requirement.
5. Build the session from the `DecodedSource` without a second pixel copy, and
   expose source metadata: post-orientation width, height, and channels. These
   are sufficient. Exposing the detected format is optional, and whether the UI
   shows it is a deferred metadata decision.
6. Map `DecodeError` and `ContractError` variants to stable `ErrorCode`
   strings, covering wrong input type, unsupported format, encoded too large,
   malformed, resource limit, and reservation failure.
7. Add real Node/WASM tests against the built package: all six codecs,
   EXIF orientation dimension swap, malformed and truncated input, wrong-type
   input, the Node 50 MP and encoded-byte limits, and caller buffer
   ownership (the caller's array is not retained or mutated, and the session
   works after the caller's buffer is detached or overwritten). Custom
   decoded-byte budget and reservation-failure tests are native only.

Acceptance checks: each codec fixture decodes once to the expected normalized
dimensions and channels; wrong-type or oversized input fails before any copy;
input reservation failure returns a coded error; every failure
exposes a stable `code`; Rust fmt, clippy and tests, the WASM build, and the
Node tests pass.

Proposed commit title: `feat: add encoded-image wasm decode boundary`

Completion checklist:

- [x] Six-codec, orientation, malformed, and limit tests pass in Node/WASM.
- [x] Type and size checks run before copying; encoded-input allocation is
  fallible with a coded error; decoder limits are documented as best effort.
- [x] Error codes and ownership behavior are documented in Rustdoc and tested.

#### 3.3.2 Add bounded original preview core and WASM

Objective: produce a bounded original-image preview frame from a decoded
session, so the browser never receives full-source pixels for display.

Ordered steps:

1. Add a core function that computes preview dimensions with a longest side of
   at most 1600 pixels, using checked arithmetic, never upscaling, and keeping
   the aspect ratio of the orientation-normalized source.
2. Resample from the normalized source into a bounded RGBA8 buffer with a
   fallible allocation, sized before allocating.
3. Expose the preview through WASM as an owned frame that follows the
   existing `PreviewFrame` take-once contract. Do not transfer or expose the
   full source.
4. Check cancellation and generation staleness at the start and end of the call
   and at every internal boundary the work is split on. Document that a
   synchronous decode or resample already in progress is not preempted.
5. Add Rust unit tests and Node/WASM tests for the dimension math, aspect
   ratio, orientation, small sources that stay unscaled, bounded output size,
   frame ownership, and cancelled or stale rejection.

Acceptance checks: output longest side is at most 1600 for every fixture
including extreme aspect ratios; output size is bounded by the preview, not the
source; the frame is owned and consumed once; stale and cancelled calls return
stable errors; the docs make no claim of synchronous preemption.

Proposed commit title: `feat: add bounded original preview frame`

Completion checklist:

- [x] Dimension, aspect, and orientation tests pass in core and WASM.
- [x] No full-source transfer; output memory is bounded.
- [x] Cancellation and stale checks are tested at boundaries.

#### 3.3.3 Add encoded worker import and adapter ownership

Objective: move import through the processing worker and adapter so the
encoded bytes, decoded session, and artifacts have one clear owner and
lifecycle.

Ordered steps:

1. Transfer the encoded `ArrayBuffer` to the worker (transfer, not clone) and
   define the import message and result types without binary payloads in JSON
   state.
2. Keep the scheduler's physical slot held through the synchronous decode
   until it returns, so no other job runs on that worker in the meantime.
3. Keep source handles and metadata ownership in the adapter, and expose
   import, ready, error and disposed states through the controller. Release
   the previous source handle only when a replacement is accepted.
4. Retain the accepted original and artwork artifacts (frames, bitmaps, object
   URLs) when a replacement fails, is cancelled, or is superseded, and dispose
   only the new unaccepted late artifacts then. Dispose the accepted pair on a
   successful replace, reset, and dispose. Settings changes invalidate the
   artwork only and retain the original.
5. Make late results, cancellation, and replacement requests return typed
   errors that preserve the currently accepted source and its metadata.
6. Add worker and adapter tests for transfer, slot retention, lifecycle
   transitions, artifact cleanup, and the late, cancel and replacement error
   cases.

Acceptance checks: encoded bytes are detached on the sender after transfer; a
failed, cancelled, or superseded import never disturbs the accepted source;
new unaccepted artifacts are released in every terminal path, and the accepted
pair is released only on a successful replace, reset, or dispose; the frontend
format check, type check, and unit tests pass.

Proposed commit title: `feat: import encoded images in processing worker`

Completion checklist:

- [x] Transfer and slot-retention tests pass.
- [x] Source lifecycle and artifact cleanup tests pass.
- [x] Late, cancel, and replacement errors preserve the accepted source.

Parent 3.3.3 keeps the scope, acceptance checks, and checklist above. It is
delivered as the three ordered, independently reviewable commits below and
stays incomplete until all three are committed and its own checklist is
satisfied. The split adds no scope and changes no phase status.

##### 3.3.3.1 Add encoded worker transport

Objective: carry encoded-input and original-preview requests through the
processing worker protocol with one queue owner and no adapter or UI change.

Ordered steps:

1. Add the production `fromEncoded` wiring from the worker protocol, client,
   engine, and host to the real WASM decode already implemented in 3.3.1; this
   wiring is required here and adds no second decode path. Also add the
   original-preview request.
2. Transfer the encoded buffer rather than cloning it, and keep binary payloads
   out of JSON state.
3. Keep one queue owner, with the physical slot held through the synchronous
   decode until it returns.
4. Enforce encoded-size and request limits before transfer, and define
   cancellation and typed late-result handling.
5. Only the tests in this substep use mocks; production code uses the real
   WASM decode through the new wiring.

Acceptance checks: the production `fromEncoded` wiring reaches the 3.3.1
decode; encoded bytes are detached on the sender after transfer; limits fail
before transfer; no second job runs on the worker while decode holds the slot;
cancel and late results return typed errors. The frontend format check, type
check, and unit tests pass.

Proposed commit title: `feat: add encoded import worker transport`

Completion checklist:

- [x] Production `fromEncoded` wiring and mock-based protocol tests pass.
- [x] Queue ownership, limits, slot retention, and transfer tests pass.
- [x] Cancel and late-result error tests pass.

##### 3.3.3.2 Add file adapter and paired preview ownership

Objective: give the adapter and controller one lifecycle for the accepted
source and its original and artwork previews.

Ordered steps:

1. Read `File` bytes lazily, only for an accepted import, and record source
   metadata from the file and the decode result.
2. Keep source ownership in the adapter and transient import states in the
   controller, without duplicating UI state inside the adapter. Release the
   previous source handle only when a replacement is accepted.
3. Retain the accepted original and artwork pair when a replacement fails, is
   cancelled, or is superseded, and dispose only the new unaccepted late
   artifacts then. Dispose the accepted pair on a successful replace, reset, and
   dispose. Settings changes invalidate the artwork only and retain the
   original.
4. Reject stale results and replacement requests with typed errors that
   preserve the accepted source and its metadata.
5. Cover each path with adapter and controller tests.

Acceptance checks: file reads are lazy; a failed, cancelled, or superseded
import never disturbs the accepted source or its original and artwork pair;
new unaccepted artifacts are disposed in those paths; the accepted pair is
disposed only on a successful replace, reset, or dispose; a settings change
invalidates the artwork and keeps the original. The frontend format check, type
check, and unit tests pass.

Proposed commit title: `feat: own file import and paired previews in adapter`

Completion checklist:

- [x] Lazy read and metadata tests pass.
- [x] Accepted-pair retention and disposal tests pass.
- [x] Artwork-only invalidation and stale/replacement tests pass.

##### 3.3.3.3 Record browser import acceptance

Objective: prove the full import path in a real browser through the real
worker, and record the result.

Ordered steps:

1. Reuse the existing native-generated encoded fixtures. Add no new binary
   fixtures and no Playwright test specs.
2. Run project CLI browser probes against the real worker, covering import,
   the bounded original preview, preserving the old source on failed,
   cancelled, and superseded replacement, and cancellation.
3. Keep probe output and reports under ignored `docs/reports/`, and state any
   browser that was unavailable instead of claiming it passed.
4. Commit a tracked README and `docs/PLAN.md` acceptance summary. Mark this
   subphase and parent 3.3.3 complete only after their acceptance checks pass;
   parent 3.3 remains incomplete until 3.3.4 also passes.

Acceptance checks: the existing fixtures import once through the real worker
with the expected dimensions; the original preview stays within its bound;
failed, cancelled, and superseded imports keep the accepted source; reports
stay ignored and only the README and plan summary are tracked.

Proposed commit title: `docs: record browser import acceptance`

Completion checklist:

- [x] Probes pass on existing fixtures with no new binary fixtures or specs.
- [x] Old-source preservation and cancellation pass through the real worker.
- [x] README and plan acceptance summary are tracked; reports stay ignored.

Acceptance recorded on one macOS host through real Chromium and WebKit module
workers. Nine existing encoded fixtures matched native original/artwork pixels;
failed, cancelled and superseded replacement retained the accepted source.
Post-transfer cancellation, bounded originals and URL cleanup passed. The
integrated source passed 184 frontend tests, format, typecheck and production
build. Detailed receipts remain ignored under docs/reports. Phase 3.3.3 is
complete; parent 3.3 still requires the import UI in 3.3.4. Firefox and native
Windows/Linux verification are not claimed.

#### 3.3.4 Add import UI and original/artwork selection

Objective: give users an accessible way to import an image and switch between
the original and artwork views, validated in a real browser.

Ordered steps:

1. Use the local responsive and desktop UX skills for all product UI changes
   before editing components.
2. Add an accessible file picker and drop target (keyboard operable, labeled,
   with a visible focus state and announced status) for the supported formats.
3. Show metadata (name, normalized dimensions, encoded size) with long file names kept
   readable through wrapping or middle truncation with the full name
   available. Showing the detected format is a deferred metadata decision, not
   a requirement here. Normalized RGBA channel count stays an API detail because
   it does not describe the original file's channels.
4. Show loading, progress, readable and actionable errors, and a cancel
   control that stays usable during processing.
5. Add the original/artwork selection on the shared source grid without
   changing processing settings. Split, zoom, and pan stay in 7.2.
6. Validate in a real browser at desktop and narrow widths: supported
   fixtures import once, previews match the core golden fixtures, and
   keyboard and accessibility semantics (roles, names, labels, focus order,
   live-region announcements) are verified. Store screenshots and traces under
   `docs/reports/`.
7. Browser automation does not count as screen-reader testing. If no manual
   assistive-technology (screen reader) run is available, report that
   explicitly instead of claiming screen-reader tests passed.

Acceptance checks: picker and drop both import each supported fixture exactly
once; long names, errors, and cancellation are fully readable; controls remain
usable while processing; keyboard operation and accessibility semantics are
verified in the browser acceptance run; manual assistive-technology testing is
either reported as done with its details or explicitly reported as
unavailable; any unavailable browser environment is reported explicitly.

Proposed commit title: `feat: add browser import and preview selection`

Completion checklist:

- [x] Picker, drop, and metadata tests pass.
- [x] Original/artwork selection and preview parity pass.
- [x] Real-browser desktop and narrow-width keyboard and accessibility
  semantics reviewed; manual assistive-technology status reported.

Validation: 184 frontend tests, formatting, type checking and production build
pass. Local Chromium and WebKit cover picker/drop for nine native fixtures,
same-file selection, original/artwork pixel parity, failed replacement and
cancellation retention, keyboard controls and accessible names. Geometry checks
cover 320–2560 CSS pixels, short windows, light/dark themes and 200% text;
production checks cover a 320x200 viewport at device scale 4. This is reflow
and device-scale evidence, not native browser zoom or physical display testing.
Manual screen-reader testing, Firefox, Windows and Linux are unavailable.
Evidence is stored locally under ignored `docs/reports/playwright/`.

Parent 3.3 stays incomplete until 3.3.1 to 3.3.4 are all committed and its
own checklist above is satisfied. PNG and text export belong to 3.4 and 6.3,
and extra HEIC/AVIF formats belong to 4.2.

### 3.4 Implement browser export and lifecycle

Objective: export PNG in the browser while correctly resetting and disposing
jobs, workers, and object URLs. UTF-8 text export belongs to 6.3.

Prerequisites: 3.3; 2.5 streaming contract.

Ordered steps:

1. Stream encoded chunks into a Blob/download path and retain the encoded Blob
   required by the browser download path.
2. Implement reset, replacement, disposal, and object URL cleanup.

Acceptance checks: exported PNG matches fixtures; reset removes exportable state;
repeated imports do not leak workers or object URLs. Text export is accepted in
6.3.

Proposed commit title: `feat: add browser exports and cleanup`

Completion checklist:

- [x] PNG export passes parity checks.
- [x] Reset/replacement lifecycle tests pass.
- [x] Resource cleanup reviewed.

#### 3.4.1 Render artwork in bounded rows

Objective: reuse the deterministic sampled grid and glyph composition for
export without allocating a full output RGBA frame.

Ordered steps:

1. Reuse the existing shared pixel composition and `render_rows` contract.
2. Enforce a separate row-batch byte ceiling and reject zero-row requests.
3. Compare concatenated rows byte-for-byte with existing rendering fixtures;
   cover partial cells, alpha, invalid dimensions and budget boundaries.

Acceptance checks: existing preview pixels are unchanged; row output matches
the full renderer; allocations are bounded by row width and requested positive
batch size. The later export task owns cancellation between row calls.

Proposed commit title: `feat: add bounded artwork row rendering`

Completion checklist:

- [x] Row parity and failure-contract tests pass.
- [x] Formatting, strict Clippy and focused processing tests pass.
- [x] Independent review confirms shared composition and bounded storage.

Validation: 61 processing tests pass, including 13 renderer acceptance tests that
compare single-row and multi-row batches with the full frame for classic and
Average sampling, and that cover partial cells, transparent sources, ranges past
the grid, `u32` overflow, and exact budget boundaries. The workspace formatting,
strict Clippy, and WASM target checks pass.

#### 3.4.2 Encode PNG incrementally

Objective: retain a PNG encoder across bounded advances and drain owned encoded
chunks without retaining a full rendered frame.

Ordered steps:

1. Combine row rendering with the pinned encoder's owned stream writer.
2. Encapsulate sink ownership and draining; enforce encoded-byte and row budgets.
3. Test decoded pixel parity, finalization, cancellation and sink failures.

Acceptance checks: each advance obeys its documented row budget; terminal jobs
release resources; completed chunks move out once; encoded storage limits are
explicit. Browser Blob accumulation retains encoded bytes and is not constant
memory. Encoding within one row remains synchronous.

Proposed commit title: `feat: add incremental PNG export task`

Completion checklist:

- [x] Pixel parity, chunk ownership and failure tests pass.
- [x] Formatting, strict Clippy and focused export tests pass.
- [x] Independent ownership and resource review passes.

Validation: 25 export and 13 adapter tests pass; workspace formatting, strict
Clippy, and the WASM target check pass. `PngStreamEncoder` lives in `export` and
`ArtworkPngTask` in `adapters`, so WASM and native share one task while `export`
stays independent of `processing`. Output is byte-identical to
`PngExportPlan::encode` through one shared encoder configuration. Because the
owned encoder keeps its sink, it flushes on finish and reports failure itself; a
pending-storage breach is a typed resource-limit error. Pending storage is not
tied to the row budget, so `PngTaskLimits` documents that it must hold one
advance's output. No WASM or browser runtime is claimed by this subphase.

#### 3.4.3 Expose export through WASM

Objective: expose generation-bound incremental export and owned binary chunks.

Ordered steps:

1. Add typed export start, advance, drain and finish contracts with stable errors.
2. Share generation/cancellation ownership with original and artwork previews.
3. Verify actual Node/WASM parity, declarations, stale jobs and copied chunks.

Acceptance checks: no binary payload enters JSON state; disposed or superseded
jobs cannot publish; drained arrays remain valid after subsequent WASM calls.

Proposed commit title: `feat: expose incremental PNG export to wasm`

Completion checklist:

- [x] Native and actual WASM parity/lifecycle tests pass.
- [x] Target build, declarations and static checks pass.
- [x] Independent boundary review passes.

Validation: 133 Rust tests and 75 Node tests against the built WASM package pass,
with formatting, strict Clippy, the WASM target check, and the frontend
typecheck. Export output is byte-identical to the native `ArtworkPngTask` for
classic and Average sampling across budgets. Export shares the session's single
job slot, generation rule, and sampler cache key with previews. `advance` keeps
one pixel budget in every stage; encoding converts it to whole grid rows with a
one-row minimum, so an advance can exceed a tiny budget by at most one grid row.
Browser limits are 256 MiB encoded output with pending storage equal to that
ceiling, because the Blob retains those bytes anyway. Chunks are copied into
JS-owned arrays. No worker or browser runtime is claimed by this subphase.

#### 3.4.4 Transfer export chunks through the worker

Objective: schedule export in the existing single physical processing slot.

Ordered steps:

1. Add typed chunk messages and transfer owned buffers with bounded advances.
2. Accumulate encoded chunks into an adapter-owned Blob with explicit limits.
3. Test replacement, cancel, reset, disposal and stale chunk cleanup.

Acceptance checks: newest-job semantics remain intact; no stale Blob publishes;
preview and export use the same accepted source and settings snapshot.

Proposed commit title: `feat: add worker PNG export transport`

Completion checklist:

- [x] Scheduler, transfer and Blob lifecycle tests pass.
- [x] Frontend formatting, types and production build pass.
- [x] Independent transport/ownership review passes.

Validation: 236 frontend tests, formatting, type checking, and the production
build pass. Export runs through the existing scheduler in the single physical
slot and transfers sequence-numbered chunks; the client fails closed on gaps or
total mismatches, and a superseded job's chunks never reach a Blob. Each
controller transition now commits one complete snapshot, runs every listener,
then disposes superseded resources, so a reentrant listener cannot pair old
settings with a new generation and a throwing listener cannot leak an artifact.
The worker posts without acknowledgement, so up to the 256 MiB ceiling can queue,
and `new Blob(parts)` can transiently hold about twice the encoded size: this is
bounded but not constant memory. No real-browser run is claimed by this subphase.

#### 3.4.5 Add PNG download and browser acceptance

Objective: expose accessible PNG download with explicit object-URL ownership.

Ordered steps:

1. Add export progress, cancellation, actionable failure and download controls.
2. Release download URLs on replacement, reset and disposal.
3. Validate actual downloaded PNG pixels, keyboard operation, responsive layout
   and repeated import/export cleanup in available browser engines.

Acceptance checks: downloaded pixels match native fixtures; reset removes
exportable state; resource counters stabilize across repeated operations.
Report unavailable browser, native and assistive-technology environments.

Proposed commit title: `feat: add browser PNG download`

Completion checklist:

- [x] Real-browser download parity and lifecycle acceptance pass.
- [x] Responsive and keyboard review passes.
- [x] README status and parent 3.4 checklist reflect verified evidence.

Validation: 294 frontend tests, formatting, type checking, and the production
build pass. Chromium 154, WebKit 26.6 and Firefox 156 ran the production bundle
on macOS arm64: 9/9 encoded fixtures and 5/5 reference cases downloaded
byte-equal to native; cancel, source replacement and setting changes during a
held export left no download and announced the cancellation; 20/20 cycles ended
with 0 live object URLs and one stable worker. Layout passed at 320, 768, 1200
and 1920 px with no overflow and 44 px minimum targets. The export panel is the
only announcer of the export lifecycle. Tab reachability is verified in Chromium
and Firefox but not in WebKit, where Tab reached no control on this host. Manual
screen-reader testing, native Windows and native Linux were unavailable.

### 4.1 Add native HEIC/AVIF dependencies and build scripts

Objective: make native HEIC/AVIF decoding explicit, pinned, and reproducible.

Prerequisites: 2.2; codec licensing and platform support decision.

Ordered steps:

1. Integrate pinned decode-only libheif with pinned libde265 and libaom codec
   dependencies; do not treat libheif as a codec provider.
2. Add platform build scripts and configuration that never assumes embedded
   libheif supplies codecs.
3. Record licenses, versions, and fallback errors.

Acceptance checks: native builds decode HEIC and AVIF fixtures on available
targets; unavailable codec/OS combinations fail with documented errors.

Proposed commit title: `build: add pinned native HEIF AVIF codecs`

Completion checklist:

- [x] Dependencies and licenses reviewed.
- [x] Native fixtures pass on available targets.
- [x] Missing-codec path tested.

Validation on macOS arm64 only: 60 unit and 13 acceptance processing tests pass
with the `heif` feature, and the default workspace passes with no libheif in its
dependency tree, including the WASM target. libheif 1.23.7, libde265 1.1.3, and
libaom 3.15.3 are pinned by SHA-256 and built decode-only; `otool -L` shows no
dynamic dependency on them. A recognized file with no available decoder returns
the typed `CodecUnavailable` error (`codec_unavailable` in WASM), so the browser
can route to a lazy decoder without a JavaScript format sniff. The `ftyp` brand
scan is capped and a size-0 box is rejected, so file payloads cannot select a
format. Linux, Windows, and x86_64 builds are written but unverified. libheif's
internal decoder memory is not charged to the working budget, which remains best
effort. Fixtures have no irot/imir transforms; that coverage belongs to 4.3.
HEVC patent licensing and LGPL-3.0 distribution terms are recorded in
`THIRD_PARTY_NOTICES.md` as unresolved and need review before any distribution.

### 4.2 Add lazy browser HEIC/AVIF decoder workers

Objective: add browser HEIC/AVIF decoding without loading codec memory into the
main thread permanently.

Prerequisites: 3.2; 4.1 codec configuration.

Ordered steps:

1. Build Emscripten decoder support with both codecs enabled and keep it lazy.
2. Load a temporary decoder worker lazily, transfer RGBA output, and terminate
   it after decode or cancellation.
3. Map codec failures and resource limits to typed UI errors.

Acceptance checks: HEIC/AVIF fixtures decode in supported browsers, cancellation
terminates the worker, and ordinary formats do not pay codec startup cost.

Proposed commit title: `feat: add lazy browser HEIF AVIF decoding`

Completion checklist:

- [x] Browser codec fixtures pass where supported.
- [x] Worker lifecycle and cancellation pass.
- [x] Startup/memory behavior measured.

Validation: 345 frontend tests, formatting, type checking and 11 decoder smoke
cases pass. Chromium 154, WebKit 26.6 and Firefox 156 imported all 13 HEIC and
AVIF fixtures through a temporary decoder worker with RGBA and classic PNG
exports byte-identical to native; the worker is terminated after success,
failure and cancellation, and PNG or JPEG imports start no decoder and fetch no
codec asset. libheif's Emscripten branch disabled HEVC deblocking and SAO; the
build now keeps both filters and fails if that upstream code changes. libheif's
ceiling is twice the caller limit, as natively. In Chromium a HEIC reaches
preview in a 98 ms median against 67 ms for a PNG; 49 MP HEIC and AVIF decode in
about 1 s with 452 and 676 MiB of decoder heap under the 1 GiB maximum, and an
over-50 MP file is rejected in about 0.15 s. There is no decode timeout; Cancel
always terminates the worker. The LGPL and HEVC distribution review remains a
human decision.

### 4.3 Complete format coverage, 8-bit paths, limits, and orientation

Objective: close format and normalization coverage across adapters.

Prerequisites: 4.1–4.2; all format fixtures.

Ordered steps:

1. Verify JPEG, PNG, WebP, BMP, TIFF, GIF first frame, HEIC, and AVIF paths.
2. Exercise grayscale, RGB, RGBA, alpha, orientation, selected TIFF page,
   GIF first frame, and 8-bit normalization.
3. Verify 50 MP and browser-lower-limit behavior for every adapter.

Acceptance checks: coverage matrix is green for available environments and
explicitly marks unavailable native/browser checks; no path allocates beyond
declared limits.

Proposed commit title: `test: complete image format coverage`

Completion checklist:

- [x] Format matrix reviewed.
- [x] Pixel-depth/orientation tests pass.
- [x] Limit behavior is consistent.

#### 4.3.1 Verify native format coverage

Objective: close format, normalization and limit coverage for the native decode
path and record it in `docs/format-coverage.md`.

Ordered steps:

1. Add reproducible fixtures for rotation, mirroring, alpha, 10-bit, grey and
   multi-image HEIC/AVIF.
2. Test each format and pixel-depth, orientation and limit property natively.
3. Mark every browser cell as not yet verified until 4.3.2.

Acceptance checks: every verified cell names a test that can fail on that
property; default and `heif` builds pass.

Proposed commit title: `test: cover native image formats`

Completion checklist:

- [x] Native format matrix reviewed.
- [x] Pixel-depth/orientation tests pass on both builds.
- [x] Limit claims match what the tests can detect.

Validation: 148 default-build and 170 `heif`-build processing tests pass on macOS
arm64 with none ignored. The matrix records 52 verified property cells, 9 not
applicable, 2 not yet verified (PNG and WebP multi-frame policy) and 1
unavailable. Independent mutation review confirmed primary-item, EXIF orientation
and stride handling are detected. Pixel limits are product-based, so a
width/height swap is observable only through the per-axis decoder limit and output
dimensions, and the matrix claims only those. PNG runs a full 50 MP decode; other
codecs prove the limit check only. libheif's internal allocation up to twice the
caller limit is untested. All 13 fixtures regenerate byte-identically on matching
tool versions.

#### 4.3.2 Verify browser format coverage

Objective: fill the browser column of `docs/format-coverage.md`.

Prerequisites: 4.2.

Ordered steps:

1. Run each format fixture through the browser import path in available engines.
2. Compare decoded pixels and exports with native references.
3. Verify the browser-lower-limit behavior for every adapter.

Acceptance checks: browser cells are verified or explicitly unavailable.

Proposed commit title: `test: complete image format coverage`

Completion checklist:

- [x] Browser matrix column reviewed.
- [x] Browser limit behavior is consistent with native.
- [x] Parent 4.3 checklist reflects verified evidence.

Validation: in Chromium 154 all 56 browser inputs export byte-identically to
native with equal decoded pixels, and limit rows reject above 50 MP with a typed,
readable error. The browser keeps the 50 MP limit by decision; a lower browser
limit is allowed, not required, so its cells are not applicable and the core's
lowered-limit support stays proven natively. Animated PNG decodes its first
frame exactly, natively and in the browser. Known risk, accepted: for animated
WebP with alpha-blended frames, the upstream image-webp compositor returns the
first frame one level low per opaque channel; a test pins that deviation so an
upstream fix is noticed, and the cell stays not verified. 151 default-build and
173 `heif`-build processing tests pass. HEIC 10-bit remains unavailable on this
host, and Firefox and WebKit were not run by decision.

### 5.1 Add the Tauri shell

Objective: package the shared UI and core behind a native Tauri window.

Prerequisites: 4.3; Tauri guidance and native target availability.

Ordered steps:

1. Add a resizable native window with default 1280×800 logical size and
   provisional, content-tested minimum 800×600 area.
2. Define narrow typed commands/events for binary processing and progress.
3. Preserve native decorations and avoid huge JSON/base64 IPC.

Acceptance checks: shell launches on available OS targets, resizing preserves
state, and command contracts are typed and bounded.

Proposed commit title: `feat: add Tauri desktop shell`

Completion checklist:

- [x] Window geometry checks pass.
- [x] IPC contract reviewed.
- [x] Available OS smoke checks pass.

Validation: formatting, clippy with warnings denied, and 30 tests pass, and the
default binary contains no smoke strings or smoke permissions. The macOS arm64
smoke run passed 24/24 checks on the final bundle and CSP: 1280×800 default, a
640×480 request clamped to 800×600, a larger resize honoured, no page reload or
CSP violation across resizes, byte-exact binary IPC, and the worker off the UI
thread. Results carry their generation and consumers must compare it: delivery
suppression is best effort, because a result crosses IPC after delivery. Shutdown
waits at most 2 s and then detaches, so a job that never checkpoints cannot hang
exit. Tauri buffers a raw IPC body before any size check, so 5.2 must not send
file bytes through invoke. Processing and progress commands land with their
callers in 5.2. Windows and Linux were not run. This subphase landed before 4.3
closed because the shell depends only on the 4.1 codec configuration.

### 5.2 Add native processing, files, dialogs, and drop

Objective: connect native file access and processing off the UI thread.

Prerequisites: 5.1; 2.x core and 2.5 export.

Ordered steps:

1. Add file dialogs, drag/drop, native file sinks, and safe path handling.
2. Run processing off the UI thread and stream progress/results over binary IPC.
3. Match browser metadata, errors, cancellation, and reset behavior.

Acceptance checks: native import/export fixtures match browser/core fixtures;
large jobs do not block the UI; dropped paths and multiple files are handled
without naïve parsing.

Proposed commit title: `feat: connect native processing and file flows`

Completion checklist:

- [x] Native file/dialog checks pass.
- [x] Off-thread and cancellation behavior passes.
- [x] Browser/native parity reviewed.

#### 5.2.1 Add the native file boundary

Objective: give the desktop app safe file access without exposing paths to the page.

Ordered steps:

1. Run open and save dialogs from Rust and hand the page opaque ids only.
2. Accept drops through typed window events with the browser's one-file rule.
3. Read through bounded, non-blocking opens and write through an atomic sink.

Acceptance checks: no path reaches the page; reads are bounded and never block on
special files; a cancelled or failed save leaves no partial file.

Proposed commit title: `feat: add native file boundary`

Completion checklist:

- [x] Path, read and sink contracts tested.
- [x] Independent trust-boundary review passes.
- [x] Smoke and capability checks pass.

Validation: 94 desktop tests, formatting, and clippy with warnings denied pass,
and the macOS arm64 smoke run passed 24/24 checks. The page holds only opaque
ids for one source and one sink, so a replaced handle fails typed instead of
leaking; dialogs use rfd from Rust, run one at a time, and grant the page no
dialog permission. Reads reject non-regular files, open without blocking, and
stop at the 256 MiB limit even if a file grows. Saves write a 0600 temporary
file beside the target, keep its permissions, write through symlinks, and
remove the temporary on failure. Real OS dialogs, drag and drop, Windows and
Linux were not exercised.

#### 5.2.2 Share the processing session with native

Objective: run the WASM session's job state machine natively from one shared
implementation.

Ordered steps:

1. Move the transport-neutral session from `crates/wasm` into `crates/adapters`.
2. Drive it on the desktop worker with bounded progress over binary IPC.
3. Stream native exports into the atomic sink.

Acceptance checks: WASM behavior and fixtures are unchanged, and native previews
and exports match the same references.

Proposed commit title: `refactor: share the processing session natively`

Completion checklist:

- [x] WASM parity unchanged after extraction.
- [x] Native preview and export match references.
- [x] Off-thread and cancellation behavior passes.

Validation: the job state machine and the argument rules moved into
`crates/adapters` and drive both the WASM binding and the desktop worker. The
JS API files and `reference.json` stay byte-identical, with 75 WASM node tests,
345 frontend tests and the root workspace passing; the desktop app passes 117
tests in its default, smoke and native-codecs builds and the 24-check smoke
run. Native previews and PNG exports match direct rendering for the reference
cases, including the classic golden. The session lock is never held across an
IPC send, launches are refused unless their generation is newer, and a
committed export is always reported while a cancelled one leaves no file.
Malformed IPC arguments return the same {code, message} as the browser, and
the frontend's encoded, pixel and budget limits are checked against Rust in CI.
The webview path of the new commands is exercised in 5.2.3.

#### 5.2.3 Connect the studio to the native adapter

Objective: let the shared UI import, preview and export through the desktop app.

Ordered steps:

1. Add a native adapter behind the existing adapter contract.
2. Wire dialogs, drops and saves into the studio's lifecycle.
3. Compare native and browser metadata, errors, cancellation and reset.

Acceptance checks: the same UI flows work natively with browser parity.

Proposed commit title: `feat: connect native processing and file flows`

Completion checklist:

- [x] Native adapter tests pass.
- [x] Browser/native parity reviewed.
- [x] Parent 5.2 checklist reflects verified evidence.

Validation: the studio selects the native adapter only inside the Tauri
webview; the browser renders immediately behind a static placeholder and its
bundle contains no Tauri code, worker or WASM changes. The real desktop app
passes the 42-check smoke flow: its PNG exports are byte-identical to the
browser references (327 B and 2,756,400 B), its UI preview matches the
reproducible UI-default reference from `reference_fixtures`, the UI keeps
painting during a large export (largest frame gap 18 ms), cancel leaves no
file, reset clears, and dialog cancels stay silent. Imports carry a generation
and commit the source only while they are still the newest launch, so a
cancelled or superseded import never replaces the displayed image. The page
may only listen for events, and malformed native replies map to readable
errors. Frontend (395), root workspace (257) and desktop (123 in each build)
tests pass, with mutations of the import gate, generation epoch, hover reset
and adoption path caught. Real OS dialogs and drag-and-drop were exercised only
through queued answers and synthetic events; Windows and Linux are unavailable
here, and Firefox and WebKit are not run by decision.

### 5.3 Add desktop menus, shortcuts, focus, appearance, and DPI

Objective: complete platform-native interaction and accessibility behavior.

Prerequisites: 5.2; UI accessibility boundary tests planned in 7.4.

Ordered steps:

1. Add menus, keyboard shortcuts, focus restoration, and native appearance
   integration.
2. Handle light/dark system appearance and DPI scaling without state loss.
3. Ensure reduced motion, high contrast, 44 px targets, and visible focus.

Acceptance checks: keyboard-only workflows, appearance changes, and DPI checks
pass on available targets with no focus trap or clipped controls.

Proposed commit title: `feat: finish desktop interaction and accessibility`

Completion checklist:

- [x] Menu and shortcut matrix passes.
- [x] DPI/theme checks pass where available.
- [x] Focus and accessibility review complete.

Validation: the desktop app has native File, Edit, View, and Window menus with
Open (CmdOrCtrl+O), Export (CmdOrCtrl+E), Original and Artwork (CmdOrCtrl+1 and
2), Render (CmdOrCtrl+R), and Cancel; the page cancels with Escape outside text
fields, while the browser keeps its own shortcuts and relies on buttons. Menu
actions reach the page as events and run through the same gate functions as the
buttons, read from the controller at action time, so an invalid draft, a missing
source, or a running export blocks them even if the menu state is stale; the
page reports enabled state through one strictly validated command, and studio
items start disabled. Focus returns to the originating or a reachable fallback
control after dialogs, exports, cancels, resets, busy phases, and menu actions,
never to body, and the browser's 3.4.5 export focus is unchanged. A three-way
System, Light, and Dark choice follows live system changes; reduced motion,
increased contrast, and forced colours are honoured, with selected options
marked by border weight and not colour alone; every target is at least 44 px.
The preview stays crisp across device pixel ratios 1, 2, and 3 and fractional
ratios without changing export bytes. The real app passes 63 smoke checks with
menu actions dispatched by ID, Chromium passes 58 interaction checks and export
sections a–i, and root (292), desktop (142 per build), and frontend (629) tests
pass, with ten mutations of gates, shortcuts, focus, DPI, and listeners caught.
Real OS menu clicks, a real appearance switch, multi-display moves, Windows,
Linux, and screen readers were not exercised.

### 5.4 Build local packages for three operating systems

Objective: verify reproducible local packaging for Windows, macOS, and Linux
without publishing.

Prerequisites: 5.3; signing/release explicitly out of scope.

Ordered steps:

1. Add local build scripts for all three targets.
2. Produce local packages only where toolchains are available.
3. Record unavailable target checks and package reproducibility evidence.

Acceptance checks: available packages install/launch locally; no package is
uploaded, signed, or published; missing target toolchains are explicit.

Proposed commit title: `build: validate local desktop packages`

Completion checklist:

- [x] Three-target scripts reviewed.
- [x] Available packages smoke-tested.
- [x] No publishing or signing side effects.

Validation: `tools/package/` holds macOS, Windows, and Linux build scripts that
build the frontend and the default desktop feature set (no native codecs and no
browser HEIF decoder, pending the open LGPL/HEVC decision) and write only
ignored outputs. They refuse to run when any signing, notarization, or updater
key Tauri reads is set, or when the configuration or an overlay enables signing,
and they restore the desktop manifest on interruption. Each package ships the
project license, the font and codec notices, and an offline-generated notice
file for the 263 Rust crates and 4 npm packages it bundles, with every license
inside an allowlist. The macOS arm64 app and DMG were built, are ad-hoc signed
only and unsealed, mount read-only, launch from a scratch copy, show their window,
and quit cleanly; the packaged binary contains no smoke or HEIF decoder code.
Two clean builds produced a byte-identical binary and app contents, while the
DMG bytes differed. Windows and Linux report unavailable on this host and were
not built. 17 package guard tests, root (292), desktop (142 per build), and
frontend (629) tests pass. Nothing was signed, notarized, uploaded, published,
or installed system-wide.

### 6.1 Add ASCII, block, and custom ramps

Objective: expose the expanded rendering modes while preserving classic mode.

Prerequisites: 2.3–2.4; UI adapter paths available.

Ordered steps:

1. Add ASCII and block modes. ASCII uses printable characters; block uses the
   Unicode block elements in text and coverage-painted tiles in PNG, because a
   font block glyph does not exactly fill a tile and would leave seams.
2. Add custom printable ASCII ramps capped at 64 characters with validation.
3. Keep classic binary mode as an explicit compatibility preset.

Acceptance checks: mode/ramp fixtures are deterministic, invalid ramps produce
typed errors, and classic output remains unchanged.

Proposed commit title: `feat: add ASCII and block render modes`

Completion checklist:

- [x] Mode fixtures pass.
- [x] Ramp bounds and printable validation pass.
- [x] Classic parity remains green.

Parent 6.1 keeps the scope and checklist above and stays incomplete until
6.1.1 and 6.1.2 are both committed. The split adds no scope.

#### 6.1.1 Add core render modes and ramps

Objective: add the ramp and mode contracts and render them through the shared
composition, with no change to the WASM JavaScript API or the UI.

Ordered steps:

1. Add a fixed-capacity `Copy` `Ramp` and a `RenderMode` of `Binary`,
   `Ascii(Ramp)`, and `Block`, keeping `RenderOptions` `Copy`.
2. Add `cell_glyph` as the single cell-to-character mapping for PNG now and for
   text export in 6.3; `Ascii` and `Block` require Average sampling.
3. Render every mode through one composition so preview, full render, and row
   rendering agree byte for byte.

Acceptance checks: mode and ramp fixtures are deterministic, invalid ramps return
typed errors, classic output is unchanged, and sampled grids survive a mode or
ramp change.

Proposed commit title: `feat: add core ASCII and block render modes`

Completion checklist:

- [x] Mode fixtures and ramp bounds pass.
- [x] Classic golden value is unchanged.
- [x] Independent differential review passes.

Validation: 162 workspace tests pass, plus 69 unit and 23 acceptance processing
tests with the `heif` feature; formatting, strict Clippy, and the WASM target
check pass. The classic golden value is unchanged. Removing the dead scaled
render path was proven safe by building the previous commit and this tree
separately and comparing 168 hashed outputs, which were identical across
downscaled previews, chunked tasks at three budgets, full renders, and row
batches; a deliberate change to the 1600 limit altered 52 of them, showing the
probe was sensitive. Mode goldens are regression pins captured from this
implementation; their semantics are covered by unit tests.

#### 6.1.2 Expose render modes through WASM and the UI

Objective: let browser and native users choose a mode and a custom ramp.

Ordered steps:

1. Pass mode and ramp through the WASM session, worker protocol, and adapter
   settings, keeping preview and export on one settings snapshot.
2. Add accessible mode and ramp controls with typed, readable ramp errors.
3. Verify in a real browser that previews and downloaded PNGs match native
   output for each mode.

Acceptance checks: each mode previews and exports with native parity, invalid
ramps show readable errors without losing the accepted source, and classic
remains the default.

Proposed commit title: `feat: expose render modes in the studio`

Completion checklist:

- [x] WASM and worker mode parity pass.
- [x] Mode and ramp controls pass keyboard and error review.
- [x] Real-browser mode parity passes.

Validation: mode and ramp travel through the shared session arguments, the WASM
binding, the worker protocol, both adapters and desktop IPC, and a supplied ramp
is validated by one rule in every mode. A shared table of 14 ramp cases is read
by both the Rust and the frontend tests, and the frontend's limits and messages
are pinned to the Rust constants. Classic stays the default, and callers that
send no mode get unchanged output. For classic, the default ASCII ramp, a
custom ramp and block, the WASM session and Chromium produce previews and PNG
exports byte-identical to the references rendered directly by the processing
renderer and the native PNG task (exports of 4,270, 5,098, 5,282 and 8,029
bytes), and the real desktop app passes 45 smoke checks, including an ASCII
preview and export equal to the reference. The mode choice is a native radio
group with its own description per option, the ramp field reports a readable
error politely, and while a draft is invalid Render, Export and the Artwork tab
are blocked without losing the accepted source. Root (270), WASM (85), desktop
(124 per build) and frontend (441) tests pass, and mutations of the binding,
the IPC ramp, UI validation, the 64-character cap and the shared messages are
caught. A screen reader, Firefox and WebKit were not used.

### 6.2 Add tone/color controls, invalidation, and bounds

Objective: implement monochrome and source-colored rendering with predictable
control limits and cache invalidation.

Prerequisites: 6.1; 2.3–2.4 caches.

Ordered steps:

1. Add foreground/background colors and source-color mode.
2. Define tone, cell, font, and ramp bounds in shared contracts, including
   explicit contrast, threshold, and invert controls.
3. Invalidate only affected caches when settings or source changes.

Acceptance checks: color, contrast, threshold, and invert fixtures pass,
controls reject invalid values, and cache invalidation never reuses stale
source/settings data.

Proposed commit title: `feat: add color controls and cache invalidation`

Completion checklist:

- [x] Color and tone fixtures pass.
- [x] Bounds are shared across UI and core.
- [x] Invalidation review complete.

Parent 6.2 keeps the scope and checklist above and stays incomplete until
6.2.1 and 6.2.2 are both committed. The split adds no scope.

#### 6.2.1 Add core tone, color, and shared bounds

Objective: add the render style, tone rules, and shared bounds in Rust, with no
change to the WASM JavaScript API or the UI.

Ordered steps:

1. Define cell, font, threshold, contrast, and ramp bounds once in `core`.
2. Add a `Copy` render style with colors, color mode, contrast, and invert,
   defaulting to the existing output.
3. Apply tone at render time in the fixed order contrast, invert, then threshold
   or ramp, through one public function.
4. Composite source color over the background by cell alpha at render time, and
   average color by alpha so hidden pixels cannot bleed into a cell.

Acceptance checks: color, contrast, threshold, and invert fixtures pass, invalid
values are rejected, default output is unchanged, and caches are reused or
invalidated exactly as the settings require.

Proposed commit title: `feat: add core tone and color rendering`

Completion checklist:

- [x] Color and tone fixtures pass with default output unchanged.
- [x] Bounds are defined once in `core` and enforced at every render entry.
- [x] Independent invalidation review passes.

Validation: workspace tests pass (core 9, adapters 13, export 25, processing 89
unit and 30 acceptance, wasm 32), plus 95 unit and 30 acceptance processing tests
with the `heif` feature, with formatting, strict Clippy, and the WASM target
check. Building the previous commit and this tree separately gave 360 identical
hashed outputs at default style across sizes, cell sizes, modes, previews, chunked
tasks, full renders, and row batches; a deliberate change altered 180 of them. An
independent check of all 51,456 luma and contrast pairs found no violations.
The sampled grid depends only on source, cell geometry, and sampling, and glyph
masks only on character and font size, so tone, color, ramp, and threshold reuse
both, shown by cache counters. This settles the matte policy deferred in 2.3:
output stays opaque, and because matting happens at render time no cached grid is
invalidated. Average color is now alpha-weighted and remains straight, while
luminance stays alpha-ignored. Four 4000x3000 cell-5 Average full renders exceed
the full-frame output ceiling and were not hashed; export is row-based and is not
subject to that ceiling.

#### 6.2.2 Expose tone and color through WASM and the UI

Objective: let users control tone and color, with the UI reading the shared bounds.

Ordered steps:

1. Pass the render style through the WASM session, worker protocol, and adapter
   settings, keeping preview and export on one settings snapshot.
2. Expose the `core` bounds to the frontend so controls never restate them.
3. Add accessible color, contrast, threshold, and invert controls.

Acceptance checks: each control previews and exports with native parity, the UI
and core agree on every bound, and changing tone or color reuses sampled grids.

Proposed commit title: `feat: expose tone and color controls`

Completion checklist:

- [x] WASM and worker style parity pass.
- [x] UI and core bounds agree from one source.
- [x] Controls pass keyboard and error review.

Validation: foreground and background colour, colour mode, contrast, and invert
travel through the shared session arguments, the WASM binding, the worker
protocol, both adapters, and desktop IPC, validated once in Rust. A shared table
of 35 colour and contrast cases is read by both the Rust and frontend tests, and
messages, default colours, and the cell, font, threshold, contrast, and ramp
bounds in `bounds.ts` are pinned to the Rust production constants as exact
declarations. Callers that send no style get unchanged output. Five styles
(custom colours, source colour, raised contrast at threshold 96, invert, and
invert with a threshold change) preview and export byte-identically to
references rendered directly by the processing renderer and native PNG task, in
the WASM session, in Chromium (exports of 9,986, 7,043, 4,372, 4,293, and 4,520
bytes), and in the real desktop app's 48-check smoke run, whose fixtures are now
written by `reference_fixtures`. A style-only change keeps the sampled grid and
glyph masks for both Classic and Average sampling, shown by cache counters. The
controls are labelled, give readable polite errors, reach 44 px, follow an exact
keyboard order, and block Render, Export, and the Artwork tab while a draft is
invalid. Root (286), WASM (95), desktop (125 per build), and frontend (521)
tests pass, and mutations of the binding, IPC invert, a core bound, UI colour
validation, cache reuse, the shared messages, and the hex rule are caught. A
screen reader, Firefox, and WebKit were not used.

### 6.3 Add plaintext grid export and parity checks

Objective: export the rendered grid as UTF-8 text and verify readable parity.

Prerequisites: 6.1–6.2; 3.4 and 5.2 export paths.

Ordered steps:

1. Define UTF-8 whitespace, newline, trailing-line, and metadata conventions;
   preserve spaces and use the same source grid as PNG export.
2. Export classic, ASCII, block, and custom-ramp grids.
3. Compare browser/native text output byte-for-byte on shared fixtures.

Acceptance checks: UTF-8 output preserves required whitespace, uses the same
grid as PNG, and is byte-identical across adapters for the same settings.

Proposed commit title: `feat: export deterministic text grids`

Completion checklist:

- [x] Text contract documented.
- [x] Cross-adapter parity passes.
- [x] Long-output memory behavior reviewed.

Parent 6.3 keeps the scope and checklist above and stays incomplete until
6.3.1 and 6.3.2 are both committed. The split adds no scope.

Text format contract, shared by every adapter: UTF-8 with no BOM; LF only, never
CR; exactly one line per grid row and `grid.width()` characters per line; every
line, including the last, ends with one LF; no metadata of any kind; spaces are
preserved exactly, including trailing spaces and all-space lines. Each character
comes from `cell_glyph` over the same grid and options as PNG export, so tone,
invert, threshold, mode, and ramp apply identically and color has no effect.
Chunks are byte-level and may split a multi-byte character, so a consumer must
concatenate bytes before decoding.

#### 6.3.1 Add the text grid contract and task

Objective: produce deterministic text rows and a resumable task that shares the
PNG export lifecycle, with no change to the WASM JavaScript API or the UI.

Ordered steps:

1. Add `text_rows` with the same validation as `render_rows` and a batch ceiling.
2. Extract the export lifecycle into one private generic core, with PNG and text
   as its two row encoders, keeping `ArtworkPngTask` behavior unchanged.
3. Add `ArtworkTextTask` over that core with bounded chunked output.

Acceptance checks: exact bytes match the contract for every mode, multi-byte
output survives one-byte chunks, and PNG output and lifecycle are unchanged.

Proposed commit title: `feat: add deterministic text grid export task`

Completion checklist:

- [x] Exact-byte format tests pass for classic, ASCII, custom-ramp, and block.
- [x] Refactored PNG task is byte-identical to the previous commit.
- [x] Independent lifecycle and memory review passes.

Validation: workspace tests pass (adapters 28, core 9, export 25, processing 101
unit and 30 acceptance, wasm 32), plus 107 unit and 30 acceptance processing tests
through the `heif` wrapper, with formatting, strict Clippy, and the WASM target
check. Building the previous commit and this tree separately gave 128 identical
PNG outputs across grids, modes, row budgets, and chunk sizes; a one-byte change
altered all of them plus the aggregate. The review found the state machine, error
variants, and release points unchanged; the only `ArtworkPngTask` test edit is one
renamed test. Peak memory is one row batch plus undrained chunks; the row ceiling
applies to the worst-case reservation, so a batch stays within it even for block
output. Cancellation granularity is one row batch, at most 16 MiB of text. A
single row over that ceiling is unreachable under the 50 MP cap.

#### 6.3.2 Export text through the browser and native adapters

Objective: offer UTF-8 text download and prove adapters agree byte for byte.

Prerequisites: 3.4; 5.2 native export path; 6.3.1.

Ordered steps:

1. Pass a text output kind through the WASM session, worker protocol, and adapter.
2. Add accessible text export and download controls alongside PNG.
3. Compare browser and native text output byte for byte on shared fixtures.

Acceptance checks: downloaded text matches the contract, uses the same grid as
PNG, and is byte-identical across adapters for the same settings.

Proposed commit title: `feat: export deterministic text grids`

Completion checklist:

- [x] WASM and worker text parity pass.
- [x] Browser and native text are byte-identical on shared fixtures.
- [x] Real-browser text download passes.

Validation: a PNG or text output kind travels through the shared session, the
WASM binding, the worker protocol, both adapters, and desktop IPC, whose export
command is now `export_file`; one Rust rule validates it and one frontend guard
mirrors it. The export panel offers a format radio group with one Export button;
a finished export keeps its own format, name, and type when the choice changes,
and the invalid-draft gate, cancel, and announcements match PNG. Text bytes are
collected as byte parts and never decoded, saved natively through the atomic
sink, and downloaded as `text/plain;charset=utf-8`. Six references rendered by
`ArtworkTextTask` and cross-checked with `text_rows` (classic, default and
custom ASCII ramps, multi-byte block, inverted with colours, which matches the
uncoloured text, and raised contrast) match byte for byte in the WASM session at
1-byte and unbounded budgets, in Chromium downloads, and in the real desktop
app's 52-check smoke run; every path follows the text contract, and the session
equals the direct text rows at budgets 1, 7, and 4,096. Earlier reference fields
are unchanged. For a 50 MP source at cell 5, block text is about 6.0 MB: the
browser peaks near 22 MB (a 10 MB grid, the byte parts, and the final Blob
copy), while native streams chunks and peaks near 11 MB; both are estimates from
the code, far below the 256 MiB encoded ceiling, whose text error is tested.
Root (292), WASM (104), desktop (132 per build), and frontend (562) tests pass,
and mutations of the output kind, per-chunk decoding, the native filter and
task, the gate, the trailing LF, CRLF writes, and the download name are caught.

### 7.1 Implement visual primitives

Objective: build reusable accessible controls, canvas, inspector, metadata,
progress, error, and dialog primitives.

Prerequisites: 6.3; frontend design guidance.

Ordered steps:

1. Add primitives with keyboard semantics, focus states, labels, and full-text
   error/name presentation.
2. Implement loading, progress, cancel, metadata, reset, and disposal states.
3. Add light/dark graphite/cobalt visual tokens and reduced-motion variants.

Acceptance checks: primitive tests cover keyboard and screen-reader semantics;
no state is hidden behind color alone or clipped overflow.

Proposed commit title: `feat: add accessible studio UI primitives`

Completion checklist:

- [x] Primitive behavior reviewed.
- [x] Accessibility tests pass.
- [x] Theme and motion variants pass.

Validation: the studio is built from accessible primitives in `frontend/src/ui/`
(buttons, native radio choices with a check-mark cue, sliders whose value sits
outside the label, validated text and colour fields, checkbox, inspector
sections, full-text metadata, determinate and indeterminate progress with
cancel, status regions, and full-text errors with retry); no modal dialog is
needed because file dialogs and menus are native. `App.tsx` shrank from 748 to
550 lines with gates, focus, menus, and shortcuts unchanged. One palette defines
the light and dark graphite and cobalt values without `light-dark()` or `:has()`
for the Safari/WKWebView 14.1 and Chrome 100 floor; the system theme applies
before JavaScript runs, and every text pairing meets 4.5:1 with controls and
focus rings at 3:1, checked by token tests. Selected options carry a check
mark and heavier border, and under forced colours they use Highlight and
HighlightText, verified by pixel sampling against system-colour probes;
keyboard focus shows a 3 px ring over the whole option while mouse clicks do
not, and reduced motion removes transitions. 680 frontend tests, Chromium
export sections a–i, 58 desktop interaction checks, forced-colour and focus
checks, and the real app's 64 smoke checks pass, with mutations of the cue,
slider label, error announcement, progress value, theme selector, and focus
ring caught. The browser HEIF acceptance run was not repeated because its
reference data was removed; Firefox, WebKit, macOS 11–13, and screen readers
were not exercised.

### 7.2 Implement comparison canvas

Objective: provide original/artwork/split comparison with zoom, pan, and fit.

Prerequisites: 7.1; 3.3 preview contract.

Ordered steps:

1. Add responsive canvas rendering and split divider.
2. Implement pointer, keyboard, and wheel zoom/pan/fit behavior.
3. Keep source metadata and settings stable while changing viewport state.

Acceptance checks: canvas geometry assertions pass at representative sizes;
viewport operations never alter export output or lose focus.

Proposed commit title: `feat: add comparison canvas controls`

Completion checklist:

- [x] Geometry assertions pass.
- [x] Keyboard/pointer viewport tests pass.
- [x] Export-state separation reviewed.

Validation: the studio shows Original, Artwork, and Split views (native menu
`CmdOrCtrl+1/2/3`) in one comparison canvas with zoom, pan, fit, and 100%
controls, wheel and pinch zoom, pointer drag, arrow-key panning, and a
keyboard-operable split divider with value text. Viewport state is UI-only: it
survives settings changes of the same dimensions, refits on a new source or
size change, and never reaches the controller, so the exported PNG hash is
identical with and without viewport use. In a split, the artwork is mapped
linearly over the whole source, matching the sampler's cell centres at
`(k + 0.5) * W / gw`, and stretched by `(W / gw) / c`; 100% means the original
is 1:1. The artwork view shows its natural pixels unstretched. Chromium
checks at 320, 768, 1280, and 1920 px confirm centred fit and that the drawn
bitmaps reach their boxes within 1 CSS px. Synthetic edge sources show the
classic artwork boundary within half a cell of the original edge (+6.8 source
px against 13.2) and the average boundary within one cell (-1.5 against 23.9);
reverting to the old `floor(W / c) * c` extent fails the half-cell check.
771 frontend tests, 140 comparison checks, Chromium export sections a–i, 58
desktop interaction checks, the real app's 64 smoke checks, 292 workspace
tests, and 145 desktop tests per feature set pass. Firefox, WebKit, real
trackpad gestures, and screen readers were not exercised.

### 7.3 Implement adaptive layout

Objective: implement the documented breakpoint and resizing behavior.

Prerequisites: 7.1–7.2; container-query support decision.

Ordered steps:

1. Implement 320–767 single-column, 768–1199 collapsible-inspector, and
   1200+ canvas/320 px inspector layouts.
2. Add intrinsic sizing, container queries, independent inspector scrolling,
   normal vertical scrolling, and wrapped toolbar behavior.
3. Preserve source/settings/viewport state through resize and orientation change.

Acceptance checks: no horizontal overflow, overlap, clipping, trimming, or
unreachable control across the full viewport matrix and short landscape cases.

Proposed commit title: `feat: add responsive studio layout`

Completion checklist:

- [x] Breakpoint geometry tests pass.
- [x] Resize/orientation state preservation passes.
- [x] Overflow and scrolling review complete.

Validation: container queries are not used, because the Safari/WKWebView 14.1
and Chrome 100 floor lacks them; page bands use media queries and components
reflow through intrinsic sizing (`flex-wrap`, `repeat(auto-fit, minmax(min(100%,
…), 1fr))`, `min()`, `clamp()`), so the layout needs no progressive
enhancement. Below 768 px the studio is one column with the inspector below the
canvas and no collapse toggle; from 768 to 1199 px the canvas comes first and
the inspector collapses through a labelled toggle; from 1200 px the inspector
scrolls on its own beside the canvas and the page does not scroll at 1280×800.
The comparison canvas now fills its frame: at 1280×800 it grew from 461×224 to
838×394 CSS px. The theme choice and view tabs fit one row at 320 px. Chromium
checks at 14 widths from 320 to 2560 px plus 640×360, 800×600, 1024×600, and
844×390, in light and dark, find no horizontal overflow, overlap, ellipsis
trimming, unreachable control, or touch target under 44 px; long names and
full errors wrap. Source, settings, zoom, split, and inspector state survive
resizing through 1440, 1024, 390, 844×390, and 1280×800 without reloading the
image. 776 layout checks pass; removing the canvas fill fails 36 of them. 775
frontend tests, 140 comparison checks, Chromium export sections a–i, 58 desktop
interaction checks, forced-colour and focus checks, and the real app's 64 smoke
checks pass. Firefox, WebKit, 200% text, and 400% zoom were not exercised here;
the latter two belong to 7.4.

### 7.4 Add layout, accessibility, and boundary tests

Objective: make UI acceptance executable across viewport, browser, and content
boundaries.

Prerequisites: 7.3; Playwright guidance from 0.2.

Ordered steps:

1. Test 320, 360, 390, 640, 767, 768, 769, 1024, 1199, 1200, 1201, 1440,
   1920, and 2560 widths plus short landscape, DPI, and light/dark cases.
2. Test 200% text, 320 px reflow, 400% zoom, long names, full errors, ramp
   extrema, loading, exports, dialogs, scrolling, and focus.
3. Run geometry assertions and screenshots in Chromium; report Firefox and
   WebKit as not run by decision, and mark unavailable native checks explicitly.

Acceptance checks: all available browser projects pass with no overlap,
trimming, masked overflow, or unreachable keyboard control; screenshots are
reviewable and reports remain under ignored `docs/reports/`.

Proposed commit title: `test: cover responsive and accessible boundaries`

Completion checklist:

- [x] Full viewport matrix passes.
- [x] Accessibility/content boundary checks pass.
- [x] Cross-browser evidence reviewed.

Validation: a Chromium boundary run under ignored `docs/reports/boundaries/`
passes 2651 checks:
- the full width matrix, short landscape at device scales 1 to 3, light and
  dark, Original and Split, with a full forward and reverse Tab walk;
- 200% text through the browser font setting;
- 320 px reflow and 400% zoom, emulated as 320×200 at device scale 4;
- long names and full errors, ramp extremes with their exact messages,
  loading and cancel, PNG and text downloads, the file chooser, scrolling,
  focus retention, and forced colours.
The run found and fixed five defects:
- the root font size was fixed in pixels, so text ignored the browser
  setting;
- under forced colours the Source colour focus ring missed its option;
- at 200% text the hex field clipped its value and choice chips overlapped or
  split words; header chips now wrap as whole chips and inspector chips drop
  columns;
- at phone width a render started from the inspector left its progress and
  Cancel out of view.
Removing a fix fails its check: a fixed width at 320 px, the forced-colour
ring, and the header and inspector word-split rules. Ten committed DOM tests
cover long names, full errors, ramp extremes, progress labelling, and focus.
785 frontend tests, 776 layout and 140 comparison checks, Chromium export
sections a–i, 58 desktop interaction checks, forced-colour and focus checks,
and the real app's 64 smoke checks pass. Firefox and WebKit were not run, by
user decision. Also not exercised:
- native OS dialogs, screen readers, real OS text-size settings, and real
  browser zoom;
- macOS 11–13, Windows, and Linux;
- the Safari/WKWebView 14.1 and Chrome 100 floor engines.

### 8.1 Run final checks in CI only

Objective: consolidate reproducible validation without adding deployment or
release behavior.

Prerequisites: 7.4; all implementation subphases accepted.

Ordered steps:

1. Run strict Rust, frontend/TypeScript, WASM, unit, integration, browser,
   native, package, and benchmark smoke checks appropriate to each platform.
2. Verify generated reports are ignored, source fixtures/notices are tracked,
   workflow permissions are least-privilege, dependency caches are scoped, and
   concurrency cancels superseded runs/jobs.
3. Keep CI limited to checks; do not publish, deploy, sign, or release.

Acceptance checks: every required CI job in the supported OS/browser matrix is
green. An unavailable environment is reported as unavailable and cannot be
declared passed; the worktree contains no accidental artifacts.

Proposed commit title: `ci: consolidate migration validation`

Completion checklist:

- [x] Required CI checks pass.
- [x] Unavailable environments are explicit.
- [x] No release side effects exist.

Validation: five check-only workflows run on every push and pull request:
- Python baseline;
- Rust: workspace format, lint, tests and the WASM target; the WASM package
  and its Node tests; a 1 MP core benchmark smoke that is not a timing gate;
- Frontend: typecheck, tests, format and build;
- Desktop: format, lint and tests for the default and smoke features on
  Linux, macOS and Windows; package guard tests on Linux and macOS; the real
  app's smoke run under Xvfb on Linux;
- Repository hygiene: generated outputs are ignored, fixtures, notices and
  lockfiles are tracked, and the worktree stays clean after builds.
Every workflow reads contents only, pins actions to full commit SHAs, keeps
credentials out of checkouts, scopes caches by OS and lockfile, cancels
superseded runs, and has a timeout; none publishes, deploys, signs or
releases. The first remote run exposed the first Linux and Windows desktop
builds: the Linux dialog backend was missing, macOS-only imports failed the
Windows lint, a package guard test assumed a macOS host, and two file tests
assumed Unix behaviour. Their fixes made every required job green. Not
covered by CI:
- the Chromium browser harnesses, which stay local by decision;
- the `native-codecs` build;
- the desktop smoke run on macOS and Windows;
- packaging and signing, by design;
- Firefox, WebKit and screen readers.

### 8.2 Complete README and contributor documentation

Objective: document the accepted product for users and maintainers.

Prerequisites: 8.1; measured benchmark results and final UI screenshots.

Ordered steps:

1. Document setup, build, project layout, architecture, supported formats,
   PNG/text exports, controls, privacy, dependency licenses, limits,
   troubleshooting, and contribution workflow.
2. Add screenshots for responsive/native states and explain browser/native
   differences.
3. Report measured performance with hardware and distinguish it from
   aspirational targets; retain codec and font notices.

Acceptance checks: a new contributor can follow README commands, supported
formats and limits are accurate, privacy claims match implementation, and
screenshots show accessible, unclipped states.

Proposed commit title: `docs: document accepted application`

Completion checklist:

- [ ] README and screenshots reviewed.
- [ ] Format/privacy/limits documentation matches code.
- [ ] Notices and measured performance retained.

### 8.3 Retire Python only after acceptance

Objective: remove or archive the Python fallback only after the replacement is
formally accepted against all gates.

Prerequisites: 8.1–8.2; root-agent acceptance decision and preserved fixtures.

Ordered steps:

1. Compare the replacement against classic parity, format, resource, export,
   accessibility, browser, and available native checks.
2. Preserve fixtures, notices, and a migration note before removal or archival.
3. Remove Python-only runtime files only if the root agent explicitly accepts
   the replacement; otherwise keep Python as a documented fallback.

Acceptance checks: all required gates are green, known risks are resolved or
accepted in writing, and no user-visible classic behavior regresses.

Proposed commit title: `refactor: retire Python fallback after acceptance`

Completion checklist:

- [ ] Acceptance decision recorded.
- [ ] Fixtures and notices retained.
- [ ] Python removal/retention is explicit and validated.

## Subphase 0.1 validation record

- Inspected `main.py`, `image_processor.py`, and `.gitignore`.
- Confirmed no pre-existing `docs/` files were present.
- This subphase changes only `docs/PLAN.md`; because it is untracked until
  approval, the root agent must recheck `git status --short` and use
  `git diff --no-index --check /dev/null docs/PLAN.md` before review/commit.

## 0.2 local validation record

- Subphase 0.2 guidance and ignore-rule changes are ready for their scoped
  local commit; later foundation subphases remain unchecked in this plan.
- `git check-ignore AGENTS.md .agents/skills/desktop-ux/SKILL.md docs/reports/test.md`
  confirms local material is ignored. `docs/PLAN.md` remains tracked.
- Bundled skill-creator `quick_validate.py` passes for all four installed/local
  skills with the ignored validation environment.
- The 0.2 guidance was independently reviewed; the root corrected omissions
  and reviewed the final local guidance and skill-source record.

## 1.1 local validation record

- `pyproject.toml` and the requirements files pin the Python dependencies and
  declare CPython 3.11–3.14 support.
- The bundled IBM Plex Mono test font and its OFL notice are retained;
  `.gitattributes` exempts only that vendor notice from whitespace diagnostics.
- The portable font intentionally changes historical Times glyph raster bytes;
  classic geometry, sampling, and threshold semantics remain unchanged.
- Headless baseline checks pass locally on Python 3.14.7/macOS. GUI launch is
  unverified because the host Python lacks `_tkinter`; other Python and OS
  combinations remain unverified.

## 1.2 local validation record

- `.venv/bin/python -m unittest discover -s tests -v` passes 14 tests with
  three expected failures documenting the tiny-image, reset/export, and
  multiple-drop defects.
- Procedural fixtures cover alpha handling, EXIF orientation, threshold
  mapping, floor dimensions, nearest-neighbor sampling, and classic output
  colors; the GPL application notice and bundled-font license remain retained.
- Independent baseline review passed with no 1.2 findings. Its only
  observation concerned a lint omission in the separate 1.3 scope.
- GUI launch and cross-platform visual checks remain unavailable on the host
  because Python lacks `_tkinter`.

## 1.3 local validation record

- The README commands and the pinned `requirements-dev.txt` match the local
  fallback checks. `.venv/bin/python -m pip check`, `.venv/bin/python -m
  compileall -q main.py image_processor.py tests`, `.venv/bin/python -m ruff
  format --check main.py image_processor.py tests`, and `.venv/bin/python -m
  ruff check main.py image_processor.py tests` pass locally.
- `.venv/bin/python -m unittest discover -s tests -v` reports 14 tests with
  three expected defect failures.
- The workflow configuration has six read-only validation jobs, checks-only
  triggers, pinned official actions, and no release, deployment, signing, or
  artifact-publication side effects. Its matrix and YAML structure were
  validated locally.
- Remote CI has not run, so remote job results are explicitly unverified and
  remain a follow-up before 8.1; this plan does not claim green CI.

## 2.1 local validation record

- The four-crate workspace pins Rust 1.85.1. Local checks use the project
  `.local` toolchain environment with task-scoped `CARGO_HOME` and
  `RUSTUP_HOME` values.
- Five native unit tests and the Rust doctests pass. `cargo fmt --all --check`,
  `cargo clippy --workspace --all-targets --locked -- -D warnings`, and the
  `wasm32-unknown-unknown` workspace check also pass locally.
- Contracts cover checked dimensions and 50 MP limits, settings and ramps,
  progress, RAII session ownership, and `AtomicBool` cancellation. No image
  rendering or codec implementation is included.
- Independent review accepted the 2.1 implementation with no findings.
- `.github/workflows/rust.yml` is ready in this same 2.1 scope; its YAML and
  basic locked fmt, Clippy, test, and WASM checks were validated locally. It
  uses read-only permissions, pinned checkout, concurrency cancellation, and
  no deploy or artifact steps. Remote CI has not run, so no remote result is
  claimed.

## 2.2 local validation record

- The processing crate pins `image` 0.25.10 with exactly six codec features:
  BMP, GIF, JPEG, PNG, TIFF, and WebP. AVIF and Rayon are not enabled. The
  Rust MSRV and toolchain are 1.88.0.
- Decoding reads headers before a second reader performs the actual decode,
  owns one normalized RGBA8 pixel buffer, applies EXIF orientation, and keeps
  the primary TIFF page and first GIF frame. Native, RGBA, and orientation
  working estimates are checked before pixel allocation; defaults are 50 MP,
  256 MiB encoded input, and 1 GiB decoded working budget, with encoded input
  excluded from the latter.
- Local reproducible checks use the project `.local` Rust 1.88.0 toolchain
  paths with task-scoped `CARGO_HOME` and `RUSTUP_HOME`: 12 native tests,
  doctests, formatting, Clippy `-D warnings`, and the WASM workspace check all
  pass after the final `max_alloc` fix.
- Browser adapters may select lower resource budgets; browser UI support is
  not implemented in this subphase. Malformed recognized PNG handling and
  grayscale, RGB, RGBA, alpha, orientation, and common-codec fixtures pass.
- Independent review accepted the final implementation with all tests passing.
  The Rust workflow reads `rust-toolchain.toml` through `rustup show` and will
  follow future toolchain updates automatically; remote CI remains unrun.

## 2.3 local validation record

- The sampler owns an immutable `Arc` source, produces classic and average
  grids, and uses a private LRU cache with a 64 MiB default. Accounting uses
  actual `Vec` capacity times `size_of::<SampleCell>()`; metadata, source
  pixels, external `Arc` references, and temporary coordinates are excluded,
  without making a global RSS claim.
- Classic sampling clamps tiny inputs to one cell and uses sequential `f64`
  center truncation matching Pillow (the 28-by-28/cell-5 coordinates are
  2, 8, 13, 19, and 25 on both axes). Its 16-bit ITU luminance coefficients,
  alpha-ignored grayscale, threshold 128, and dark-to-1 mapping are tested.
- Average sampling uses ceil grids, actual partial-edge areas, average
  grayscale, and straight RGBA channels while ignoring alpha for luminance.
  Matte policy is intentionally deferred; any future matte change requires
  cache invalidation. Threshold is not a cache key because the source grids
  are immutable and reusable.
- Eighteen native tests, formatting, Clippy `-D warnings`, and the WASM build
  pass locally. Independent review accepted the implementation. Rendering and
  font-aspect correction remain deferred to 2.4.

## 2.4 local validation record

- The processing renderer uses pinned `fontdue` 0.9.4 and the bundled IBM Plex
  Mono font under its adjacent SIL Open Font License notice. Classic cells use
  opaque black/green RGBA8 output with symmetric clipping; byte parity with the
  legacy Python/Pillow glyph raster is intentionally not claimed.
- Average rendering derives checked, rounded source/output aspect geometry;
  preview and export reuse one source grid, and preview longest edge is capped
  at 1600 pixels. Glyph-mask cache accounting is bounded to retained mask
  payload capacity and excludes font storage, metadata, and caller buffers.
- Twenty-eight processing tests (22 unit and 6 independent acceptance tests)
  passed. Processing formatting and strict Clippy pass;
  the root acceptance run also passed the WASM compilation check. No browser or
  native replacement is claimed by this phase.

## 2.5 local validation record

- Native streaming writes directly to a temporary file and browser streaming
  retains bounded encoded chunks for the Blob path. Fifteen export tests pass,
  including PNG decode, native/chunk byte equality, short writes, sink/IEND
  failures, row validation, cancellation, generation mismatch, and output
  limits. Partial output on error must be discarded; no atomic rename policy is
  claimed.
- Release benchmarks use a deterministic checker/gradient PNG, classic cell
  size 20, font size 20, threshold 128, green/black RGBA8 output, 256 MiB
  output ceiling, and Fast/NoFilter PNG settings. The three measured source
  sizes are 1 MP (1000x1000), 12 MP (4000x3000), and 50 MP (10000x5000).
- Authorized hardware evidence: Apple M1 Pro, 10 cores, 32 GiB RAM, Darwin
  27 arm64, Rust 1.88.0 release profile. Whole-process peak RSS was 22,233,088
  bytes (1 MP), 161,267,712 bytes (12 MP), and 611,483,648 bytes (50 MP).
  These are whole-process measurements including fixture generation, not
  stage-specific native-export memory; retained payload counts are reported
  separately and are not RSS claims.
- Stage timings and output sizes are recorded in the ignored
  `docs/reports/core-bench.md`. The 12 MP and 50 MP cached-preview targets are
  informational (under 100 ms and under 250 ms respectively), not CI gates.
  Results are release measurements from this macOS host and do not establish
  cross-platform or arbitrary-image performance.
- Final workspace acceptance reports 46 Rust tests passing (including 15 export
  tests), strict all-target Clippy and formatting checks passing, and the
  locked WASM workspace check passing. Python fallback checks remain separate;
  no browser UI or native replacement is claimed by this phase.

## 3.1 local validation record

- The frontend workspace pins Node 24.15.0 and the reviewed React, Vite,
  TypeScript, Vitest, and Prettier versions. Its typed adapter/controller
  boundary keeps sessions, binary payloads, and revocable preview handles out
  of JSON-safe snapshots.
- Eighteen frontend tests, strict TypeScript typechecking, production build,
  and Prettier checks pass locally. The demo adapter is fixture-only: real file
  conversion, PNG export, and the browser processing worker remain later work.
- Chromium checks passed at all tested widths from 320 through 2560 CSS px plus
  640x360, including preview fit, reset, text export, and inspector transitions.
  A WebKit rendered check passed. The Firefox profile launched but its browser
  verification remains unverified. The real React lifecycle probe was invalid
  because it loaded a second React runtime, so no lifecycle browser pass is
  claimed; hook lifecycle behavior is covered by code review and unit tests.
