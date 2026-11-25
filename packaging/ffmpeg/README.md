# Bundling FFmpeg

Release packages bundle an LGPL FFmpeg so videos play without a separate
install. Source builds do not include it automatically.

## How the release build works

- **macOS:** `build-macos.sh` downloads FFmpeg 9.0, builds arm64 and x86_64
  static binaries with `--disable-gpl --disable-nonfree`, lipos them, and writes
  them to `packaging/ffmpeg/bin/`. `make-app.sh` copies them into
  `Photo Roulette.app/Contents/MacOS/`.
- **Windows:** `fetch-windows.ps1` downloads a pinned BtbN LGPL shared build,
  verifies its SHA-256, and copies `ffmpeg.exe`, `ffprobe.exe`, the DLLs, and the
  license into `packaging/windows/ffmpeg/`. The Inno Setup installer includes
  them next to the app.
- **Linux:** `fetch-linux.sh` downloads a pinned BtbN LGPL shared build, verifies
  its SHA-256, and prepares `packaging/linux/ffmpeg/`. The `.deb` installs the
  binaries and shared libraries under `/usr/lib/photo-roulette/`; the portable
  tarball keeps them in a `ffmpeg/` subfolder.
- The app searches next to its executable, in a `ffmpeg/` subfolder, in
  `/usr/lib/photo-roulette/`, and on `PATH`, so bundled and external copies both
  work.

## Licensing

Only bundle a build whose license you can comply with. These scripts use LGPL
builds:

- macOS: FFmpeg source, built with `--disable-gpl --disable-nonfree`.
- Windows/Linux: BtbN's `lgpl`/`lgpl-shared` artifacts.

Do **not** use a GPL build for the default release. GPL builds (for example ones
with x264/x265) require GPL compliance and are not compatible with shipping the
app under its current non-commercial terms.

Every package includes the FFmpeg license and a `SOURCE.txt` with the exact
source and build location. If you change the FFmpeg version or download URL,
update the checksum in the fetch script.

## Manual use

```sh
# macOS universal static build
./packaging/ffmpeg/build-macos.sh
FFMPEG_BIN_DIR=packaging/ffmpeg/bin ./packaging/macos/make-dmg.sh

# Windows (from PowerShell)
./packaging/ffmpeg/fetch-windows.ps1
iscc packaging/windows/PhotoRoulette.iss

# Linux
./packaging/ffmpeg/fetch-linux.sh
FFMPEG_DIR=packaging/linux/ffmpeg ./packaging/linux/build-packages.sh
```
