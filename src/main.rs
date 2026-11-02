//! OpenScreenRec — a free and open-source screen recorder, 100% Rust.
//!
//! M3 build: frameless always-on-top toolbar, tray icon with menu,
//! global hotkeys (F9 record/stop, F10 pause), region picker overlay,
//! countdown, persistent settings — the recording MVP.

mod app;
mod controller;
mod recorder;

use std::time::Duration;

use eframe::egui;
use osr_capture::CaptureTarget;
use osr_common::APP_NAME;
use osr_encode::VideoCodec;
use recorder::{AudioSelection, RecordRequest};

/// Frames per second used by the smoke-test path.
const SMOKE_FPS: u32 = 30;

fn main() -> eframe::Result<()> {
    tracing_subscriber::fmt::init();
    tracing::info!(
        "starting {} v{} ({})",
        APP_NAME,
        env!("CARGO_PKG_VERSION"),
        osr_capture::backend::backend_name()
    );

    if let Some(smoke) = parse_smoke() {
        return match run_smoke(smoke) {
            Ok(()) => Ok(()),
            Err(message) => {
                eprintln!("smoke-record failed: {message}");
                std::process::exit(1);
            }
        };
    }

    let settings = osr_common::settings::Settings::load();
    let controller = controller::Controller::new();
    controller.lock().set_request(app::build_request(
        &settings,
        CaptureTarget::Monitor { id: 0 },
    ));

    // Tray and hotkeys are created on the main thread, as both crates
    // require on some platforms.
    let tray = match app::create_tray() {
        Ok(tray) => tray,
        Err(message) => {
            eprintln!("tray icon init failed: {message}");
            std::process::exit(1);
        }
    };
    let hotkeys = match app::create_hotkeys() {
        Ok(hotkeys) => hotkeys,
        Err(message) => {
            eprintln!("global hotkey init failed: {message}");
            std::process::exit(1);
        }
    };

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_decorations(false)
            .with_resizable(false)
            .with_always_on_top()
            .with_inner_size(egui::vec2(app::TOOLBAR_SIZE[0], app::TOOLBAR_SIZE[1]))
            .with_icon(app::app_icon()),
        ..Default::default()
    };

    eframe::run_native(
        APP_NAME,
        options,
        Box::new(|cc| {
            let app::TrayHandles { tray, ids, items } = tray;
            let app::HotkeyHandles {
                manager,
                record,
                pause,
            } = hotkeys;
            app::spawn_command_thread(controller.clone(), cc.egui_ctx.clone(), ids, record, pause);
            Ok(Box::new(app::App::new(
                controller,
                settings,
                tray,
                items,
                app::HotkeyHandles {
                    manager,
                    record,
                    pause,
                },
            )))
        }),
    )
}

// ---------------------------------------------------------------------------
// Smoke test: `openscreenrec --smoke-record [seconds] [x y w h]`
// ---------------------------------------------------------------------------

struct Smoke {
    secs: u64,
    region: Option<(i32, i32, u32, u32)>,
}

fn parse_smoke() -> Option<Smoke> {
    let args: Vec<String> = std::env::args().collect();
    let position = args.iter().position(|arg| arg == "--smoke-record")?;
    let secs = args
        .get(position + 1)
        .and_then(|value| value.parse().ok())
        .unwrap_or(3);
    let region = match (
        args.get(position + 2),
        args.get(position + 3),
        args.get(position + 4),
        args.get(position + 5),
    ) {
        (Some(x), Some(y), Some(w), Some(h)) => {
            match (
                x.parse::<i32>(),
                y.parse::<i32>(),
                w.parse::<u32>(),
                h.parse::<u32>(),
            ) {
                (Ok(x), Ok(y), Ok(w), Ok(h)) => Some((x, y, w, h)),
                _ => None,
            }
        }
        _ => None,
    };
    Some(Smoke { secs, region })
}

fn run_smoke(smoke: Smoke) -> Result<(), String> {
    let target = match smoke.region {
        Some((x, y, width, height)) => CaptureTarget::Region {
            x,
            y,
            width,
            height,
        },
        None => CaptureTarget::Monitor { id: 0 },
    };
    let output = osr_common::default_recordings_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(osr_common::timestamped_recording_name(".mp4"));
    let request = RecordRequest {
        target,
        fps: SMOKE_FPS,
        codec: VideoCodec::H264,
        output,
        audio: AudioSelection::System,
        countdown_secs: 0,
    };
    let session = recorder::start(request)?;
    println!(
        "Recording {} for {}s...",
        match smoke.region {
            Some((x, y, w, h)) => format!("region {x},{y} {w}x{h}"),
            None => "the primary monitor".into(),
        },
        smoke.secs
    );
    std::thread::sleep(Duration::from_secs(smoke.secs));
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
