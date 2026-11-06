#!/bin/sh
# Install the latest skills-man release (the `skill` binary) on macOS or Linux.
#
#   curl -fsSL https://raw.githubusercontent.com/nomyfan/skills-man/main/install.sh | sh
#
# Set SKILLS_MAN_INSTALL_DIR to change where `skill` goes (default: ~/.local/bin).

set -eu

REPO="nomyfan/skills-man"
INSTALL_DIR="${SKILLS_MAN_INSTALL_DIR:-$HOME/.local/bin}"

err() {
  echo "error: $*" >&2
  exit 1
}

case "$(uname -s)" in
  Darwin) os="apple-darwin" ;;
  Linux) os="unknown-linux-musl" ;;
  *) err "unsupported OS: $(uname -s). On Windows, use install.ps1." ;;
esac

case "$(uname -m)" in
  x86_64 | amd64) arch="x86_64" ;;
  arm64 | aarch64) arch="aarch64" ;;
  *) err "unsupported architecture: $(uname -m)" ;;
esac

target="$arch-$os"
url="https://github.com/$REPO/releases/latest/download/skill-$target.tar.gz"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

echo "Downloading $url"
if command -v curl >/dev/null 2>&1; then
  curl -fsSL "$url" -o "$tmp/skill.tar.gz"
elif command -v wget >/dev/null 2>&1; then
  wget -qO "$tmp/skill.tar.gz" "$url"
else
  err "curl or wget is required"
fi

tar -xzf "$tmp/skill.tar.gz" -C "$tmp"
mkdir -p "$INSTALL_DIR"
# The archive holds a single file named after the target, e.g. skill-aarch64-apple-darwin.
mv "$tmp/skill-$target" "$INSTALL_DIR/skill"
chmod +x "$INSTALL_DIR/skill"

echo "Installed $("$INSTALL_DIR/skill" --version) to $INSTALL_DIR/skill"

case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;
  *)
    echo
    echo "$INSTALL_DIR is not on your PATH. Add this line to your shell profile:"
    echo "  export PATH=\"$INSTALL_DIR:\$PATH\""
    ;;
esac
