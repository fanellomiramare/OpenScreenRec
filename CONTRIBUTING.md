# Contributing to OpenScreenRec

Thanks for your interest in contributing! This document covers everything you need.

## Development setup

Requirements:

- [Rust](https://rustup.rs) — the `rust-toolchain.toml` file pins the channel to `stable` (1.85+, edition 2024)
- Platform notes:
  - **Windows**: MSVC build tools (Visual Studio Build Tools with "Desktop development with C++")
  - **macOS**: Xcode Command Line Tools (`xcode-select --install`)
  - **Linux**: a standard distro toolchain **plus the audio/video development packages** — Debian/Ubuntu: `sudo apt install libasound2-dev libpipewire-0.3-dev libspa-0.2-dev libpulse-dev`; Fedora: `alsa-lib-devel pipewire-devel pulseaudio-libs`; Arch: `alsa-lib pipewire libpulse`. X11 capture is pure Rust; `xvfb` + `x11-apps` enable the functional X11 test locally.

Get started:

```sh
git clone https://github.com/fanellomiramare/OpenScreenRec
cd OpenScreenRec
cargo build
cargo test --workspace
cargo run            # launches the app
```

## Project layout

| Crate                      | Responsibility                                        |
| -------------------------- | ----------------------------------------------------- |
| `openscreenrec` (`src/`)   | egui application: UI, hotkeys, tray, orchestration    |
| `osr-common`               | config types, well-known paths, small helpers         |
| `osr-capture`              | `FrameSource` trait + per-OS capture backends         |
| `osr-audio`                | microphone & system-loopback capture, mixing          |
| `osr-encode`               | video/audio compression (FFmpeg pipe now, HW later)   |
| `osr-mux`                  | container writing (MP4, MKV/WebM)                     |

Design rules to keep in mind:

- The UI never touches OS capture APIs directly — only the traits in `libs/`.
- OS-specific code lives inside its crate, `cfg`-gated, one module per platform.
- One monotonic microsecond clock for all A/V timestamps.
- `libs/*` must stay UI-free; `src/` must stay free of raw OS APIs.

## CI gates

Every pull request must pass CI on **Windows, macOS and Linux**:

- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- `cargo deny check` (license compatibility + security advisories)

## Commit messages: Conventional Commits

This project uses [Conventional Commits](https://www.conventionalcommits.org) — the release automation reads them to bump versions and write the changelog.

| Type       | Meaning                              | Version effect   |
| ---------- | ------------------------------------ | ---------------- |
| `feat:`    | new feature                          | 0.1.0 → 0.2.0    |
| `fix:`     | bug fix                              | 0.1.0 → 0.1.1    |
| `feat!:` / breaking | incompatible change         | 0.2.0 → 0.3.0    |
| `docs:`, `chore:`, `ci:`, `refactor:`, `perf:`, `test:` | no release by themselves | — |

Examples:

```
feat(capture): implement Windows Graphics Capture backend
fix(audio): keep mic and system streams aligned during pause
docs: extend architecture notes with the muxer design
```

## Adding dependencies

To keep the project MIT-clean and trustworthy:

- Licenses allowed: MIT, Apache-2.0, BSD-2/3-Clause, ISC, Zlib, CC0-1.0, MPL-2.0, Unicode-3.0 (the enforced list lives in `deny.toml`)
- No telemetry, no "phone home" crates
- Prefer pure-Rust crates; an external FFmpeg binary is the only planned third-party runtime dependency
- Credit non-obvious design inspiration in the README and in crate docs

## Release process (maintainers)

Releases are fully automated:

1. Commits merged into `main` trigger **release-plz**, which opens (or updates) a *Release PR* that bumps the workspace version and regenerates `CHANGELOG.md` from the commits.
2. Merging that PR makes release-plz create the `vX.Y.Z` git tag and the GitHub Release.
3. The **Release** workflow then builds artifacts for every platform and attaches them to the release (Windows zip, macOS dmg — Apple Silicon and Intel — Linux tarball; AppImage/deb/rpm land with M5).

> Note: while the Release PR is created with the default `GITHUB_TOKEN`, its CI checks won't run (a GitHub limitation). Optionally add a `RELEASE_PLZ_PAT` secret (a personal access token with `repo` scope) — the workflows pick it up automatically and use it for both the PR and the tag push.

Nightly builds from `main` are planned for M3–M4.

## Code style

- `rustfmt` defaults (`cargo fmt` before every push)
- Clippy-clean code (`-D warnings` is enforced)
- Write comments only where the code can't explain itself — constraints, not narration
