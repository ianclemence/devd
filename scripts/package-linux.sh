#!/usr/bin/env sh
# Package the native devd window for Linux: a .deb that `apt` installs with its
# dependencies, and a .tar.gz for everything else.
#
# Usage:  scripts/package-linux.sh
#
# Expects `cargo build --release -p devd-app -p devd-cli` to have run on an x86_64 Linux machine.
# Writes dist/devd_<version>_amd64.deb and dist/devd-<version>-linux-x86_64.tar.gz.
set -eu

repo=$(cd "$(dirname "$0")/.." && pwd)
cd "$repo"

version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)
[ -x target/release/devd-app ] || { echo "missing target/release/devd-app: build it first" >&2; exit 1; }
[ -x target/release/devd ] || { echo "missing target/release/devd: build it first" >&2; exit 1; }
mkdir -p dist

desktop_entry() {
  cat <<DESKTOP
[Desktop Entry]
Type=Application
Name=devd
GenericName=Development environment manager
Comment=Runtimes, sites, HTTPS, Git and a team of coding agents, each action approved and recorded
Exec=devd-app %u
Icon=devd
Terminal=false
Categories=Development;
Keywords=php;nginx;mariadb;agents;sites;
MimeType=x-scheme-handler/devd;
StartupWMClass=devd
DESKTOP
}

# ---------------------------------------------------------------- .deb
root="dist/deb/devd_${version}_amd64"
rm -rf "$root"
mkdir -p "$root/DEBIAN" "$root/usr/bin" "$root/usr/share/applications" "$root/usr/share/icons/hicolor/512x512/apps" "$root/usr/share/doc/devd"
install -m 0755 target/release/devd target/release/devd-app "$root/usr/bin/"
install -m 0644 assets/devd-logo.png "$root/usr/share/icons/hicolor/512x512/apps/devd.png"
install -m 0644 LICENSE "$root/usr/share/doc/devd/copyright"
desktop_entry > "$root/usr/share/applications/devd.desktop"
chmod 0644 "$root/usr/share/applications/devd.desktop"

size=$(du -sk "$root/usr" | cut -f1)
cat > "$root/DEBIAN/control" <<CONTROL
Package: devd
Version: ${version}
Architecture: amd64
Maintainer: Ian Clemence <batricupid@gmail.com>
Installed-Size: ${size}
Section: devel
Priority: optional
Homepage: https://github.com/ianclemence/devd
Depends: libvulkan1, libxkbcommon0, libxkbcommon-x11-0, libwayland-client0, libxcb1, libfontconfig1, libssl3 | libssl1.1
Recommends: mesa-vulkan-drivers
Description: Agent-native development environment manager
 devd manages PHP, nginx and MariaDB, local HTTPS and a team of coding agents,
 with every action approved and recorded. This package is the native window and
 the command line (devd).
 .
 The window is drawn on the GPU through Vulkan: a machine without a Vulkan driver
 (mesa-vulkan-drivers, or the vendor's own) cannot open it, and the command line
 still works.
CONTROL

dpkg-deb --build --root-owner-group "$root" "dist/devd_${version}_amd64.deb"

# ------------------------------------------------------------- .tar.gz
tree="dist/tar/devd-${version}-linux-x86_64"
rm -rf "$tree"
mkdir -p "$tree/bin" "$tree/share/applications" "$tree/share/icons/hicolor/512x512/apps"
install -m 0755 target/release/devd target/release/devd-app "$tree/bin/"
install -m 0644 assets/devd-logo.png "$tree/share/icons/hicolor/512x512/apps/devd.png"
desktop_entry > "$tree/share/applications/devd.desktop"
install -m 0644 LICENSE README.md "$tree/"
cat > "$tree/INSTALL.txt" <<TEXT
devd ${version} for Linux (x86_64)

Per-user install (no root):
  mkdir -p ~/.local && cp -r bin share ~/.local/
  update-desktop-database ~/.local/share/applications 2>/dev/null || true
  # make sure ~/.local/bin is on your PATH, then run:  devd-app

The window needs a Vulkan driver (mesa-vulkan-drivers on most distributions) and these
libraries: libvulkan, libxkbcommon(-x11), libwayland-client, libxcb, fontconfig.
The command line (devd) needs none of them.
TEXT
tar -C dist/tar -czf "dist/devd-${version}-linux-x86_64.tar.gz" "devd-${version}-linux-x86_64"

echo "wrote dist/devd_${version}_amd64.deb and dist/devd-${version}-linux-x86_64.tar.gz"
