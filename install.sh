#!/usr/bin/env bash
# ZedPlus install script — macOS and Linux.
# Downloads the latest release binary and installs it to ~/.local/bin.
# Usage (from DMG or project root):
#   ./install.sh               # install bundled binary (when run from DMG)
#   ./install.sh --uninstall   # remove zedplus

set -euo pipefail

INSTALL_DIR="${HOME}/.local/bin"
BIN_NAME="zedplus"
BUNDLED="$(cd "$(dirname "$0")" && pwd)/$BIN_NAME"

err() { echo "error: $*" >&2; exit 1; }
info() { echo "  $*"; }

if [[ "${1:-}" == "--uninstall" ]]; then
    rm -f "$INSTALL_DIR/$BIN_NAME"
    info "Removed $INSTALL_DIR/$BIN_NAME"
    exit 0
fi

# ── Install bundled binary if present (DMG / local build) ────────────────────
if [[ -f "$BUNDLED" ]]; then
    mkdir -p "$INSTALL_DIR"
    install -m 0755 "$BUNDLED" "$INSTALL_DIR/$BIN_NAME"
    info "Installed $INSTALL_DIR/$BIN_NAME"
    "$INSTALL_DIR/$BIN_NAME" --version
else
    err "Binary not found at $BUNDLED. Download a release DMG from GitHub."
fi

# ── Add ~/.local/bin to PATH if missing ──────────────────────────────────────
SHELL_RC=""
case "${SHELL:-}" in
    */zsh)  SHELL_RC="${ZDOTDIR:-$HOME}/.zshrc" ;;
    */bash) SHELL_RC="$HOME/.bashrc" ;;
esac

if [[ -n "$SHELL_RC" && -f "$SHELL_RC" ]]; then
    if ! grep -q 'local/bin' "$SHELL_RC" 2>/dev/null; then
        echo 'export PATH="$HOME/.local/bin:$PATH"' >> "$SHELL_RC"
        info "Added ~/.local/bin to PATH in $SHELL_RC — restart your shell or run: source $SHELL_RC"
    fi
fi

echo ""
echo "ZedPlus installed. Run: zedplus --help"
