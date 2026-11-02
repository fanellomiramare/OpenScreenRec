//! The OpenScreenRec application: a frameless always-on-top toolbar, an
//! in-place settings view, a snapshot-based region picker, the tray
//! icon and the global hotkeys.
//!
//! Recording control is shared with the tray/hotkey command thread via
//! [`crate::controller::Controller`]; closing the window hides it (the
//! tray keeps working), quitting belongs to the tray menu.
//!
//! Threading note: muda menu item handles are main-thread only, so they
//! live here; the command thread only carries the `MenuId`s (Send) and
//! this view re-syncs the labels whenever the recording state changes.

use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use eframe::egui;
use eframe::egui::{Color32, Context, FontId, Id, Pos2, Rect, Sense, StrokeKind, pos2, vec2};
use global_hotkey::GlobalHotKeyEvent;
use global_hotkey::GlobalHotKeyManager;
use global_hotkey::hotkey::{Code, HotKey};
use osr_capture::CaptureTarget;
use osr_common::APP_NAME;
use osr_common::settings::Settings;
use osr_encode::VideoCodec;
use tray_icon::TrayIcon;
use tray_icon::TrayIconBuilder;
use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};

use crate::controller::Controller;
use crate::recorder::{AudioSelection, RecordRequest};

/// The flat pastel record-dot icon (see tools/gen_icon.py).
const ICON_APP_PNG: &[u8] = include_bytes!("../assets/icon-256.png");
const ICON_TRAY_PNG: &[u8] = include_bytes!("../assets/tray-32.png");

pub const TOOLBAR_SIZE: [f32; 2] = [320.0, 88.0];
const SETTINGS_SIZE: [f32; 2] = [320.0, 430.0];

#[derive(Clone, Copy, PartialEq, Eq)]
enum View {
    Toolbar,
    Settings,
    RegionSelect,
    /// macOS: screen recording permission not granted yet (TCC).
    PermissionNeeded,
}

enum AppEvent {
    Snapshot(Result<egui::ColorImage, String>),
}

struct OverlayState {
    texture: egui::TextureHandle,
    drag_start: Option<Pos2>,
    drag_end: Option<Pos2>,
}

/// Build the next recording request from the current settings.
pub fn build_request(settings: &Settings, target: CaptureTarget) -> RecordRequest {
    let audio = match (settings.microphone, settings.system_audio) {
        (true, true) => AudioSelection::Both,
        (true, false) => AudioSelection::Microphone,
        (false, true) => AudioSelection::System,
        (false, false) => AudioSelection::None,
    };
    let output_dir = settings
        .output_dir
        .clone()
        .unwrap_or_else(|| osr_common::default_recordings_dir().unwrap_or_else(std::env::temp_dir));
    RecordRequest {
        target,
        fps: settings.fps.clamp(5, 120),
        codec: VideoCodec::H264,
        output: output_dir.join(osr_common::timestamped_recording_name(".mp4")),
        audio,
        countdown_secs: settings.countdown_secs,
    }
}

// ---------------------------------------------------------------------------
// Tray + hotkeys (created on the main thread, as both crates require)
// ---------------------------------------------------------------------------

/// Menu ids only: these cross into the command thread (`MenuId` is Send).
pub struct TrayIds {
    pub record: MenuId,
    pub pause: MenuId,
    pub open: MenuId,
    pub quit: MenuId,
}

/// Menu item handles: main-thread only (muda is `Rc`-backed).
pub struct TrayMenuItems {
    record_item: MenuItem,
    pause_item: MenuItem,
    synced_recording: Option<bool>,
}

pub struct TrayHandles {
    pub tray: TrayIcon,
    pub ids: TrayIds,
    pub items: TrayMenuItems,
}

/// Tray icon with the recording menu.
pub fn create_tray() -> Result<TrayHandles, String> {
    let ids = TrayIds {
        record: MenuId::new("osr-record"),
        pause: MenuId::new("osr-pause"),
        open: MenuId::new("osr-open"),
        quit: MenuId::new("osr-quit"),
    };

    let record_item = MenuItem::with_id(ids.record.clone(), "Record (F9)", true, None);
    let pause_item = MenuItem::with_id(ids.pause.clone(), "Pause/Resume (F10)", false, None);
    let open_item = MenuItem::with_id(ids.open.clone(), "Open", true, None);
    let quit_item = MenuItem::with_id(ids.quit.clone(), "Quit", true, None);

    let menu = Menu::new();
    menu.append_items(&[
        &record_item,
        &pause_item,
        &PredefinedMenuItem::separator(),
        &open_item,
        &quit_item,
    ])
    .map_err(|e| e.to_string())?;

    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip("OpenScreenRec — F9 record, F10 pause")
        .with_icon(tray_icon_rgba())
        .build()
        .map_err(|e| e.to_string())?;

    Ok(TrayHandles {
        tray,
        ids,
        items: TrayMenuItems {
            record_item,
            pause_item,
            synced_recording: None,
        },
    })
}

/// The tray variant: the bare coral dot on transparency.
fn tray_icon_rgba() -> tray_icon::Icon {
    let image = image::load_from_memory(ICON_TRAY_PNG)
        .expect("the embedded tray icon is a valid PNG")
        .into_rgba8();
    let (width, height) = image.dimensions();
    tray_icon::Icon::from_rgba(image.into_raw(), width, height).expect("static icon data is valid")
}

/// Decode an embedded PNG into the egui window icon.
pub fn app_icon() -> egui::IconData {
    let image = image::load_from_memory(ICON_APP_PNG)
        .expect("the embedded app icon is a valid PNG")
        .into_rgba8();
    let (width, height) = image.dimensions();
    egui::IconData {
        width,
        height,
        rgba: image.into_raw(),
    }
}

pub struct HotkeyHandles {
    pub manager: GlobalHotKeyManager,
    pub record: HotKey,
    pub pause: HotKey,
}

pub fn create_hotkeys() -> Result<HotkeyHandles, String> {
    let manager = GlobalHotKeyManager::new().map_err(|e| e.to_string())?;
    let record = HotKey::new(None, Code::F9);
    let pause = HotKey::new(None, Code::F10);
    manager.register(record).map_err(|e| e.to_string())?;
    manager.register(pause).map_err(|e| e.to_string())?;
    Ok(HotkeyHandles {
        manager,
        record,
        pause,
    })
}

/// Polls tray menu and global hotkey events; recording actions go
/// through the shared controller, window actions through egui commands.
pub fn spawn_command_thread(
    controller: Controller,
    ctx: Context,
    ids: TrayIds,
    record_key: HotKey,
    pause_key: HotKey,
) {
    std::thread::Builder::new()
        .name("osr-commands".into())
        .spawn(move || {
            loop {
                for event in MenuEvent::receiver().try_iter() {
                    if event.id == ids.record {
                        toggle_record(&controller, &ctx);
                    } else if event.id == ids.pause {
                        controller.lock().toggle_pause();
                    } else if event.id == ids.open {
                        show_window(&ctx);
                    } else if event.id == ids.quit {
                        controller.lock().stop_blocking();
                        controller.request_quit();
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        return;
                    }
                }
                for event in GlobalHotKeyEvent::receiver().try_iter() {
                    if event.id() == record_key.id() {
                        toggle_record(&controller, &ctx);
                    } else if event.id() == pause_key.id() {
                        controller.lock().toggle_pause();
                    }
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        })
        .expect("command thread spawns");
}

fn toggle_record(controller: &Controller, ctx: &Context) {
    if let Err(message) = controller.lock().toggle_record() {
        tracing::error!("failed to toggle recording: {message}");
    }
    ctx.request_repaint();
}

fn show_window(ctx: &Context) {
    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
}

// ---------------------------------------------------------------------------
// Application
// ---------------------------------------------------------------------------

pub struct App {
    controller: Controller,
    settings: Settings,
    view: View,
    sized_view: bool,
    status: Option<String>,
    events_tx: Sender<AppEvent>,
    events_rx: Receiver<AppEvent>,
    overlay: Option<OverlayState>,
    menu_items: TrayMenuItems,
    _tray: TrayIcon,
    _hotkeys: HotkeyHandles,
}

impl App {
    pub fn new(
        controller: Controller,
        settings: Settings,
        tray: TrayIcon,
        items: TrayMenuItems,
        hotkeys: HotkeyHandles,
    ) -> Self {
        let (events_tx, events_rx) = channel();
        let view = if osr_capture::backend::screen_recording_granted() {
            View::Toolbar
        } else {
            View::PermissionNeeded
        };
        Self {
            controller,
            settings,
            view,
            sized_view: false,
            status: None,
            events_tx,
            events_rx,
            overlay: None,
            menu_items: items,
            _tray: tray,
            _hotkeys: hotkeys,
        }
    }

    fn set_view(&mut self, ctx: &Context, view: View) {
        self.view = view;
        self.sized_view = true;
        let size = match view {
            View::Settings => SETTINGS_SIZE,
            View::PermissionNeeded => [320.0, 200.0],
            _ => TOOLBAR_SIZE,
        };
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(vec2(size[0], size[1])));
    }

    /// Keep the tray menu labels in step with the recording state
    /// (main-thread only, and only when the state actually changed).
    fn sync_menu(&mut self, recording: bool) {
        if self.menu_items.synced_recording != Some(recording) {
            self.menu_items.record_item.set_text(if recording {
                "Stop recording (F9)"
            } else {
                "Record (F9)"
            });
            self.menu_items.pause_item.set_enabled(recording);
            self.menu_items.synced_recording = Some(recording);
        }
    }

    fn begin_region_select(&mut self, ctx: &Context) {
        if self.controller.lock().is_recording() {
            return;
        }
        self.view = View::RegionSelect;
        self.overlay = None;
        // Hide first so the window does not appear inside the snapshot.
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        let tx = self.events_tx.clone();
        let repaint = ctx.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(250));
            let event = AppEvent::Snapshot(match osr_capture::grab_monitor_frame(0) {
                Ok(frame) => {
                    let mut rgba = frame.data;
                    for pixel in rgba.as_chunks_mut::<4>().0 {
                        pixel.swap(0, 2); // BGRA -> RGBA
                    }
                    Ok(egui::ColorImage::from_rgba_unmultiplied(
                        [frame.width as usize, frame.height as usize],
                        &rgba,
                    ))
                }
                Err(e) => Err(e.to_string()),
            });
            let _ = tx.send(event);
            repaint.request_repaint();
        });
    }

    fn poll_app_events(&mut self, ctx: &Context) {
        while let Ok(event) = self.events_rx.try_recv() {
            match event {
                AppEvent::Snapshot(Ok(image)) => {
                    let texture =
                        ctx.load_texture("region-snapshot", image, egui::TextureOptions::default());
                    self.overlay = Some(OverlayState {
                        texture,
                        drag_start: None,
                        drag_end: None,
                    });
                    ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(true));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
                AppEvent::Snapshot(Err(message)) => {
                    self.status = Some(format!("Region snapshot failed: {message}"));
                    self.exit_overlay(ctx);
                }
            }
        }
    }

    fn exit_overlay(&mut self, ctx: &Context) {
        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(vec2(
            TOOLBAR_SIZE[0],
            TOOLBAR_SIZE[1],
        )));
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        self.overlay = None;
        self.view = View::Toolbar;
    }

    fn toolbar_ui(&mut self, ui: &mut egui::Ui) {
        let view = self.controller.lock().view();
        let ctx = ui.ctx().clone();

        ui.add_space(4.0);
        ui.horizontal(|ui| {
            // The header doubles as the drag handle for the frameless window.
            let title = ui.label(egui::RichText::new(APP_NAME).strong());
            let drag = ui.interact(title.rect, Id::unique("osr-title-drag"), Sense::drag());
            if drag.drag_started() {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("Hide").clicked() {
                    ui.ctx()
                        .send_viewport_cmd(egui::ViewportCommand::Visible(false));
                }
                if !view.recording && ui.small_button("Settings").clicked() {
                    self.set_view(&ctx, View::Settings);
                }
            });
        });
        ui.separator();

        ui.horizontal(|ui| {
            if view.recording {
                if let Some(left) = view.countdown {
                    ui.label(format!("Starting in {:.0}s…", left.as_secs_f32().ceil()));
                } else if view.paused {
                    ui.label(egui::RichText::new("Paused").strong());
                } else {
                    ui.label(
                        egui::RichText::new(format!(
                            "Recording {:.0}s @ {} fps",
                            view.elapsed.as_secs_f32(),
                            self.settings.fps
                        ))
                        .strong()
                        .color(Color32::from_rgb(226, 72, 72)),
                    );
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .button(if view.paused { "Resume" } else { "Pause" })
                        .clicked()
                    {
                        self.controller.lock().toggle_pause();
                    }
                    if ui.button("Stop").clicked() {
                        self.controller.lock().request_stop();
                    }
                });
            } else {
                if ui.button("Record").clicked()
                    && let Err(message) = self.controller.lock().toggle_record()
                {
                    self.status = Some(format!("Failed to start: {message}"));
                }
                if ui.button("Select region").clicked() {
                    self.begin_region_select(ui.ctx());
                }
                ui.weak(format!("{} fps", self.settings.fps));
            }
        });

        let message = view
            .last_result
            .map(|result| match result {
                Ok(path) => format!("Saved {}", path.display()),
                Err(error) => format!("Recording failed: {error}"),
            })
            .or_else(|| self.status.clone());
        if let Some(message) = message {
            ui.separator();
            ui.add(egui::Label::new(egui::RichText::new(message).weak()).truncate());
        }
    }

    fn settings_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if ui.button("Back").clicked() {
                self.set_view(&ctx, View::Toolbar);
                return;
            }
            ui.label(egui::RichText::new("Settings").strong());
        });
        ui.separator();

        ui.add(
            egui::Slider::new(&mut self.settings.fps, 15..=60)
                .step_by(5.0)
                .suffix(" fps"),
        );
        ui.add(
            egui::Slider::new(&mut self.settings.countdown_secs, 0..=10)
                .suffix(" s")
                .text("countdown before recording (0 = off)"),
        );
        ui.checkbox(&mut self.settings.microphone, "Record microphone");
        ui.checkbox(&mut self.settings.system_audio, "Record system audio");

        ui.horizontal(|ui| {
            ui.label("Folder:");
            let shown = self.settings.output_dir.clone().unwrap_or_else(|| {
                osr_common::default_recordings_dir().unwrap_or_else(std::env::temp_dir)
            });
            ui.add(
                egui::Label::new(egui::RichText::new(shown.display().to_string()).weak())
                    .truncate(),
            );
        });
        if ui.button("Change folder…").clicked()
            && let Some(dir) = rfd::FileDialog::new()
                .set_title("Choose where recordings are saved")
                .pick_folder()
        {
            self.settings.output_dir = Some(dir);
        }

        ui.add_space(6.0);
        ui.weak("Hotkeys: F9 record/stop · F10 pause/resume");
        if ui.button("Save").clicked() {
            self.settings.save();
            let request = build_request(&self.settings, CaptureTarget::Monitor { id: 0 });
            self.controller.lock().set_request(request);
            self.set_view(&ctx, View::Toolbar);
        }
    }

    fn region_ui(&mut self, ui: &mut egui::Ui) {
        let Some(overlay) = self.overlay.as_mut() else {
            return; // still preparing the snapshot
        };
        let screen = ui.max_rect();
        ui.painter().image(
            overlay.texture.id(),
            screen,
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );
        ui.painter()
            .rect_filled(screen, 0.0, Color32::from_black_alpha(110));

        let response = ui.allocate_rect(screen, Sense::drag());
        if response.drag_started() {
            overlay.drag_start = response.interact_pointer_pos();
            overlay.drag_end = None;
        }
        if overlay.drag_start.is_some() {
            overlay.drag_end = response.interact_pointer_pos();
        }

        let selection = match (overlay.drag_start, overlay.drag_end) {
            (Some(start), Some(end)) if start != end => Some(Rect::from_two_pos(start, end)),
            _ => None,
        };
        if let Some(rect) = selection {
            ui.painter()
                .rect_filled(rect, 0.0, Color32::from_white_alpha(24));
            ui.painter().rect_stroke(
                rect,
                0.0,
                egui::Stroke::new(2.0, Color32::from_rgb(255, 200, 60)),
                StrokeKind::Middle,
            );
            ui.painter().text(
                rect.left_top() + vec2(6.0, 6.0),
                egui::Align2::LEFT_TOP,
                format!("{} × {} px", rect.width() as i32, rect.height() as i32),
                FontId::default(),
                Color32::WHITE,
            );
        }
        ui.painter().text(
            screen.left_top() + vec2(12.0, 12.0),
            egui::Align2::LEFT_TOP,
            "Drag to select the area to record — ESC cancels",
            FontId::default(),
            Color32::WHITE,
        );

        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.exit_overlay(ui.ctx());
            return;
        }

        if response.drag_stopped()
            && let Some(rect) = selection
            && rect.width() >= 8.0
            && rect.height() >= 8.0
        {
            // egui works in points; the capture backend wants physical pixels.
            let scale = ui.ctx().native_pixels_per_point().unwrap_or(1.0);
            let target = CaptureTarget::Region {
                x: (rect.left() * scale).round() as i32,
                y: (rect.top() * scale).round() as i32,
                width: (rect.width() * scale).round() as u32,
                height: (rect.height() * scale).round() as u32,
            };
            self.controller
                .lock()
                .set_request(build_request(&self.settings, target));
            self.exit_overlay(ui.ctx());
            if let Err(message) = self.controller.lock().start() {
                self.status = Some(format!("Failed to start: {message}"));
            }
        }
    }

    fn permission_ui(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        ui.label(egui::RichText::new("Screen recording permission").strong());
        ui.add_space(2.0);
        ui.label(
            "macOS needs you to allow OpenScreenRec in System Settings → Privacy & Security → Screen Recording.",
        );
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui.button("Grant access…").clicked() {
                osr_capture::backend::request_screen_recording_permission();
            }
            if ui.button("Check again").clicked()
                && osr_capture::backend::screen_recording_granted()
            {
                self.status = Some("Screen recording permission granted.".into());
                self.set_view(ui.ctx(), View::Toolbar);
            }
        });
        ui.add_space(6.0);
        ui.weak("If capturing still fails after granting, quit (tray → Quit) and reopen the app — macOS applies the change then.");
        ui.weak("Microphone access is requested automatically the first time you enable it.");
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Close-to-tray: hide instead of quitting — unless a real exit
        // was requested from the tray menu.
        if ui.ctx().input(|i| i.viewport().close_requested()) && !self.controller.is_quitting() {
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }

        self.poll_app_events(ui.ctx());
        let recording = {
            let mut state = self.controller.lock();
            state.poll();
            state.is_recording()
        };
        self.sync_menu(recording);

        // Apply the per-view window size once, after the viewport exists.
        if !self.sized_view {
            self.sized_view = true;
            let size = match self.view {
                View::Settings => SETTINGS_SIZE,
                View::PermissionNeeded => [320.0, 200.0],
                _ => TOOLBAR_SIZE,
            };
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::InnerSize(vec2(size[0], size[1])));
        }

        // macOS TCC: poll the permission while the user is approving it.
        if self.view == View::PermissionNeeded && osr_capture::backend::screen_recording_granted() {
            self.status = Some("Screen recording permission granted.".into());
            self.set_view(ui.ctx(), View::Toolbar);
        }

        match self.view {
            View::Toolbar => self.toolbar_ui(ui),
            View::Settings => self.settings_ui(ui),
            View::RegionSelect => self.region_ui(ui),
            View::PermissionNeeded => self.permission_ui(ui),
        }

        let keep_polling = recording || self.view == View::PermissionNeeded;
        if keep_polling {
            ui.ctx().request_repaint_after(Duration::from_millis(200));
        }
    }

    fn on_exit(&mut self) {
        self.controller.lock().stop_blocking();
    }
}
