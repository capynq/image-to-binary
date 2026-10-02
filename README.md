# Image to Binary

A local image-to-binary-art desktop tool. The current reference application is
Python/Tkinter; the Rust-powered desktop and browser replacement is planned in
[docs/PLAN.md](docs/PLAN.md), not implemented yet.

## Current behavior

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
