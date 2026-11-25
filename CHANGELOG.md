# Changelog

## 0.3.0 — 2026-11-25

- Cross-platform video audio using `rodio` (macOS, Windows, Linux).
- ffmpeg remains the video decoder and is still installed separately.
- Seeking and in-place buffering keep the current video frame visible instead of reloading the stage.
- Play/pause icon centered and reduced in size; draggable timeline added.

## 0.2.0 — 2026-11-25

The first public release.

- Import photos and videos from a folder, including subfolders.
- Random shuffle bag: every item is shown once per round, with fresh rounds afterwards.
- Photo support: JPEG, PNG, GIF, WebP, BMP, TIFF, QOI, TGA, HDR, EXR, and HEIC/AVIF on macOS.
- Video playback through ffmpeg: MP4, MOV, M4V, WMV, AVI, MKV, WebM, and more.
- Pause/resume by clicking the video, draggable timeline seeking, restart control, and `←`/`→` seeking.
- EXIF rotation, large-image downscaling, transparency, filename overlay, drag-and-drop, and fullscreen.
- Native macOS audio playback with `AVAudioPlayer`; Windows/Linux video frames play silently for now.
- macOS DMG, Windows Inno Setup installer, Linux `.deb` and portable tarball packaging.
- CI and tagged-release workflows for macOS, Windows, and Linux.
