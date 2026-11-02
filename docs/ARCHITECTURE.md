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

| Crate                      | Responsibility                                     |
| -------------------------- | -------------------------------------------------- |
| `openscreenrec` (`src/`)   | egui application: UI, hotkeys, tray, orchestration |
| `osr-common`               | config types, well-known paths, small helpers      |
| `osr-capture`              | `FrameSource` trait + per-OS capture backends      |
| `osr-audio`                | mic & system-loopback capture, mixing              |
| `osr-encode`               | video/audio compression                            |
| `osr-mux`                  | container writing (MP4, MKV/WebM)                  |

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
