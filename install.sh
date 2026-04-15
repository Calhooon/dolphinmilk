#!/bin/sh
# Dolphin Milk installer.
#
#   curl -fsSL https://raw.githubusercontent.com/calhooon/dolphinmilk/main/install.sh | sh
#
# Detects OS/arch, downloads the matching prebuilt binary from the latest
# GitHub Release, and installs it to a directory on PATH.
#
# Honored env vars:
#   DM_VERSION   — pin to a specific release tag (default: latest)
#   DM_INSTALL_DIR — override install directory (default: $HOME/.local/bin)

set -eu

REPO="calhooon/dolphinmilk"
BIN_NAME="dolphin-milk"
DEFAULT_INSTALL_DIR="${HOME}/.local/bin"
INSTALL_DIR="${DM_INSTALL_DIR:-$DEFAULT_INSTALL_DIR}"
VERSION="${DM_VERSION:-latest}"

# ---- pretty output ---------------------------------------------------------

bold=""
dim=""
red=""
green=""
yellow=""
reset=""
if [ -t 1 ] && command -v tput >/dev/null 2>&1; then
  bold=$(tput bold || true)
  dim=$(tput dim || true)
  red=$(tput setaf 1 || true)
  green=$(tput setaf 2 || true)
  yellow=$(tput setaf 3 || true)
  reset=$(tput sgr0 || true)
fi

say()  { printf '%s\n' "$*"; }
info() { printf '%s==>%s %s\n' "$green" "$reset" "$*"; }
warn() { printf '%s!!%s %s\n' "$yellow" "$reset" "$*" >&2; }
die()  { printf '%serror:%s %s\n' "$red" "$reset" "$*" >&2; exit 1; }

# ---- detect platform -------------------------------------------------------

uname_os=$(uname -s 2>/dev/null || echo unknown)
uname_arch=$(uname -m 2>/dev/null || echo unknown)

case "$uname_os" in
  Darwin) os="darwin" ;;
  Linux)  os="linux" ;;
  *) die "unsupported OS: $uname_os (this installer supports macOS and Linux; on Windows use 'cargo install --git https://github.com/${REPO}')" ;;
esac

case "$uname_arch" in
  x86_64|amd64) arch="amd64" ;;
  arm64|aarch64) arch="arm64" ;;
  *) die "unsupported architecture: $uname_arch" ;;
esac

# arm64 Linux not yet shipped as a prebuilt binary — fall through to cargo install.
if [ "$os" = "linux" ] && [ "$arch" = "arm64" ]; then
  warn "ARM64 Linux prebuilt binary is not yet available."
  say "    Build from source instead:"
  say "      ${bold}cargo install --git https://github.com/${REPO}${reset}"
  exit 1
fi

asset="${BIN_NAME}-${os}-${arch}"
info "Detected ${bold}${os}-${arch}${reset}"

# ---- find download URL -----------------------------------------------------

if [ "$VERSION" = "latest" ]; then
  release_url="https://github.com/${REPO}/releases/latest/download/${asset}"
else
  release_url="https://github.com/${REPO}/releases/download/${VERSION}/${asset}"
fi

# ---- check for download tool -----------------------------------------------

if command -v curl >/dev/null 2>&1; then
  fetch() { curl -fsSL "$1" -o "$2"; }
elif command -v wget >/dev/null 2>&1; then
  fetch() { wget -qO "$2" "$1"; }
else
  die "neither curl nor wget found"
fi

# ---- download into a temp dir ----------------------------------------------

tmpdir=$(mktemp -d 2>/dev/null || mktemp -d -t dolphin-milk-install)
trap 'rm -rf "$tmpdir"' EXIT INT HUP TERM

info "Downloading ${asset}"
say "    from ${dim}${release_url}${reset}"

if ! fetch "$release_url" "${tmpdir}/${BIN_NAME}"; then
  die "download failed — release ${VERSION} may not exist or asset ${asset} is missing"
fi

chmod +x "${tmpdir}/${BIN_NAME}"

# macOS quarantine — strip if present so the user doesn't get a Gatekeeper dialog
if [ "$os" = "darwin" ] && command -v xattr >/dev/null 2>&1; then
  xattr -d com.apple.quarantine "${tmpdir}/${BIN_NAME}" 2>/dev/null || true
fi

# ---- install ---------------------------------------------------------------

if [ ! -d "$INSTALL_DIR" ]; then
  info "Creating ${INSTALL_DIR}"
  mkdir -p "$INSTALL_DIR"
fi

target="${INSTALL_DIR}/${BIN_NAME}"
info "Installing to ${bold}${target}${reset}"
mv "${tmpdir}/${BIN_NAME}" "$target"

# ---- PATH check ------------------------------------------------------------

case ":${PATH}:" in
  *":${INSTALL_DIR}:"*) on_path=1 ;;
  *) on_path=0 ;;
esac

say ""
say "${green}${bold}✓ dolphin-milk installed${reset}"
say ""

if [ "$on_path" -eq 0 ]; then
  warn "${INSTALL_DIR} is not on your PATH"
  say "    Add it with one of:"
  say "      ${bold}echo 'export PATH=\"\$PATH:${INSTALL_DIR}\"' >> ~/.zshrc${reset}"
  say "      ${bold}echo 'export PATH=\"\$PATH:${INSTALL_DIR}\"' >> ~/.bashrc${reset}"
  say ""
fi

say "Next steps:"
say "  ${bold}dolphin-milk init${reset}        # first-run setup"
say "  ${bold}dolphin-milk status${reset}      # check wallet + balance"
say "  ${bold}dolphin-milk think \"hi\"${reset}  # first paid LLM call (a few hundred sats)"
say "  ${bold}dolphin-milk serve${reset}       # web UI on http://localhost:8080/ui/"
say ""
say "Docs: https://github.com/${REPO}"
