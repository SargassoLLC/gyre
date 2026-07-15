#!/usr/bin/env sh
# Gyre — Install Script
# Usage: curl -fsSL https://raw.githubusercontent.com/SargassoLLC/gyre/main/install.sh | sh
#
# Downloads the correct binary for your OS/arch from GitHub Releases,
# verifies the SHA256 checksum, and installs to /usr/local/bin/gyre.
#
# Options (set as env vars before piping):
#   GYRE_VERSION   — install a specific version (default: latest)
#   GYRE_INSTALL_DIR — install location (default: /usr/local/bin)
#   GYRE_NO_CHECKSUM — set to "1" to skip checksum verification (not recommended)

set -eu

# ── Configuration ────────────────────────────────────────────────────────────

REPO="SargassoLLC/gyre"
INSTALL_DIR="${GYRE_INSTALL_DIR:-/usr/local/bin}"
BINARY_NAME="gyre"

# ── Helpers ───────────────────────────────────────────────────────────────────

say() {
    printf '\033[1;32m==>\033[0m %s\n' "$*"
}

say_warn() {
    printf '\033[1;33m⚠ \033[0m %s\n' "$*" >&2
}

say_err() {
    printf '\033[1;31m✗\033[0m %s\n' "$*" >&2
}

die() {
    say_err "$*"
    exit 1
}

need() {
    command -v "$1" >/dev/null 2>&1 || die "Required tool not found: $1. Please install it and retry."
}

# ── Detect OS and architecture ────────────────────────────────────────────────

detect_platform() {
    OS="$(uname -s)"
    ARCH="$(uname -m)"

    case "$OS" in
        Darwin)
            case "$ARCH" in
                arm64 | aarch64) TARGET="aarch64-apple-darwin" ;;
                x86_64)          TARGET="x86_64-apple-darwin" ;;
                *) die "Unsupported macOS architecture: $ARCH" ;;
            esac
            ;;
        Linux)
            case "$ARCH" in
                aarch64 | arm64) TARGET="aarch64-unknown-linux-gnu" ;;
                x86_64)          TARGET="x86_64-unknown-linux-gnu" ;;
                *) die "Unsupported Linux architecture: $ARCH. Only x86_64 and aarch64 are supported." ;;
            esac
            ;;
        *)
            die "Unsupported operating system: $OS. Gyre supports macOS and Linux."
            ;;
    esac

    echo "$TARGET"
}

# ── Fetch latest version from GitHub API ─────────────────────────────────────

fetch_latest_version() {
    need curl

    LATEST=$(curl -fsSL \
        -H "Accept: application/vnd.github.v3+json" \
        "https://api.github.com/repos/${REPO}/releases/latest" \
        | grep '"tag_name"' \
        | sed -E 's/.*"tag_name": "([^"]+)".*/\1/')

    [ -n "$LATEST" ] || die "Could not determine the latest Gyre version. Check your internet connection."
    echo "$LATEST"
}

# ── Download and verify ───────────────────────────────────────────────────────

download_and_install() {
    VERSION="$1"
    TARGET="$2"

    BASE_URL="https://github.com/${REPO}/releases/download/${VERSION}"

    # cargo-dist archives are unversioned (gyre-<target>.tar.gz); releases
    # before the pipeline consolidation used gyre-<version>-<target>.tar.gz.
    # Try the current naming first, fall back to the legacy one.
    ARCHIVE_NAME="gyre-${TARGET}.tar.gz"
    if ! curl -fsIL "${BASE_URL}/${ARCHIVE_NAME}" >/dev/null 2>&1; then
        ARCHIVE_NAME="gyre-${VERSION}-${TARGET}.tar.gz"
    fi
    CHECKSUM_NAME="${ARCHIVE_NAME}.sha256"
    ARCHIVE_URL="${BASE_URL}/${ARCHIVE_NAME}"
    CHECKSUM_URL="${BASE_URL}/${CHECKSUM_NAME}"

    TMPDIR="$(mktemp -d)"
    # shellcheck disable=SC2064
    trap "rm -rf '$TMPDIR'" EXIT INT TERM

    ARCHIVE_PATH="${TMPDIR}/${ARCHIVE_NAME}"
    CHECKSUM_PATH="${TMPDIR}/${CHECKSUM_NAME}"

    say "Downloading Gyre ${VERSION} for ${TARGET}..."
    curl -fsSL --retry 3 --retry-delay 2 -o "$ARCHIVE_PATH" "$ARCHIVE_URL" \
        || die "Failed to download ${ARCHIVE_URL}"

    say "Downloading checksum..."
    curl -fsSL --retry 3 --retry-delay 2 -o "$CHECKSUM_PATH" "$CHECKSUM_URL" \
        || die "Failed to download checksum from ${CHECKSUM_URL}"

    # Verify checksum
    if [ "${GYRE_NO_CHECKSUM:-0}" != "1" ]; then
        say "Verifying SHA256 checksum..."

        # Read expected hash from the .sha256 file (first field)
        EXPECTED=$(awk '{print $1}' "$CHECKSUM_PATH")

        # Compute actual hash (platform-specific commands)
        if command -v sha256sum >/dev/null 2>&1; then
            ACTUAL=$(sha256sum "$ARCHIVE_PATH" | awk '{print $1}')
        elif command -v shasum >/dev/null 2>&1; then
            ACTUAL=$(shasum -a 256 "$ARCHIVE_PATH" | awk '{print $1}')
        else
            say_warn "No SHA256 tool found (sha256sum or shasum). Skipping checksum verification."
            say_warn "Install sha256sum or shasum for secure installs."
            ACTUAL="$EXPECTED"
        fi

        if [ "$ACTUAL" != "$EXPECTED" ]; then
            die "Checksum mismatch!\n  Expected: $EXPECTED\n  Got:      $ACTUAL\nDownload may be corrupted. Aborting."
        fi
        say "✓ Checksum verified"
    else
        say_warn "Checksum verification skipped (GYRE_NO_CHECKSUM=1)"
    fi

    # Extract binary
    say "Extracting..."
    tar -xzf "$ARCHIVE_PATH" -C "$TMPDIR"
    [ -f "${TMPDIR}/${BINARY_NAME}" ] || die "Binary '${BINARY_NAME}' not found in archive."
    chmod +x "${TMPDIR}/${BINARY_NAME}"

    # Install
    say "Installing to ${INSTALL_DIR}/${BINARY_NAME}..."

    if [ -w "$INSTALL_DIR" ]; then
        mv "${TMPDIR}/${BINARY_NAME}" "${INSTALL_DIR}/${BINARY_NAME}"
    elif command -v sudo >/dev/null 2>&1; then
        say "  (sudo required for ${INSTALL_DIR})"
        sudo mv "${TMPDIR}/${BINARY_NAME}" "${INSTALL_DIR}/${BINARY_NAME}"
    else
        # Try installing to ~/.local/bin as fallback
        LOCAL_BIN="$HOME/.local/bin"
        mkdir -p "$LOCAL_BIN"
        mv "${TMPDIR}/${BINARY_NAME}" "${LOCAL_BIN}/${BINARY_NAME}"
        INSTALL_DIR="$LOCAL_BIN"
        say_warn "Installed to ${INSTALL_DIR} (sudo not available)"
        say_warn "Add ${LOCAL_BIN} to your PATH if not already present"
    fi
}

# ── Verify installation ───────────────────────────────────────────────────────

verify_install() {
    INSTALLED_PATH="${INSTALL_DIR}/${BINARY_NAME}"
    if [ -x "$INSTALLED_PATH" ]; then
        INSTALLED_VERSION=$("$INSTALLED_PATH" --version 2>/dev/null || echo "unknown")
        say "✓ Gyre installed successfully: ${INSTALLED_VERSION}"
    else
        die "Installation failed: binary not found at ${INSTALLED_PATH}"
    fi
}

# ── Check PATH ────────────────────────────────────────────────────────────────

check_path() {
    case ":$PATH:" in
        *":${INSTALL_DIR}:"*)
            : # already in PATH
            ;;
        *)
            say_warn "${INSTALL_DIR} is not in your PATH."
            say_warn "Add the following to your shell profile (~/.bashrc, ~/.zshrc, etc.):"
            say_warn "  export PATH=\"${INSTALL_DIR}:\$PATH\""
            ;;
    esac
}

# ── Main ──────────────────────────────────────────────────────────────────────

main() {
    printf '\n'
    printf '   ██████╗ ██╗   ██╗██████╗ ███████╗\n'
    printf '  ██╔════╝ ╚██╗ ██╔╝██╔══██╗██╔════╝\n'
    printf '  ██║  ███╗ ╚████╔╝ ██████╔╝█████╗  \n'
    printf '  ██║   ██║  ╚██╔╝  ██╔══██╗██╔══╝  \n'
    printf '  ╚██████╔╝   ██║   ██║  ██║███████╗\n'
    printf '   ╚═════╝    ╚═╝   ╚═╝  ╚═╝╚══════╝\n'
    printf '   Ambient AI OS — Installer\n\n'

    need curl

    TARGET=$(detect_platform)
    say "Detected platform: ${TARGET}"

    # Determine version to install
    if [ -n "${GYRE_VERSION:-}" ]; then
        # Strip leading 'v' then re-add to normalise (e.g. "1.0.0" → "v1.0.0")
        VERSION="v${GYRE_VERSION#v}"
        say "Installing requested version: ${VERSION}"
    else
        say "Fetching latest release..."
        VERSION=$(fetch_latest_version)
        say "Latest version: ${VERSION}"
    fi

    # Check if already up-to-date
    if command -v gyre >/dev/null 2>&1; then
        CURRENT=$(gyre --version 2>/dev/null | grep -oE '[0-9]+\.[0-9]+\.[0-9]+[^[:space:]]*' | head -1 || echo "")
        CURRENT_V="v${CURRENT#v}"
        if [ "$CURRENT_V" = "$VERSION" ]; then
            say "Gyre ${VERSION} is already installed. Nothing to do."
            say "Run 'gyre update' to check for newer releases."
            exit 0
        fi
    fi

    download_and_install "$VERSION" "$TARGET"
    verify_install
    check_path

    printf '\n'
    printf '  \033[1;32m✓ Gyre is ready!\033[0m\n\n'
    printf '  Next steps:\n'
    printf '    gyre init          — Set up your first AI agent\n'
    printf '    gyre serve --help  — Run an agent\n'
    printf '    gyre update        — Self-update Gyre\n\n'
    printf '  Documentation: https://github.com/SargassoLLC/gyre\n\n'
}

main "$@"
