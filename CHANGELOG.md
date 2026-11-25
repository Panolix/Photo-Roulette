# Changelog

## 0.4.0 — 2026-11-25

- Folder scanning now runs in the background with a live file count and a cancel button.
- Volume slider, `M` mute shortcut, and persisted volume.
- Back navigation: the `‹ BACK` button or `Backspace` returns to the previous item.
- Settings are remembered across launches: last folder, volume, filename overlay, and window geometry.
- Video is capped at 1920 px and small videos are no longer upscaled.
- Audio is extracted as compact AAC/M4A with a WAV fallback, so long videos no longer create giant temp files.
- A setup dialog can locate ffmpeg, remembers its path, and explains platform-specific install commands.
- HEIC/AVIF photos can be decoded through ffmpeg on Windows and Linux.
- Optional ffmpeg bundling hooks for all three platform packages.
- `--help` and `--version` CLI flags.
- Trimmed unused image features, removing an AVIF encoder dependency chain from the build.
- 17 tests, including a generated-video pipeline test, scan cancellation/progress, history, and settings.
- CI now runs `cargo deny`, a CLI smoke test, and package-content checks; releases publish SHA-256 checksums.
- Restart control redrawn with a clearer circular-arrow icon; pointer cursor over the player; app icon glow contained.
- Typography and spacing pass: smaller wordmark, consistent label sizes, compact volume control, and equal breathing room above and below the canvas.
- Removed the full-width panel separator lines so the rounded canvas border is the only divider.

## 0.3.0 — 2026-11-25

- Cross-platform video audio using `rodio` (macOS, Windows, Linux).
- ffmpeg remains the video decoder and is still installed separately.
- Seeking and in-place buffering keep the current video frame visible instead of reloading the stage.
- Play/pause icon centered and reduced in size; draggable timeline added.
- Restart control redrawn with a clear circular-arrow icon and no competing outer ring.
- App icon cleaned up so no glow bleeds outside the rounded square.

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
