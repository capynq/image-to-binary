#!/usr/bin/env bash
# Builds pinned, decode-only static libde265, libaom and libheif for one Rust target.
#
# Output: .local/native-codecs/<rust-target-triple>/{include,lib,lib/pkgconfig,share/licenses}
# Re-running is idempotent: a finished component is skipped when its version,
# archive hash and configure flags still match its stamp file.
#
# Requirements: cmake, ninja and pkg-config on PATH (see requirements.txt),
# clang, tar, curl. x86_64 hosts also need nasm or yasm for libaom assembly.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
# shellcheck source=versions.env
. "$SCRIPT_DIR/versions.env"

NATIVE_ROOT="${NATIVE_CODECS_ROOT:-$REPO_ROOT/.local/native-codecs}"
SRC_DIR="$NATIVE_ROOT/src"
BUILD_DIR="$NATIVE_ROOT/build"

die() {
    echo "build.sh: error: $*" >&2
    exit 1
}

need() {
    command -v "$1" >/dev/null 2>&1 || die "required tool '$1' not found on PATH; $2"
}

host_triple() {
    local os arch
    os="$(uname -s)"
    arch="$(uname -m)"
    case "$os/$arch" in
        Darwin/arm64) echo aarch64-apple-darwin ;;
        Darwin/x86_64) echo x86_64-apple-darwin ;;
        Linux/x86_64) echo x86_64-unknown-linux-gnu ;;
        Linux/aarch64) echo aarch64-unknown-linux-gnu ;;
        *) die "unsupported host $os/$arch; add its Rust triple here" ;;
    esac
}

sha256_of() {
    if command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | cut -d' ' -f1
    else
        sha256sum "$1" | cut -d' ' -f1
    fi
}

cpu_count() {
    getconf _NPROCESSORS_ONLN 2>/dev/null || echo 4
}

HOST_TRIPLE="$(host_triple)"
TRIPLE="${RUST_TARGET:-$HOST_TRIPLE}"
if [ "$TRIPLE" != "$HOST_TRIPLE" ]; then
    die "cross-compiling to $TRIPLE is not supported (host is $HOST_TRIPLE); run on a matching host"
fi

PREFIX="$NATIVE_ROOT/$TRIPLE"

need cmake "install with: .local/tools/bin/python -m pip install -r tools/native-codecs/requirements.txt"
need ninja "install with: .local/tools/bin/python -m pip install -r tools/native-codecs/requirements.txt"
need pkg-config "install with: .local/tools/bin/python -m pip install -r tools/native-codecs/requirements.txt"
need curl "install curl or use the system curl"
need tar "install tar"
need clang "install the Xcode command line tools or LLVM clang"

case "$TRIPLE" in
    x86_64-*)
        if ! command -v nasm >/dev/null 2>&1 && ! command -v yasm >/dev/null 2>&1; then
            die "x86_64 libaom assembly requires nasm (or yasm) on PATH; without it libaom silently falls back to slow C code. Install nasm with your package manager and rerun. This script does not install packages."
        fi
        ;;
esac

mkdir -p "$SRC_DIR" "$BUILD_DIR" "$PREFIX"

fetch_verified() {
    local name="$1" url="$2" expected="$3"
    local dir="$SRC_DIR/$name"
    local file="$dir/$(basename "$url")"
    mkdir -p "$dir"
    if [ -f "$file" ] && [ "$(sha256_of "$file")" = "$expected" ]; then
        return 0
    fi
    rm -f "$file"
    local tmp="$file.part"
    curl -fsSL --retry 3 -o "$tmp" "$url" || die "download failed: $url"
    local actual
    actual="$(sha256_of "$tmp")"
    if [ "$actual" != "$expected" ]; then
        rm -f "$tmp"
        die "SHA-256 mismatch for $url: expected $expected, got $actual"
    fi
    mv "$tmp" "$file"
}

# Unpacks a verified tarball into a fresh build directory and prints its path.
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

# Skips a component whose stamp matches version, archive hash and flags.
is_current() {
    local name="$1" fingerprint="$2"
    local stamp
    stamp="$(stamp_path "$name")"
    [ -f "$stamp" ] && [ "$(cat "$stamp")" = "$fingerprint" ]
}

write_stamp() {
    local name="$1" fingerprint="$2"
    mkdir -p "$PREFIX/.stamps"
    printf '%s\n' "$fingerprint" > "$(stamp_path "$name")"
}

common_cmake_args() {
    echo \
        -G Ninja \
        -DCMAKE_BUILD_TYPE=Release \
        -DCMAKE_INSTALL_PREFIX="$PREFIX" \
        -DCMAKE_PREFIX_PATH="$PREFIX" \
        -DCMAKE_POSITION_INDEPENDENT_CODE=ON \
        -DCMAKE_OSX_DEPLOYMENT_TARGET="$MACOSX_DEPLOYMENT_TARGET" \
        -DCMAKE_POLICY_VERSION_MINIMUM=3.5 \
        -DBUILD_SHARED_LIBS=OFF
}

cmake_build_install() {
    local name="$1" src="$2"
    shift 2
    local bdir="$BUILD_DIR/$name-cmake"
    rm -rf "$bdir"
    PKG_CONFIG_PATH="$PREFIX/lib/pkgconfig" \
        cmake -S "$src" -B "$bdir" $(common_cmake_args) "$@"
    cmake --build "$bdir" --parallel "$(cpu_count)"
    cmake --install "$bdir"
}

copy_licenses() {
    local name="$1" src="$2"
    local dest="$PREFIX/share/licenses/$name"
    mkdir -p "$dest"
    local f
    for f in "$src"/COPYING "$src"/LICENSE "$src"/PATENTS "$src"/AUTHORS; do
        if [ -f "$f" ]; then
            cp "$f" "$dest/"
        fi
    done
}

build_libde265() {
    local fingerprint="libde265 $LIBDE265_VERSION $LIBDE265_SHA256 decoder-only sdl=off"
    if is_current libde265 "$fingerprint"; then
        echo "libde265 $LIBDE265_VERSION: up to date"
        return 0
    fi
    fetch_verified libde265 "$LIBDE265_URL" "$LIBDE265_SHA256"
    local src
    src="$(unpack_fresh libde265 "$SRC_DIR/libde265/$(basename "$LIBDE265_URL")")"
    cmake_build_install libde265 "$src" \
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
    local fingerprint="libaom $LIBAOM_VERSION $LIBAOM_SHA256 decoder-only examples=off tests=off tools=off docs=off"
    if is_current libaom "$fingerprint"; then
        echo "libaom $LIBAOM_VERSION: up to date"
        return 0
    fi
    fetch_verified libaom "$LIBAOM_URL" "$LIBAOM_SHA256"
    local src
    src="$(unpack_fresh libaom "$SRC_DIR/libaom/$(basename "$LIBAOM_URL")")"
    cmake_build_install libaom "$src" \
        -DCONFIG_AV1_ENCODER=0 \
        -DCONFIG_AV1_DECODER=1 \
        -DENABLE_EXAMPLES=OFF \
        -DENABLE_TESTS=OFF \
        -DENABLE_TOOLS=OFF \
        -DENABLE_DOCS=OFF \
        -DENABLE_APPS=OFF \
        -DENABLE_CCACHE=OFF
    copy_licenses libaom "$src"
    write_stamp libaom "$fingerprint"
}

build_libheif() {
    local fingerprint="libheif $LIBHEIF_VERSION $LIBHEIF_SHA256 static decoder=libde265+aom no-plugins all-other-codecs-off"
    if is_current libheif "$fingerprint"; then
        echo "libheif $LIBHEIF_VERSION: up to date"
        return 0
    fi
    fetch_verified libheif "$LIBHEIF_URL" "$LIBHEIF_SHA256"
    local src
    src="$(unpack_fresh libheif "$SRC_DIR/libheif/$(basename "$LIBHEIF_URL")")"
    cmake_build_install libheif "$src" \
        -DENABLE_PLUGIN_LOADING=OFF \
        -DWITH_LIBDE265=ON \
        -DWITH_LIBDE265_PLUGIN=OFF \
        -DWITH_AOM_DECODER=ON \
        -DWITH_AOM_DECODER_PLUGIN=OFF \
        -DWITH_AOM_ENCODER=OFF \
        -DWITH_AOM_ENCODER_PLUGIN=OFF \
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

# Fails the build if an encoder symbol leaked into a decode-only archive.
check_decode_only() {
    local lib
    for lib in libde265.a libaom.a libheif.a; do
        [ -f "$PREFIX/lib/$lib" ] || die "missing $PREFIX/lib/$lib after build"
    done
    if nm "$PREFIX/lib/libaom.a" 2>/dev/null | grep -q 'aom_codec_av1_cx'; then
        die "libaom.a contains AV1 encoder symbols; CONFIG_AV1_ENCODER=0 was not honored"
    fi
}

build_libde265
build_libaom
build_libheif
check_decode_only

echo "native codecs ready: $PREFIX"
echo "use with: export PKG_CONFIG_LIBDIR=\"$PREFIX/lib/pkgconfig\" SYSTEM_DEPS_LINK=static"
