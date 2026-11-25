//! Video preparation and frame streaming.
//!
//! Frames are decoded by the system `ffmpeg` binary and streamed as RGBA
//! images into egui. Audio is extracted to a temporary WAV and played with
//! `rodio`, giving clean pause/resume/seek behavior on macOS, Windows and Linux.
//!
//! This supports the formats ffmpeg can decode (MOV, MP4, WMV, AVI, MKV, WebM, …).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::thread;

use eframe::egui::ColorImage;
use image::ImageReader;
use rodio::{Decoder, DeviceSinkBuilder, MixerDeviceSink, Player};

const FPS: u32 = 30;
const MAX_DIMENSION: u32 = 1920;
const FRAME_CHANNEL_CAPACITY: usize = 6;

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);
static CUSTOM_FFMPEG: Mutex<Option<PathBuf>> = Mutex::new(None);

#[derive(Clone, Debug)]
pub struct VideoInfo {
    pub duration: Option<f64>,
    pub audio_path: Option<PathBuf>,
}

pub enum VideoMessage {
    Ready(VideoInfo),
    Frame(ColorImage),
    End,
    Error(String),
}

pub struct VideoLoad {
    pub rx: Receiver<VideoMessage>,
}

pub struct AudioPlayer {
    // Keep the output stream alive for as long as the player exists.
    _stream: MixerDeviceSink,
    player: Player,
}

impl AudioPlayer {
    pub fn pause(&self) {
        self.player.pause();
    }

    pub fn resume(&self) {
        self.player.play();
    }

    pub fn set_volume(&self, volume: f32) {
        self.player.set_volume(volume.clamp(0.0, 1.0));
    }
}

impl Drop for AudioPlayer {
    fn drop(&mut self) {
        self.player.stop();
    }
}

/// Start a background job that prepares a video and streams decoded frames.
pub fn start(path: PathBuf, start_offset: f64) -> Result<VideoLoad, String> {
    let (tx, rx) = sync_channel(FRAME_CHANNEL_CAPACITY);
    thread::Builder::new()
        .name("video-decode".to_owned())
        .spawn(move || run(path, start_offset, tx))
        .map_err(|error| format!("could not start video decoder: {error}"))?;
    Ok(VideoLoad { rx })
}

pub fn is_available() -> bool {
    find_ffmpeg().is_some()
}

fn run(path: PathBuf, start_offset: f64, tx: SyncSender<VideoMessage>) {
    let Some(ffmpeg) = ffmpeg() else {
        let _ = tx.send(VideoMessage::Error(format!(
            "Video playback needs ffmpeg. {}",
            install_hint()
        )));
        return;
    };

    let (width, height, poster) = match extract_poster(&ffmpeg, &path, start_offset) {
        Ok(value) => value,
        Err(error) => {
            let _ = tx.send(VideoMessage::Error(error));
            return;
        }
    };

    let audio_path = extract_audio(&ffmpeg, &path, start_offset);
    let duration = probe_duration(&path);

    let info = VideoInfo {
        duration,
        audio_path: audio_path.clone(),
    };

    let mut frame_child = match spawn_frame_decoder(&ffmpeg, &path, start_offset, width, height) {
        Ok(child) => child,
        Err(error) => {
            cleanup_audio(&audio_path);
            let _ = tx.send(VideoMessage::Error(error));
            return;
        }
    };

    if tx.send(VideoMessage::Ready(info)).is_err() {
        stop_child(&mut frame_child);
        cleanup_audio(&audio_path);
        return;
    }
    if tx.send(VideoMessage::Frame(poster)).is_err() {
        stop_child(&mut frame_child);
        cleanup_audio(&audio_path);
        return;
    }

    let Some(mut stdout) = frame_child.stdout.take() else {
        stop_child(&mut frame_child);
        cleanup_audio(&audio_path);
        let _ = tx.send(VideoMessage::Error(
            "Could not read frames from ffmpeg.".to_owned(),
        ));
        return;
    };

    let frame_size = width.saturating_mul(height).saturating_mul(4);
    if frame_size == 0 {
        stop_child(&mut frame_child);
        cleanup_audio(&audio_path);
        let _ = tx.send(VideoMessage::Error("Video has no video frames.".to_owned()));
        return;
    }

    let mut buffer = vec![0u8; frame_size];
    let mut first_raw_frame = true;
    let mut failed = None;

    loop {
        match read_frame(&mut stdout, &mut buffer) {
            Ok(true) => {
                // The poster is already frame zero; skip the duplicate raw frame.
                if first_raw_frame {
                    first_raw_frame = false;
                    continue;
                }

                let image = ColorImage::from_rgba_unmultiplied([width, height], &buffer);
                if tx.send(VideoMessage::Frame(image)).is_err() {
                    cleanup_audio(&audio_path);
                    stop_child(&mut frame_child);
                    return;
                }
            }
            Ok(false) => break,
            Err(error) => {
                failed = Some(error);
                break;
            }
        }
    }

    let _ = frame_child.wait();
    if let Some(error) = failed {
        let _ = tx.send(VideoMessage::Error(error));
    } else {
        let _ = tx.send(VideoMessage::End);
    }
}

fn spawn_frame_decoder(
    ffmpeg: &Path,
    path: &Path,
    start_offset: f64,
    width: usize,
    height: usize,
) -> Result<Child, String> {
    let filter = format!("scale={width}:{height}:flags=lanczos,fps={FPS}");
    let mut command = Command::new(ffmpeg);
    command.args(["-v", "error", "-nostdin"]);
    if start_offset > 0.0 {
        command.arg("-ss").arg(format!("{start_offset:.3}"));
    }
    command
        .arg("-i")
        .arg(path)
        .arg("-map")
        .arg("0:v:0")
        .arg("-vf")
        .arg(filter)
        .arg("-an")
        .arg("-f")
        .arg("rawvideo")
        .arg("-pix_fmt")
        .arg("rgba")
        .arg("-")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("ffmpeg could not start: {error}"))
}

fn extract_poster(
    ffmpeg: &Path,
    path: &Path,
    start_offset: f64,
) -> Result<(usize, usize, ColorImage), String> {
    let mut command = Command::new(ffmpeg);
    command.args(["-v", "error", "-nostdin"]);
    if start_offset > 0.0 {
        command.arg("-ss").arg(format!("{start_offset:.3}"));
    }
    let output = command
        .arg("-i")
        .arg(path)
        .arg("-map")
        .arg("0:v:0")
        .arg("-vf")
        .arg(format!(
            "scale=w='min({MAX_DIMENSION},iw)':h='min({MAX_DIMENSION},ih)':force_original_aspect_ratio=decrease:flags=lanczos"
        ))
        .args(["-frames:v", "1", "-f", "image2pipe", "-vcodec", "png", "-"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|error| format!("ffmpeg could not start: {error}"))?;

    if !output.status.success() {
        let message = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "ffmpeg couldn't read this video: {}",
            message.trim().lines().next().unwrap_or("unknown error")
        ));
    }

    let image = ImageReader::new(std::io::Cursor::new(output.stdout))
        .with_guessed_format()
        .map_err(|error| format!("video frame error: {error}"))?
        .decode()
        .map_err(|error| format!("video frame error: {error}"))?
        .into_rgba8();

    let width = image.width() as usize;
    let height = image.height() as usize;
    let image = ColorImage::from_rgba_unmultiplied([width, height], image.as_raw());
    Ok((width, height, image))
}

fn extract_audio(ffmpeg: &Path, path: &Path, start_offset: f64) -> Option<PathBuf> {
    // Prefer a small AAC/M4A track. It keeps hours-long videos from creating
    // gigantic temporary WAV files. Fall back to WAV if this ffmpeg build has
    // no AAC encoder.
    extract_audio_as(
        ffmpeg,
        path,
        start_offset,
        "m4a",
        &[
            "-c:a",
            "aac",
            "-b:a",
            "160k",
            "-movflags",
            "+faststart",
            "-f",
            "mp4",
        ],
        256,
    )
    .or_else(|| {
        extract_audio_as(
            ffmpeg,
            path,
            start_offset,
            "wav",
            &["-c:a", "pcm_s16le"],
            44,
        )
    })
}

fn extract_audio_as(
    ffmpeg: &Path,
    path: &Path,
    start_offset: f64,
    extension: &str,
    codec_args: &[&str],
    minimum_size: u64,
) -> Option<PathBuf> {
    let output = temp_file(extension);
    let mut command = Command::new(ffmpeg);
    command.args(["-v", "error", "-nostdin"]);
    if start_offset > 0.0 {
        command.arg("-ss").arg(format!("{start_offset:.3}"));
    }
    let result = command
        .arg("-i")
        .arg(path)
        .args(["-map", "0:a:0?", "-vn", "-ac", "2", "-ar", "44100"])
        .args(codec_args)
        .arg("-y")
        .arg(&output)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    let ok = result.is_ok_and(|status| status.success())
        && output
            .metadata()
            .is_ok_and(|metadata| metadata.len() > minimum_size);
    if ok {
        Some(output)
    } else {
        let _ = fs::remove_file(output);
        None
    }
}

fn probe_duration(path: &Path) -> Option<f64> {
    let ffprobe = ffprobe()?;
    let output = Command::new(ffprobe)
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
        ])
        .arg(path)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|duration| *duration > 0.0)
}

fn read_frame(stdout: &mut impl std::io::Read, buffer: &mut [u8]) -> Result<bool, String> {
    use std::io::ErrorKind;

    let mut filled = 0;
    while filled < buffer.len() {
        match stdout.read(&mut buffer[filled..]) {
            Ok(0) if filled == 0 => return Ok(false),
            Ok(0) => return Err("Video ended in the middle of a frame.".to_owned()),
            Ok(read) => filled += read,
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Err(error) => return Err(format!("Video frame read failed: {error}")),
        }
    }
    Ok(true)
}

pub fn play_audio(path: &Path, volume: f32) -> Option<AudioPlayer> {
    let mut stream = match DeviceSinkBuilder::open_default_sink() {
        Ok(stream) => stream,
        Err(error) => {
            eprintln!("Photo Roulette: audio output unavailable: {error}");
            return None;
        }
    };
    stream.log_on_drop(false);
    let player = Player::connect_new(stream.mixer());

    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) => {
            eprintln!("Photo Roulette: could not open audio: {error}");
            return None;
        }
    };
    let source = match Decoder::try_from(file) {
        Ok(source) => source,
        Err(error) => {
            eprintln!("Photo Roulette: could not decode audio: {error}");
            return None;
        }
    };

    player.append(source);
    player.set_volume(volume.clamp(0.0, 1.0));
    player.play();
    Some(AudioPlayer {
        _stream: stream,
        player,
    })
}

pub fn cleanup_audio(path: &Option<PathBuf>) {
    if let Some(path) = path {
        let _ = fs::remove_file(path);
    }
}

fn stop_child(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn temp_file(extension: &str) -> PathBuf {
    let unique = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "photo-roulette-audio-{}-{unique}.{extension}",
        std::process::id()
    ))
}

pub fn set_custom_ffmpeg(path: Option<PathBuf>) {
    if let Ok(mut custom) = CUSTOM_FFMPEG.lock() {
        *custom = path;
    }
}

pub fn custom_ffmpeg() -> Option<PathBuf> {
    CUSTOM_FFMPEG.lock().ok().and_then(|custom| custom.clone())
}

pub fn validate_ffmpeg(path: &Path) -> bool {
    Command::new(path)
        .arg("-version")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .is_ok_and(|output| {
            output.status.success()
                && String::from_utf8_lossy(&output.stdout).contains("ffmpeg version")
        })
}

pub fn install_hint() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "Install it with `brew install ffmpeg`, or locate it in the app."
    }
    #[cfg(target_os = "windows")]
    {
        "Install it with `winget install Gyan.FFmpeg`, or locate ffmpeg.exe in the app."
    }
    #[cfg(target_os = "linux")]
    {
        "Install it with your package manager (for example `sudo apt install ffmpeg`), or locate it in the app."
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        "Install ffmpeg and make sure it is on PATH, or locate it in the app."
    }
}

pub fn find_ffmpeg() -> Option<PathBuf> {
    ffmpeg()
}

fn ffmpeg() -> Option<PathBuf> {
    find_binary("ffmpeg")
}

fn ffprobe() -> Option<PathBuf> {
    if let Some(ffmpeg) = ffmpeg()
        && let Some(parent) = ffmpeg.parent()
    {
        let name = if cfg!(windows) {
            "ffprobe.exe"
        } else {
            "ffprobe"
        };
        let sibling = parent.join(name);
        if sibling.is_file() {
            return Some(sibling);
        }
    }
    find_binary("ffprobe")
}

fn find_binary(name: &str) -> Option<PathBuf> {
    if name == "ffmpeg" && std::env::var_os("PHOTO_ROULETTE_DISABLE_FFMPEG").is_some() {
        return None;
    }

    let file_name = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_owned()
    };

    // An explicitly located ffmpeg always wins.
    if name == "ffmpeg"
        && let Some(custom) = custom_ffmpeg()
        && custom.is_file()
    {
        return Some(custom);
    }

    // A binary bundled next to the app wins over anything on PATH. The .deb
    // keeps its optional copy in /usr/lib/photo-roulette.
    if let Ok(current_exe) = std::env::current_exe()
        && let Some(directory) = current_exe.parent()
    {
        for candidate in [
            directory.join(&file_name),
            directory.join("../lib/photo-roulette").join(&file_name),
        ] {
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    #[cfg(target_os = "macos")]
    for directory in ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"] {
        let candidate = Path::new(directory).join(&file_name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    #[cfg(target_os = "linux")]
    for directory in ["/usr/bin", "/usr/local/bin", "/opt/homebrew/bin"] {
        let candidate = Path::new(directory).join(&file_name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    #[cfg(windows)]
    for directory in [r"C:\ffmpeg\bin", r"C:\Program Files\ffmpeg\bin"] {
        let candidate = Path::new(directory).join(&file_name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    let path_var = std::env::var_os("PATH")?;
    for directory in std::env::split_paths(&path_var) {
        let candidate = directory.join(&file_name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::{VideoInfo, VideoMessage, cleanup_audio, ffmpeg, start};
    use std::fs;
    use std::path::PathBuf;
    use std::process::Command;
    use std::sync::mpsc::RecvTimeoutError;
    use std::time::{Duration, Instant};

    fn fixture_dir(name: &str) -> PathBuf {
        let unique = super::TEMP_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "photo-roulette-test-{}-{unique}-{name}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("create fixture folder");
        path
    }

    #[test]
    fn decodes_frames_and_prepares_audio() {
        let Some(ffmpeg) = ffmpeg() else {
            eprintln!("skipping video pipeline test: ffmpeg is not installed");
            return;
        };

        let dir = fixture_dir("video-pipeline");
        let video = dir.join("clip.mp4");
        let status = Command::new(&ffmpeg)
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc=size=320x240:rate=30",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:sample_rate=44100",
                "-t",
                "1.2",
                "-c:v",
                "mpeg4",
                "-q:v",
                "5",
                "-pix_fmt",
                "yuv420p",
                "-c:a",
                "aac",
                "-b:a",
                "96k",
                "-y",
            ])
            .arg(&video)
            .status()
            .expect("run ffmpeg fixture");
        assert!(status.success(), "ffmpeg could not create the test video");

        let load = start(video, 0.0).expect("start video decode");
        let mut info: Option<VideoInfo> = None;
        let mut poster_size = None;
        let mut saw_frame = false;
        let deadline = Instant::now() + Duration::from_secs(20);

        while Instant::now() < deadline {
            match load.rx.recv_timeout(Duration::from_secs(2)) {
                Ok(VideoMessage::Ready(ready)) => info = Some(ready),
                Ok(VideoMessage::Frame(frame)) => {
                    if poster_size.is_none() {
                        poster_size = Some(frame.size);
                    }
                    saw_frame = true;
                    break;
                }
                Ok(VideoMessage::End) => break,
                Ok(VideoMessage::Error(error)) => panic!("video pipeline error: {error}"),
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }

        assert!(saw_frame, "the decoder did not produce a frame");
        assert_eq!(
            poster_size,
            Some([320, 240]),
            "small videos should not be upscaled"
        );
        let info = info.expect("the decoder did not report video info");
        assert!(
            info.duration.is_some_and(|duration| duration > 0.5),
            "expected a duration near 1.2 seconds"
        );

        let audio_path = info.audio_path.expect("expected an extracted audio track");
        assert!(audio_path.is_file(), "audio file should exist");
        assert!(
            audio_path
                .metadata()
                .is_ok_and(|metadata| metadata.len() > 1024),
            "audio file should not be empty"
        );
        cleanup_audio(&Some(audio_path));
        let _ = fs::remove_dir_all(dir);
    }
}
