#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/../.."

VERSION="9.0"
URL="https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-n${VERSION}-latest-linux64-lgpl-shared-${VERSION}.tar.xz"
SHA256="da868ed6eae906e5063ab7def1c5ea5f0c7a9118f8dcddc8190e24f830b4c1db"
DEST="${1:-packaging/linux/ffmpeg}"

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

curl -fsSL "$URL" -o "$TMP/ffmpeg.tar.xz"
if command -v sha256sum >/dev/null 2>&1; then
    printf '%s  %s\n' "$SHA256" "$TMP/ffmpeg.tar.xz" | sha256sum -c -
else
    printf '%s  %s\n' "$SHA256" "$TMP/ffmpeg.tar.xz" | shasum -a 256 -c -
fi
tar -xJf "$TMP/ffmpeg.tar.xz" -C "$TMP"

ROOT="$(find "$TMP" -maxdepth 1 -type d -name 'ffmpeg-*' | head -n 1)"
if [ -z "$ROOT" ] || [ ! -x "$ROOT/bin/ffmpeg" ]; then
    echo "Could not find the extracted ffmpeg build." >&2
    exit 1
fi

rm -rf "$DEST"
mkdir -p "$DEST/bin" "$DEST/lib"
cp -a "$ROOT/bin/ffmpeg" "$ROOT/bin/ffprobe" "$DEST/bin/"
cp -a "$ROOT/lib/." "$DEST/lib/"
cp "$ROOT/LICENSE.txt" "$DEST/LICENSE.txt"
cat > "$DEST/SOURCE.txt" <<EOF
FFmpeg $VERSION (LGPL shared build)
Built by BtbN/FFmpeg-Builds: https://github.com/BtbN/FFmpeg-Builds
Source: https://ffmpeg.org/releases/ffmpeg-$VERSION.tar.xz
Download: $URL
SHA-256: $SHA256

The binary is redistributed unmodified. See LICENSE.txt (LGPL) and
https://ffmpeg.org/ for the corresponding source.
EOF

if [ "$(uname -s)" = "Linux" ]; then
    LD_LIBRARY_PATH="$DEST/lib" "$DEST/bin/ffmpeg" -version | head -n 1
else
    echo "Prepared $DEST (run the binary check on Linux)."
fi
