#![cfg_attr(all(not(debug_assertions), not(test)), windows_subsystem = "windows")]

mod deck;
mod history;
mod loader;
mod settings;
mod theme;
mod video;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::time::{Duration, Instant};

use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, Id, LayerId, Layout, Margin, Order, RichText,
    Sense, Stroke, StrokeKind, Vec2, pos2, vec2,
};

use deck::Round;
use history::History;
use loader::LoadedPhoto;
use settings::Settings;
use theme::{
    ACCENT, ACCENT_HOVER, ACCENT_PRESSED, BG, BORDER, CARD, CARD_BACK, GOOD, INK, MUTED, MUTED_DIM,
    TEXT,
};

/// A little bit of suspense is half the fun.
const MIN_SUSPENSE: Duration = Duration::from_millis(720);
const REVEAL_SECONDS: f32 = 0.32;
const TAU: f32 = std::f32::consts::TAU;

#[derive(Clone, Copy)]
enum VideoAction {
    TogglePause,
    Replay,
    BeginScrub(f64),
    UpdateScrub(f64),
    CommitScrub(f64),
}

#[derive(Clone, Copy, Default)]
struct LoadIntent {
    counts_as_dealt: bool,
    record_history: bool,
    history_pos: Option<usize>,
}

struct CurrentPhoto {
    texture: egui::TextureHandle,
    path: PathBuf,
    revealed_at: Instant,
}

struct PendingLoad {
    path: PathBuf,
    generation: u64,
    started: Instant,
    result: Option<Result<LoadedPhoto, String>>,
    intent: LoadIntent,
}

struct PendingVideo {
    path: PathBuf,
    started: Instant,
    rx: Option<Receiver<video::VideoMessage>>,
    info: Option<video::VideoInfo>,
    first_frame: Option<egui::ColorImage>,
    error: Option<String>,
    ended: bool,
    intent: LoadIntent,
    start_offset: f64,
}

impl PendingVideo {
    fn start(path: PathBuf, intent: LoadIntent, start_offset: f64) -> Result<Self, String> {
        let load = video::start(path.clone(), start_offset)?;
        Ok(Self {
            path,
            started: Instant::now(),
            rx: Some(load.rx),
            info: None,
            first_frame: None,
            error: None,
            ended: false,
            intent,
            start_offset,
        })
    }
}

impl Drop for PendingVideo {
    fn drop(&mut self) {
        if let Some(info) = &self.info {
            video::cleanup_audio(&info.audio_path);
        }
        if let Some(rx) = &self.rx {
            while let Ok(message) = rx.try_recv() {
                if let video::VideoMessage::Ready(info) = message {
                    video::cleanup_audio(&info.audio_path);
                }
            }
        }
    }
}

struct CurrentVideo {
    texture: egui::TextureHandle,
    path: PathBuf,
    duration: Option<f64>,
    rx: Receiver<video::VideoMessage>,
    started_at: Instant,
    next_frame_due: Instant,
    base_offset: f64,
    ended: bool,
    paused_at: Option<Instant>,
    scrubbing: bool,
    scrub_target: Option<f64>,
    error: Option<String>,
    audio: Option<video::AudioPlayer>,
    audio_path: Option<PathBuf>,
}

impl CurrentVideo {
    fn update(&mut self, ctx: &egui::Context) {
        if self.ended || self.paused_at.is_some() {
            return;
        }

        let now = Instant::now();
        let frame_duration = Duration::from_secs_f64(1.0 / 30.0);
        let mut newest_frame = None;

        while now >= self.next_frame_due {
            match self.rx.try_recv() {
                Ok(video::VideoMessage::Frame(image)) => {
                    newest_frame = Some(image);
                    self.next_frame_due += frame_duration;
                }
                Ok(video::VideoMessage::End) => {
                    self.ended = true;
                    break;
                }
                Ok(video::VideoMessage::Error(error)) => {
                    self.error = Some(error);
                    self.ended = true;
                    break;
                }
                Ok(video::VideoMessage::Ready(_)) => {}
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.ended = true;
                    break;
                }
            }
        }

        if let Some(image) = newest_frame {
            self.texture.set(image, egui::TextureOptions::LINEAR);
        }

        if !self.ended {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
    }

    fn elapsed(&self) -> f64 {
        self.base_offset
            + self
                .paused_at
                .unwrap_or_else(Instant::now)
                .saturating_duration_since(self.started_at)
                .as_secs_f64()
    }

    fn is_paused(&self) -> bool {
        self.paused_at.is_some()
    }

    fn toggle_pause(&mut self) {
        if self.ended || self.scrubbing {
            return;
        }

        let now = Instant::now();
        if let Some(paused_at) = self.paused_at.take() {
            let pause_duration = now.saturating_duration_since(paused_at);
            self.started_at += pause_duration;
            self.next_frame_due += pause_duration;
            if let Some(audio) = &self.audio {
                audio.resume();
            }
        } else {
            self.paused_at = Some(now);
            if let Some(audio) = &self.audio {
                audio.pause();
            }
        }
    }

    fn pause(&mut self) {
        if self.paused_at.is_none() {
            self.paused_at = Some(Instant::now());
            if let Some(audio) = &self.audio {
                audio.pause();
            }
        }
    }

    fn begin_scrub(&mut self, target: f64) {
        if self.paused_at.is_none() {
            self.paused_at = Some(Instant::now());
            if let Some(audio) = &self.audio {
                audio.pause();
            }
        }
        self.scrubbing = true;
        self.scrub_target = Some(target);
    }

    fn update_scrub(&mut self, target: f64) {
        if self.scrubbing {
            self.scrub_target = Some(target);
        }
    }

    fn set_volume(&self, volume: f32) {
        if let Some(audio) = &self.audio {
            audio.set_volume(volume);
        }
    }

    fn stop_audio(&mut self) {
        self.audio = None;
        video::cleanup_audio(&self.audio_path);
        self.audio_path = None;
    }
}

impl Drop for CurrentVideo {
    fn drop(&mut self) {
        self.stop_audio();
    }
}

struct LoadMessage {
    generation: u64,
    path: PathBuf,
    result: Result<LoadedPhoto, String>,
}

#[derive(Clone, Copy)]
enum NoticeKind {
    Info,
    Error,
}

struct Notice {
    text: String,
    kind: NoticeKind,
    until: f64,
}

struct ScanState {
    rx: Receiver<Result<loader::MediaLibrary, loader::ScanError>>,
    cancel: Arc<AtomicBool>,
    found: Arc<AtomicUsize>,
    started: Instant,
}

struct SmokeTest {
    started: Instant,
    ticks: u32,
    expect_media: bool,
    dealt: bool,
    closing: bool,
}

struct PhotoRoulette {
    library: Vec<PathBuf>,
    round: Option<Round>,
    current: Option<CurrentPhoto>,
    current_video: Option<CurrentVideo>,
    last_shown: Option<PathBuf>,
    pending: Option<PendingLoad>,
    pending_video: Option<PendingVideo>,
    generation: u64,
    tx: Sender<LoadMessage>,
    rx: Receiver<LoadMessage>,
    settings: Settings,
    last_volume: f32,
    history: History,
    scan: Option<ScanState>,
    last_roots: Vec<PathBuf>,
    show_ffmpeg_dialog: bool,
    notice: Option<Notice>,
    restart_icon: egui::TextureHandle,
    smoke_test: Option<SmokeTest>,
}

impl PhotoRoulette {
    fn new(
        cc: &eframe::CreationContext<'_>,
        initial_folder: Option<PathBuf>,
        smoke_test: bool,
    ) -> Self {
        theme::install(&cc.egui_ctx);
        cc.egui_ctx
            .send_viewport_cmd(egui::ViewportCommand::SetTheme(egui::SystemTheme::Dark));

        let restart_image = image::load_from_memory(include_bytes!("../assets/icons/restart.png"))
            .expect("the bundled restart icon should be valid")
            .to_rgba8();
        let restart_icon = cc.egui_ctx.load_texture(
            "restart-icon",
            egui::ColorImage::from_rgba_unmultiplied(
                [
                    restart_image.width() as usize,
                    restart_image.height() as usize,
                ],
                restart_image.as_raw(),
            ),
            egui::TextureOptions::LINEAR.with_mipmap_mode(Some(egui::TextureFilter::Linear)),
        );

        let settings = Settings::load();
        if let Some(ffmpeg) = settings.ffmpeg_path.as_ref().filter(|path| path.is_file()) {
            video::set_custom_ffmpeg(Some(ffmpeg.clone()));
        }
        let last_volume = if settings.volume > 0.01 {
            settings.volume
        } else {
            0.85
        };
        let startup_folder = if smoke_test {
            initial_folder
        } else {
            initial_folder.or_else(|| {
                settings
                    .last_folder
                    .as_ref()
                    .filter(|path| path.exists())
                    .cloned()
            })
        };
        let expect_media = startup_folder.is_some();
        let smoke_test = smoke_test.then(|| SmokeTest {
            started: Instant::now(),
            ticks: 0,
            expect_media,
            dealt: false,
            closing: false,
        });

        let (tx, rx) = channel();
        let mut app = Self {
            library: Vec::new(),
            round: None,
            current: None,
            current_video: None,
            last_shown: None,
            pending: None,
            pending_video: None,
            generation: 0,
            tx,
            rx,
            settings,
            last_volume,
            history: History::default(),
            scan: None,
            last_roots: Vec::new(),
            show_ffmpeg_dialog: false,
            notice: None,
            restart_icon,
            smoke_test,
        };

        if let Some(folder) = startup_folder {
            app.start_scan(vec![folder], &cc.egui_ctx);
        }

        app
    }

    fn start_scan(&mut self, roots: Vec<PathBuf>, ctx: &egui::Context) {
        if roots.is_empty() {
            return;
        }

        if let Some(scan) = self.scan.take() {
            scan.cancel.store(true, Ordering::Relaxed);
        }
        self.pending = None;
        self.pending_video = None;
        self.show_ffmpeg_dialog = false;

        let generation = self.generation.wrapping_add(1);
        self.generation = generation;
        let cancel = Arc::new(AtomicBool::new(false));
        let found = Arc::new(AtomicUsize::new(0));
        let (tx, rx) = channel();

        let thread_roots = roots.clone();
        let thread_cancel = Arc::clone(&cancel);
        let thread_found = Arc::clone(&found);
        let spawn = std::thread::Builder::new()
            .name("media-scan".to_owned())
            .spawn(move || {
                let result = loader::collect_media_interruptible(
                    &thread_roots,
                    &thread_cancel,
                    &thread_found,
                );
                let _ = tx.send(result);
            });

        match spawn {
            Ok(_) => {
                self.last_roots = roots;
                self.scan = Some(ScanState {
                    rx,
                    cancel,
                    found,
                    started: Instant::now(),
                });
                ctx.request_repaint();
            }
            Err(error) => {
                self.notify(
                    ctx,
                    NoticeKind::Error,
                    format!("Could not start the folder scan: {error}"),
                );
            }
        }
    }

    fn cancel_scan(&mut self, ctx: &egui::Context) {
        if let Some(scan) = self.scan.take() {
            scan.cancel.store(true, Ordering::Relaxed);
            ctx.request_repaint();
        }
    }

    fn poll_scan(&mut self, ctx: &egui::Context) {
        let Some(scan) = self.scan.as_ref() else {
            return;
        };

        let result = match scan.rx.try_recv() {
            Ok(result) => Some(result),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err(loader::ScanError::Message(
                "The folder scan stopped unexpectedly.".to_owned(),
            ))),
        };
        let Some(result) = result else {
            ctx.request_repaint_after(Duration::from_millis(100));
            return;
        };

        self.scan = None;
        match result {
            Ok(library) => self.apply_library(library, ctx),
            Err(loader::ScanError::Cancelled) => {}
            Err(loader::ScanError::Message(message)) => {
                self.notify(ctx, NoticeKind::Error, message);
            }
        }
        ctx.request_repaint();
    }

    fn apply_library(&mut self, library: loader::MediaLibrary, ctx: &egui::Context) {
        let mut media = library.media;
        let had_videos = media.iter().any(|path| loader::is_video_file(path));
        let skipped_videos = if video::is_available() {
            0
        } else {
            let before = media.len();
            media.retain(|path| !loader::is_video_file(path));
            before - media.len()
        };

        if media.is_empty() {
            self.notify(
                ctx,
                NoticeKind::Error,
                if skipped_videos > 0 {
                    "Only videos were found, and video playback needs ffmpeg.".to_owned()
                } else {
                    "No playable media found in that folder.".to_owned()
                },
            );
            if had_videos && skipped_videos > 0 {
                self.show_ffmpeg_dialog = true;
            }
            return;
        }

        self.pending = None;
        self.pending_video = None;
        self.current = None;
        self.current_video = None;
        self.last_shown = None;
        self.round = None;
        self.history.clear();
        self.library = media;
        self.start_round();

        let images = self
            .library
            .iter()
            .filter(|path| loader::is_image_file(path))
            .count();
        let videos = self
            .library
            .iter()
            .filter(|path| loader::is_video_file(path))
            .count();
        let mut message = match (images, videos) {
            (1, 0) => "Loaded 1 photo.".to_owned(),
            (1, 1) => "Loaded 1 photo and 1 video.".to_owned(),
            (n, 0) => format!("Loaded {n} photos."),
            (0, 1) => "Loaded 1 video.".to_owned(),
            (0, n) => format!("Loaded {n} videos."),
            (n, 1) => format!("Loaded {n} photos and 1 video."),
            (n, m) => format!("Loaded {n} photos and {m} videos."),
        };
        let skipped = library.skipped + skipped_videos;
        if skipped > 0 {
            message.push_str(&format!(
                " Skipped {} unreadable {}.",
                skipped,
                if skipped == 1 { "file" } else { "files" }
            ));
        }
        if skipped_videos > 0 {
            message.push_str(" Videos need ffmpeg.");
            self.show_ffmpeg_dialog = true;
        }
        if library.skipped_transcoded > 0 && !video::is_available() {
            message.push_str(" HEIC/AVIF needs ffmpeg on this platform.");
            self.show_ffmpeg_dialog = true;
        }
        self.notify(ctx, NoticeKind::Info, message);

        if self.smoke_test.is_none() && self.last_roots.len() == 1 && self.last_roots[0].is_dir() {
            self.settings.last_folder = Some(self.last_roots[0].clone());
            self.settings.save();
        }

        ctx.request_repaint();
    }

    fn open_import(&mut self, ctx: &egui::Context) {
        let picked = rfd::FileDialog::new()
            .set_title("Choose a folder of photos and videos")
            .pick_folder();

        if let Some(folder) = picked {
            self.start_scan(vec![folder], ctx);
        }
    }

    fn start_round(&mut self) {
        if self.library.is_empty() {
            self.round = None;
            return;
        }

        let number = self.round.as_ref().map_or(1, |round| round.number + 1);
        let mut rng = rand::rng();
        self.round = Some(Round::new(
            &self.library,
            number,
            self.last_shown.as_deref(),
            &mut rng,
        ));
    }

    fn deal(&mut self, ctx: &egui::Context) {
        if self.scan.is_some() || self.pending.is_some() || self.pending_video.is_some() {
            return;
        }

        if self.library.is_empty() {
            self.open_import(ctx);
            return;
        }

        if self.round.as_ref().is_none_or(|round| round.is_complete()) {
            self.start_round();
        }

        let next = self.round.as_mut().and_then(Round::next);
        let Some(path) = next else {
            self.notify(ctx, NoticeKind::Error, "The deck is empty.".to_owned());
            return;
        };

        let intent = LoadIntent {
            counts_as_dealt: true,
            record_history: true,
            history_pos: None,
        };
        self.load_path(path, intent, 0.0, ctx);
    }

    fn previous(&mut self, ctx: &egui::Context) {
        if self.scan.is_some() || self.pending.is_some() || self.pending_video.is_some() {
            return;
        }
        let Some((pos, path)) = self.history.previous() else {
            return;
        };
        let intent = LoadIntent {
            counts_as_dealt: false,
            record_history: false,
            history_pos: Some(pos),
        };
        self.load_path(path, intent, 0.0, ctx);
    }

    fn load_path(
        &mut self,
        path: PathBuf,
        intent: LoadIntent,
        start_offset: f64,
        ctx: &egui::Context,
    ) {
        self.current = None;
        self.current_video = None;

        if loader::is_video_file(&path) {
            match PendingVideo::start(path.clone(), intent, start_offset) {
                Ok(pending) => self.pending_video = Some(pending),
                Err(error) => {
                    self.handle_media_failure(path, error, ctx);
                    return;
                }
            }
        } else {
            self.pending = Some(PendingLoad {
                path: path.clone(),
                generation: self.generation,
                started: Instant::now(),
                result: None,
                intent,
            });

            let tx = self.tx.clone();
            let generation = self.generation;
            let thread_path = path.clone();
            let spawn_result = std::thread::Builder::new()
                .name("photo-decode".to_owned())
                .spawn(move || {
                    let result = std::panic::catch_unwind(|| loader::load_photo(&thread_path))
                        .unwrap_or_else(|_| Err("the image decoder panicked".to_owned()));
                    let _ = tx.send(LoadMessage {
                        generation,
                        path: thread_path,
                        result,
                    });
                });

            if let Err(error) = spawn_result {
                self.pending = None;
                self.handle_media_failure(
                    path,
                    format!("could not start image decoder: {error}"),
                    ctx,
                );
                return;
            }
        }

        ctx.request_repaint();
    }

    fn handle_media_failure(&mut self, path: PathBuf, error: String, ctx: &egui::Context) {
        eprintln!("Photo Roulette: could not open {}: {error}", path.display());

        self.library.retain(|media| media != &path);
        self.history.remove(&path);
        if let Some(round) = self.round.as_mut() {
            round.mark_unreadable();
        }

        if self.library.is_empty() {
            self.round = None;
            self.history.clear();
            self.notify(
                ctx,
                NoticeKind::Error,
                "That was the last readable file. Import a folder to try again.".to_owned(),
            );
            ctx.send_viewport_cmd(egui::ViewportCommand::Title("Photo Roulette".to_owned()));
        } else {
            self.notify(
                ctx,
                NoticeKind::Error,
                format!("Couldn't open {}. Skipped it.", display_name(&path)),
            );
        }

        ctx.request_repaint();
    }

    fn poll_image_load(&mut self, ctx: &egui::Context) {
        while let Ok(message) = self.rx.try_recv() {
            if let Some(pending) = self.pending.as_mut()
                && pending.generation == message.generation
                && pending.path == message.path
            {
                pending.result = Some(message.result);
            }
        }

        let Some(pending) = self.pending.as_ref() else {
            return;
        };

        if pending.result.is_none() || pending.started.elapsed() < MIN_SUSPENSE {
            ctx.request_repaint_after(Duration::from_millis(33));
            return;
        }

        let pending = self.pending.take().expect("pending load was just checked");
        let result = pending.result.expect("load result was just checked");
        let intent = pending.intent;

        match result {
            Ok(loaded) => {
                let texture = ctx.load_texture(
                    format!("photo:{}", pending.path.display()),
                    loaded.image,
                    egui::TextureOptions::LINEAR
                        .with_mipmap_mode(Some(egui::TextureFilter::Linear)),
                );

                self.current = Some(CurrentPhoto {
                    texture,
                    path: pending.path.clone(),
                    revealed_at: Instant::now(),
                });
                self.apply_load_intent(&pending.path, intent);
                self.last_shown = Some(pending.path);

                ctx.request_repaint();
            }
            Err(error) => self.handle_media_failure(pending.path, error, ctx),
        }
    }

    fn apply_load_intent(&mut self, path: &Path, intent: LoadIntent) {
        if intent.record_history {
            self.history.record(path.to_path_buf());
        } else if let Some(pos) = intent.history_pos {
            self.history.set_pos(pos);
        }
        if intent.counts_as_dealt
            && let Some(round) = self.round.as_mut()
        {
            round.mark_dealt();
        }
    }

    fn poll_video_load(&mut self, ctx: &egui::Context) {
        let Some(pending) = self.pending_video.as_mut() else {
            return;
        };

        if (pending.info.is_none() || pending.first_frame.is_none())
            && let Some(rx) = pending.rx.as_ref()
        {
            loop {
                match rx.try_recv() {
                    Ok(video::VideoMessage::Ready(info)) => pending.info = Some(info),
                    Ok(video::VideoMessage::Frame(image)) => {
                        pending.first_frame = Some(image);
                        break;
                    }
                    Ok(video::VideoMessage::End) => {
                        pending.ended = true;
                        break;
                    }
                    Ok(video::VideoMessage::Error(error)) => {
                        pending.error = Some(error);
                        break;
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        pending
                            .error
                            .get_or_insert_with(|| "video decoder disconnected".to_owned());
                        break;
                    }
                }
            }
        }

        let ready = pending.info.is_some() && pending.first_frame.is_some();
        let suspense_done =
            !pending.intent.counts_as_dealt || pending.started.elapsed() >= MIN_SUSPENSE;
        if pending.error.is_none() && (!ready || !suspense_done) {
            ctx.request_repaint_after(Duration::from_millis(33));
            return;
        }

        let mut pending = self
            .pending_video
            .take()
            .expect("pending video was just checked");
        let intent = pending.intent;
        if let Some(error) = pending.error.take() {
            if intent.counts_as_dealt {
                self.handle_media_failure(pending.path.clone(), error, ctx);
            } else {
                eprintln!("Photo Roulette: video load failed: {error}");
                self.notify(
                    ctx,
                    NoticeKind::Error,
                    "Couldn't open this video.".to_owned(),
                );
            }
            return;
        }

        let (Some(mut info), Some(first_frame), Some(rx)) = (
            pending.info.take(),
            pending.first_frame.take(),
            pending.rx.take(),
        ) else {
            if intent.counts_as_dealt {
                self.handle_media_failure(
                    pending.path.clone(),
                    "the video decoder returned no frames".to_owned(),
                    ctx,
                );
            } else {
                self.notify(
                    ctx,
                    NoticeKind::Error,
                    "Couldn't open this video.".to_owned(),
                );
            }
            return;
        };

        let texture = ctx.load_texture(
            format!("video:{}", pending.path.display()),
            first_frame,
            egui::TextureOptions::LINEAR,
        );
        let now = Instant::now();
        let audio_path = info.audio_path.take();
        let audio = audio_path
            .as_deref()
            .and_then(|path| video::play_audio(path, self.settings.volume));

        self.current_video = Some(CurrentVideo {
            texture,
            path: pending.path.clone(),
            duration: info.duration,
            rx,
            started_at: now,
            next_frame_due: now + Duration::from_secs_f64(1.0 / 30.0),
            base_offset: pending.start_offset,
            ended: pending.ended,
            paused_at: None,
            scrubbing: false,
            scrub_target: None,
            error: None,
            audio,
            audio_path,
        });
        self.apply_load_intent(&pending.path, intent);
        self.last_shown = Some(pending.path.clone());

        ctx.request_repaint();
    }

    fn replay_video(&mut self, ctx: &egui::Context) {
        if self.pending.is_some() || self.pending_video.is_some() {
            return;
        }
        let Some(path) = self.current_video.as_ref().map(|video| video.path.clone()) else {
            return;
        };

        match PendingVideo::start(path, LoadIntent::default(), 0.0) {
            Ok(pending) => {
                if let Some(video) = self.current_video.as_mut() {
                    video.pause();
                }
                self.pending_video = Some(pending);
                ctx.request_repaint();
            }
            Err(error) => {
                eprintln!("Photo Roulette: could not replay video: {error}");
                self.notify(
                    ctx,
                    NoticeKind::Error,
                    "Couldn't replay this video.".to_owned(),
                );
            }
        }
    }

    fn seek_video(&mut self, ctx: &egui::Context, target: f64) {
        if self.pending.is_some() || self.pending_video.is_some() {
            return;
        }
        let Some(video) = &self.current_video else {
            return;
        };
        let path = video.path.clone();
        let target = match video.duration {
            Some(duration) => target.clamp(0.0, (duration - 0.15).max(0.0)),
            None => target.max(0.0),
        };

        match PendingVideo::start(path, LoadIntent::default(), target) {
            Ok(pending) => {
                if let Some(video) = self.current_video.as_mut() {
                    video.pause();
                }
                self.pending_video = Some(pending);
                ctx.request_repaint();
            }
            Err(error) => {
                eprintln!("Photo Roulette: could not seek video: {error}");
                self.notify(
                    ctx,
                    NoticeKind::Error,
                    "Couldn't seek this video.".to_owned(),
                );
            }
        }
    }

    fn update_current_video(&mut self, ctx: &egui::Context) {
        let mut error = None;
        if let Some(video) = self.current_video.as_mut() {
            video.update(ctx);
            error = video.error.take();
            if error.is_some() {
                video.stop_audio();
            }
        }

        if let Some(error) = error {
            eprintln!("Photo Roulette: video playback error: {error}");
            self.notify(
                ctx,
                NoticeKind::Error,
                "This video couldn't play all the way through.".to_owned(),
            );
        }
    }

    fn notify(&mut self, ctx: &egui::Context, kind: NoticeKind, text: String) {
        let now = ctx.input(|input| input.time);
        let lifetime = match kind {
            NoticeKind::Info => 4.0,
            NoticeKind::Error => 6.0,
        };
        self.notice = Some(Notice {
            text,
            kind,
            until: now + lifetime,
        });
    }

    fn handle_input(&mut self, ctx: &egui::Context) {
        let dropped: Vec<PathBuf> = ctx.input(|input| {
            input
                .raw
                .dropped_files
                .iter()
                .filter_map(|file| file.path.clone())
                .collect()
        });
        if !dropped.is_empty() {
            self.start_scan(dropped, ctx);
        }

        let focused = ctx.memory(|memory| memory.focused().is_some());
        let (
            space,
            replay_key,
            pause_key,
            previous_key,
            mute_key,
            seek_left,
            seek_right,
            fullscreen_key,
            open_key,
            fullscreen_now,
        ) = ctx.input(|input| {
            (
                input.key_pressed(egui::Key::Space) && input.modifiers.is_none(),
                input.key_pressed(egui::Key::R) && input.modifiers.is_none(),
                input.key_pressed(egui::Key::P) && input.modifiers.is_none(),
                input.key_pressed(egui::Key::Backspace) && input.modifiers.is_none(),
                input.key_pressed(egui::Key::M) && input.modifiers.is_none(),
                input.key_pressed(egui::Key::ArrowLeft) && input.modifiers.is_none(),
                input.key_pressed(egui::Key::ArrowRight) && input.modifiers.is_none(),
                input.key_pressed(egui::Key::F) && input.modifiers.is_none(),
                input.modifiers.command && input.key_pressed(egui::Key::O),
                input.viewport().fullscreen.unwrap_or(false),
            )
        });

        if fullscreen_key {
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!fullscreen_now));
        }

        if open_key {
            self.open_import(ctx);
        }

        if replay_key {
            self.replay_video(ctx);
        }

        if pause_key && let Some(video) = self.current_video.as_mut() {
            video.toggle_pause();
            ctx.request_repaint();
        }

        if mute_key {
            self.toggle_mute();
        }

        if previous_key && !focused {
            self.previous(ctx);
        }

        if (seek_left || seek_right)
            && self.pending.is_none()
            && self.pending_video.is_none()
            && let Some(video) = &self.current_video
            && !video.scrubbing
        {
            let delta = if seek_left { -5.0 } else { 5.0 };
            self.seek_video(ctx, video.elapsed() + delta);
        }

        if space && !focused && !self.library.is_empty() && self.scan.is_none() {
            self.deal(ctx);
        }
    }

    fn toggle_mute(&mut self) {
        if self.settings.volume > 0.001 {
            self.last_volume = self.settings.volume;
            self.settings.volume = 0.0;
        } else {
            self.settings.volume = self.last_volume.clamp(0.05, 1.0);
        }
        self.apply_volume();
        self.settings.save();
    }

    fn apply_volume(&mut self) {
        self.settings.volume = self.settings.volume.clamp(0.0, 1.0);
        if self.settings.volume > 0.001 {
            self.last_volume = self.settings.volume;
        }
        if let Some(video) = &self.current_video {
            video.set_volume(self.settings.volume);
        }
    }

    fn smoke_test_tick(&mut self, ctx: &egui::Context) {
        let Some(smoke) = self.smoke_test.as_ref() else {
            return;
        };
        if smoke.closing {
            return;
        }

        let ticks = smoke.ticks + 1;
        let started = smoke.started;
        let expect_media = smoke.expect_media;
        let dealt = smoke.dealt;
        if let Some(smoke) = self.smoke_test.as_mut() {
            smoke.ticks = ticks;
        }

        if started.elapsed() > Duration::from_secs(30) {
            eprintln!("GUI smoke test timed out");
            std::process::exit(1);
        }
        if ticks < 3 || self.scan.is_some() {
            ctx.request_repaint();
            return;
        }
        if self.pending.is_some() || self.pending_video.is_some() {
            ctx.request_repaint();
            return;
        }
        if self.library.is_empty() {
            if expect_media {
                eprintln!("GUI smoke test failed: no media loaded");
                std::process::exit(1);
            }
            println!("GUI smoke test ok (empty state)");
            if let Some(smoke) = self.smoke_test.as_mut() {
                smoke.closing = true;
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        if !dealt {
            if let Some(smoke) = self.smoke_test.as_mut() {
                smoke.dealt = true;
            }
            self.deal(ctx);
            ctx.request_repaint();
            return;
        }
        if self.current.is_some() || self.current_video.is_some() {
            println!("GUI smoke test ok (media rendered)");
            if let Some(smoke) = self.smoke_test.as_mut() {
                smoke.closing = true;
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        ctx.request_repaint();
    }

    fn top_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("top-bar")
            .resizable(false)
            .show_separator_line(false)
            .frame(egui::Frame::NONE.fill(BG).inner_margin(Margin {
                left: 30,
                right: 30,
                top: 18,
                bottom: 10,
            }))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 0.0;
                            ui.label(
                                RichText::new("PHOTO ")
                                    .font(theme::display_font(30.0))
                                    .color(TEXT)
                                    .extra_letter_spacing(0.6),
                            );
                            ui.label(
                                RichText::new("ROULETTE")
                                    .font(theme::display_font(30.0))
                                    .color(ACCENT)
                                    .extra_letter_spacing(0.6),
                            );
                        });
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(
                                    "Deal the evidence. Photos and videos, one at a time.",
                                )
                                .font(theme::body_font(13.0))
                                .color(MUTED),
                            );
                            ui.label(
                                RichText::new(format!("v{}", env!("CARGO_PKG_VERSION")))
                                    .font(theme::bold_font(9.0))
                                    .color(MUTED_DIM),
                            );
                        });
                    });

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let label = if self.library.is_empty() {
                            "IMPORT FOLDER"
                        } else {
                            "CHANGE FOLDER"
                        };
                        let response = secondary_button(ui, label, vec2(158.0, 40.0));
                        if response.clicked() {
                            response.surrender_focus();
                            self.open_import(ctx);
                        }
                    });
                });
            });
    }

    fn bottom_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("bottom-bar")
            .resizable(false)
            .show_separator_line(false)
            .frame(egui::Frame::NONE.fill(BG).inner_margin(Margin {
                left: 30,
                right: 30,
                top: 10,
                bottom: 18,
            }))
            .show(ctx, |ui| {
                self.draw_status_row(ui, ctx);
                ui.add_space(10.0);

                ui.vertical_centered(|ui| {
                    let enabled = self.scan.is_none()
                        && self.pending.is_none()
                        && self.pending_video.is_none();
                    let can_go_back = self.history.can_go_back();
                    let label = self.primary_label();
                    ui.horizontal(|ui| {
                        let row_width = 104.0 + ui.spacing().item_spacing.x + 400.0;
                        ui.add_space(((ui.available_width() - row_width) * 0.5).max(0.0));
                        let back = ui.add_enabled_ui(can_go_back, |ui| {
                            secondary_button(ui, "‹ BACK", vec2(104.0, 56.0))
                        });
                        if back.inner.clicked() {
                            back.inner.surrender_focus();
                            self.previous(ctx);
                        }
                        let response = primary_button(ui, label, vec2(400.0, 56.0), enabled);
                        if response.clicked() {
                            response.surrender_focus();
                            self.deal(ctx);
                        }
                    });

                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(self.hint_text())
                            .font(theme::body_font(11.0))
                            .color(MUTED)
                            .extra_letter_spacing(0.5),
                    );
                });
            });
    }

    fn draw_status_row(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.horizontal(|ui| {
            match &self.round {
                Some(round) => {
                    let complete = round.is_complete()
                        && self.pending.is_none()
                        && (self.pending_video.is_none() || self.current_video.is_some());
                    let label = if complete {
                        format!("ROUND {} COMPLETE", round.number)
                    } else {
                        format!("ROUND {}", round.number)
                    };
                    ui.label(
                        RichText::new(label)
                            .font(theme::bold_font(10.5))
                            .color(if complete { GOOD } else { ACCENT })
                            .extra_letter_spacing(1.4),
                    );
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(format!("{} / {} DEALT", round.dealt, round.total))
                            .font(theme::bold_font(10.5))
                            .color(TEXT)
                            .extra_letter_spacing(0.7),
                    );
                }
                None => {
                    ui.label(
                        RichText::new("WAITING FOR MEDIA")
                            .font(theme::bold_font(10.5))
                            .color(MUTED)
                            .extra_letter_spacing(1.4),
                    );
                }
            }

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let fullscreen = ctx.input(|input| input.viewport().fullscreen.unwrap_or(false));
                let fullscreen_label = if fullscreen {
                    "EXIT FULL SCREEN"
                } else {
                    "FULL SCREEN"
                };
                let fullscreen_response = text_button(ui, fullscreen_label);
                if fullscreen_response.clicked() {
                    fullscreen_response.surrender_focus();
                    ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!fullscreen));
                }

                ui.add_space(8.0);

                ui.scope(|ui| {
                    ui.spacing_mut().slider_width = 84.0;
                    ui.spacing_mut().interact_size.y = 18.0;
                    ui.spacing_mut().slider_rail_height = 4.0;
                    let mut volume = self.settings.volume;
                    let slider = ui.add(
                        egui::Slider::new(&mut volume, 0.0..=1.0)
                            .show_value(false)
                            .trailing_fill(true),
                    );
                    if slider.changed() {
                        self.settings.volume = volume;
                        self.apply_volume();
                    }
                    if slider.drag_stopped() || slider.clicked() || slider.lost_focus() {
                        self.settings.save();
                    }
                    slider.on_hover_text("Volume (M to mute)");
                    ui.label(
                        RichText::new("VOL")
                            .font(theme::bold_font(10.0))
                            .color(MUTED)
                            .extra_letter_spacing(1.0),
                    );
                });

                ui.add_space(8.0);

                ui.scope(|ui| {
                    let widgets = &mut ui.style_mut().visuals.widgets;
                    for widget in [
                        &mut widgets.inactive,
                        &mut widgets.hovered,
                        &mut widgets.active,
                    ] {
                        widget.corner_radius = CornerRadius::same(4);
                    }
                    let response = ui.checkbox(
                        &mut self.settings.show_filenames,
                        RichText::new("SHOW FILE NAMES")
                            .font(theme::bold_font(10.0))
                            .color(MUTED)
                            .extra_letter_spacing(1.0),
                    );
                    if response.clicked() {
                        response.surrender_focus();
                        self.settings.save();
                    }
                });
            });
        });
    }

    fn primary_label(&self) -> &'static str {
        if self.scan.is_some() {
            "SCANNING…"
        } else if self.pending.is_some()
            || (self.pending_video.is_some() && self.current_video.is_none())
        {
            "SHUFFLING…"
        } else if self.library.is_empty() {
            "CHOOSE A FOLDER"
        } else if self.round.as_ref().is_some_and(Round::is_complete) {
            "START A NEW ROUND"
        } else if self.round.as_ref().is_none_or(|round| round.dealt == 0) {
            "DEAL THE FIRST ONE"
        } else {
            "DEAL THE NEXT ONE"
        }
    }

    fn hint_text(&self) -> &'static str {
        if self.scan.is_some() {
            "LOOKING THROUGH THE FOLDER…"
        } else if self.pending.is_some() {
            "PICK A CARD, ANY CARD"
        } else if self.pending_video.is_some() {
            if self.current_video.is_some() {
                "BUFFERING THE SEEK…"
            } else {
                "PICK A CARD, ANY CARD"
            }
        } else if self.library.is_empty() {
            "DROP A FOLDER ANYWHERE · PHOTOS + VIDEOS"
        } else if let Some(video) = &self.current_video {
            if video.is_paused() {
                "CLICK VIDEO TO PLAY · DRAG TO SEEK · M MUTE · R RESTART · SPACE NEXT"
            } else {
                "CLICK VIDEO TO PAUSE · DRAG TO SEEK · M MUTE · R RESTART · SPACE NEXT"
            }
        } else if self.round.as_ref().is_some_and(Round::is_complete) {
            "EVERY ITEM GOT ITS MOMENT · NEW ROUND = FRESH SHUFFLE"
        } else {
            "NO REPEATS IN A ROUND · SPACE TO DEAL · F FOR FULL SCREEN"
        }
    }

    fn stage(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(BG).inner_margin(Margin {
                left: 30,
                right: 30,
                top: 10,
                bottom: 10,
            }))
            .show(ctx, |ui| {
                let (card, _) = ui.allocate_exact_size(ui.available_size(), Sense::hover());
                if card.height() < 120.0 {
                    return;
                }

                let painter = ui.painter_at(card);
                painter.rect(
                    card,
                    CornerRadius::same(22),
                    CARD,
                    Stroke::new(1.0_f32, BORDER),
                    StrokeKind::Inside,
                );
                theme::corner_marks(&painter, card);

                let buffering = self.pending_video.is_some() && self.current_video.is_some();
                if let Some(scan) = &self.scan {
                    let time = ctx.input(|input| input.time) as f32;
                    let found = scan.found.load(Ordering::Relaxed);
                    let elapsed = scan.started.elapsed().as_secs_f32();
                    draw_scanning(&painter, card, found, elapsed, time);
                    let cancel_rect = egui::Rect::from_center_size(
                        pos2(card.center().x, card.center().y + 94.0),
                        vec2(150.0, 38.0),
                    );
                    let cancel = ui.put(
                        cancel_rect,
                        egui::Button::new(
                            RichText::new("CANCEL")
                                .font(theme::bold_font(10.5))
                                .extra_letter_spacing(1.0),
                        )
                        .fill(CARD_BACK)
                        .stroke(Stroke::new(1.0_f32, BORDER)),
                    );
                    if cancel.clicked() {
                        cancel.surrender_focus();
                        self.cancel_scan(ctx);
                    }
                } else if self.pending.is_some() || (self.pending_video.is_some() && !buffering) {
                    let time = ctx.input(|input| input.time) as f32;
                    draw_loading(&painter, card, time, self.pending_video.is_some());
                } else if let Some(video) = &self.current_video {
                    let action = draw_video(
                        ui,
                        &painter,
                        card,
                        video,
                        self.settings.show_filenames,
                        buffering,
                        &self.restart_icon,
                    );
                    if let Some(action) = action {
                        match action {
                            VideoAction::TogglePause => {
                                if let Some(video) = self.current_video.as_mut() {
                                    video.toggle_pause();
                                    ctx.request_repaint();
                                }
                            }
                            VideoAction::Replay => self.replay_video(ctx),
                            VideoAction::BeginScrub(target) => {
                                if let Some(video) = self.current_video.as_mut() {
                                    video.begin_scrub(target);
                                    ctx.request_repaint();
                                }
                            }
                            VideoAction::UpdateScrub(target) => {
                                if let Some(video) = self.current_video.as_mut() {
                                    video.update_scrub(target);
                                    ctx.request_repaint();
                                }
                            }
                            VideoAction::CommitScrub(target) => {
                                self.seek_video(ctx, target);
                            }
                        }
                    }
                } else if let Some(current) = &self.current {
                    draw_photo(
                        ui,
                        &painter,
                        card,
                        current,
                        self.settings.show_filenames,
                        ctx,
                    );
                } else {
                    draw_empty(&painter, card, self.library.len(), self.round.as_ref());
                }

                self.draw_notice(&painter, card, ctx);
            });
    }

    fn ffmpeg_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_ffmpeg_dialog {
            return;
        }

        let mut open = true;
        let mut close = false;
        egui::Window::new("VIDEO NEEDS FFMPEG")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.set_max_width(440.0);
                ui.add_space(4.0);
                ui.label(
                    RichText::new("Videos are decoded by ffmpeg. Photos already work without it.")
                        .font(theme::body_font(13.0))
                        .color(TEXT),
                );
                ui.add_space(8.0);
                ui.label(
                    RichText::new(video::install_hint())
                        .font(theme::body_font(12.0))
                        .color(MUTED),
                );
                ui.add_space(14.0);

                ui.horizontal(|ui| {
                    if secondary_button(ui, "LOCATE FFMPEG…", vec2(190.0, 38.0)).clicked() {
                        let picked = rfd::FileDialog::new()
                            .set_title("Choose the ffmpeg executable")
                            .pick_file();
                        if let Some(path) = picked {
                            if video::validate_ffmpeg(&path) {
                                video::set_custom_ffmpeg(Some(path.clone()));
                                self.settings.ffmpeg_path = Some(path);
                                self.settings.save();
                                self.show_ffmpeg_dialog = false;
                                close = true;
                                let roots = self.last_roots.clone();
                                if !roots.is_empty() {
                                    self.start_scan(roots, ctx);
                                }
                            } else {
                                self.notify(
                                    ctx,
                                    NoticeKind::Error,
                                    "That file doesn't look like a working ffmpeg executable."
                                        .to_owned(),
                                );
                            }
                        }
                    }

                    ui.hyperlink_to(
                        RichText::new("DOWNLOAD FFMPEG")
                            .font(theme::bold_font(10.5))
                            .color(ACCENT),
                        "https://ffmpeg.org/download.html",
                    );
                });

                ui.add_space(8.0);
                if text_button(ui, "NOT NOW").clicked() {
                    close = true;
                }
            });

        if close || !open {
            self.show_ffmpeg_dialog = false;
        }
    }

    fn draw_notice(&mut self, painter: &egui::Painter, card: egui::Rect, ctx: &egui::Context) {
        let now = ctx.input(|input| input.time);
        let Some(notice) = &self.notice else {
            return;
        };

        if now >= notice.until {
            self.notice = None;
            return;
        }

        let fade = ((notice.until - now) / 0.45).clamp(0.0, 1.0) as f32;
        let (background, border, foreground) = match notice.kind {
            NoticeKind::Info => (
                Color32::from_rgb(25, 25, 34),
                BORDER,
                Color32::from_rgb(238, 235, 229),
            ),
            NoticeKind::Error => (
                Color32::from_rgb(48, 23, 30),
                Color32::from_rgb(118, 49, 64),
                Color32::from_rgb(255, 199, 209),
            ),
        };

        let font = theme::body_font(12.0);
        let galley =
            painter.layout_no_wrap(notice.text.clone(), font, fade_color(foreground, fade));
        let size = galley.size() + vec2(24.0, 12.0);
        let rect = egui::Rect::from_center_size(
            pos2(card.center().x, card.top() + 20.0 + size.y * 0.5),
            size,
        );

        painter.rect(
            rect,
            CornerRadius::same(9),
            fade_color(background, fade),
            Stroke::new(1.0_f32, fade_color(border, fade)),
            StrokeKind::Inside,
        );
        painter.galley(
            rect.min + vec2(12.0, 6.0),
            galley,
            fade_color(foreground, fade),
        );

        ctx.request_repaint_after(Duration::from_millis(50));
    }

    fn drop_overlay(&self, ctx: &egui::Context) {
        let hovering = ctx.input(|input| !input.raw.hovered_files.is_empty());
        if !hovering {
            return;
        }

        let screen = ctx.screen_rect();
        let painter = ctx.layer_painter(LayerId::new(Order::Foreground, Id::new("drop-overlay")));
        painter.rect_filled(
            screen,
            CornerRadius::ZERO,
            Color32::from_rgba_unmultiplied(8, 8, 12, 228),
        );

        let inner = screen.shrink(26.0);
        painter.rect(
            inner,
            CornerRadius::same(24),
            Color32::from_rgba_unmultiplied(255, 77, 109, 16),
            Stroke::new(2.0_f32, ACCENT),
            StrokeKind::Inside,
        );

        let center = screen.center();
        painter.text(
            center - vec2(0.0, 26.0),
            Align2::CENTER_CENTER,
            "DROP THE EVIDENCE",
            theme::display_font(34.0),
            TEXT,
        );
        painter.text(
            center + vec2(0.0, 20.0),
            Align2::CENTER_CENTER,
            "A folder, or a pile of photos and videos. We'll take it from there.",
            theme::body_font(14.0),
            MUTED,
        );
        painter.text(
            center + vec2(0.0, 44.0),
            Align2::CENTER_CENTER,
            "JPG · PNG · HEIC · WEBP · MP4 · MOV · WMV · and a few more",
            theme::body_font(10.5),
            MUTED_DIM,
        );

        ctx.request_repaint();
    }
}

impl eframe::App for PhotoRoulette {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_scan(ctx);
        self.poll_image_load(ctx);
        self.poll_video_load(ctx);
        self.update_current_video(ctx);
        self.handle_input(ctx);

        self.top_bar(ctx);
        self.bottom_bar(ctx);
        self.stage(ctx);
        self.drop_overlay(ctx);
        self.ffmpeg_dialog(ctx);
        self.smoke_test_tick(ctx);
    }
}

fn primary_button(ui: &mut egui::Ui, label: &str, size: Vec2, enabled: bool) -> egui::Response {
    ui.scope(|ui| {
        let widgets = &mut ui.style_mut().visuals.widgets;

        widgets.noninteractive.weak_bg_fill = CARD_BACK;
        widgets.noninteractive.bg_fill = CARD_BACK;
        widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, BORDER);
        widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, MUTED);
        widgets.noninteractive.corner_radius = CornerRadius::same(15);

        widgets.inactive.weak_bg_fill = ACCENT;
        widgets.inactive.bg_fill = ACCENT;
        widgets.inactive.bg_stroke = Stroke::NONE;
        widgets.inactive.fg_stroke = Stroke::new(1.0_f32, INK);
        widgets.inactive.corner_radius = CornerRadius::same(15);
        widgets.inactive.expansion = 0.0;

        widgets.hovered.weak_bg_fill = ACCENT_HOVER;
        widgets.hovered.bg_fill = ACCENT_HOVER;
        widgets.hovered.bg_stroke = Stroke::NONE;
        widgets.hovered.fg_stroke = Stroke::new(1.0_f32, INK);
        widgets.hovered.corner_radius = CornerRadius::same(15);
        widgets.hovered.expansion = 0.0;

        widgets.active.weak_bg_fill = ACCENT_PRESSED;
        widgets.active.bg_fill = ACCENT_PRESSED;
        widgets.active.bg_stroke = Stroke::NONE;
        widgets.active.fg_stroke = Stroke::new(1.0_f32, INK);
        widgets.active.corner_radius = CornerRadius::same(15);
        widgets.active.expansion = 0.0;

        let button = egui::Button::new(
            RichText::new(label)
                .font(theme::bold_font(15.0))
                .extra_letter_spacing(1.2),
        )
        .min_size(size);

        let response = ui.add_enabled(enabled, button);
        if enabled {
            let top = response.rect.top() + 1.0;
            ui.painter().line_segment(
                [
                    pos2(response.rect.left() + 18.0, top),
                    pos2(response.rect.right() - 18.0, top),
                ],
                Stroke::new(1.0_f32, Color32::from_white_alpha(36)),
            );
        }
        response
    })
    .inner
}

fn secondary_button(ui: &mut egui::Ui, label: &str, size: Vec2) -> egui::Response {
    ui.scope(|ui| {
        let widgets = &mut ui.style_mut().visuals.widgets;

        widgets.inactive.weak_bg_fill = Color32::from_rgb(26, 26, 35);
        widgets.inactive.bg_fill = Color32::from_rgb(26, 26, 35);
        widgets.inactive.bg_stroke = Stroke::new(1.0_f32, BORDER);
        widgets.inactive.fg_stroke = Stroke::new(1.0_f32, TEXT);
        widgets.inactive.corner_radius = CornerRadius::same(10);
        widgets.inactive.expansion = 0.0;

        widgets.hovered.weak_bg_fill = Color32::from_rgb(38, 38, 49);
        widgets.hovered.bg_fill = Color32::from_rgb(38, 38, 49);
        widgets.hovered.bg_stroke = Stroke::new(1.0_f32, ACCENT);
        widgets.hovered.fg_stroke = Stroke::new(1.0_f32, TEXT);
        widgets.hovered.corner_radius = CornerRadius::same(10);
        widgets.hovered.expansion = 0.0;

        widgets.active.weak_bg_fill = Color32::from_rgb(22, 22, 30);
        widgets.active.bg_fill = Color32::from_rgb(22, 22, 30);
        widgets.active.bg_stroke = Stroke::new(1.0_f32, ACCENT);
        widgets.active.fg_stroke = Stroke::new(1.0_f32, TEXT);
        widgets.active.corner_radius = CornerRadius::same(10);
        widgets.active.expansion = 0.0;

        ui.add(
            egui::Button::new(
                RichText::new(label)
                    .font(theme::bold_font(11.5))
                    .extra_letter_spacing(1.2),
            )
            .min_size(size),
        )
    })
    .inner
}

fn text_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.scope(|ui| {
        ui.spacing_mut().button_padding = vec2(4.0, 3.0);
        let widgets = &mut ui.style_mut().visuals.widgets;

        widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
        widgets.inactive.bg_stroke = Stroke::NONE;
        widgets.inactive.fg_stroke = Stroke::new(1.0_f32, MUTED);
        widgets.inactive.expansion = 0.0;

        widgets.hovered.weak_bg_fill = Color32::TRANSPARENT;
        widgets.hovered.bg_stroke = Stroke::NONE;
        widgets.hovered.fg_stroke = Stroke::new(1.0_f32, TEXT);
        widgets.hovered.expansion = 0.0;

        widgets.active.weak_bg_fill = Color32::TRANSPARENT;
        widgets.active.bg_stroke = Stroke::NONE;
        widgets.active.fg_stroke = Stroke::new(1.0_f32, ACCENT);
        widgets.active.expansion = 0.0;

        ui.add(
            egui::Button::new(
                RichText::new(label)
                    .font(theme::bold_font(10.5))
                    .extra_letter_spacing(1.0),
            )
            .frame(false),
        )
    })
    .inner
}

fn draw_photo(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    card: egui::Rect,
    current: &CurrentPhoto,
    show_filename: bool,
    ctx: &egui::Context,
) {
    let elapsed = current.revealed_at.elapsed().as_secs_f32();
    let progress = (elapsed / REVEAL_SECONDS).clamp(0.0, 1.0);
    let eased = 1.0 - (1.0 - progress).powi(3);

    let texture_size = current.texture.size();
    if texture_size[0] == 0 || texture_size[1] == 0 {
        return;
    }
    let aspect = texture_size[0] as f32 / texture_size[1] as f32;
    let container = card.shrink2(vec2(30.0, 26.0));
    let fitted = theme::fit_rect(container, aspect);
    let scale = 0.985 + 0.015 * eased;
    let rect = egui::Rect::from_center_size(fitted.center(), fitted.size() * scale);
    let alpha = (eased * 255.0).round() as u8;

    let shadow = egui::epaint::Shadow {
        offset: [0, 12],
        blur: 34,
        spread: 0,
        color: Color32::from_black_alpha((eased * 130.0) as u8),
    };
    painter.add(shadow.as_shape(rect, CornerRadius::same(10)));

    egui::Image::new(egui::load::SizedTexture::new(
        current.texture.id(),
        current.texture.size_vec2(),
    ))
    .corner_radius(10)
    .tint(Color32::from_white_alpha(alpha))
    .paint_at(ui, rect);

    painter.rect_stroke(
        rect,
        CornerRadius::same(10),
        Stroke::new(1.0_f32, Color32::from_white_alpha((eased * 38.0) as u8)),
        StrokeKind::Inside,
    );

    if show_filename && rect.height() > 90.0 && rect.width() > 170.0 {
        let name = current
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("photo");
        let max_chars = (rect.width() / 7.5).clamp(10.0, 48.0) as usize;
        let name = theme::truncate_middle(name, max_chars);
        let font = theme::body_font(11.5);
        let galley = painter.layout_no_wrap(name, font, TEXT);
        let chip = egui::Rect::from_min_size(
            rect.left_bottom() + vec2(12.0, -galley.size().y - 22.0),
            galley.size() + vec2(20.0, 10.0),
        );
        painter.rect(
            chip,
            CornerRadius::same(7),
            Color32::from_rgba_unmultiplied(8, 8, 12, 205),
            Stroke::new(1.0_f32, Color32::from_white_alpha(28)),
            StrokeKind::Inside,
        );
        painter.galley(chip.min + vec2(10.0, 5.0), galley, TEXT);
    }

    if progress < 1.0 {
        ctx.request_repaint();
    }
}

fn draw_video(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    card: egui::Rect,
    video: &CurrentVideo,
    show_filename: bool,
    buffering: bool,
    restart_icon: &egui::TextureHandle,
) -> Option<VideoAction> {
    let elapsed = video.started_at.elapsed().as_secs_f32();
    let reveal = (elapsed / REVEAL_SECONDS).clamp(0.0, 1.0);
    let eased = 1.0 - (1.0 - reveal).powi(3);

    let texture_size = video.texture.size();
    if texture_size[0] == 0 || texture_size[1] == 0 {
        return None;
    }
    let aspect = texture_size[0] as f32 / texture_size[1] as f32;
    let container = card.shrink2(vec2(30.0, 26.0));
    let fitted = theme::fit_rect(container, aspect);
    let scale = 0.985 + 0.015 * eased;
    let rect = egui::Rect::from_center_size(fitted.center(), fitted.size() * scale);
    let alpha = (eased * 255.0).round() as u8;

    let shadow = egui::epaint::Shadow {
        offset: [0, 12],
        blur: 34,
        spread: 0,
        color: Color32::from_black_alpha((eased * 130.0) as u8),
    };
    painter.add(shadow.as_shape(rect, CornerRadius::same(10)));

    egui::Image::new(egui::load::SizedTexture::new(
        video.texture.id(),
        video.texture.size_vec2(),
    ))
    .corner_radius(10)
    .tint(Color32::from_white_alpha(alpha))
    .paint_at(ui, rect);

    painter.rect_stroke(
        rect,
        CornerRadius::same(10),
        Stroke::new(1.0_f32, Color32::from_white_alpha((eased * 38.0) as u8)),
        StrokeKind::Inside,
    );

    let duration = video.duration.filter(|duration| *duration > 0.0);
    let progress = if video.ended {
        1.0
    } else {
        duration
            .map(|duration| (video.elapsed() / duration).clamp(0.0, 1.0) as f32)
            .unwrap_or(0.0)
    };

    // VIDEO badge with a small play triangle.
    let label = if video.ended { "VIDEO ENDED" } else { "VIDEO" };
    let galley = painter.layout_no_wrap(label.to_owned(), theme::bold_font(9.5), TEXT);
    let pill = egui::Rect::from_min_size(
        rect.left_top() + vec2(12.0, 12.0),
        galley.size() + vec2(30.0, 9.0),
    );
    painter.rect(
        pill,
        CornerRadius::same(6),
        Color32::from_rgba_unmultiplied(8, 8, 12, 205),
        Stroke::new(1.0_f32, Color32::from_white_alpha(36)),
        StrokeKind::Inside,
    );
    let center_y = pill.center().y;
    let triangle_left = pill.left() + 10.0;
    painter.add(egui::Shape::convex_polygon(
        vec![
            pos2(triangle_left, center_y - 4.0),
            pos2(triangle_left, center_y + 4.0),
            pos2(triangle_left + 7.0, center_y),
        ],
        ACCENT,
        Stroke::NONE,
    ));
    painter.galley(pill.min + vec2(22.0, 4.5), galley, TEXT);

    if buffering {
        let galley = painter.layout_no_wrap("BUFFERING…".to_owned(), theme::bold_font(9.5), ACCENT);
        let bubble = egui::Rect::from_center_size(
            pos2(rect.center().x, rect.top() + 22.0),
            galley.size() + vec2(20.0, 10.0),
        );
        painter.rect(
            bubble,
            CornerRadius::same(7),
            Color32::from_rgba_unmultiplied(8, 8, 12, 225),
            Stroke::new(1.0_f32, Color32::from_white_alpha(34)),
            StrokeKind::Inside,
        );
        painter.galley(bubble.min + vec2(10.0, 5.0), galley, ACCENT);
        return None;
    }

    let track = egui::Rect::from_min_max(
        pos2(rect.left() + 14.0, rect.bottom() - 11.0),
        pos2(rect.right() - 14.0, rect.bottom() - 7.0),
    );
    let timeline_hit = egui::Rect::from_min_max(
        pos2(rect.left() + 8.0, rect.bottom() - 28.0),
        pos2(rect.right() - 8.0, rect.bottom() - 2.0),
    );
    let restart_rect = egui::Rect::from_center_size(
        pos2(rect.right() - 28.0, rect.bottom() - 50.0),
        vec2(34.0, 34.0),
    );

    let pointer_pos = ui.input(|input| input.pointer.interact_pos());
    let primary_pressed = ui.input(|input| input.pointer.primary_pressed());
    let primary_down = ui.input(|input| input.pointer.primary_down());
    let primary_released = ui.input(|input| input.pointer.primary_released());
    let timeline_hovered =
        duration.is_some() && pointer_pos.is_some_and(|pointer| timeline_hit.contains(pointer));
    let restart_hovered = pointer_pos.is_some_and(|pointer| restart_rect.contains(pointer));
    let target_seconds = |pointer: egui::Pos2| {
        duration.map_or(0.0, |duration| {
            let fraction = ((pointer.x - track.left()) / track.width()).clamp(0.0, 1.0);
            fraction as f64 * duration
        })
    };

    let video_response = ui.interact(rect, ui.id().with("video-toggle"), Sense::click());
    if timeline_hovered || video.scrubbing || video_response.hovered() || restart_hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }

    let mut action = None;

    if primary_pressed && let Some(pointer) = pointer_pos {
        if duration.is_some() && timeline_hit.contains(pointer) {
            action = Some(VideoAction::BeginScrub(target_seconds(pointer)));
        } else if restart_rect.contains(pointer) {
            action = Some(VideoAction::Replay);
        }
    }

    if action.is_none()
        && video.scrubbing
        && primary_down
        && let Some(pointer) = pointer_pos
    {
        action = Some(VideoAction::UpdateScrub(target_seconds(pointer)));
    }

    if video.scrubbing
        && primary_released
        && let Some(target) = video
            .scrub_target
            .or_else(|| pointer_pos.map(target_seconds))
    {
        action = Some(VideoAction::CommitScrub(target));
    }

    if action.is_none()
        && video_response.clicked()
        && !timeline_hovered
        && !restart_hovered
        && !video.scrubbing
    {
        action = Some(if video.ended {
            VideoAction::Replay
        } else {
            VideoAction::TogglePause
        });
    }

    let display_progress = if let (Some(duration), Some(target)) = (duration, video.scrub_target) {
        (target / duration).clamp(0.0, 1.0) as f32
    } else {
        progress
    };
    let display_target = video.scrub_target.unwrap_or_else(|| video.elapsed());

    // Timeline.
    painter.rect_filled(track, CornerRadius::same(2), Color32::from_black_alpha(150));
    if display_progress > 0.0 {
        let filled = egui::Rect::from_min_max(
            track.min,
            pos2(track.left() + track.width() * display_progress, track.max.y),
        );
        painter.rect_filled(filled, CornerRadius::same(2), ACCENT);
    }

    if let Some(duration) = duration {
        let knob_x = track.left() + track.width() * display_progress;
        let knob_radius = if timeline_hovered || video.scrubbing {
            6.0
        } else {
            4.0
        };
        painter.circle_filled(pos2(knob_x, track.center().y), knob_radius, ACCENT);
        painter.circle_stroke(
            pos2(knob_x, track.center().y),
            knob_radius,
            Stroke::new(1.0_f32, Color32::from_white_alpha(170)),
        );

        if timeline_hovered || video.scrubbing {
            let label = format!(
                "{} / {}",
                format_duration(display_target),
                format_duration(duration)
            );
            let galley = painter.layout_no_wrap(label, theme::body_font(10.0), TEXT);
            let bubble_size = galley.size() + vec2(16.0, 8.0);
            let bubble_x = (knob_x - bubble_size.x * 0.5)
                .clamp(rect.left() + 8.0, rect.right() - bubble_size.x - 8.0);
            let bubble = egui::Rect::from_min_size(
                pos2(bubble_x, track.top() - bubble_size.y - 8.0),
                bubble_size,
            );
            painter.rect(
                bubble,
                CornerRadius::same(5),
                Color32::from_rgba_unmultiplied(8, 8, 12, 220),
                Stroke::new(1.0_f32, Color32::from_white_alpha(30)),
                StrokeKind::Inside,
            );
            painter.galley(bubble.min + vec2(8.0, 4.0), galley, TEXT);
        }
    }

    if show_filename && rect.height() > 90.0 && rect.width() > 170.0 {
        let name = video
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("video");
        let max_chars = (rect.width() / 7.5).clamp(10.0, 48.0) as usize;
        let name = theme::truncate_middle(name, max_chars);
        let font = theme::body_font(11.5);
        let galley = painter.layout_no_wrap(name, font, TEXT);
        let chip = egui::Rect::from_min_size(
            rect.left_bottom() + vec2(12.0, -galley.size().y - 36.0),
            galley.size() + vec2(20.0, 10.0),
        );
        painter.rect(
            chip,
            CornerRadius::same(7),
            Color32::from_rgba_unmultiplied(8, 8, 12, 205),
            Stroke::new(1.0_f32, Color32::from_white_alpha(28)),
            StrokeKind::Inside,
        );
        painter.galley(chip.min + vec2(10.0, 5.0), galley, TEXT);
    }

    // Restart button. No outer ring: the circular arrow is the whole icon.
    let restart_bg = if restart_hovered {
        Color32::from_rgba_unmultiplied(8, 8, 12, 220)
    } else {
        Color32::from_rgba_unmultiplied(8, 8, 12, 130)
    };
    painter.circle_filled(restart_rect.center(), 17.0, restart_bg);
    painter.image(
        restart_icon.id(),
        egui::Rect::from_center_size(restart_rect.center(), vec2(22.0, 22.0)),
        egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
        if restart_hovered { ACCENT_HOVER } else { TEXT },
    );

    // Play/pause button, centered on the video.
    if !video.scrubbing && (video.ended || video.is_paused() || video_response.hovered()) {
        let overlay = egui::Rect::from_center_size(rect.center(), vec2(56.0, 56.0));
        painter.circle_filled(overlay.center(), 28.0, Color32::from_black_alpha(145));

        if video.ended {
            painter.image(
                restart_icon.id(),
                egui::Rect::from_center_size(overlay.center(), vec2(30.0, 30.0)),
                egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                TEXT,
            );
        } else if video.is_paused() {
            painter.add(egui::Shape::convex_polygon(
                vec![
                    pos2(overlay.center().x - 8.0, overlay.center().y - 10.0),
                    pos2(overlay.center().x - 8.0, overlay.center().y + 10.0),
                    pos2(overlay.center().x + 8.0, overlay.center().y),
                ],
                TEXT,
                Stroke::NONE,
            ));
        } else {
            let bar_width = 4.0;
            let bar_height = 16.0;
            let gap = 6.0;
            let total_width = bar_width * 2.0 + gap;
            let left = overlay.center().x - total_width * 0.5;
            painter.rect_filled(
                egui::Rect::from_min_size(
                    pos2(left, overlay.center().y - bar_height * 0.5),
                    vec2(bar_width, bar_height),
                ),
                CornerRadius::same(1),
                TEXT,
            );
            painter.rect_filled(
                egui::Rect::from_min_size(
                    pos2(
                        left + bar_width + gap,
                        overlay.center().y - bar_height * 0.5,
                    ),
                    vec2(bar_width, bar_height),
                ),
                CornerRadius::same(1),
                TEXT,
            );
        }
    }

    if reveal < 1.0 {
        ui.ctx().request_repaint();
    }

    action
}

fn format_duration(seconds: f64) -> String {
    let seconds = seconds.max(0.0).round() as u64;
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    let secs = seconds % 60;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{secs:02}")
    } else {
        format!("{minutes}:{secs:02}")
    }
}

fn draw_empty(painter: &egui::Painter, card: egui::Rect, count: usize, round: Option<&Round>) {
    let has_media = count > 0;
    let complete = round.is_some_and(Round::is_complete);
    let center_x = card.center().x;

    let (title, subtitle, pill, pill_color) = if !has_media {
        (
            "READY FOR THE CRINGE?".to_owned(),
            "Drop a folder anywhere, or import one above.\nPhotos and videos. No peeking."
                .to_owned(),
            "ONE AT A TIME · NO REPEATS".to_owned(),
            ACCENT,
        )
    } else if complete {
        let total = round.map_or(count, |round| round.total);
        (
            "THAT'S THE WHOLE ROLL".to_owned(),
            format!("All {total} items have been dealt.\nStart a new round for a fresh shuffle."),
            format!("ROUND {} COMPLETE", round.map_or(1, |round| round.number)),
            GOOD,
        )
    } else {
        let number = round.map_or(1, |round| round.number);
        let total = round.map_or(count, |round| round.total);
        (
            "THE DECK IS READY".to_owned(),
            format!(
                "{count} {} ready to deal.\nHit the button below when everyone is ready.",
                if count == 1 { "item" } else { "items" }
            ),
            format!("ROUND {number} · {total} ITEMS"),
            ACCENT,
        )
    };

    painter.text(
        pos2(center_x, card.center().y - 46.0),
        Align2::CENTER_CENTER,
        title,
        theme::display_font(28.0),
        TEXT,
    );
    painter.text(
        pos2(center_x, card.center().y + 2.0),
        Align2::CENTER_CENTER,
        subtitle,
        theme::body_font(13.0),
        MUTED,
    );

    let font = theme::bold_font(9.5);
    let text = pill.to_uppercase();
    let galley = painter.layout_no_wrap(text, font, pill_color);
    let pill_rect = egui::Rect::from_center_size(
        pos2(center_x, card.center().y + 54.0),
        galley.size() + vec2(20.0, 10.0),
    );
    painter.rect(
        pill_rect,
        CornerRadius::same(99),
        Color32::from_rgba_unmultiplied(pill_color.r(), pill_color.g(), pill_color.b(), 16),
        Stroke::new(
            1.0_f32,
            Color32::from_rgba_unmultiplied(pill_color.r(), pill_color.g(), pill_color.b(), 70),
        ),
        StrokeKind::Inside,
    );
    painter.galley(pill_rect.min + vec2(10.0, 5.0), galley, pill_color);
}

fn draw_scanning(painter: &egui::Painter, card: egui::Rect, found: usize, elapsed: f32, time: f32) {
    draw_spinner(painter, card.center() - vec2(0.0, 32.0), time);

    let dots = ".".repeat(1 + (time * 2.5) as usize % 3);
    painter.text(
        pos2(card.center().x, card.center().y + 22.0),
        Align2::CENTER_CENTER,
        format!("SCANNING THE EVIDENCE{dots}"),
        theme::bold_font(11.0),
        ACCENT,
    );
    painter.text(
        pos2(card.center().x, card.center().y + 44.0),
        Align2::CENTER_CENTER,
        if found == 0 {
            format!("Looking through subfolders… {:.0}s", elapsed)
        } else {
            format!(
                "{found} {} found · {:.0}s",
                if found == 1 { "file" } else { "files" },
                elapsed
            )
        },
        theme::body_font(11.0),
        MUTED,
    );
}

fn draw_loading(painter: &egui::Painter, card: egui::Rect, time: f32, is_video: bool) {
    draw_spinner(painter, card.center() - vec2(0.0, 32.0), time);

    let dots = ".".repeat(1 + (time * 2.5) as usize % 3);
    let label = if is_video {
        format!("PREPARING THE VIDEO{dots}")
    } else {
        format!("SHUFFLING THE EVIDENCE{dots}")
    };
    let sublabel = if is_video {
        "Decoding with ffmpeg."
    } else {
        "No peeking."
    };

    painter.text(
        pos2(card.center().x, card.center().y + 22.0),
        Align2::CENTER_CENTER,
        label,
        theme::bold_font(11.0),
        ACCENT,
    );
    painter.text(
        pos2(card.center().x, card.center().y + 44.0),
        Align2::CENTER_CENTER,
        sublabel,
        theme::body_font(11.0),
        MUTED_DIM,
    );
}

fn draw_spinner(painter: &egui::Painter, center: egui::Pos2, time: f32) {
    let radius = 30.0;
    painter.circle_stroke(
        center,
        radius,
        Stroke::new(3.0_f32, Color32::from_white_alpha(18)),
    );

    let start = time * 2.4;
    let sweep = TAU * 0.82;
    let segments = 48;
    for index in 0..segments {
        let t0 = index as f32 / segments as f32;
        let t1 = (index + 1) as f32 / segments as f32;
        let angle0 = start + sweep * t0;
        let angle1 = start + sweep * t1;
        let alpha = ((t1.powf(2.2)) * 235.0) as u8;
        painter.line_segment(
            [
                center + vec2(angle0.cos(), angle0.sin()) * radius,
                center + vec2(angle1.cos(), angle1.sin()) * radius,
            ],
            Stroke::new(
                3.0_f32,
                Color32::from_rgba_unmultiplied(ACCENT.r(), ACCENT.g(), ACCENT.b(), alpha),
            ),
        );
    }

    let head = start + sweep;
    painter.circle_filled(center + vec2(head.cos(), head.sin()) * radius, 2.2, ACCENT);
}

fn fade_color(color: Color32, fade: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(
        color.r(),
        color.g(),
        color.b(),
        (color.a() as f32 * fade).round() as u8,
    )
}

fn display_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| theme::truncate_middle(name, 36))
        .unwrap_or_else(|| "that file".to_owned())
}

fn app_icon() -> egui::IconData {
    let image = image::load_from_memory(include_bytes!("../assets/PhotoRoulette-256.png"))
        .expect("the bundled app icon should be valid")
        .into_rgba8();

    egui::IconData {
        width: image.width(),
        height: image.height(),
        rgba: image.into_raw(),
    }
}

fn main() -> eframe::Result {
    let mut initial_folder = None;
    let mut smoke_test = false;
    for argument in std::env::args().skip(1) {
        match argument.as_str() {
            "-h" | "--help" => {
                print_help();
                return Ok(());
            }
            "-V" | "--version" => {
                println!("photo-roulette {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            "--smoke-test" => smoke_test = true,
            value if value.starts_with('-') => {
                eprintln!("Unknown option: {value}\n");
                print_help();
                std::process::exit(2);
            }
            value => initial_folder = Some(PathBuf::from(value)),
        }
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1000.0, 780.0])
            .with_min_inner_size([860.0, 640.0])
            .with_icon(app_icon()),
        ..Default::default()
    };

    eframe::run_native(
        "Photo Roulette",
        options,
        Box::new(move |cc| Ok(Box::new(PhotoRoulette::new(cc, initial_folder, smoke_test)))),
    )
}

fn print_help() {
    println!(
        "Photo Roulette {version}\n\
         Deal photos and videos one at a time, with no repeats in a round.\n\n\
         USAGE:\n\
         \x20   photo-roulette [FOLDER]\n\n\
         OPTIONS:\n\
         \x20   -h, --help         Show this help\n\
         \x20   -V, --version      Show the version\n\
         \x20   --smoke-test       Open, verify startup, and exit (CI)\n\n\
         CONTROLS:\n\
         \x20   Space             Deal the next item\n\
         \x20   Backspace         Go back to the previous item\n\
         \x20   Click video       Pause / play\n\
         \x20   Drag timeline     Seek\n\
         \x20   Left / Right      Seek 5 seconds\n\
         \x20   R                 Restart the current video\n\
         \x20   M                 Mute / unmute\n\
         \x20   F                 Toggle fullscreen\n\n\
         Video playback needs ffmpeg. Photos work without it.",
        version = env!("CARGO_PKG_VERSION")
    );
}
