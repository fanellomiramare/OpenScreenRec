//! OpenScreenRec — a free and open-source screen recorder, 100% Rust.
//!
//! M1 build: primary-monitor video capture (Windows) → MP4 via the
//! FFmpeg pipe encoder. Audio and the full control UX land in M2–M3.

mod recorder;

use std::time::Duration;

use eframe::egui;
use osr_encode::VideoCodec;

use recorder::RecordRequest;

/// Frames per second used until the settings UI lands (M3).
const DEFAULT_FPS: u32 = 30;

fn main() -> eframe::Result<()> {
    tracing_subscriber::fmt::init();
    tracing::info!(
        "starting {} v{} ({})",
        osr_common::APP_NAME,
        env!("CARGO_PKG_VERSION"),
        osr_capture::backend::backend_name()
    );

    if let Some(seconds) = smoke_record_seconds() {
        return match smoke_record(seconds) {
            Ok(()) => Ok(()),
            Err(message) => {
                eprintln!("smoke-record failed: {message}");
                std::process::exit(1);
            }
        };
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([460.0, 300.0])
            .with_min_inner_size([380.0, 240.0]),
        ..Default::default()
    };

    eframe::run_native(
        osr_common::APP_NAME,
        options,
        Box::new(|_cc| Ok(Box::new(App::default()))),
    )
}

/// `--smoke-record [seconds]`: record the primary monitor headlessly
/// (no UI) and exit. Used as an end-to-end pipeline check.
fn smoke_record_seconds() -> Option<u64> {
    let args: Vec<String> = std::env::args().collect();
    let position = args.iter().position(|arg| arg == "--smoke-record")?;
    Some(
        args.get(position + 1)
            .and_then(|value| value.parse().ok())
            .unwrap_or(3),
    )
}

fn smoke_record(seconds: u64) -> Result<(), String> {
    let output = osr_common::default_recordings_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(osr_common::timestamped_recording_name(".mp4"));
    let request = RecordRequest {
        monitor_id: 0,
        fps: DEFAULT_FPS,
        codec: VideoCodec::H264,
        output,
    };
    let session = recorder::start(request)?;
    println!("Recording the primary monitor for {seconds}s...");
    std::thread::sleep(Duration::from_secs(seconds));
    let path = session.join()?;
    let size = std::fs::metadata(&path)
        .map_err(|e| format!("output file missing: {e}"))?
        .len();
    println!("Saved {} ({size} bytes)", path.display());
    if size < 1_024 {
        return Err("output file is suspiciously small".into());
    }
    Ok(())
}

#[derive(Default)]
struct App {
    session: Option<recorder::Session>,
    status: Option<String>,
}

impl App {
    fn default_output_path() -> String {
        let dir = osr_common::default_recordings_dir().unwrap_or_else(std::env::temp_dir);
        dir.join(osr_common::timestamped_recording_name(".mp4"))
            .display()
            .to_string()
    }

    fn start_recording(&mut self) {
        let request = RecordRequest {
            monitor_id: 0,
            fps: DEFAULT_FPS,
            codec: VideoCodec::H264,
            output: Self::default_output_path().into(),
        };
        match recorder::start(request) {
            Ok(session) => {
                self.status = None;
                self.session = Some(session);
            }
            Err(message) => self.status = Some(format!("Failed to start: {message}")),
        }
    }

    fn poll_session(&mut self) {
        let mut finished = None;
        if let Some(session) = &mut self.session
            && let Some(result) = session.try_result()
        {
            finished = Some(result);
        }
        if let Some(result) = finished {
            self.session = None;
            self.status = Some(match result {
                Ok(path) => format!("Saved {}", path.display()),
                Err(message) => format!("Recording failed: {message}"),
            });
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll_session();
        ui.vertical_centered(|ui| {
            ui.add_space(16.0);
            ui.heading(osr_common::APP_NAME);
            ui.weak(format!(
                "v{} — 100% Rust, MIT licensed",
                env!("CARGO_PKG_VERSION")
            ));
            ui.add_space(12.0);
            ui.separator();
            ui.add_space(12.0);

            match &mut self.session {
                Some(session) => {
                    ui.label(format!(
                        "● Recording — {:.0}s @ {} fps",
                        session.elapsed().as_secs_f32(),
                        DEFAULT_FPS
                    ));
                    ui.weak(format!("→ {}", session.output().display()));
                    ui.add_space(8.0);
                    if ui.button("⏹ Stop").clicked() {
                        session.request_stop();
                    }
                    ui.ctx().request_repaint_after(Duration::from_millis(200));
                }
                None => {
                    if ui.button("● Record primary monitor").clicked() {
                        self.start_recording();
                    }
                    ui.weak(format!("{} fps → Videos folder", DEFAULT_FPS));
                }
            }

            if let Some(message) = &self.status {
                ui.add_space(12.0);
                ui.separator();
                ui.add_space(12.0);
                ui.label(message);
            }

            ui.add_space(16.0);
            ui.weak("Your recordings never leave your machine.");
        });
    }

    fn on_exit(&mut self) {
        // Never leave an ffmpeg process behind when the window closes.
        if let Some(session) = self.session.take()
            && let Ok(path) = session.join()
        {
            tracing::info!("finalized {} on exit", path.display());
        }
    }
}
