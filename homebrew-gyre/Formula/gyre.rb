# typed: false
# frozen_string_literal: true

# Homebrew formula for Gyre — Ambient AI OS
#
# Usage:
#   brew tap sac916/gyre
#   brew install gyre
#
# To update this formula with new checksums after a release:
#   1. Download the .sha256 files from the GitHub release
#   2. Update the sha256 values in the `bottle do` block below
#   3. Update the VERSION constant

class Gyre < Formula
  desc "Gyre — Ambient AI OS. Cognitive agents, memory, and tribe orchestration."
  homepage "https://github.com/sac916/gyre"
  version "0.1.0"

  # ── Platform-specific bottles (pre-built binaries) ─────────────────────────
  #
  # Checksums are populated by the CI pipeline after each release.
  # To find the correct sha256 for a given version, download the .sha256 file
  # from https://github.com/sac916/gyre/releases and paste it below.
  #
  # Format:
  #   sha256 cellar: :any_skip_relocation, <arch>_<macos>: "<sha256 of .tar.gz>"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/sac916/gyre/releases/download/v#{version}/gyre-v#{version}-aarch64-apple-darwin.tar.gz"
      # sha256 "REPLACE_WITH_AARCH64_APPLE_DARWIN_SHA256"
      sha256 "0000000000000000000000000000000000000000000000000000000000000000"
    else
      url "https://github.com/sac916/gyre/releases/download/v#{version}/gyre-v#{version}-x86_64-apple-darwin.tar.gz"
      # sha256 "REPLACE_WITH_X86_64_APPLE_DARWIN_SHA256"
      sha256 "0000000000000000000000000000000000000000000000000000000000000000"
    end
  end

  # ── Installation ───────────────────────────────────────────────────────────

  def install
    bin.install "gyre"
  end

  # ── Shell completions (generated at install time) ─────────────────────────

  def post_install
    # Generate shell completions if the binary supports it
    # gyre completions bash > /dev/null 2>&1 && true
  end

  # ── Tests ─────────────────────────────────────────────────────────────────

  test do
    assert_match "gyre", shell_output("#{bin}/gyre --version")
    # Verify the binary runs and responds to --help
    system "#{bin}/gyre", "--help"
  end

  # ── Caveats ───────────────────────────────────────────────────────────────

  def caveats
    <<~EOS
      Gyre has been installed. To get started:

        gyre init

      This will guide you through setting up your first AI agent.

      For full documentation:
        https://github.com/sac916/gyre

      To update Gyre:
        gyre update
        # or:
        brew upgrade gyre
    EOS
  end
end
