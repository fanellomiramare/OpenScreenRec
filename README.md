# OpenScreenRec

[![CI](https://github.com/fanellomiramare/OpenScreenRec/actions/workflows/ci.yml/badge.svg)](https://github.com/fanellomiramare/OpenScreenRec/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

**OpenScreenRec** is a free and open-source screen recorder, written in **100% Rust** — no Electron, no web tech, no telemetry. Lightweight, local-first and cross-platform. Your recordings never leave your machine.

> 🚧 OpenScreenRec is in early development. The first installers ship with milestone **M5** — until then, [build from source](#build-from-source) and follow the [roadmap](#roadmap).

## Philosophy

- **100% Rust** — the whole application, UI included (egui). No Electron, no browser engine, no web views.
- **Local-first & private** — no account, no cloud, no telemetry. Nothing ever leaves your machine.
- **Lightweight** — hardware-accelerated capture and encoding wherever the OS provides it; the recorder should be invisible to what you're doing.
- **Fully open source** — MIT licensed. Every third-party reference is credited in [Credits & references](#credits--references).

## Downloads

### Windows

| Package              | Status              |
| -------------------- | ------------------- |
| Installer (`.exe`)   | coming with **M5**  |
| Portable (`.zip`)    | coming with **M5**  |

### macOS

| Package                       | Status              |
| ----------------------------- | ------------------- |
| Apple Silicon (`.dmg`)        | coming with **M5**  |
| Intel (`.dmg`)                | coming with **M5**  |

### Linux

| Package            | Status              |
| ------------------ | ------------------- |
| AppImage           | coming with **M5**  |
| `.deb` / `.rpm`    | coming with **M5**  |
| Flatpak            | planned             |
| AUR                | planned             |

## Build from source

Requirements: [Rust](https://rustup.rs) 1.85+ (the toolchain file pins `stable`).

```sh
git clone https://github.com/fanellomiramare/OpenScreenRec
cd OpenScreenRec
cargo build --release
```

Run it:

```sh
./target/release/openscreenrec        # Linux / macOS
target\release\openscreenrec.exe      # Windows
```

## Roadmap

| Milestone | Scope                                                                       | Status |
| --------- | --------------------------------------------------------------------------- | ------ |
| **M0**    | Workspace, CI on 3 OS, licensing, docs, release automation                  | ✅     |
| **M1**    | Video capture (Windows first) → MP4 via FFmpeg pipe                         | ✅     |
| **M2**    | Microphone + system audio, A/V sync, pause                                  | ✅     |
| **M3**    | Floating toolbar, hotkeys, tray, region picker — **MVP complete**           | 🚧     |
| **M4**    | macOS backend (ScreenCaptureKit, permissions, TCC)                          |        |
| **M5**    | Linux backend (X11 / Wayland portal) + installers for all three OS          |        |
| **M6**    | Hardware encoders, window capture, audio tracks, GIF/WebM, replay buffer, webcam PiP | |

## Repository layout

```
OpenScreenRec/
├─ src/                # the application (egui UI, orchestration)
├─ libs/
│  ├─ capture/         # osr-capture — FrameSource trait + per-OS backends
│  ├─ audio/           # osr-audio   — mic + system loopback capture, mixing
│  ├─ encode/          # osr-encode  — video/audio compression
│  ├─ mux/             # osr-mux     — container writing (MP4/MKV/WebM)
│  └─ common/          # osr-common  — config, paths, shared helpers
├─ docs/               # architecture notes
├─ appimage/ flatpak/  # Linux packaging (with M5)
└─ .github/workflows/  # CI (3 OS), release-plz, release artifacts
```

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the pipeline design.

## Credits & references

OpenScreenRec is written from scratch. These are the open-source projects that inspire its design, and the libraries it builds on — all credited where used:

**Design inspiration**

- [RustDesk](https://github.com/rustdesk/rustdesk) — repository structure and multi-OS release organization
- [Cap](https://github.com/CapSoftware/Cap) and its [scap](https://github.com/CapSoftware/scap) crate — cross-platform capture design
- [GPU Screen Recorder](https://github.com/dec05eba/gpu_screen_recorder) — GPU-first, low-impact recording
- [OBS Studio](https://github.com/obsproject/obs-studio) and [Kooha](https://github.com/SeaDve/Kooha) — feature model
- [vokoscreenNG](https://github.com/e-koch/vokoscreenNG), [Kap](https://github.com/wulkano/Kap), [ScreenToGif](https://github.com/NickeManarin/ScreenToGif) — small-but-useful features

**Key Rust crates**

- [egui](https://github.com/emilk/egui) — the UI
- [windows-capture](https://github.com/NiiightmareXD/windows-capture) — Windows Graphics Capture backend
- [screencapturekit](https://github.com/svtslf/screencapturekit) — macOS capture backend
- [cpal](https://github.com/RustAudio/cpal) — audio input
- [tray-icon](https://github.com/tauri-apps/tray-icon) and [global-hotkey](https://github.com/tauri-apps/global-hotkey) — desktop integration
- [release-plz](https://github.com/MarcoIeni/release-plz) — automated versioning and changelogs

## Contributing

Contributions are welcome! Read [CONTRIBUTING.md](CONTRIBUTING.md) to get started — the short version: Conventional Commits, CI must pass on all three platforms, and every dependency must keep the project MIT-clean.

## License

[MIT](LICENSE) — © The OpenScreenRec Contributors. Third-party crate licenses are continuously checked in CI with [cargo-deny](https://github.com/EmbarkStudios/cargo-deny).
