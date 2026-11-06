#!/usr/bin/env bash
# Runs cargo with static, pinned libheif/libde265/libaom from build.sh for the host target.
#
# Usage: tools/native-codecs/cargo-with-codecs.sh <cargo args...>
# Example: tools/native-codecs/cargo-with-codecs.sh test -p image-to-binary-processing --features heif --locked
#
# Only the host triple is supported; run build.sh first. Other cargo invocations
# (default features, wasm32) do not need this wrapper.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
NATIVE_ROOT="${NATIVE_CODECS_ROOT:-$REPO_ROOT/.local/native-codecs}"

die() {
    echo "cargo-with-codecs.sh: error: $*" >&2
    exit 1
}

host_triple() {
    case "$(uname -s)/$(uname -m)" in
        Darwin/arm64) echo aarch64-apple-darwin ;;
        Darwin/x86_64) echo x86_64-apple-darwin ;;
        Linux/x86_64) echo x86_64-unknown-linux-gnu ;;
        Linux/aarch64) echo aarch64-unknown-linux-gnu ;;
        *) die "unsupported host $(uname -s)/$(uname -m); see tools/native-codecs/README.md" ;;
    esac
}

PREFIX="$NATIVE_ROOT/$(host_triple)"
for lib in libheif.a libde265.a libaom.a; do
    [ -f "$PREFIX/lib/$lib" ] || die "$PREFIX/lib/$lib is missing; run tools/native-codecs/build.sh first"
done
[ -f "$PREFIX/lib/pkgconfig/libheif.pc" ] || die "$PREFIX/lib/pkgconfig/libheif.pc is missing; run tools/native-codecs/build.sh first"

export PKG_CONFIG_LIBDIR="$PREFIX/lib/pkgconfig"
export SYSTEM_DEPS_LINK=static

cd "$REPO_ROOT"
exec cargo "$@"
