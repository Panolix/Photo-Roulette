#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/../.."

VERSION="${FFMPEG_VERSION:-9.0}"
SOURCE_SHA256="${SOURCE_SHA256:-7f607a00dd0d28a729d5a4811205812eef01cf6ef6155025febb6f36a9062d52}"
OUT="${FFMPEG_OUT:-packaging/ffmpeg/bin}"
WORK="${FFMPEG_BUILD_DIR:-build/ffmpeg}"
JOBS="${JOBS:-$(sysctl -n hw.ncpu 2>/dev/null || echo 4)}"
MIN_MACOS="${MIN_MACOS:-11.0}"

mkdir -p "$OUT" "$WORK"

SRC="$WORK/ffmpeg-$VERSION"
if [ ! -d "$SRC" ]; then
    ARCHIVE="$WORK/ffmpeg-$VERSION.tar.xz"
    if [ ! -f "$ARCHIVE" ]; then
        curl -fsSL "https://ffmpeg.org/releases/ffmpeg-$VERSION.tar.xz" -o "$ARCHIVE"
    fi
    printf '%s  %s\n' "$SOURCE_SHA256" "$ARCHIVE" | shasum -a 256 -c -
    tar -xJf "$ARCHIVE" -C "$WORK"
fi
SRC="$(cd "$WORK" && pwd)/ffmpeg-$VERSION"

build_arch() {
    local arch="$1"
    local build="$WORK/build-$arch"
    local flags="-arch $arch -mmacosx-version-min=$MIN_MACOS -O2"

    mkdir -p "$build"
    build="$(cd "$build" && pwd)"
    (
        cd "$build"
        "$SRC/configure" \
            --prefix="$build/out" \
            --target-os=darwin \
            --arch="$arch" \
            --cc=clang \
            --host-cc=clang \
            --enable-cross-compile \
            --extra-cflags="$flags" \
            --extra-ldflags="$flags" \
            --disable-gpl \
            --disable-nonfree \
            --disable-doc \
            --disable-debug \
            --disable-network \
            --disable-autodetect \
            --disable-ffplay \
            --enable-static \
            --disable-shared
        make -j"$JOBS"
        make install
    )
}

build_arch arm64
build_arch x86_64

for tool in ffmpeg ffprobe; do
    strip -x "$WORK/build-arm64/out/bin/$tool"
    strip -x "$WORK/build-x86_64/out/bin/$tool"
    lipo -create \
        "$WORK/build-arm64/out/bin/$tool" \
        "$WORK/build-x86_64/out/bin/$tool" \
        -output "$OUT/$tool"
    chmod +x "$OUT/$tool"
done

cp "$SRC/COPYING.LGPLv2.1" "$OUT/LICENSE"
cat > "$OUT/SOURCE.txt" <<EOF
FFmpeg $VERSION
Source: https://ffmpeg.org/releases/ffmpeg-$VERSION.tar.xz
Build script: https://github.com/Panolix/Photo-Roulette/blob/main/packaging/ffmpeg/build-macos.sh
License: LGPL-2.1-or-later (see LICENSE)

This is an unmodified LGPL build of FFmpeg with --disable-gpl and
--disable-nonfree. The universal binary is built for macOS $MIN_MACOS and
later with the script above.
EOF

"$OUT/ffmpeg" -version | head -n 1
lipo -info "$OUT/ffmpeg"
