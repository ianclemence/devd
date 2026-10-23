#!/bin/sh
# Install devd on Linux.
#
# The Windows installer is a per-user NSIS package; this is the Linux
# equivalent: no administrator rights, everything under the user's own
# directories, and a script that can also undo itself.
#
#   ./install-linux.sh                     install, then run the first-time setup
#   ./install-linux.sh --no-setup          install only
#   ./install-linux.sh --uninstall         remove the program, keep your data
#   ./install-linux.sh --uninstall --purge remove the program and your data
#
# The data - your projects, databases, certificates and settings - lives in
# ${XDG_DATA_HOME:-$HOME/.local/share}/devd. Removing the program never
# removes it unless you ask for it with --purge.

set -eu

BINARY=""
DESKTOP_BINARY=""
DO_SETUP=1
UNINSTALL=0
PURGE=0
PREFIX=""

usage() {
  sed -n '2,20p' "$0" | sed 's/^# \{0,1\}//'
  exit "${1:-0}"
}

while [ $# -gt 0 ]; do
  case "$1" in
    --binary) BINARY="${2:?--binary needs a path}"; shift 2 ;;
    --desktop-binary) DESKTOP_BINARY="${2:?--desktop-binary needs a path}"; shift 2 ;;
    --no-setup) DO_SETUP=0; shift ;;
    --uninstall) UNINSTALL=1; shift ;;
    --purge) PURGE=1; UNINSTALL=1; shift ;;
    --prefix) PREFIX="${2:?--prefix needs a path}"; shift 2 ;;
    -h|--help) usage 0 ;;
    *) echo "unknown option: $1" >&2; usage 1 ;;
  esac
done

BIN_DIR="${PREFIX:-${XDG_BIN_HOME:-$HOME/.local/bin}}"
DATA_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/devd"
APP_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
ICON_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor/256x256/apps"

say() { printf '%s\n' "$*"; }
step() { printf '\n==> %s\n' "$*"; }

uninstall() {
  step "Stopping devd"
  if command -v devd >/dev/null 2>&1; then
    devd stack down >/dev/null 2>&1 || true
  fi

  step "Removing the program"
  rm -f "$BIN_DIR/devd"
  rm -f "$BIN_DIR/devd-desktop"
  rm -f "$APP_DIR/devd.desktop"
  rm -f "$ICON_DIR/devd.png"
  say "    removed $BIN_DIR/devd"

  if [ "$PURGE" -eq 1 ]; then
    step "Removing your data"
    rm -rf "$DATA_DIR"
    say "    removed $DATA_DIR"
  else
    say ""
    say "    Your data is still in $DATA_DIR."
    say "    Remove it with: $0 --uninstall --purge"
  fi
}

if [ "$UNINSTALL" -eq 1 ]; then
  uninstall
  exit 0
fi

if [ -z "$BINARY" ]; then
  echo "No devd binary was given." >&2
  echo "Build one with: cargo build --release -p devd-cli" >&2
  echo "then run: $0 --binary target/release/devd" >&2
  exit 1
fi

if [ ! -f "$BINARY" ]; then
  echo "There is no file at $BINARY" >&2
  exit 1
fi

step "Installing devd"
mkdir -p "$BIN_DIR"
cp "$BINARY" "$BIN_DIR/devd"
chmod 0755 "$BIN_DIR/devd"
say "    $BIN_DIR/devd"

if [ -n "$DESKTOP_BINARY" ] && [ -f "$DESKTOP_BINARY" ]; then
  cp "$DESKTOP_BINARY" "$BIN_DIR/devd-desktop"
  chmod 0755 "$BIN_DIR/devd-desktop"
  say "    $BIN_DIR/devd-desktop"
else
  say "    no desktop binary was supplied: the terminal interface is the whole application here"
fi

case ":$PATH:" in
  *":$BIN_DIR:"*) : ;;
  *) say ""
     say "    $BIN_DIR is not on your PATH. Add it with:"
     say "        export PATH=\"$BIN_DIR:\$PATH\"" ;;
esac

if [ -f "assets/devd-logo.png" ] && [ -d "$ICON_DIR" ]; then
  cp "assets/devd-logo.png" "$ICON_DIR/devd.png" 2>/dev/null || true
fi

if [ -f "assets/devd.desktop" ]; then
  mkdir -p "$APP_DIR"
  sed "s|@EXEC@|$BIN_DIR/devd|g" "assets/devd.desktop" > "$APP_DIR/devd.desktop"
  chmod 0644 "$APP_DIR/devd.desktop"
  say "    $APP_DIR/devd.desktop"
fi

if [ "$DO_SETUP" -eq 1 ]; then
  step "Setting up"
  say "    The setup reports each phase as it goes:"
  say "      1. Requirements - what is missing, and what this run will do about it"
  say "      2. Runtime - devd uses the system PHP on Linux"
  say "      3. Services - nginx and MariaDB come from your package manager"
  say "      4. Starting the stack"
  say "      5. First site - a site is created and a certificate is issued"
  say "      6. Certificate authority - trusted for this user"
  say ""
  "$BIN_DIR/devd" setup --yes || {
    say ""
    say "    The setup did not finish. Run it again when you are ready:"
    say "        $BIN_DIR/devd setup --yes"
    exit 1
  }
fi

say ""
say "devd is installed."
say "    app     $BIN_DIR/devd ui"
say "    doctor  $BIN_DIR/devd doctor"
say "    data    $DATA_DIR"
