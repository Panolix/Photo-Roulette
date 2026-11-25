mod deck;
mod loader;
mod theme;
mod video;

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::time::{Duration, Instant};

use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, Id, LayerId, Layout, Margin, Order, RichText,
    Sense, Stroke, StrokeKind, Vec2, pos2, vec2,
};

use deck::Round;
use loader::LoadedPhoto;
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
}

struct PendingVideo {
    path: PathBuf,
    started: Instant,
    rx: Option<Receiver<video::VideoMessage>>,
    info: Option<video::VideoInfo>,
    first_frame: Option<egui::ColorImage>,
    error: Option<String>,
    ended: bool,
    counts_as_dealt: bool,
    start_offset: f64,
}

impl PendingVideo {
    fn start(path: PathBuf, counts_as_dealt: bool, start_offset: f64) -> Result<Self, String> {
        let load = video::start(path.clone(), start_offset)?;
        Ok(Self {
            path,
            started: Instant::now(),
            rx: Some(load.rx),
            info: None,
            first_frame: None,
            error: None,
            ended: false,
            counts_as_dealt,
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

struct PhotoRoulette {
    library: Vec<PathBuf>,
    folder_label: Option<String>,
    round: Option<Round>,
    current: Option<CurrentPhoto>,
    current_video: Option<CurrentVideo>,
    last_shown: Option<PathBuf>,
    pending: Option<PendingLoad>,
    pending_video: Option<PendingVideo>,
    generation: u64,
    tx: Sender<LoadMessage>,
    rx: Receiver<LoadMessage>,
    show_filenames: bool,
    notice: Option<Notice>,
    restart_icon: egui::TextureHandle,
}

impl PhotoRoulette {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
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

        let (tx, rx) = channel();
        let mut app = Self {
            library: Vec::new(),
            folder_label: None,
            round: None,
            current: None,
            current_video: None,
            last_shown: None,
            pending: None,
            pending_video: None,
            generation: 0,
            tx,
            rx,
            show_filenames: false,
            notice: None,
            restart_icon,
        };

        // Bonus: `photo-roulette ~/Pictures/cringe` opens straight into a folder.
        if let Some(argument) = std::env::args().nth(1)
            && !argument.starts_with('-')
        {
            app.import_roots(vec![PathBuf::from(argument)], &cc.egui_ctx);
        }

        app
    }

    fn import_roots(&mut self, roots: Vec<PathBuf>, ctx: &egui::Context) {
        if roots.is_empty() {
            return;
        }

        match loader::collect_media(&roots) {
            Ok(library) => {
                let mut media = library.media;
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
                    return;
                }

                self.generation = self.generation.wrapping_add(1);
                self.pending = None;
                self.pending_video = None;
                self.current = None;
                self.current_video = None;
                self.last_shown = None;
                self.round = None;
                self.library = media;
                self.folder_label = Some(library.label.clone());
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
                    message.push_str(" Install ffmpeg to play videos.");
                }
                self.notify(ctx, NoticeKind::Info, message);

                ctx.send_viewport_cmd(egui::ViewportCommand::Title(format!(
                    "Photo Roulette — {}",
                    library.label
                )));
                ctx.request_repaint();
            }
            Err(message) => self.notify(ctx, NoticeKind::Error, message),
        }
    }

    fn open_import(&mut self, ctx: &egui::Context) {
        let picked = rfd::FileDialog::new()
            .set_title("Choose a folder of photos and videos")
            .pick_folder();

        if let Some(folder) = picked {
            self.import_roots(vec![folder], ctx);
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
        if self.pending.is_some() || self.pending_video.is_some() {
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

        self.current = None;
        self.current_video = None;

        if loader::is_video_file(&path) {
            match PendingVideo::start(path.clone(), true, 0.0) {
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
        if let Some(round) = self.round.as_mut() {
            round.mark_unreadable();
        }

        if self.library.is_empty() {
            self.round = None;
            self.folder_label = None;
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
                self.last_shown = Some(pending.path);

                if let Some(round) = self.round.as_mut() {
                    round.mark_dealt();
                }

                ctx.request_repaint();
            }
            Err(error) => self.handle_media_failure(pending.path, error, ctx),
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
        let suspense_done = !pending.counts_as_dealt || pending.started.elapsed() >= MIN_SUSPENSE;
        if pending.error.is_none() && (!ready || !suspense_done) {
            ctx.request_repaint_after(Duration::from_millis(33));
            return;
        }

        let mut pending = self
            .pending_video
            .take()
            .expect("pending video was just checked");
        let counts_as_dealt = pending.counts_as_dealt;
        if let Some(error) = pending.error.take() {
            if counts_as_dealt {
                self.handle_media_failure(pending.path.clone(), error, ctx);
            } else {
                eprintln!("Photo Roulette: video replay failed: {error}");
                self.notify(
                    ctx,
                    NoticeKind::Error,
                    "Couldn't replay this video.".to_owned(),
                );
            }
            return;
        }

        let (Some(mut info), Some(first_frame), Some(rx)) = (
            pending.info.take(),
            pending.first_frame.take(),
            pending.rx.take(),
        ) else {
            if counts_as_dealt {
                self.handle_media_failure(
                    pending.path.clone(),
                    "the video decoder returned no frames".to_owned(),
                    ctx,
                );
            } else {
                self.notify(
                    ctx,
                    NoticeKind::Error,
                    "Couldn't replay this video.".to_owned(),
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
        let audio = audio_path.as_deref().and_then(video::play_audio);

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
        self.last_shown = Some(pending.path.clone());

        if counts_as_dealt && let Some(round) = self.round.as_mut() {
            round.mark_dealt();
        }

        ctx.request_repaint();
    }

    fn replay_video(&mut self, ctx: &egui::Context) {
        if self.pending.is_some() || self.pending_video.is_some() {
            return;
        }
        let Some(path) = self.current_video.as_ref().map(|video| video.path.clone()) else {
            return;
        };

        match PendingVideo::start(path, false, 0.0) {
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

        match PendingVideo::start(path, false, target) {
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
            self.import_roots(dropped, ctx);
        }

        let focused = ctx.memory(|memory| memory.focused().is_some());
        let (
            space,
            replay_key,
            pause_key,
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

        if (seek_left || seek_right)
            && self.pending.is_none()
            && self.pending_video.is_none()
            && let Some(video) = &self.current_video
            && !video.scrubbing
        {
            let delta = if seek_left { -5.0 } else { 5.0 };
            self.seek_video(ctx, video.elapsed() + delta);
        }

        if space && !focused && !self.library.is_empty() {
            self.deal(ctx);
        }
    }

    fn top_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("top-bar")
            .resizable(false)
            .frame(egui::Frame::NONE.fill(BG).inner_margin(Margin {
                left: 30,
                right: 30,
                top: 24,
                bottom: 12,
            }))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 0.0;
                            ui.label(
                                RichText::new("PHOTO ")
                                    .font(theme::display_font(34.0))
                                    .color(TEXT)
                                    .extra_letter_spacing(0.6),
                            );
                            ui.label(
                                RichText::new("ROULETTE")
                                    .font(theme::display_font(34.0))
                                    .color(ACCENT)
                                    .extra_letter_spacing(0.6),
                            );
                        });
                        ui.label(
                            RichText::new("Deal the evidence. Photos and videos, one at a time.")
                                .font(theme::body_font(13.5))
                                .color(MUTED),
                        );
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
                    let enabled = self.pending.is_none() && self.pending_video.is_none();
                    let label = self.primary_label();
                    let response = primary_button(ui, label, vec2(430.0, 56.0), enabled);
                    if response.clicked() {
                        response.surrender_focus();
                        self.deal(ctx);
                    }

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

                    if let Some(folder) = &self.folder_label {
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new(format!("· {}", theme::truncate_middle(folder, 26)))
                                .font(theme::body_font(11.0))
                                .color(MUTED),
                        );
                    }
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
                    let widgets = &mut ui.style_mut().visuals.widgets;
                    for widget in [
                        &mut widgets.inactive,
                        &mut widgets.hovered,
                        &mut widgets.active,
                    ] {
                        widget.corner_radius = CornerRadius::same(4);
                    }
                    let response = ui.checkbox(
                        &mut self.show_filenames,
                        RichText::new("SHOW FILE NAMES")
                            .font(theme::bold_font(10.0))
                            .color(MUTED)
                            .extra_letter_spacing(1.0),
                    );
                    if response.clicked() {
                        response.surrender_focus();
                    }
                });
            });
        });
    }

    fn primary_label(&self) -> &'static str {
        if self.pending.is_some() || (self.pending_video.is_some() && self.current_video.is_none())
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
        if self.pending.is_some() {
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
                "CLICK VIDEO TO PLAY · DRAG TO SEEK · R TO RESTART · SPACE NEXT"
            } else {
                "CLICK VIDEO TO PAUSE · DRAG TO SEEK · R TO RESTART · SPACE NEXT"
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
                top: 0,
                bottom: 2,
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
                if self.pending.is_some() || (self.pending_video.is_some() && !buffering) {
                    let time = ctx.input(|input| input.time) as f32;
                    draw_loading(&painter, card, time, self.pending_video.is_some());
                } else if let Some(video) = &self.current_video {
                    let action = draw_video(
                        ui,
                        &painter,
                        card,
                        video,
                        self.show_filenames,
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
                    draw_photo(ui, &painter, card, current, self.show_filenames, ctx);
                } else {
                    draw_empty(
                        &painter,
                        card,
                        self.library.len(),
                        self.folder_label.as_deref(),
                        self.round.as_ref(),
                    );
                }

                self.draw_notice(&painter, card, ctx);
            });
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
        self.poll_image_load(ctx);
        self.poll_video_load(ctx);
        self.update_current_video(ctx);
        self.handle_input(ctx);

        self.top_bar(ctx);
        self.bottom_bar(ctx);
        self.stage(ctx);
        self.drop_overlay(ctx);
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
                    .font(theme::bold_font(10.5))
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

fn draw_empty(
    painter: &egui::Painter,
    card: egui::Rect,
    count: usize,
    folder: Option<&str>,
    round: Option<&Round>,
) {
    let has_media = count > 0;
    let complete = round.is_some_and(Round::is_complete);

    let group_top = card.center().y - 150.0;
    let stack_center = pos2(card.center().x, group_top + 82.0);

    stack_cards(
        painter,
        stack_center,
        Color32::from_rgb(29, 29, 39),
        Color32::from_rgba_unmultiplied(ACCENT.r(), ACCENT.g(), ACCENT.b(), 90),
    );
    draw_photo_icon(painter, stack_center, ACCENT);

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
                "{count} items from {}.\nHit the button below when everyone is ready.",
                theme::truncate_middle(folder.unwrap_or("your folder"), 36)
            ),
            format!("ROUND {number} · {total} ITEMS"),
            ACCENT,
        )
    };

    painter.text(
        pos2(card.center().x, group_top + 218.0),
        Align2::CENTER_CENTER,
        title,
        theme::display_font(28.0),
        TEXT,
    );
    painter.text(
        pos2(card.center().x, group_top + 258.0),
        Align2::CENTER_CENTER,
        subtitle,
        theme::body_font(13.0),
        MUTED,
    );

    let font = theme::bold_font(9.5);
    let text = pill.to_uppercase();
    let galley = painter.layout_no_wrap(text, font, pill_color);
    let pill_rect = egui::Rect::from_center_size(
        pos2(card.center().x, group_top + 300.0),
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

fn draw_loading(painter: &egui::Painter, card: egui::Rect, time: f32, is_video: bool) {
    let center = card.center() - vec2(0.0, 16.0);
    draw_shuffle_wheel(painter, center, 140.0, time);

    let bob = (time * 2.2).sin() * 3.0;
    let stack_center = center + vec2(0.0, bob);
    stack_cards(
        painter,
        stack_center,
        Color32::from_rgb(30, 30, 41),
        Color32::from_rgba_unmultiplied(ACCENT.r(), ACCENT.g(), ACCENT.b(), 110),
    );
    painter.text(
        stack_center,
        Align2::CENTER_CENTER,
        "?",
        theme::display_font(62.0),
        ACCENT,
    );

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
        pos2(card.center().x, card.center().y + 166.0),
        Align2::CENTER_CENTER,
        label,
        theme::bold_font(11.0),
        ACCENT,
    );
    painter.text(
        pos2(card.center().x, card.center().y + 188.0),
        Align2::CENTER_CENTER,
        sublabel,
        theme::body_font(11.0),
        MUTED_DIM,
    );
}

fn draw_shuffle_wheel(painter: &egui::Painter, center: egui::Pos2, radius: f32, time: f32) {
    painter.circle_stroke(
        center,
        radius,
        Stroke::new(1.0_f32, Color32::from_white_alpha(18)),
    );

    for i in 0..36 {
        let angle = i as f32 / 36.0 * TAU - time * 0.1;
        let length = if i % 3 == 0 { 9.0 } else { 4.0 };
        let p0 = center + vec2(angle.cos(), angle.sin()) * (radius - 1.0);
        let p1 = center + vec2(angle.cos(), angle.sin()) * (radius + length);
        painter.line_segment(
            [p0, p1],
            Stroke::new(
                1.0_f32,
                Color32::from_white_alpha(if i % 3 == 0 { 34 } else { 18 }),
            ),
        );
    }

    let head = (time * 1.8) % TAU;
    for i in 0..16 {
        let t = i as f32 / 15.0;
        let angle = head - t * 0.8;
        let alpha = ((1.0 - t).powi(3) * 240.0) as u8;
        let p0 = center + vec2(angle.cos(), angle.sin()) * (radius - 13.0);
        let p1 = center + vec2((angle + 0.055).cos(), (angle + 0.055).sin()) * (radius + 4.0);
        painter.line_segment(
            [p0, p1],
            Stroke::new(
                2.5_f32,
                Color32::from_rgba_unmultiplied(ACCENT.r(), ACCENT.g(), ACCENT.b(), alpha),
            ),
        );
    }

    let head_pos = center + vec2(head.cos(), head.sin()) * radius;
    painter.circle_filled(head_pos, 3.2, ACCENT);
}

fn stack_cards(
    painter: &egui::Painter,
    center: egui::Pos2,
    front_fill: Color32,
    front_border: Color32,
) {
    let back = egui::Rect::from_center_size(center + vec2(-38.0, 18.0), vec2(204.0, 146.0));
    painter.rect(
        back,
        CornerRadius::same(14),
        Color32::from_rgb(16, 16, 22),
        Stroke::new(1.0_f32, theme::BORDER_SOFT),
        StrokeKind::Inside,
    );

    let middle = egui::Rect::from_center_size(center + vec2(34.0, 9.0), vec2(208.0, 150.0));
    painter.rect(
        middle,
        CornerRadius::same(15),
        CARD_BACK,
        Stroke::new(1.0_f32, BORDER),
        StrokeKind::Inside,
    );

    let front = egui::Rect::from_center_size(center, vec2(218.0, 164.0));
    painter.rect(
        front,
        CornerRadius::same(16),
        front_fill,
        Stroke::new(1.0_f32, front_border),
        StrokeKind::Inside,
    );
}

fn draw_photo_icon(painter: &egui::Painter, center: egui::Pos2, color: Color32) {
    let rect = egui::Rect::from_center_size(center, vec2(64.0, 48.0));
    painter.rect(
        rect,
        CornerRadius::same(6),
        Color32::from_rgb(14, 14, 20),
        Stroke::new(1.6_f32, color),
        StrokeKind::Inside,
    );

    let mountain = |points: [egui::Pos2; 3], alpha: u8| {
        painter.add(egui::Shape::convex_polygon(
            points.to_vec(),
            Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha),
            Stroke::NONE,
        ));
    };

    mountain(
        [
            pos2(rect.left() + 7.0, rect.bottom() - 8.0),
            pos2(rect.left() + 25.0, rect.top() + 19.0),
            pos2(rect.left() + 43.0, rect.bottom() - 8.0),
        ],
        120,
    );
    mountain(
        [
            pos2(rect.left() + 27.0, rect.bottom() - 8.0),
            pos2(rect.left() + 42.0, rect.top() + 23.0),
            pos2(rect.right() - 7.0, rect.bottom() - 8.0),
        ],
        75,
    );
    painter.circle_filled(pos2(rect.right() - 13.0, rect.top() + 13.0), 3.8, color);
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
        Box::new(|cc| Ok(Box::new(PhotoRoulette::new(cc)))),
    )
}
