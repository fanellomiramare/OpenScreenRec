# Changelog

All notable changes to this project are documented in this file.
Format based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow [Semantic Versioning](https://semver.org) (0.x while pre-1.0). Automated by [release-plz](https://github.com/MarcoIeni/release-plz).

## [Unreleased]

### Added

- Initial workspace: the `openscreenrec` application (egui shell) and the library crates `osr-common`, `osr-capture`, `osr-audio`, `osr-encode`, `osr-mux`.
- CI on Windows, macOS and Linux (formatting, build, tests, clippy, license/advisory checks).
- Automated versioning and changelog via release-plz, and a release workflow that attaches per-OS artifacts to GitHub Releases.
- MIT license, contributing guide, architecture notes.
