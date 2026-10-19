#!/usr/bin/env sh
# Build, test and package devd for Linux.
#
# Run this on any Linux machine, or inside WSL or a Linux container. It runs the
# whole test suite, builds the release binary, exercises it, and writes the
# artifact and its checksum to dist/.
#
# Usage:
#   ./scripts/release-linux.sh
#
# Requirements: rustc, cargo, cc (for the bundled SQLite), and coreutils.
set -eu

repo=$(cd "$(dirname "$0")/.." && pwd)
cd "$repo"

version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)
if [ -z "$version" ]; then
    echo "could not read the version from Cargo.toml" >&2
    exit 1
fi

target=${TARGET:-$(rustc -vV | sed -n 's/^host: //p')}
if [ -z "$target" ]; then
    echo "could not determine the build target; set TARGET explicitly" >&2
    exit 1
fi
name="devd-v${version}-${target}"

echo "==> devd ${version} for ${target}"

echo "==> cargo test --workspace"
cargo test --workspace

echo "==> cargo build --release"
cargo build --release -p devd-cli --target "$target"

bin="target/${target}/release/devd"
if [ ! -x "$bin" ]; then
    echo "the build did not produce $bin" >&2
    exit 1
fi

# The artifact is only published after it has run. A temporary home keeps this
# away from whatever is already installed on the machine.
home=$(mktemp -d)
trap 'rm -rf "$home"' EXIT

echo "==> smoke test"
reported=$("$bin" --version)
if [ "$reported" != "devd ${version}" ]; then
    echo "expected 'devd ${version}', got '${reported}'" >&2
    exit 1
fi
echo "    $reported"

DEVD_HOME="$home" "$bin" capabilities --json > /dev/null
echo "    capability registry: ok"

# Not fatal: a busy port or a missing Git is a property of the machine, not of
# the binary. It is printed so a broken build is visible in the log.
echo "==> devd doctor (informational)"
DEVD_HOME="$home" "$bin" doctor || true

mkdir -p dist
cp "$bin" "dist/$name"
chmod +x "dist/$name"

(cd dist && sha256sum "$name" > SHA256SUMS.txt)

echo
echo "wrote dist/$name"
cat "dist/SHA256SUMS.txt"
