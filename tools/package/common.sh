#!/usr/bin/env bash
# Shared guards for local desktop packaging. Sourced by build-*.sh, verify-macos.sh and the tests.
#
# Packages are written under .local/packages, which is git-ignored. Signing, notarization, updater
# signatures, uploads and publishing are never performed: a signing variable, overlay or config key
# makes the build refuse to start rather than being silently removed.
set -euo pipefail

PACKAGE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$PACKAGE_DIR/../.." && pwd)"
DESKTOP_DIR="${PACKAGE_DESKTOP_DIR:-$REPO_ROOT/crates/desktop}"
FRONTEND_DIR="${PACKAGE_FRONTEND_DIR:-$REPO_ROOT/frontend}"
PACKAGE_OUTPUT_ROOT="${PACKAGE_OUTPUT_ROOT:-$REPO_ROOT/.local/packages}"
TAURI_CLI="${TAURI_CLI:-$REPO_ROOT/.local/tauri-cli/bin/cargo-tauri}"
TAURI_CLI_VERSION="2.11.5"

EXIT_UNAVAILABLE=69
EXIT_SIGNING_CONFIGURED=78

# Variables that tauri-cli 2.11.5 or tauri-bundler 2.9.4 read to sign, notarize, or override config.
SIGNING_VARIABLES=(
    APPLE_API_ISSUER
    APPLE_API_KEY
    APPLE_API_KEY_PATH
    APPLE_CERTIFICATE
    APPLE_CERTIFICATE_PASSWORD
    APPLE_ID
    APPLE_PASSWORD
    APPLE_PROVIDER_SHORT_NAME
    APPLE_SIGNING_IDENTITY
    APPLE_TEAM_ID
    TAURI_CONFIG
    TAURI_PRIVATE_KEY
    TAURI_PRIVATE_KEY_PASSWORD
    TAURI_PRIVATE_KEY_PATH
    TAURI_SIGNING_PRIVATE_KEY
    TAURI_SIGNING_PRIVATE_KEY_PASSWORD
    TAURI_SIGNING_PRIVATE_KEY_PATH
    TAURI_SIGNING_RPM_KEY
    TAURI_SIGNING_RPM_KEY_PASSPHRASE
    TAURI_WINDOWS_SIGNTOOL_PATH
)

SMOKE_MARKERS=(
    "--smoke"
    "image-to-binary.desktop-smoke/1"
    "smoke/probe.js"
    "smoke-driver"
    "smoke-watchdog"
    "smoke_e2e"
    "smoke_page"
    "smoke_worker_probe"
)

CODEC_GLUE_NAME="heif_decode"
HEIF_DISABLE_VARIABLE="IMAGE_TO_BINARY_DISABLE_HEIF_GLUE"

die() {
    echo "package: error: $*" >&2
    exit 1
}

# Refuses, naming only the variables (never their values), before any work starts.
refuse_signing_environment() {
    local configured=()
    local name
    for name in "${SIGNING_VARIABLES[@]}"; do
        if [ -n "${!name:-}" ]; then
            configured+=("$name")
        fi
    done
    if [ "${#configured[@]}" -gt 0 ]; then
        echo "package: refusing to run: signing variables are set: ${configured[*]}" >&2
        echo "package: these scripts never sign, notarize, upload or publish; unset them and retry" >&2
        exit "$EXIT_SIGNING_CONFIGURED"
    fi
}

# Refuses overlay configs (tauri.<platform>.conf.json) and tauri.conf.json settings that enable signing.
refuse_signing_configuration() {
    local overlays
    overlays="$(find "$DESKTOP_DIR" -maxdepth 1 -name 'tauri*.conf.json' ! -name tauri.conf.json -print)"
    if [ -n "$overlays" ]; then
        echo "package: refusing to run: config overlays present: $(echo "$overlays" | xargs -n1 basename | tr '\n' ' ')" >&2
        exit "$EXIT_SIGNING_CONFIGURED"
    fi
    local reasons
    reasons="$(command -v node >/dev/null 2>&1 && node -e '
const fs = require("fs");
const config = JSON.parse(fs.readFileSync(process.argv[1], "utf8"));
const bundle = config.bundle ?? {};
const windows = bundle.windows ?? {};
const macos = bundle.macOS ?? {};
const reasons = [];
if (windows.certificateThumbprint) reasons.push("bundle.windows.certificateThumbprint is set");
if (windows.signCommand) reasons.push("bundle.windows.signCommand is set");
if (macos.signingIdentity !== null && macos.signingIdentity !== undefined) reasons.push("bundle.macOS.signingIdentity is not null");
if (bundle.createUpdaterArtifacts !== false) reasons.push("bundle.createUpdaterArtifacts is not false");
process.stdout.write(reasons.join("\n"));
' "$DESKTOP_DIR/tauri.conf.json")" || die "cannot read $DESKTOP_DIR/tauri.conf.json"
    if [ -n "$reasons" ]; then
        echo "package: refusing to run: $(echo "$reasons" | tr '\n' ';')" >&2
        exit "$EXIT_SIGNING_CONFIGURED"
    fi
}

# Prints the explicit unavailable message and exits with EXIT_UNAVAILABLE (69, sysexits EX_UNAVAILABLE).
unavailable() {
    local target="$1"
    shift
    echo "UNAVAILABLE: $target: $*" >&2
    echo "package: no package was built for $target; exit status $EXIT_UNAVAILABLE means its toolchain or host is missing" >&2
    exit "$EXIT_UNAVAILABLE"
}

require_rust_target() {
    local target="$1"
    local label="$2"
    command -v rustup >/dev/null 2>&1 \
        || unavailable "$label" "rustup is not on PATH, so the $target target cannot be checked"
    rustup target list --installed 2>/dev/null | grep -qx "$target" \
        || unavailable "$label" "Rust target $target is not installed; this script does not install targets"
}

require_tauri_cli() {
    local label="$1"
    [ -x "$TAURI_CLI" ] \
        || unavailable "$label" "cargo-tauri is not at $TAURI_CLI; install it locally with: cargo install tauri-cli --version =$TAURI_CLI_VERSION --locked --root .local/tauri-cli"
}

# Refuses a non-empty output directory, so one build's files never mix with another's. A stale
# SHA256SUMS.txt is the only entry that may be present, and it is removed before placing outputs.
require_empty_output() {
    local dir="$1"
    [ -d "$dir" ] || return 0
    local entry
    for entry in "$dir"/* "$dir"/.[!.]*; do
        [ -e "$entry" ] || continue
        [ "$(basename "$entry")" = "SHA256SUMS.txt" ] && continue
        die "refusing to build: $entry already exists; move or remove the previous package first"
    done
}

# Builds the frontend that the desktop crate embeds. The HEIF/AVIF decoder glue is disabled so the
# package never contains the LGPL/HEVC browser decoder; the browser build keeps it.
build_frontend() {
    [ -d "$FRONTEND_DIR/node_modules" ] || die "frontend/node_modules is missing; run npm ci in frontend first"
    (cd "$FRONTEND_DIR" && env "$HEIF_DISABLE_VARIABLE=1" npm run build)
    check_no_codec_glue "$FRONTEND_DIR/dist"
}

# Fails if the given file or directory contains the browser decoder glue, by name or by content.
check_no_codec_glue() {
    local target="$1"
    if [ -d "$target" ]; then
        local named
        named="$(find "$target" -name "${CODEC_GLUE_NAME}*" -print)"
        [ -z "$named" ] || die "codec decoder asset present in $target: $named"
        local hits
        hits="$(grep -r -a -l -F -e "$CODEC_GLUE_NAME" "$target" || true)"
        [ -z "$hits" ] || die "codec decoder name found in $hits"
    else
        if grep -a -q -F -e "$CODEC_GLUE_NAME" "$target"; then
            die "codec decoder name found in $target"
        fi
    fi
}

# Fails if any smoke-only marker appears in the given file.
check_no_smoke_markers() {
    local file="$1"
    local marker
    for marker in "${SMOKE_MARKERS[@]}"; do
        if grep -a -F -q -e "$marker" "$file"; then
            die "smoke marker '$marker' found in $file; the package must not contain smoke code"
        fi
    done
}

# Restores crates/desktop/Cargo.toml from the backup taken by run_tauri_build, then removes the backup.
restore_package_manifest() {
    if [ -n "${PACKAGE_MANIFEST_BACKUP:-}" ] && [ -f "$PACKAGE_MANIFEST_BACKUP" ]; then
        if ! cmp -s "$PACKAGE_MANIFEST_BACKUP" "$PACKAGE_MANIFEST"; then
            cp "$PACKAGE_MANIFEST_BACKUP" "$PACKAGE_MANIFEST"
            echo "package: restored $PACKAGE_MANIFEST after cargo-tauri rewrote it" >&2
        fi
        rm -f "$PACKAGE_MANIFEST_BACKUP"
    fi
    PACKAGE_MANIFEST_BACKUP=""
}

# Runs the default feature set (no smoke, no native-codecs) with signing and updater artifacts disabled.
# cargo-tauri rewrites crates/desktop/Cargo.toml during builds. The EXIT trap restores it on normal
# exit and on INT, TERM or HUP, so an interrupted build leaves no rewritten manifest or stale backup.
run_tauri_build() {
    local bundles="$1"
    PACKAGE_MANIFEST="$DESKTOP_DIR/Cargo.toml"
    PACKAGE_MANIFEST_BACKUP="$(mktemp "$REPO_ROOT/.local/Cargo.toml.XXXXXX")"
    cp "$PACKAGE_MANIFEST" "$PACKAGE_MANIFEST_BACKUP"
    trap restore_package_manifest EXIT
    trap 'exit 130' INT
    trap 'exit 143' TERM
    trap 'exit 129' HUP
    local status=0
    (cd "$DESKTOP_DIR" && CARGO_NET_OFFLINE=true "$TAURI_CLI" build --ci --bundles "$bundles") || status=$?
    restore_package_manifest
    trap - EXIT INT TERM HUP
    return "$status"
}

# Writes the third-party notices that the desktop bundle ships under licenses/.
generate_notices() {
    "$PACKAGE_DIR/generate-notices.sh" "$1"
}

# Moves a finished artifact into the package directory. Existing outputs are never overwritten.
place_artifact() {
    local source="$1"
    local destination="$2"
    [ -e "$destination" ] && die "refusing to overwrite $destination; move or remove it first"
    mv "$source" "$destination"
}

crate_version() {
    sed -n 's/^version = "\(.*\)"$/\1/p' "$DESKTOP_DIR/Cargo.toml" | head -n 1
}

# Writes size and SHA-256 for every file in the package directory, excluding the manifest itself.
write_manifest() {
    local package_dir="$1"
    local manifest="$package_dir/SHA256SUMS.txt"
    (
        cd "$package_dir"
        find . -mindepth 1 -maxdepth 1 ! -name SHA256SUMS.txt -print | sed 's|^\./||' | LC_ALL=C sort | while IFS= read -r entry; do
            if [ -d "$entry" ]; then
                find "$entry" -type f -print0 | LC_ALL=C sort -z | xargs -0 shasum -a 256
            else
                shasum -a 256 "$entry"
            fi
        done
    ) > "$manifest"
    echo "package: wrote $manifest"
}
