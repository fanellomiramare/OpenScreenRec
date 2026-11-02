# Architecture

OpenScreenRec is a Cargo workspace in the RustDesk style: one binary crate (the application) plus focused library crates, one concern each.

## Pipeline

```
                    ┌─────────────┐
 (screen, windows)─▶│ osr-capture │──▶ VideoFrame (BGRA, µs pts)
                    └─────────────┘          │
                    ┌─────────────┐          ▼
   (mic, system)───▶│ osr-audio   │──▶  osr-encode ──▶ osr-mux ──▶ file
                    └─────────────┘   (FFmpeg pipe (MP4 / MKV / WebM)
                                       today, HW
                                       encoders later)
       osr-common: config types, well-known paths, shared helpers
       src/: the egui application — UI, hotkeys, tray, orchestration
```

All A/V timestamps live on one monotonic microsecond clock, which makes mixing, pause/resume and A/V sync trivial to reason about.

## Crates

| Crate                      | Responsibility                                                      |
| -------------------------- | ------------------------------------------------------------------- |
| `openscreenrec` (`src/`)   | egui application: UI, hotkeys, tray, orchestration                  |
| `osr-common`               | config types, well-known paths, pipeline clock, shared A/V types    |
| `osr-capture`              | `FrameSource` trait + per-OS capture backends                       |
| `osr-audio`                | mic & system-loopback capture, mixing                               |
| `osr-encode`               | video/audio compression                                             |
| `osr-mux`                  | container writing (MP4, MKV/WebM)                                   |

## Current status (M5 — all three platforms)

- **Windows video**: live — Windows Graphics Capture via `windows-capture`; full-monitor capture and region capture (the monitor is captured, each frame is cropped on the GPU via `Frame::buffer_crop`).
- **macOS video**: live — ScreenCaptureKit via `screencapturekit` (macOS 12.3+); monitor capture and region capture (each frame cropped in software off the locked pixel buffer), with TCC preflight/request exposed to the UI.
- **Linux video**: live — X11 via `x11rb` (pure Rust, pull-based XGetImage, server-side region crop) and Wayland via the desktop portal (`ashpd`) + PipeWire (`pipewire`), chosen per session (`WAYLAND_DISPLAY` vs `DISPLAY`). The X11 backend is functionally tested in CI under Xvfb.
- **Audio**: live — microphone via `cpal` (all platforms); system audio via WASAPI loopback (`wasapi`, Windows), ScreenCaptureKit audio (macOS) and the PulseAudio monitor (`libpulse-simple`, Linux — under PipeWire desktops the monitor is provided by `pipewire-pulse`).
  - Windows: an idle render endpoint is suspended, so loopback capture would deliver nothing at all; the backend runs a silent keep-alive render stream on the same endpoint to keep it active.
- **Encoding**: live — the FFmpeg pipe encoder turns BGRA frames into MP4 (H.264); audio lands in temporary WAV files and the mux step combines them (video stream-copied, audio AAC). The FFmpeg binary stays external, so no license is inherited.
- **Pause/resume**: live — freezing both timelines at once keeps A/V aligned, and paused time (and countdown time) leaves the output entirely.
- **UX**: frameless always-on-top toolbar (egui), close-to-tray with a tray menu (`tray-icon`, ksni backend on Linux), global hotkeys F9/F10 (`global-hotkey`), snapshot-based region picker, countdown, persistent settings, and a macOS permission view (preflight → system prompt → re-check loop).
- **Distribution**: Windows — portable zip + NSIS installer (icon embedded in the exe); macOS — `.app` inside `.dmg`, ad-hoc signed or with the project's stable self-signed certificate (secrets) — see [MACOS.md](MACOS.md); Linux — AppImage (self-contained), `.deb` and `.rpm` (depend on the audio/video stack; FFmpeg stays an external runtime dependency), plus a Homebrew cask for macOS installs without Gatekeeper.
- The recorder holds a constant output FPS by duplicating the last frame when the screen doesn't repaint — durations match recorded wall-clock time on static screens.

## Per-OS plan

| OS      | Capture API                                        | System audio        | Hardware encode              | Packaging                    |
| ------- | -------------------------------------------------- | ------------------- | ---------------------------- | ---------------------------- |
| Windows | Windows Graphics Capture (`windows-capture`)       | WASAPI loopback     | Media Foundation / NVENC / QSV / AMF | installer + portable zip |
| macOS   | ScreenCaptureKit (`screencapturekit`)              | ScreenCaptureKit audio | VideoToolbox              | notarized dmg (AS + Intel)   |
| Linux   | XSHM (X11) · desktop portal + PipeWire (Wayland)   | Pulse/PipeWire monitor | VAAPI / NVENC             | AppImage, deb, rpm, Flatpak  |

## Design rules

- The UI never touches OS capture APIs directly — only the traits in `libs/`.
- OS-specific code is `cfg`-gated inside its crate, one module per platform.
- Encoding ships as an external FFmpeg process over pipes for M1–M3 (keeps the license permissive and iteration fast); native hardware encoders arrive in M6, at which point `osr-mux` takes over container writing.
- Frame rate limiting belongs to the pipeline, not to capture backends: backends deliver at the OS compositor's pace.
- Dependencies must stay MIT-compatible (enforced by `cargo-deny` in CI); design inspiration is always credited (README → Credits & references).

## Versioning & releases

Conventional Commits → release-plz opens a Release PR (version bump + CHANGELOG) → merging it tags `vX.Y.Z` and publishes the GitHub Release → the Release workflow attaches per-OS artifacts. Details in [CONTRIBUTING.md](../CONTRIBUTING.md).
