#!/usr/bin/env bash
set -euo pipefail

REPO="pupkit-labs/echo-pup-rust"
BINARY="echopup"
INSTALL_DIR="${ECHOPUP_INSTALL_DIR:-$HOME/.local/bin}"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
CYAN='\033[0;36m'
NC='\033[0m'

info()  { echo -e "${CYAN}[info]${NC}  $*"; }
ok()    { echo -e "${GREEN}[ok]${NC}    $*"; }
warn()  { echo -e "${YELLOW}[warn]${NC}  $*"; }
error() { echo -e "${RED}[error]${NC} $*" >&2; exit 1; }

detect_platform() {
  local os arch
  os="$(uname -s)"
  arch="$(uname -m)"

  case "$os" in
    Linux)  os="unknown-linux-gnu" ;;
    Darwin) os="apple-darwin" ;;
    *)      error "Unsupported OS: $os" ;;
  esac

  case "$arch" in
    x86_64|amd64)  arch="x86_64" ;;
    aarch64|arm64) arch="aarch64" ;;
    *)             error "Unsupported architecture: $arch" ;;
  esac

  echo "${arch}-${os}"
}

get_latest_version() {
  local url="https://api.github.com/repos/${REPO}/releases/latest"
  if command -v curl &>/dev/null; then
    curl -fsSL "$url" | grep '"tag_name"' | sed -E 's/.*"([^"]+)".*/\1/'
  elif command -v wget &>/dev/null; then
    wget -qO- "$url" | grep '"tag_name"' | sed -E 's/.*"([^"]+)".*/\1/'
  else
    error "curl or wget is required"
  fi
}

download() {
  local url="$1" dest="$2"
  if command -v curl &>/dev/null; then
    curl -fsSL -o "$dest" "$url"
  elif command -v wget &>/dev/null; then
    wget -qO "$dest" "$url"
  fi
}

main() {
  local version="${1:-}"

  info "Detecting platform..."
  local target
  target="$(detect_platform)"
  ok "Platform: $target"

  if [ -z "$version" ]; then
    info "Fetching latest version..."
    version="$(get_latest_version)"
    [ -z "$version" ] && error "Failed to determine latest version"
  fi
  ok "Version: $version"

  local archive="${BINARY}-${target}.tar.xz"
  local url="https://github.com/${REPO}/releases/download/${version}/${archive}"

  info "Downloading ${archive}..."
  local tmpdir
  tmpdir="$(mktemp -d)"
  trap 'rm -rf "$tmpdir"' EXIT

  download "$url" "${tmpdir}/${archive}"
  ok "Downloaded"

  info "Extracting..."
  tar xJf "${tmpdir}/${archive}" -C "$tmpdir"

  info "Installing to ${INSTALL_DIR}..."
  mkdir -p "$INSTALL_DIR"
  mv "${tmpdir}/${BINARY}" "${INSTALL_DIR}/${BINARY}"
  chmod +x "${INSTALL_DIR}/${BINARY}"
  ok "Installed ${BINARY} to ${INSTALL_DIR}/${BINARY}"

  # Check if INSTALL_DIR is in PATH
  if ! echo "$PATH" | tr ':' '\n' | grep -qx "$INSTALL_DIR"; then
    warn "${INSTALL_DIR} is not in your PATH"
    echo ""
    echo "  Add it to your shell profile:"
    echo ""
    echo "    export PATH=\"${INSTALL_DIR}:\$PATH\""
    echo ""
  fi

  ok "Done! Run '${BINARY} --version' to verify."
}

main "$@"
