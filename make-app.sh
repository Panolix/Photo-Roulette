#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")"

APP_NAME="Photo Roulette"
APP="dist/${APP_NAME}.app"
BUNDLE_ID="com.panolix.photoroulette"
VERSION="${VERSION:-$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)}"

echo "Building Photo Roulette ${VERSION} (release)..."
if [ "${UNIVERSAL:-0}" = "1" ]; then
    rustup target add aarch64-apple-darwin x86_64-apple-darwin >/dev/null
    cargo build --release --locked --target aarch64-apple-darwin
    cargo build --release --locked --target x86_64-apple-darwin
    lipo -create \
        target/aarch64-apple-darwin/release/photo-roulette \
        target/x86_64-apple-darwin/release/photo-roulette \
        -output target/release/photo-roulette
else
    cargo build --release --locked
fi

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

cp target/release/photo-roulette "$APP/Contents/MacOS/photo-roulette"
cp assets/PhotoRoulette.icns "$APP/Contents/Resources/PhotoRoulette.icns"
mkdir -p "$APP/Contents/Resources/licenses"
cp assets/fonts/OFL-ArchivoBlack.txt "$APP/Contents/Resources/licenses/"
cp assets/fonts/OFL-SpaceGrotesk.txt "$APP/Contents/Resources/licenses/"
cp assets/icons/LICENSE-bootstrap-icons.txt "$APP/Contents/Resources/licenses/"
cp LICENSE "$APP/Contents/Resources/licenses/LICENSE.txt"
cp COMMERCIAL.md "$APP/Contents/Resources/licenses/COMMERCIAL.md"
cp NOTICE "$APP/Contents/Resources/licenses/NOTICE.md"
cp CLA.md "$APP/Contents/Resources/licenses/CLA.md"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleDevelopmentRegion</key>
    <string>en</string>
    <key>CFBundleDisplayName</key>
    <string>${APP_NAME}</string>
    <key>CFBundleExecutable</key>
    <string>photo-roulette</string>
    <key>CFBundleIconFile</key>
    <string>PhotoRoulette</string>
    <key>CFBundleIdentifier</key>
    <string>${BUNDLE_ID}</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>CFBundleName</key>
    <string>${APP_NAME}</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>${VERSION}</string>
    <key>CFBundleVersion</key>
    <string>1</string>
    <key>LSApplicationCategoryType</key>
    <string>public.app-category.entertainment</string>
    <key>LSMinimumSystemVersion</key>
    <string>11.0</string>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>NSHumanReadableCopyright</key>
    <string>Copyright © 2026 Panagiotis Smponias. Non-commercial use under CC BY-NC-SA 4.0.</string>
</dict>
</plist>
PLIST

# Ad-hoc signing keeps Gatekeeper a little happier with a locally built app.
codesign --force --deep --sign - "$APP" >/dev/null 2>&1 || true

echo
if ! [ -x /opt/homebrew/bin/ffmpeg ] && ! [ -x /usr/local/bin/ffmpeg ] && ! command -v ffmpeg >/dev/null 2>&1; then
    echo "Note: ffmpeg was not found. Photos will work; videos will be skipped."
    echo "      Install it with: brew install ffmpeg"
    echo
fi
echo "Built: $APP"
echo "Open it with: open \"$APP\""
echo "Tip: drag it into /Applications to keep it around."
