#!/bin/sh
# Axyr installer — https://github.com/clumsyquest/axyr
#
#   curl -fsSL https://raw.githubusercontent.com/clumsyquest/axyr/main/install.sh | sh
#
# Detects your platform, downloads the latest released `axyr` agent, verifies
# its checksum, and installs it to ~/.local/bin — no sudo, no surprises.
#
#   AXYR_VERSION=v0.1.0   pin a release instead of latest
#   AXYR_INSTALL_DIR=...  install somewhere else

set -eu

REPO="clumsyquest/axyr"
RELEASES="https://github.com/$REPO/releases"
VERSION="${AXYR_VERSION:-latest}"
INSTALL_DIR="${AXYR_INSTALL_DIR:-$HOME/.local/bin}"

say()  { printf '%s\n' "$*" >&2; }
fail() { say "axyr install: error: $*"; exit 1; }

command -v curl >/dev/null 2>&1 || fail "curl is required"

# --- platform -----------------------------------------------------------
os=$(uname -s)
arch=$(uname -m)
windows=""
case "$os" in
  Linux)  os="unknown-linux-gnu" ;;
  Darwin) os="apple-darwin" ;;
  # Git Bash / MSYS2 / Cygwin: install the native Windows binary right here.
  # (Native PowerShell users: see install.ps1 in the repo.)
  MINGW*|MSYS*|CYGWIN*) windows=1; os="pc-windows-msvc" ;;
  *) fail "unsupported OS: $os (Linux, macOS and Windows)" ;;
esac
case "$arch" in
  x86_64|amd64)  arch="x86_64" ;;
  aarch64|arm64) arch="aarch64" ;;
  *) fail "unsupported architecture: $arch" ;;
esac
target="$arch-$os"
if [ -n "$windows" ]; then
  asset="axyr-$target.zip"
  bin="axyr.exe"
else
  command -v tar >/dev/null 2>&1 || fail "tar is required"
  asset="axyr-$target.tar.gz"
  bin="axyr"
fi

# --- download + verify ---------------------------------------------------
if [ "$VERSION" = "latest" ]; then
  base="$RELEASES/latest/download"
else
  base="$RELEASES/download/$VERSION"
fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT INT TERM

say "downloading axyr ($target, $VERSION) ..."
curl -fsSL "$base/$asset" -o "$tmp/$asset" \
  || fail "download failed — is there a release asset for $target at $RELEASES ?"
curl -fsSL "$base/SHA256SUMS" -o "$tmp/SHA256SUMS" \
  || fail "could not download SHA256SUMS"

# sha256sum on Linux, shasum on stock macOS.
if command -v sha256sum >/dev/null 2>&1; then
  sum=$(sha256sum "$tmp/$asset" | cut -d' ' -f1)
else
  sum=$(shasum -a 256 "$tmp/$asset" | cut -d' ' -f1)
fi
expected=$(grep " $asset\$" "$tmp/SHA256SUMS" | cut -d' ' -f1)
[ -n "$expected" ] || fail "no checksum for $asset in SHA256SUMS"
[ "$sum" = "$expected" ] || fail "checksum mismatch for $asset (got $sum, expected $expected)"

# --- install -------------------------------------------------------------
if [ -n "$windows" ]; then
  # Git Bash usually ships unzip; PowerShell's Expand-Archive is the fallback.
  if command -v unzip >/dev/null 2>&1; then
    unzip -oq "$tmp/$asset" -d "$tmp"
  else
    powershell.exe -NoProfile -Command \
      "Expand-Archive -Force -LiteralPath '$(cygpath -w "$tmp/$asset")' -DestinationPath '$(cygpath -w "$tmp")'" \
      || fail "could not extract $asset (need unzip or PowerShell)"
  fi
else
  tar -xzf "$tmp/$asset" -C "$tmp"
fi
mkdir -p "$INSTALL_DIR"
install -m 755 "$tmp/$bin" "$INSTALL_DIR/$bin" 2>/dev/null \
  || { cp "$tmp/$bin" "$INSTALL_DIR/$bin" && chmod 755 "$INSTALL_DIR/$bin"; }

installed_version=$("$INSTALL_DIR/$bin" --version 2>/dev/null) \
  || fail "installed binary failed to run ($INSTALL_DIR/$bin --version)"
say "installed $installed_version -> $INSTALL_DIR/$bin"

# --- post-install hints ---------------------------------------------------
case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;
  *) say ""
     say "note: $INSTALL_DIR is not on your PATH. Add it, e.g.:"
     say "  echo 'export PATH=\"$INSTALL_DIR:\$PATH\"' >> ~/.$(basename "${SHELL:-bash}")rc" ;;
esac

# Talking to a debug probe as a plain user needs udev rules on Linux.
if [ "$(uname -s)" = "Linux" ] && [ "$(id -u)" != "0" ]; then
  if ! ls /etc/udev/rules.d/*probe*.rules >/dev/null 2>&1; then
    say ""
    say "note: to access the debug probe without root, install the probe-rs udev rules:"
    say "  curl -fsSL https://probe.rs/files/69-probe-rs.rules | sudo tee /etc/udev/rules.d/69-probe-rs.rules >/dev/null"
    say "  sudo udevadm control --reload && sudo udevadm trigger"
  fi
fi

say ""
say "done. Plug your board in and run:  axyr"
