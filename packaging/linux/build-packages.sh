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
Depends: libc6, libgl1, libx11-6, libxkbcommon0, libwayland-client0, libegl1, libasound2
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
tar -czf "$TARBALL" -C "$(dirname "$PORTABLE")" "$(basename "$PORTABLE")"

echo "Built:"
echo "  dist/photo-roulette_${VERSION}_${DEB_ARCH}.deb"
echo "  $TARBALL"
