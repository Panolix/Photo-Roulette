#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/../.."

VERSION="${VERSION:-$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)}"
DEB_ARCH="${DEB_ARCH:-amd64}"
BIN="target/release/photo-roulette"
DEB_ROOT="dist/deb/photo-roulette_${VERSION}_${DEB_ARCH}"
TARBALL="dist/photo-roulette-${VERSION}-linux-x86_64.tar.gz"
PORTABLE="dist/linux-portable/photo-roulette-${VERSION}"

echo "Building Photo Roulette ${VERSION} for Linux..."
cargo build --release --locked

rm -rf "$DEB_ROOT" "$PORTABLE"
mkdir -p "$DEB_ROOT/DEBIAN" \
         "$DEB_ROOT/usr/bin" \
         "$DEB_ROOT/usr/share/applications" \
         "$DEB_ROOT/usr/share/icons/hicolor/256x256/apps" \
         "$DEB_ROOT/usr/share/doc/photo-roulette"

install -m 755 "$BIN" "$DEB_ROOT/usr/bin/photo-roulette"

# Optional bundled ffmpeg. FFMPEG_DIR is a BtbN-style build with bin/ and lib/;
# FFMPEG_BIN_DIR is a flat static build. See packaging/ffmpeg/README.md.
if [ -n "${FFMPEG_DIR:-}" ]; then
    mkdir -p "$DEB_ROOT/usr/lib/photo-roulette"
    install -m 755 "$FFMPEG_DIR/bin/ffmpeg" \
        "$DEB_ROOT/usr/lib/photo-roulette/ffmpeg"
    install -m 755 "$FFMPEG_DIR/bin/ffprobe" \
        "$DEB_ROOT/usr/lib/photo-roulette/ffprobe"
    if [ -d "$FFMPEG_DIR/lib" ]; then
        cp -a "$FFMPEG_DIR/lib/." "$DEB_ROOT/usr/lib/photo-roulette/"
    fi
    if [ -f "$FFMPEG_DIR/LICENSE.txt" ]; then
        install -m 644 "$FFMPEG_DIR/LICENSE.txt" \
            "$DEB_ROOT/usr/share/doc/photo-roulette/FFMPEG-LICENSE.txt"
    fi
    if [ -f "$FFMPEG_DIR/SOURCE.txt" ]; then
        install -m 644 "$FFMPEG_DIR/SOURCE.txt" \
            "$DEB_ROOT/usr/share/doc/photo-roulette/FFMPEG-SOURCE.txt"
    fi
elif [ -n "${FFMPEG_BIN_DIR:-}" ]; then
    mkdir -p "$DEB_ROOT/usr/lib/photo-roulette"
    for binary in ffmpeg ffprobe; do
        if [ -f "$FFMPEG_BIN_DIR/$binary" ]; then
            install -m 755 "$FFMPEG_BIN_DIR/$binary" \
                "$DEB_ROOT/usr/lib/photo-roulette/$binary"
        fi
    done
    if [ -f "$FFMPEG_BIN_DIR/LICENSE" ]; then
        install -m 644 "$FFMPEG_BIN_DIR/LICENSE" \
            "$DEB_ROOT/usr/share/doc/photo-roulette/FFMPEG-LICENSE.txt"
    fi
    if [ -f "$FFMPEG_BIN_DIR/SOURCE.txt" ]; then
        install -m 644 "$FFMPEG_BIN_DIR/SOURCE.txt" \
            "$DEB_ROOT/usr/share/doc/photo-roulette/FFMPEG-SOURCE.txt"
    fi
fi
install -m 644 packaging/linux/photo-roulette.desktop \
  "$DEB_ROOT/usr/share/applications/photo-roulette.desktop"
install -m 644 assets/PhotoRoulette-256.png \
  "$DEB_ROOT/usr/share/icons/hicolor/256x256/apps/photo-roulette.png"
install -m 644 README.md LICENSE COMMERCIAL.md NOTICE CLA.md \
  "$DEB_ROOT/usr/share/doc/photo-roulette/"
install -m 644 assets/fonts/OFL-ArchivoBlack.txt assets/fonts/OFL-SpaceGrotesk.txt \
  assets/icons/LICENSE-bootstrap-icons.txt \
  "$DEB_ROOT/usr/share/doc/photo-roulette/"

cat > "$DEB_ROOT/DEBIAN/control" <<EOF
Package: photo-roulette
Version: ${VERSION}
Section: graphics
Priority: optional
Architecture: ${DEB_ARCH}
Maintainer: Panagiotis Smponias <info@cityframe-photography.com>
Homepage: https://github.com/Panolix/Photo-Roulette
Depends: libc6, libgcc-s1, libgl1, libx11-6, libxkbcommon0, libwayland-client0, libegl1, libasound2
Recommends: ffmpeg
Description: A tiny photo and video roulette for group cringe sessions
 Import a folder of photos and videos, then deal one random item at a time.
 Every item is shown once per round. Video playback needs ffmpeg; audio is
 handled by rodio on macOS, Windows and Linux.
EOF

chmod 755 "$DEB_ROOT/usr/bin/photo-roulette"
dpkg-deb --build --root-owner-group "$DEB_ROOT" \
  "dist/photo-roulette_${VERSION}_${DEB_ARCH}.deb" >/dev/null

mkdir -p "$PORTABLE"
cp "$BIN" "$PORTABLE/photo-roulette"
cp README.md LICENSE COMMERCIAL.md NOTICE CLA.md "$PORTABLE/"
cp packaging/linux/photo-roulette.desktop "$PORTABLE/"
cp assets/PhotoRoulette-256.png "$PORTABLE/photo-roulette.png"
cp assets/fonts/OFL-ArchivoBlack.txt assets/fonts/OFL-SpaceGrotesk.txt \
  assets/icons/LICENSE-bootstrap-icons.txt "$PORTABLE/"
if [ -n "${FFMPEG_DIR:-}" ]; then
    mkdir -p "$PORTABLE/ffmpeg"
    install -m 755 "$FFMPEG_DIR/bin/ffmpeg" "$PORTABLE/ffmpeg/ffmpeg"
    install -m 755 "$FFMPEG_DIR/bin/ffprobe" "$PORTABLE/ffmpeg/ffprobe"
    if [ -d "$FFMPEG_DIR/lib" ]; then
        cp -a "$FFMPEG_DIR/lib/." "$PORTABLE/ffmpeg/"
    fi
    if [ -f "$FFMPEG_DIR/LICENSE.txt" ]; then
        cp "$FFMPEG_DIR/LICENSE.txt" "$PORTABLE/ffmpeg/FFMPEG-LICENSE.txt"
    fi
    if [ -f "$FFMPEG_DIR/SOURCE.txt" ]; then
        cp "$FFMPEG_DIR/SOURCE.txt" "$PORTABLE/ffmpeg/FFMPEG-SOURCE.txt"
    fi
elif [ -n "${FFMPEG_BIN_DIR:-}" ]; then
    for binary in ffmpeg ffprobe; do
        if [ -f "$FFMPEG_BIN_DIR/$binary" ]; then
            install -m 755 "$FFMPEG_BIN_DIR/$binary" "$PORTABLE/$binary"
        fi
    done
    if [ -f "$FFMPEG_BIN_DIR/LICENSE" ]; then
        cp "$FFMPEG_BIN_DIR/LICENSE" "$PORTABLE/FFMPEG-LICENSE.txt"
    fi
    if [ -f "$FFMPEG_BIN_DIR/SOURCE.txt" ]; then
        cp "$FFMPEG_BIN_DIR/SOURCE.txt" "$PORTABLE/FFMPEG-SOURCE.txt"
    fi
fi
tar -czf "$TARBALL" -C "$(dirname "$PORTABLE")" "$(basename "$PORTABLE")"

echo "Built:"
echo "  dist/photo-roulette_${VERSION}_${DEB_ARCH}.deb"
echo "  $TARBALL"
