#!/usr/bin/env bash
# Builds pinned, decode-only static libde265, libaom and libheif for Emscripten,
# then links the lazy browser glue into .local/wasm-codecs/dist.
#
# Output: .local/wasm-codecs/dist/heif_decode.mjs and heif_decode.wasm.
# Re-running is idempotent: the SDK, each codec and the glue are skipped when
# their stamp (version, archive hash, flags) still matches.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
# shellcheck source=../native-codecs/versions.env
. "$REPO_ROOT/tools/native-codecs/versions.env"
# shellcheck source=versions.env
. "$SCRIPT_DIR/versions.env"

WASM_ROOT="${WASM_CODECS_ROOT:-$REPO_ROOT/.local/wasm-codecs}"
EMSDK_DIR="$REPO_ROOT/.local/emsdk"
NATIVE_SRC_DIR="$REPO_ROOT/.local/native-codecs/src"
SRC_DIR="$WASM_ROOT/src"
BUILD_DIR="$WASM_ROOT/build"
PREFIX="$WASM_ROOT/prefix"
DIST_DIR="$WASM_ROOT/dist"
JOBS="$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 4)"

MAXIMUM_MEMORY_BYTES=1073741824
STACK_SIZE_BYTES=1048576

die() {
    echo "build.sh: error: $*" >&2
    exit 1
}

need() {
    command -v "$1" >/dev/null 2>&1 || die "required tool '$1' not found on PATH"
}

sha256_of() {
    if command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | cut -d' ' -f1
    else
        sha256sum "$1" | cut -d' ' -f1
    fi
}

ensure_emsdk() {
    need git
    need curl
    if [ ! -d "$EMSDK_DIR/.git" ]; then
        git clone --depth 1 --branch "$EMSDK_VERSION" "$EMSDK_REPO" "$EMSDK_DIR"
    fi
    local head
    head="$(git -C "$EMSDK_DIR" rev-parse HEAD)"
    [ "$head" = "$EMSDK_COMMIT" ] || die "emsdk HEAD $head does not match pinned $EMSDK_COMMIT for $EMSDK_VERSION"
    "$EMSDK_DIR/emsdk" install "$EMSDK_VERSION"
    "$EMSDK_DIR/emsdk" activate "$EMSDK_VERSION"
    set +u
    # shellcheck source=/dev/null
    . "$EMSDK_DIR/emsdk_env.sh" >/dev/null
    set -u
    export PATH="$EMSDK_DIR/upstream/bin:$PATH"
    local version_line
    version_line="$(emcc --version 2>/dev/null | head -n 1)"
    case "$version_line" in
        *" $EMSDK_VERSION "*) ;;
        *) die "emcc does not report $EMSDK_VERSION: $version_line" ;;
    esac
    echo "$version_line"
}

fetch_verified() {
    local name="$1" url="$2" expected="$3"
    local base dir file seed tmp
    base="$(basename "$url")"
    dir="$SRC_DIR/$name"
    file="$dir/$base"
    seed="$NATIVE_SRC_DIR/$name/$base"
    mkdir -p "$dir"
    if [ -f "$file" ] && [ "$(sha256_of "$file")" = "$expected" ]; then
        return 0
    fi
    rm -f "$file"
    if [ -f "$seed" ] && [ "$(sha256_of "$seed")" = "$expected" ]; then
        cp "$seed" "$file"
        return 0
    fi
    tmp="$file.part"
    curl -fsSL --retry 3 -o "$tmp" "$url" || die "download failed: $url"
    local actual
    actual="$(sha256_of "$tmp")"
    if [ "$actual" != "$expected" ]; then
        rm -f "$tmp"
        die "SHA-256 mismatch for $url: expected $expected, got $actual"
    fi
    mv "$tmp" "$file"
}

unpack_fresh() {
    local name="$1" archive="$2"
    local out="$BUILD_DIR/$name"
    rm -rf "$out"
    mkdir -p "$out"
    tar -xzf "$archive" -C "$out" --strip-components=1
    echo "$out"
}

stamp_path() {
    echo "$PREFIX/.stamps/$1"
}

is_current() {
    local stamp
    stamp="$(stamp_path "$1")"
    [ -f "$stamp" ] && [ "$(cat "$stamp")" = "$2" ]
}

write_stamp() {
    mkdir -p "$PREFIX/.stamps"
    printf '%s\n' "$2" > "$(stamp_path "$1")"
}

copy_licenses() {
    local name="$1" src="$2"
    local dest="$PREFIX/share/licenses/$name" f
    mkdir -p "$dest"
    for f in "$src"/COPYING "$src"/LICENSE "$src"/PATENTS "$src"/AUTHORS; do
        if [ -f "$f" ]; then
            cp "$f" "$dest/"
        fi
    done
}

# Every codec build sees only the wasm prefix through pkg-config, never host libraries.
wasm_cmake() {
    local name="$1" src="$2"
    shift 2
    local bdir="$BUILD_DIR/$name-cmake"
    rm -rf "$bdir"
    PKG_CONFIG_LIBDIR="$PREFIX/lib/pkgconfig" PKG_CONFIG_PATH="" \
        emcmake cmake -S "$src" -B "$bdir" -G Ninja \
        -DCMAKE_BUILD_TYPE=Release \
        -DCMAKE_INSTALL_PREFIX="$PREFIX" \
        -DCMAKE_PREFIX_PATH="$PREFIX" \
        -DBUILD_SHARED_LIBS=OFF \
        -DCMAKE_POLICY_VERSION_MINIMUM=3.5 \
        "$@"
    emmake cmake --build "$bdir" --parallel "$JOBS"
    emmake cmake --install "$bdir"
}

build_libde265() {
    local fingerprint="emscripten libde265 $LIBDE265_VERSION $LIBDE265_SHA256 decoder-library simd=off tools=off sdl=off"
    if is_current libde265 "$fingerprint"; then
        echo "libde265 $LIBDE265_VERSION: up to date"
        return 0
    fi
    fetch_verified libde265 "$LIBDE265_URL" "$LIBDE265_SHA256"
    local src
    src="$(unpack_fresh libde265 "$SRC_DIR/libde265/$(basename "$LIBDE265_URL")")"
    wasm_cmake libde265 "$src" \
        -DENABLE_SIMD=OFF \
        -DENABLE_DECODER=OFF \
        -DENABLE_ENCODER=OFF \
        -DENABLE_SDL=OFF \
        -DENABLE_SHERLOCK265=OFF \
        -DENABLE_INTERNAL_DEVELOPMENT_TOOLS=OFF \
        -DWITH_FUZZERS=OFF
    copy_licenses libde265 "$src"
    write_stamp libde265 "$fingerprint"
}

build_libaom() {
    local fingerprint="emscripten libaom $LIBAOM_VERSION $LIBAOM_SHA256 decoder-only target=generic runtime-cpu=off multithread=off"
    if is_current libaom "$fingerprint"; then
        echo "libaom $LIBAOM_VERSION: up to date"
        return 0
    fi
    fetch_verified libaom "$LIBAOM_URL" "$LIBAOM_SHA256"
    local src
    src="$(unpack_fresh libaom "$SRC_DIR/libaom/$(basename "$LIBAOM_URL")")"
    wasm_cmake libaom "$src" \
        -DAOM_TARGET_CPU=generic \
        -DCONFIG_RUNTIME_CPU_DETECT=0 \
        -DCONFIG_MULTITHREAD=0 \
        -DCONFIG_AV1_ENCODER=0 \
        -DCONFIG_AV1_DECODER=1 \
        -DCONFIG_WEBM_IO=0 \
        -DENABLE_EXAMPLES=OFF \
        -DENABLE_TESTS=OFF \
        -DENABLE_TESTDATA=OFF \
        -DENABLE_TOOLS=OFF \
        -DENABLE_DOCS=OFF \
        -DENABLE_APPS=OFF \
        -DENABLE_CCACHE=OFF
    copy_licenses libaom "$src"
    write_stamp libaom "$fingerprint"
}

# libheif disables HEVC deblocking and SAO for every Emscripten build. That changes HEIC output
# against the native build (AVIF is unaffected), so the override is removed from the source copy.
keep_loop_filters() {
    local plugin="$1/libheif/plugins/decoder_libde265.cc" found
    found="$(grep -cE 'DE265_DECODER_PARAM_DISABLE_(DEBLOCKING|SAO), 1\);' "$plugin" || true)"
    [ "$found" = 2 ] || die "expected 2 loop-filter overrides in $plugin, found ${found:-none}; upstream changed"
    sed -E -i.orig '/DE265_DECODER_PARAM_DISABLE_(DEBLOCKING|SAO), 1\);/d' "$plugin"
    rm -f "$plugin.orig"
}

build_libheif() {
    local fingerprint="emscripten libheif $LIBHEIF_VERSION $LIBHEIF_SHA256 libde265 $LIBDE265_VERSION $LIBDE265_SHA256 libaom $LIBAOM_VERSION $LIBAOM_SHA256 static decoder=libde265+aom no-plugins no-threads no-embind standalone-define all-other-codecs-off loop-filters-kept"
    if is_current libheif "$fingerprint"; then
        echo "libheif $LIBHEIF_VERSION: up to date"
        return 0
    fi
    fetch_verified libheif "$LIBHEIF_URL" "$LIBHEIF_SHA256"
    local src
    src="$(unpack_fresh libheif "$SRC_DIR/libheif/$(basename "$LIBHEIF_URL")")"
    keep_loop_filters "$src"
    wasm_cmake libheif "$src" \
        -DCMAKE_C_FLAGS="-D__EMSCRIPTEN_STANDALONE_WASM__=1" \
        -DCMAKE_CXX_FLAGS="-D__EMSCRIPTEN_STANDALONE_WASM__=1" \
        -DENABLE_PLUGIN_LOADING=OFF \
        -DENABLE_MULTITHREADING_SUPPORT=OFF \
        -DWITH_LIBDE265=ON \
        -DWITH_LIBDE265_PLUGIN=OFF \
        -DLIBDE265_INCLUDE_DIR="$PREFIX/include" \
        -DLIBDE265_LIBRARY="$PREFIX/lib/libde265.a" \
        -DWITH_AOM_DECODER=ON \
        -DWITH_AOM_DECODER_PLUGIN=OFF \
        -DWITH_AOM_ENCODER=OFF \
        -DWITH_AOM_ENCODER_PLUGIN=OFF \
        -DAOM_INCLUDE_DIR="$PREFIX/include" \
        -DAOM_LIBRARY="$PREFIX/lib/libaom.a" \
        -DWITH_X265=OFF \
        -DWITH_X265_PLUGIN=OFF \
        -DWITH_X264=OFF \
        -DWITH_X264_PLUGIN=OFF \
        -DWITH_KVAZAAR=OFF \
        -DWITH_UVG266=OFF \
        -DWITH_VVDEC=OFF \
        -DWITH_VVENC=OFF \
        -DWITH_DAV1D=OFF \
        -DWITH_SvtEnc=OFF \
        -DWITH_RAV1E=OFF \
        -DWITH_OpenH264_DECODER=OFF \
        -DWITH_FFMPEG_DECODER=OFF \
        -DWITH_JPEG_DECODER=OFF \
        -DWITH_JPEG_ENCODER=OFF \
        -DWITH_OpenJPEG_DECODER=OFF \
        -DWITH_OpenJPEG_ENCODER=OFF \
        -DWITH_OPENJPH_ENCODER=OFF \
        -DWITH_UNCOMPRESSED_CODEC=OFF \
        -DWITH_WEBCODECS=OFF \
        -DWITH_HEADER_COMPRESSION=OFF \
        -DWITH_LIBSHARPYUV=OFF \
        -DWITH_EXAMPLES=OFF \
        -DWITH_EXAMPLE_HEIF_THUMB=OFF \
        -DWITH_EXAMPLE_HEIF_VIEW=OFF \
        -DWITH_GDK_PIXBUF=OFF \
        -DBUILD_TESTING=OFF \
        -DBUILD_DOCUMENTATION=OFF \
        -DBUILD_DEVELOPMENT_TOOLS=OFF \
        -DWITH_FUZZERS=OFF
    copy_licenses libheif "$src"
    write_stamp libheif "$fingerprint"
}

check_decode_only() {
    local lib symbols
    need llvm-nm
    for lib in libde265.a libaom.a libheif.a; do
        [ -f "$PREFIX/lib/$lib" ] || die "missing $PREFIX/lib/$lib after build"
    done
    symbols="$(llvm-nm "$PREFIX/lib/libaom.a")" || die "llvm-nm could not list $PREFIX/lib/libaom.a"
    if printf '%s\n' "$symbols" | grep -q 'aom_codec_av1_cx'; then
        die "libaom.a contains AV1 encoder symbols; CONFIG_AV1_ENCODER=0 was not honored"
    fi
}

link_glue() {
    local glue="$SCRIPT_DIR/heif_decode.c"
    local exports="_hwc_decode,_hwc_width,_hwc_height,_hwc_pixels,_hwc_message,_hwc_release,_malloc,_free"
    mkdir -p "$DIST_DIR"
    emcc -O3 -std=c11 -Wall -Wextra -Werror \
        -I"$PREFIX/include" \
        "$glue" \
        "$PREFIX/lib/libheif.a" "$PREFIX/lib/libde265.a" "$PREFIX/lib/libaom.a" \
        -sDEFAULT_TO_CXX=1 \
        -sMODULARIZE=1 \
        -sEXPORT_ES6=1 \
        -sEXPORT_NAME=createHeifDecoderModule \
        -sENVIRONMENT=worker,node \
        -sSINGLE_FILE=0 \
        -sALLOW_MEMORY_GROWTH=1 \
        -sMAXIMUM_MEMORY="$MAXIMUM_MEMORY_BYTES" \
        -sSTACK_SIZE="$STACK_SIZE_BYTES" \
        -sFILESYSTEM=0 \
        -sEXPORTED_FUNCTIONS="$exports" \
        -sEXPORTED_RUNTIME_METHODS=HEAPU8 \
        -o "$DIST_DIR/heif_decode.mjs"
}

rm -f "$DIST_DIR"/heif_decode.*

need cmake
need ninja
need pkg-config
need curl
need tar

ensure_emsdk
mkdir -p "$SRC_DIR" "$BUILD_DIR" "$PREFIX"

build_libde265
build_libaom
build_libheif
check_decode_only

link_glue

echo "wasm codecs ready: $DIST_DIR/heif_decode.mjs"
