# Homebrew cask for OpenScreenRec.
#
# Recommended install channel for macOS users: cask installs do not get
# the quarantine attribute, so Gatekeeper never prompts (docs/MACOS.md).
#
# One-time setup of a personal tap:
#   brew tap-new <user>/tap
#   cp packaging/openscreenrec.rb <tap-path>/Casks/openscreenrec.rb
# then users run: brew install --cask <user>/tap/openscreenrec
#
# NOTE: update `version` and `sha256` after each release — the release
# workflow prints the dmg checksums in its job summary.
cask "openscreenrec" do
  version "0.1.0"
  sha256 :no_check

  url "https://github.com/fanellomiramare/OpenScreenRec/releases/download/v#{version}/openscreenrec-macos-arm64.dmg"
  name "OpenScreenRec"
  desc "Free and open-source screen recorder, 100% Rust"
  homepage "https://github.com/fanellomiramare/OpenScreenRec"

  app "OpenScreenRec.app"
end
