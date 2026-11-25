# Bundling ffmpeg (optional)

Photo Roulette works without ffmpeg for photos. Videos need an `ffmpeg` binary;
the app looks for one next to its own executable, in common install locations,
and on `PATH`.

You can bundle a copy with a release so users do not have to install it.

## Where to put the binaries

- **macOS:** set `FFMPEG_BIN_DIR=/path/to/bin` when running `make-app.sh` or
  `packaging/macos/make-dmg.sh`. `ffmpeg`, `ffprobe`, and an optional `LICENSE`
  are copied into `Photo Roulette.app/Contents/MacOS/`.
- **Windows:** put `ffmpeg.exe`, `ffprobe.exe`, and an optional `LICENSE` in
  `packaging/windows/ffmpeg/` before building the Inno Setup installer.
- **Linux:** set `FFMPEG_BIN_DIR=/path/to/bin` when running
  `packaging/linux/build-packages.sh`. The binaries are bundled in the portable
  tarball next to the executable, and under `/usr/lib/photo-roulette/` in the
  `.deb` (the app searches there).

## Licensing

Only bundle a build whose license you can comply with. FFmpeg is usually
distributed under LGPL-2.1-or-later, but many prebuilt "full" builds are GPL
because they include GPL-only components such as x264. A GPL build cannot be
slipped into a non-commercial package without meeting the GPL's source and
relinking requirements.

For a redistributable app, use an LGPL build (for example BtbN's `lgpl` or
`lgpl-shared` Windows/Linux artifacts, or build ffmpeg yourself with
`--disable-gpl --disable-nonfree`), include its license text, and keep the
corresponding source available. Native H.264/HEVC/AAC decoding is available in
an LGPL build; you do not need x264 just to play videos.
