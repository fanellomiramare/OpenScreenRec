# OpenScreenRec on macOS

OpenScreenRec is **not notarized** — the project has no paid Apple Developer Program membership — so this document explains exactly what to expect as a user, and how maintainers keep the experience as smooth as the free tooling allows.

## For users

### Installation

1. Download `openscreenrec-macos-<arch>.dmg` from the [releases page](https://github.com/fanellomiramare/OpenScreenRec/releases) (`arm64` for Apple Silicon, `x64` for Intel Macs).
2. Open the `.dmg` and drag **OpenScreenRec.app** into *Applications*.
3. The first launch is the only extra step — see below. **Homebrew users** (recommended): `brew install --cask openscreenrec` skips this entirely, because Homebrew does not mark downloaded apps with the quarantine attribute — no Gatekeeper prompt at all (cask lands with M5).

### First launch: the one-time Gatekeeper approval

Because the app is not notarized, macOS blocks the first launch of an app downloaded from the internet:

1. Open the app (Launchpad/Finder). A dialog says Apple could not verify it.
2. Cancel the dialog, then open **System Settings → Privacy & Security** and scroll to the *Security* section: an entry **"OpenScreenRec was blocked…"** appears with an **Open Anyway** button.
3. Click **Open Anyway**, then **Open** in the confirmation dialog and authenticate with your password/Touch ID.
4. From now on the app opens normally. Note: after installing a **new version**, repeat steps 1–3 once (the approval is per downloaded copy).

If anything goes wrong, the terminal-free alternative is right-click → Open on the app; on recent macOS versions that path was tightened, so the *Privacy & Security* route above is the reliable one.

### Permissions (one-time, required for everyone)

- **Screen Recording** — required. OpenScreenRec asks you to grant it on first launch (*Grant access…* opens the right pane of System Settings); enable the OpenScreenRec toggle there, then use *Check again* in the app. If capturing still fails right after granting, quit the app (tray → Quit) and reopen it — macOS applies the change on relaunch.
- **Microphone** — only if you enable it in settings. macOS shows its own prompt the first time; approve once.

> After updates the app may ask for the Screen Recording permission again. This does **not** happen when the release was signed with the project's stable certificate (see below) — that is exactly what the certificate is for.

### Build from source

No Gatekeeper involved at all:

```sh
git clone https://github.com/fanellomiramare/OpenScreenRec
cd OpenScreenRec
cargo build --release
```

Requires macOS 12.3+ and Xcode Command Line Tools.

## For maintainers

### Why a stable certificate matters

TCC tracks permissions per code identity. Ad-hoc signatures change with every build, so macOS would re-ask for the Screen Recording permission after **every** release. Signing all releases with the same self-signed certificate keeps the identity stable and the permissions sticky. A self-signed code-signing certificate costs nothing and requires no Apple account (a free-Apple-ID "Apple Development" certificate is *not* usable here: it expires after 7 days).

### Create the certificate (once, on any Mac)

1. Open **Keychain Access → Certificate Assistant → Create a Certificate**.
2. Name: `OpenScreenRec Dev`, Identity Type: *Self-Signed Root*, Certificate Type: *Code Signing*; create it.
3. Export the certificate + private key as a `.p12` (right-click the certificate → Export; set a strong password).

### Publish it as GitHub secrets

| Secret              | Value                                                     |
| ------------------- | --------------------------------------------------------- |
| `MACOS_CERT_P12`    | base64 of the `.p12` (`base64 -i OpenScreenRecDev.p12`)    |
| `MACOS_CERT_PASSWORD` | the export password                                     |
| `MACOS_CERT_IDENTITY` | the certificate's Common Name, e.g. `OpenScreenRec Dev` |

The release workflow picks these up automatically: with secrets present it signs the `.app` with the stable identity; without them it falls back to ad-hoc signing. Keep the `.p12` file offline and never commit it.

### What the stable certificate does *not* provide

It is self-signed, so Gatekeeper still asks for the *Open Anyway* approval on direct downloads (this is the notarization we deliberately skipped), and it cannot be used for notarized, auto-updating distributions. The Homebrew cask (M5) is the friction-free channel for most users.
