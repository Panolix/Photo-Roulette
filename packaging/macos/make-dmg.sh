#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/../.."

VERSION="${VERSION:-$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)}"
if [ "${UNIVERSAL:-0}" = "1" ]; then
    ARCH="universal"
else
    ARCH="$(uname -m)"
fi
APP="dist/Photo Roulette.app"
STAGE="dist/dmg-stage"
DMG="dist/Photo-Roulette-${VERSION}-macOS-${ARCH}.dmg"

./make-app.sh

rm -rf "$STAGE"
mkdir -p "$STAGE"
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"

rm -f "$DMG"
hdiutil create \
  -volname "Photo Roulette" \
  -srcfolder "$STAGE" \
  -ov \
  -format UDZO \
  "$DMG" >/dev/null

rm -rf "$STAGE"

echo "Built: $DMG"
