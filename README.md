# Photo Roulette

<p align="center">
  <img src="assets/PhotoRoulette-256.png?v=0.3.0" width="128" alt="Photo Roulette icon">
</p>

[![Platform](https://img.shields.io/badge/Platform-macOS%20%7C%20Windows%20%7C%20Linux-blue)](https://github.com/Panolix/Photo-Roulette/releases)
[![License: CC BY-NC-SA 4.0](https://img.shields.io/badge/License-CC_BY--NC--SA_4.0-blue.svg)](https://creativecommons.org/licenses/by-nc-sa/4.0/)
[![Built with Rust + egui](https://img.shields.io/badge/Built%20with-Rust%20%2B%20egui-orange)](https://github.com/emilk/egui)
[![CI](https://github.com/Panolix/Photo-Roulette/actions/workflows/ci.yml/badge.svg)](https://github.com/Panolix/Photo-Roulette/actions/workflows/ci.yml)

A tiny desktop app for the group chat's worst photos — and videos.

Point it at a folder of high-school evidence, hit **Deal the first one**, and it
shows one random photo or video at a time. Every item gets dealt exactly once
per round, so nobody has to see the same cursed memory twice.

Everything stays local. No account, cloud service, upload, or tracking.

## Features

- Imports a folder of photos and videos, including subfolders.
- Deals one random item at a time with a short shuffling animation.
- Uses a shuffle bag: no repeats until every item in the round has been shown.
- Starts a new round with a fresh shuffle when the deck runs out.
- Avoids opening a new round with the item that just closed the previous one.
- Photos and video frames are decoded in the background, so the UI stays responsive.
- Click a video to pause/resume; drag the timeline to seek; restart without advancing the round.
- Fullscreen mode for group viewing.
- Optional filename overlay.
- Drag a folder (or a pile of files) straight onto the window.
- Handles EXIF rotation, large images, transparency, HEIC/AVIF (macOS), and many video formats.
- No repeats are based on file paths; make sure your folder doesn't contain duplicate copies.

## Download

Grab a build from the [latest release](https://github.com/Panolix/Photo-Roulette/releases/latest).

| OS | Artifact | Notes |
| --- | --- | --- |
| macOS (Apple Silicon + Intel) | `Photo-Roulette-*-macOS-universal.dmg` | Drag **Photo Roulette** into Applications. Ad-hoc signed, not notarized. |
| Windows x64 | `Photo-Roulette-Setup-*.exe` | Inno Setup installer. |
| Linux x86_64 | `photo-roulette_*_amd64.deb` or `photo-roulette-*-linux-x86_64.tar.gz` | `sudo apt install ./photo-roulette_*_amd64.deb` |

The macOS app is ad-hoc signed because it is built locally without an Apple
Developer ID. On a machine other than the build machine, Gatekeeper may require
right-click → **Open** the first time.

## Video support

Video playback requires [ffmpeg](https://ffmpeg.org/) to be installed:

```sh
# macOS
brew install ffmpeg

# Windows (PowerShell)
winget install Gyan.FFmpeg

# Ubuntu / Debian
sudo apt install ffmpeg
```

The app looks for ffmpeg next to the executable, in common install locations,
and in `PATH`. It supports the formats ffmpeg can decode, including **MP4, MOV,
M4V, WMV, AVI, MKV, and WebM**. Videos are decoded to a maximum of 1280 px at
30 fps.

Video playback, including sound, works on macOS, Windows, and Linux when
ffmpeg is available. If ffmpeg is missing, videos are skipped and the app stays
in photo-only mode.

## Controls

| Action | How |
| --- | --- |
| Import / change folder | Button in the top right, or drop a folder on the window |
| Deal the next item | Big button, or press `Space` |
| Pause / play video | Click the video, or press `P` |
| Seek video | Drag the timeline, or use `←` / `→` |
| Restart video | Circular arrow on the video, or press `R` |
| Fullscreen | `F`, or the fullscreen button |
| Open the folder picker | `Cmd+O` / `Ctrl+O` |

## Build from source

You need a recent stable Rust toolchain: <https://rustup.rs>

```sh
git clone https://github.com/Panolix/Photo-Roulette.git
cd Photo-Roulette
cargo run --release
```

Or start with a folder already loaded:

```sh
cargo run --release -- ~/Pictures/high-school
```

Platform build requirements:

- **macOS:** Xcode Command Line Tools.
- **Windows:** Microsoft C++ Build Tools.
- **Linux:** the usual OpenGL/X11/Wayland development packages used by
  `eframe`/`winit`, plus `pkg-config` and `libasound2-dev` for audio.

The release build is recommended; debug builds decode photos and video at debug
speed.

## Installers and packaging

```sh
# macOS .app
./make-app.sh

# macOS .dmg
./packaging/macos/make-dmg.sh

# Windows installer (run on Windows with Inno Setup installed)
iscc packaging/windows/PhotoRoulette.iss /DMyAppVersion=0.2.0

# Linux .deb and portable tarball (run on Linux)
VERSION=0.2.0 ./packaging/linux/build-packages.sh
```

GitHub Actions builds the macOS DMG, Windows installer, Linux `.deb`, and Linux
tarball for each tagged release. Tag a release with:

```sh
git tag v0.2.0
git push origin v0.2.0
```

The workflows live in `.github/workflows/ci.yml` and `.github/workflows/release.yml`.

## Media formats

Photos: JPG, PNG, GIF, WebP, BMP, TIFF, QOI, TGA, HDR, EXR, and more. On macOS,
HEIC/HEIF and AVIF are supported through the system's ImageIO decoder (`sips`).

Videos: MP4, MOV, M4V, WMV, AVI, MKV, WebM, FLV, MPEG, 3GP, and other formats
ffmpeg can decode.

Unreadable files are skipped automatically. Folders are scanned recursively;
hidden folders and macOS photo libraries/app bundles are ignored.

## How the "no repeats" part works

When a folder is imported, the app builds a shuffled queue (a shuffle bag). Each
deal pops one item off the queue. Once the queue is empty, the round is complete;
pressing the button again builds a new shuffled queue from all media. Within a
round there can be no duplicate file paths, and a single-item library is the
only unavoidable repeat.

Visually identical copies saved as different files are still separate items.
Remove duplicates before the party if you don't want them to appear twice.

## Known limitations

- macOS app builds are ad-hoc signed and not notarized.
- Video controls support play/pause, seek, and restart, but not volume, speed,
  or subtitle controls.
- No RAW/PSD support.
- The app does not modify or delete anything in the imported folder.
- The macOS app does not bundle ffmpeg; install it separately for video.

## Credits

- **Archivo Black** — Omnibus-Type, SIL Open Font License
- **Space Grotesk** — Florian Karsten, SIL Open Font License
- Video decoding via [ffmpeg](https://ffmpeg.org/); not bundled with the app.
- Built with [egui](https://github.com/emilk/egui) / eframe.

License files for the fonts are in `assets/fonts/`.

## License

This project is licensed under [CC BY-NC-SA 4.0](https://creativecommons.org/licenses/by-nc-sa/4.0/).

**License text:** [human-readable summary](https://creativecommons.org/licenses/by-nc-sa/4.0/) · [full legal code](https://creativecommons.org/licenses/by-nc-sa/4.0/legalcode.en) · [SPDX entry](https://spdx.org/licenses/CC-BY-NC-SA-4.0.html) · [LICENSE](LICENSE)

**Source-available, not open source.** You may download, use, fork, modify and
share it for non-commercial purposes, provided you credit **Panagiotis
Smponias**, link back to this repository, state what you changed, and license
your version under the same terms.

**Commercial use requires a paid license** — see [COMMERCIAL.md](COMMERCIAL.md).
Attribution, share-alike and trademark terms: [NOTICE](NOTICE).

## Author

**Panagiotis Smponias**

- Email: info@cityframe-photography.com
- GitHub: [@Panolix](https://github.com/Panolix)
- Website: [City Frame Photography](https://cityframe-photography.com)

## Acknowledgments

- The group chat, for the evidence.
- [ffmpeg](https://ffmpeg.org/) for decoding video.
- [egui](https://github.com/emilk/egui) and the Rust ecosystem.
