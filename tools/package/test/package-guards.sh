#!/usr/bin/env bash
# Guard tests for local desktop packaging. Each check can fail; none is a no-op.
#
# Runs no cargo-tauri build. Uses fixtures for overlays, Cargo.toml restore, scanners, notices and
# output placement. PACKAGE_UNDER_TEST=<dir> points the tests at another copy of tools/package, which
# is how a deliberately mutated script is shown to fail.
set -euo pipefail

TEST_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PACKAGE_DIR="$(cd "${PACKAGE_UNDER_TEST:-$TEST_DIR/..}" && pwd)"
REPO_ROOT="$(cd "$TEST_DIR/../../.." && pwd)"

# The signing variables the guard must refuse. Hard-coded here, not read from common.sh.
EXPECTED_SIGNING_VARIABLES=(
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

SCRATCH="$(mktemp -d "$REPO_ROOT/.local/package-test.XXXXXX")"
cleanup() {
    find "$SCRATCH" -depth -delete 2>/dev/null || true
    rmdir "$SCRATCH" 2>/dev/null || true
}
trap cleanup EXIT

passes=0
failures=0
pass() {
    passes=$((passes + 1))
    echo "ok: $1"
}
fail() {
    failures=$((failures + 1))
    echo "FAIL: $1" >&2
}
check() {
    local name="$1"
    shift
    if "$@"; then
        pass "$name"
    else
        fail "$name"
    fi
}

# Runs a build script in a clean environment; sets RUN_STATUS and RUN_STDERR.
run_script() {
    local script="$1"
    shift
    local passthrough=()
    local name
    for name in RUSTUP_HOME CARGO_HOME; do
        if [ -n "${!name:-}" ]; then
            passthrough+=("$name=${!name}")
        fi
    done
    RUN_STATUS=0
    env -i PATH="$PATH" HOME="${HOME:-/tmp}" ${passthrough[@]+"${passthrough[@]}"} \
        "$@" "$PACKAGE_DIR/$script" >"$SCRATCH/stdout.txt" 2>"$SCRATCH/stderr.txt" || RUN_STATUS=$?
    RUN_STDERR="$(cat "$SCRATCH/stderr.txt")"
}

# Checks run in subshells so that the sourced guard functions may exit.
source_common() {
    # shellcheck source=/dev/null
    source "$PACKAGE_DIR/common.sh"
}

# 1. The guard's list matches the hard-coded expectation exactly.
test_signing_list_matches() {
    local actual expected
    actual="$(
        source_common
        printf '%s\n' "${SIGNING_VARIABLES[@]}" | LC_ALL=C sort
    )"
    expected="$(printf '%s\n' "${EXPECTED_SIGNING_VARIABLES[@]}" | LC_ALL=C sort)"
    if [ "$actual" = "$expected" ]; then
        return 0
    fi
    diff <(echo "$expected") <(echo "$actual") >&2 || true
    return 1
}
check "signing list equals the hard-coded expectation" test_signing_list_matches

# 2. Each expected variable makes the macOS build refuse with 78, names it, and never prints its value.
test_each_signing_variable_refuses() {
    local canary="canary-value-do-not-print"
    local name
    for name in "${EXPECTED_SIGNING_VARIABLES[@]}"; do
        run_script build-macos.sh "$name=$canary" "PACKAGE_OUTPUT_ROOT=$SCRATCH/refused-$name"
        if [ "$RUN_STATUS" -ne 78 ]; then
            echo "$name: expected exit 78, got $RUN_STATUS" >&2
            return 1
        fi
        if ! grep -q "$name" "$SCRATCH/stderr.txt" || grep -q "$canary" "$SCRATCH/stderr.txt"; then
            echo "$name: refusal does not name it or prints its value" >&2
            return 1
        fi
        if [ -e "$SCRATCH/refused-$name" ]; then
            echo "$name: refusal created the output directory" >&2
            return 1
        fi
    done
}
check "each signing variable refuses before any work" test_each_signing_variable_refuses

# 3. Windows and Linux scripts refuse signing variables before their host check.
test_other_targets_refuse() {
    local script
    for script in build-windows.sh build-linux.sh; do
        run_script "$script" "APPLE_SIGNING_IDENTITY=x" "PACKAGE_OUTPUT_ROOT=$SCRATCH/refused-$script"
        [ "$RUN_STATUS" -eq 78 ] || { echo "$script: status $RUN_STATUS" >&2; return 1; }
    done
}
check "windows and linux refuse signing variables first" test_other_targets_refuse

# 4. Config-level refusals: overlays, Windows signing keys, macOS signing identity, updater artifacts.
make_desktop_fixture() {
    local dir="$1"
    mkdir -p "$dir"
    cp "$REPO_ROOT/crates/desktop/tauri.conf.json" "$dir/tauri.conf.json"
    cp "$REPO_ROOT/crates/desktop/Cargo.toml" "$dir/Cargo.toml"
}
config_refused() {
    local dir="$1"
    local status=0
    (PACKAGE_DESKTOP_DIR="$dir" source_common; refuse_signing_configuration) >/dev/null 2>&1 || status=$?
    [ "$status" -eq 78 ]
}
config_accepted() {
    local dir="$1"
    (PACKAGE_DESKTOP_DIR="$dir" source_common; refuse_signing_configuration) >/dev/null 2>&1
}
edit_config() {
    local file="$1"
    local script="$2"
    node -e "
const fs = require('fs');
const path = process.argv[1];
const config = JSON.parse(fs.readFileSync(path, 'utf8'));
(${script})(config);
fs.writeFileSync(path, JSON.stringify(config, null, 2));
" "$file"
}
test_config_refusals() {
    local base="$SCRATCH/config-clean"
    make_desktop_fixture "$base"
    config_accepted "$base" || { echo "clean fixture rejected" >&2; return 1; }

    local dir
    dir="$SCRATCH/config-overlay"; make_desktop_fixture "$dir"; echo '{}' > "$dir/tauri.windows.conf.json"
    config_refused "$dir" || { echo "overlay not refused" >&2; return 1; }

    dir="$SCRATCH/config-thumb"; make_desktop_fixture "$dir"
    edit_config "$dir/tauri.conf.json" 'c => { c.bundle.windows = { certificateThumbprint: "ABC" } }'
    config_refused "$dir" || { echo "certificateThumbprint not refused" >&2; return 1; }

    dir="$SCRATCH/config-signcmd"; make_desktop_fixture "$dir"
    edit_config "$dir/tauri.conf.json" 'c => { c.bundle.windows = { signCommand: "signtool %1" } }'
    config_refused "$dir" || { echo "signCommand not refused" >&2; return 1; }

    dir="$SCRATCH/config-identity"; make_desktop_fixture "$dir"
    edit_config "$dir/tauri.conf.json" 'c => { c.bundle.macOS.signingIdentity = "Developer ID Application: x" }'
    config_refused "$dir" || { echo "signingIdentity not refused" >&2; return 1; }

    dir="$SCRATCH/config-updater"; make_desktop_fixture "$dir"
    edit_config "$dir/tauri.conf.json" 'c => { c.bundle.createUpdaterArtifacts = true }'
    config_refused "$dir" || { echo "createUpdaterArtifacts not refused" >&2; return 1; }
}
check "overlays and signing config keys are refused" test_config_refusals

# 5. The checked-in tauri.conf.json states the policy explicitly.
test_tauri_conf_json() {
    node -e '
const fs = require("fs");
const config = JSON.parse(fs.readFileSync(process.argv[1], "utf8"));
const bundle = config.bundle ?? {};
const windows = bundle.windows ?? {};
const failures = [];
if (bundle.createUpdaterArtifacts !== false) failures.push("createUpdaterArtifacts is not false");
if (bundle.macOS?.signingIdentity !== null) failures.push("macOS.signingIdentity is not null");
for (const key of ["certificateThumbprint", "signCommand", "digestAlgorithm", "timestampUrl"]) {
  if (windows[key] !== undefined) failures.push(`bundle.windows.${key} is present`);
}
if (failures.length) { console.error(failures.join("; ")); process.exit(1); }
' "$REPO_ROOT/crates/desktop/tauri.conf.json"
}
check "tauri.conf.json disables signing and updater artifacts" test_tauri_conf_json

test_no_overlays_in_repo() {
    local overlays
    overlays="$(find "$REPO_ROOT/crates/desktop" -maxdepth 1 -name 'tauri*.conf.json' ! -name tauri.conf.json -print)"
    [ -z "$overlays" ]
}
check "no tauri overlay config exists in crates/desktop" test_no_overlays_in_repo

# 6. Unavailable exits name their cause. A packaging script reports the first check its host fails:
# the host OS or architecture when it does not match, otherwise the missing cargo-tauri. The
# pkg-config stub lets the Linux script reach the cargo-tauri check without installed WebKitGTK.
test_unavailable_causes() {
    local host machine macos_cause linux_cause linux_shim
    host="$(uname -s)"
    machine="$(uname -m)"
    linux_shim="$SCRATCH/linux-shim"
    mkdir -p "$linux_shim"
    printf '#!/usr/bin/env bash\nexit 0\n' >"$linux_shim/pkg-config"
    chmod +x "$linux_shim/pkg-config"

    macos_cause="cargo-tauri is not at"
    if [ "$host" != Darwin ]; then
        macos_cause="host is $host, not macOS"
    elif [ "$machine" != arm64 ]; then
        macos_cause="host is $machine, not arm64"
    fi
    run_script build-macos.sh "TAURI_CLI=$SCRATCH/no-such-cargo-tauri" "PACKAGE_OUTPUT_ROOT=$SCRATCH/unavailable-macos"
    [ "$RUN_STATUS" -eq 69 ] || { echo "macos: status $RUN_STATUS" >&2; return 1; }
    grep -q "^UNAVAILABLE: macos-arm64: $macos_cause" "$SCRATCH/stderr.txt" \
        || { echo "macos message does not name its cause: $macos_cause" >&2; return 1; }
    [ ! -e "$SCRATCH/unavailable-macos" ] || { echo "unavailable run created output" >&2; return 1; }

    linux_cause="cargo-tauri is not at"
    if [ "$host" != Linux ]; then
        linux_cause="host is $host, not Linux"
    elif [ "$machine" != x86_64 ]; then
        linux_cause="host is $machine, not x86_64"
    fi
    run_script build-linux.sh "PATH=$linux_shim:$PATH" "TAURI_CLI=$SCRATCH/no-such-cargo-tauri" "PACKAGE_OUTPUT_ROOT=$SCRATCH/unavailable-linux"
    [ "$RUN_STATUS" -eq 69 ] || { echo "linux: status $RUN_STATUS" >&2; return 1; }
    grep -q "^UNAVAILABLE: linux-x86_64: $linux_cause" "$SCRATCH/stderr.txt" \
        || { echo "linux message does not name its cause: $linux_cause" >&2; return 1; }
    [ ! -e "$SCRATCH/unavailable-linux" ] || { echo "unavailable linux run created output" >&2; return 1; }

    run_script build-windows.sh "PACKAGE_OUTPUT_ROOT=$SCRATCH/unavailable-windows"
    [ "$RUN_STATUS" -eq 69 ] || { echo "windows: status $RUN_STATUS" >&2; return 1; }
    grep -q "^UNAVAILABLE: windows-x86_64: host is $host, not Windows" "$SCRATCH/stderr.txt" \
        || { echo "windows message does not name the host" >&2; return 1; }
}
check "unavailable messages name their actual cause" test_unavailable_causes

# 7. Cargo.toml is restored after a SIGINT during cargo-tauri, and the backup is removed.
test_restore_on_sigint() {
    local fixture="$SCRATCH/restore-desktop"
    mkdir -p "$fixture"
    cp "$REPO_ROOT/crates/desktop/Cargo.toml" "$fixture/Cargo.toml"
    cp "$fixture/Cargo.toml" "$SCRATCH/restore-original.toml"
    local fake="$SCRATCH/fake-cargo-tauri"
    cat > "$fake" <<'SH'
#!/bin/sh
printf '\n# rewritten by fake cargo-tauri\n' >> Cargo.toml
echo $$ > "$FAKE_PID_FILE"
exec sleep 60
SH
    chmod +x "$fake"
    local driver="$SCRATCH/driver.sh"
    cat > "$driver" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
source "$PACKAGE_SCRIPT_DIR/common.sh"
run_tauri_build app
SH
    local pidfile="$SCRATCH/fake.pid"
    local backups_before
    backups_before="$(find "$REPO_ROOT/.local" -maxdepth 1 -name 'Cargo.toml.*' | wc -l | tr -d ' ')"

    # Bash starts background jobs with SIGINT ignored, so the driver is re-executed with INT reset to
    # its default, as an interactive terminal would deliver it.
    env PACKAGE_DESKTOP_DIR="$fixture" TAURI_CLI="$fake" FAKE_PID_FILE="$pidfile" \
        PACKAGE_SCRIPT_DIR="$PACKAGE_DIR" \
        /usr/bin/perl -e '$SIG{INT} = "DEFAULT"; exec { $ARGV[0] } @ARGV' bash "$driver" >"$SCRATCH/driver.log" 2>&1 &
    local driver_pid=$!
    local waited=0
    while ! { [ -s "$pidfile" ] && grep -q "rewritten by fake" "$fixture/Cargo.toml"; }; do
        waited=$((waited + 1))
        if [ "$waited" -gt 100 ]; then
            kill -INT "$driver_pid" 2>/dev/null || true
            echo "fake cargo-tauri never rewrote the manifest" >&2
            return 1
        fi
        sleep 0.2
    done
    local child_pid
    child_pid="$(cat "$pidfile")"
    kill -INT "$driver_pid" "$child_pid" 2>/dev/null || true

    local status=0
    wait "$driver_pid" || status=$?
    if kill -0 "$child_pid" 2>/dev/null; then
        kill -KILL "$child_pid" 2>/dev/null || true
        echo "fake cargo-tauri survived SIGINT" >&2
        return 1
    fi
    cmp -s "$SCRATCH/restore-original.toml" "$fixture/Cargo.toml" \
        || { echo "Cargo.toml was not restored after SIGINT" >&2; return 1; }
    local backups_after
    backups_after="$(find "$REPO_ROOT/.local" -maxdepth 1 -name 'Cargo.toml.*' | wc -l | tr -d ' ')"
    [ "$backups_after" -eq "$backups_before" ] || { echo "stale backup left in .local" >&2; return 1; }
    [ "$status" -ne 0 ] || { echo "interrupted build reported success" >&2; return 1; }
}
check "SIGINT during cargo-tauri restores Cargo.toml and removes the backup" test_restore_on_sigint

# 8. Output placement never overwrites, and a non-empty output directory refuses the build.
test_place_and_destinations() {
    local dir="$SCRATCH/place"
    mkdir -p "$dir/out"
    echo one > "$dir/source-a"
    (source_common; place_artifact "$dir/source-a" "$dir/out/a") || return 1
    [ -f "$dir/out/a" ] && [ ! -e "$dir/source-a" ] || return 1
    echo two > "$dir/source-b"
    local status=0
    (source_common; place_artifact "$dir/source-b" "$dir/out/a") >/dev/null 2>&1 || status=$?
    [ "$status" -ne 0 ] || { echo "place_artifact overwrote an existing file" >&2; return 1; }
    [ -f "$dir/source-b" ] || { echo "refused placement consumed the source" >&2; return 1; }

    mkdir -p "$dir/stale"
    echo x > "$dir/stale/SHA256SUMS.txt"
    (source_common; require_empty_output "$dir/stale") || { echo "stale manifest alone should be allowed" >&2; return 1; }
    echo y > "$dir/stale/leftover.dmg"
    status=0
    (source_common; require_empty_output "$dir/stale") >/dev/null 2>&1 || status=$?
    [ "$status" -ne 0 ] || { echo "non-empty output directory was accepted" >&2; return 1; }
}
check "placement refuses overwrites and non-empty output directories" test_place_and_destinations

# 9. Smoke and codec scans fail on planted content and pass on clean content.
test_scans() {
    local planted="$SCRATCH/planted-smoke.bin" clean="$SCRATCH/clean.bin"
    printf 'header --smoke-report tail' > "$planted"
    printf 'header only' > "$clean"
    local status=0
    (source_common; check_no_smoke_markers "$planted") >/dev/null 2>&1 || status=$?
    [ "$status" -ne 0 ] || { echo "smoke scan missed a planted marker" >&2; return 1; }
    (source_common; check_no_smoke_markers "$clean") >/dev/null 2>&1 || { echo "smoke scan rejected clean file" >&2; return 1; }

    local dist="$SCRATCH/dist-heif"
    mkdir -p "$dist/assets"
    printf 'x' > "$dist/assets/heif_decode-abc.wasm"
    status=0
    (source_common; check_no_codec_glue "$dist") >/dev/null 2>&1 || status=$?
    [ "$status" -ne 0 ] || { echo "codec scan missed a decoder asset" >&2; return 1; }

    dist="$SCRATCH/dist-text"
    mkdir -p "$dist"
    printf 'import heif_decode from "x"' > "$dist/app.js"
    status=0
    (source_common; check_no_codec_glue "$dist") >/dev/null 2>&1 || status=$?
    [ "$status" -ne 0 ] || { echo "codec scan missed decoder name in content" >&2; return 1; }

    dist="$SCRATCH/dist-clean"
    mkdir -p "$dist"
    printf 'plain' > "$dist/app.js"
    (source_common; check_no_codec_glue "$dist") || { echo "codec scan rejected clean dist" >&2; return 1; }
}
check "smoke and codec scans catch planted content" test_scans

# 10. The packaged binary, when present, carries no smoke or codec glue.
test_packaged_binary_clean() {
    local binary="$REPO_ROOT/.local/packages/macos-arm64/Image to Binary.app/Contents/MacOS/image-to-binary-desktop"
    if [ ! -f "$binary" ]; then
        echo "skip: no packaged binary at $binary" >&2
        return 0
    fi
    (source_common; check_no_smoke_markers "$binary"; check_no_codec_glue "$binary")
}
check "packaged macOS binary has no smoke or codec glue" test_packaged_binary_clean

# 11. Notices: real offline run, a fixture with a violation, one with an unresolved license, one clean.
write_notice_fixture() {
    local kind="$1"
    local root="$SCRATCH/notices-$kind"
    mkdir -p "$root/dep" "$root/app" "$root/npm"
    echo "fixture" > "$root/app/Cargo.toml"
    echo "Fixture license text." > "$root/dep/LICENSE"
    local dep_license="\"MIT\""
    [ "$kind" = "violation" ] && dep_license="\"GPL-2.0-only\""
    [ "$kind" = "unresolved" ] && dep_license="null"
    [ "$kind" = "canonical" ] && rm -f "$root/dep/LICENSE"
    cat > "$root/metadata.json" <<JSON
{"packages":[
 {"id":"app 0.1.0 (path+file://$root/app)","name":"app","version":"0.1.0","source":null,"license":"GPL-3.0-only","authors":[],"repository":null,"manifest_path":"$root/app/Cargo.toml"},
 {"id":"dep 1.0.0 (registry+https://github.com/rust-lang/crates.io-index)","name":"dep","version":"1.0.0","source":"registry+https://github.com/rust-lang/crates.io-index","license":$dep_license,"authors":["Fixture Author <fixture@example.invalid>"],"repository":"https://example.invalid/dep","manifest_path":"$root/dep/Cargo.toml"}
],
"resolve":{"root":"app 0.1.0 (path+file://$root/app)","nodes":[
 {"id":"app 0.1.0 (path+file://$root/app)","deps":[{"pkg":"dep 1.0.0 (registry+https://github.com/rust-lang/crates.io-index)","dep_kinds":[{"kind":null,"target":null}]}]},
 {"id":"dep 1.0.0 (registry+https://github.com/rust-lang/crates.io-index)","deps":[]}
]}}
JSON
    echo '{"name":"fixture","dependencies":{}}' > "$root/npm.json"
    echo "$root"
}
run_notices() {
    local root="$1"
    local status=0
    NOTICES_METADATA="$root/metadata.json" NOTICES_NPM="$root/npm.json" NOTICES_OUTPUT="$root/out.txt" \
        PACKAGE_FRONTEND_DIR="$root" "$PACKAGE_DIR/generate-notices.sh" aarch64-apple-darwin >"$root/log.txt" 2>&1 || status=$?
    echo "$status"
}
test_notices() {
    local root status
    root="$(write_notice_fixture clean)"
    status="$(run_notices "$root")"
    [ "$status" -eq 0 ] || { echo "clean fixture: status $status" >&2; cat "$root/log.txt" >&2; return 1; }
    grep -q "^dep 1.0.0" "$root/out.txt" || { echo "clean fixture: dep missing from notices" >&2; return 1; }
    grep -q "Fixture license text." "$root/out.txt" || { echo "clean fixture: license text missing" >&2; return 1; }

    root="$(write_notice_fixture violation)"
    status="$(run_notices "$root")"
    [ "$status" -eq 1 ] || { echo "violation fixture: expected 1, got $status" >&2; return 1; }
    [ ! -e "$root/out.txt" ] || { echo "violation fixture wrote notices" >&2; return 1; }

    root="$(write_notice_fixture unresolved)"
    status="$(run_notices "$root")"
    [ "$status" -eq 2 ] || { echo "unresolved fixture: expected 2, got $status" >&2; return 1; }
    [ ! -e "$root/out.txt" ] || { echo "unresolved fixture wrote notices" >&2; return 1; }

    root="$(write_notice_fixture canonical)"
    status="$(run_notices "$root")"
    [ "$status" -eq 0 ] || { echo "canonical fixture: status $status" >&2; return 1; }
    grep -q "from canonical MIT" "$root/out.txt" || { echo "canonical fixture did not use the canonical MIT text" >&2; return 1; }
    awk '/^dep 1.0.0$/{f=1} f&&n<6{print; n++}' "$root/out.txt" | grep -q "copyright holders (from Cargo.toml authors): Fixture Author" \
        || { echo "canonical fixture: holders not printed beside the component" >&2; return 1; }
    awk '/^dep 1.0.0$/{f=1} f&&n<8{print; n++}' "$root/out.txt" | grep -q "canonical template" \
        || { echo "canonical fixture: reference not marked as a canonical template" >&2; return 1; }
}
check "notices fail on violations and unresolved licenses, and use canonical texts" test_notices

test_canonical_templates_are_neutral() {
    local file
    for file in "$PACKAGE_DIR"/licenses/*.txt; do
        if grep -q "^Copyright" "$file"; then
            echo "$(basename "$file") contains a line starting with Copyright" >&2
            return 1
        fi
        head -n 1 "$file" | grep -q "^Canonical text taken from " \
            || { echo "$(basename "$file") lacks its provenance header" >&2; return 1; }
    done
}
check "canonical license templates carry no copyright line and name their donor" test_canonical_templates_are_neutral

test_notices_real_target() {
    local out="$SCRATCH/notices-real.txt"
    NOTICES_OUTPUT="$out" "$PACKAGE_DIR/generate-notices.sh" aarch64-apple-darwin >"$SCRATCH/notices-real.log" 2>&1 \
        || { cat "$SCRATCH/notices-real.log" >&2; return 1; }
    [ -s "$out" ] || return 1
    grep -q "^Frontend npm packages shipped (production): 4$" "$out"
}
check "real offline notices run for the macOS target" test_notices_real_target

# 12. The browser HEIF glue is included without the flag and excluded with it.
test_heif_flag() {
    if [ ! -f "$REPO_ROOT/.local/wasm-codecs/dist/heif_decode.wasm" ]; then
        echo "skip: no codec dist at .local/wasm-codecs/dist" >&2
        return 0
    fi
    local on="$SCRATCH/heif-on" off="$SCRATCH/heif-off"
    (cd "$REPO_ROOT/frontend" && npx vite build --outDir "$on" --emptyOutDir >/dev/null 2>&1) || return 1
    (cd "$REPO_ROOT/frontend" && IMAGE_TO_BINARY_DISABLE_HEIF_GLUE=1 npx vite build --outDir "$off" --emptyOutDir >/dev/null 2>&1) || return 1
    [ -n "$(find "$on" -name 'heif_decode*.wasm' -print)" ] || { echo "browser build lost the decoder" >&2; return 1; }
    [ -z "$(find "$off" -name 'heif_decode*' -print)" ] || { echo "flagged build still ships the decoder" >&2; return 1; }
}
check "browser build keeps the HEIF decoder; the packaging flag removes it" test_heif_flag

test_gitignore() {
    git -C "$REPO_ROOT" check-ignore -q .local/packages/macos-arm64/example.app \
        && git -C "$REPO_ROOT" check-ignore -q .local/packages/windows-x86_64/example-setup.exe \
        && git -C "$REPO_ROOT" check-ignore -q .local/packages/linux-x86_64/example.AppImage \
        && git -C "$REPO_ROOT" check-ignore -q .local/notices/THIRD_PARTY_LICENSES.txt \
        && git -C "$REPO_ROOT" check-ignore -q docs/reports/package/example.txt
}
check "package, notices and report outputs are git-ignored" test_gitignore

test_syntax() {
    local script
    for script in common.sh build-macos.sh build-windows.sh build-linux.sh verify-macos.sh generate-notices.sh; do
        bash -n "$PACKAGE_DIR/$script" || return 1
    done
}
check "all package scripts parse" test_syntax

echo "package guard tests: $passes passed, $failures failed"
[ "$failures" -eq 0 ]
