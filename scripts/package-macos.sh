#!/usr/bin/env sh
# Package the native devd window as a macOS app in a disk image.
#
# Usage:  scripts/package-macos.sh <target-triple> <arch-label>
#   e.g.  scripts/package-macos.sh aarch64-apple-darwin arm64
#         scripts/package-macos.sh x86_64-apple-darwin x86_64
#
# Expects `cargo build --release --target <triple> -p devd-app -p devd-cli` to have run.
# Writes dist/devd-<version>-macos-<arch>.dmg: open it, drag devd to Applications.
#
# The app is signed ad hoc (`codesign -s -`), which makes it a valid, runnable bundle
# but is not an Apple Developer signature, so the first launch needs a right-click and
# Open (the release notes say so). Notarisation needs an Apple Developer account.
set -eu

target=${1:?target triple, e.g. aarch64-apple-darwin}
arch=${2:?arch label, e.g. arm64}
repo=$(cd "$(dirname "$0")/.." && pwd)
cd "$repo"

version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)
bin="target/$target/release"
[ -x "$bin/devd-app" ] || { echo "missing $bin/devd-app: build it first" >&2; exit 1; }
[ -x "$bin/devd" ] || { echo "missing $bin/devd: build it first" >&2; exit 1; }

stage="dist/dmg-$arch"
app="$stage/devd.app"
rm -rf "$stage"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources" dist

cp "$bin/devd-app" "$app/Contents/MacOS/devd-app"
# The command line travels inside the app, so `devd` is one step away for anyone who wants it.
cp "$bin/devd" "$app/Contents/MacOS/devd"

# The icon: the one logo, at every size macOS asks for.
iconset=$(mktemp -d)/devd.iconset
mkdir -p "$iconset"
for s in 16 32 128 256 512; do
  sips -z "$s" "$s" assets/devd-logo.png --out "$iconset/icon_${s}x${s}.png" >/dev/null
  d=$((s * 2))
  sips -z "$d" "$d" assets/devd-logo.png --out "$iconset/icon_${s}x${s}@2x.png" >/dev/null
done
iconutil -c icns -o "$app/Contents/Resources/devd.icns" "$iconset"

cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>devd</string>
  <key>CFBundleDisplayName</key><string>devd</string>
  <key>CFBundleIdentifier</key><string>io.github.ianclemence.devd</string>
  <key>CFBundleExecutable</key><string>devd-app</string>
  <key>CFBundleIconFile</key><string>devd</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>${version}</string>
  <key>CFBundleVersion</key><string>${version}</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>LSApplicationCategoryType</key><string>public.app-category.developer-tools</string>
  <key>CFBundleURLTypes</key>
  <array>
    <dict>
      <key>CFBundleURLName</key><string>devd link</string>
      <key>CFBundleURLSchemes</key><array><string>devd</string></array>
    </dict>
  </array>
</dict>
</plist>
PLIST

codesign --force --deep --sign - "$app"
codesign --verify --deep --strict "$app"

ln -s /Applications "$stage/Applications"
out="dist/devd-$version-macos-$arch.dmg"
rm -f "$out"
hdiutil create -volname "devd $version" -srcfolder "$stage" -ov -format UDZO "$out"
echo "wrote $out"
