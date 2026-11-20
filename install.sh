#!/bin/sh
# installs the sand binary of the latest release on linux
# usage: curl -fsSL https://raw.githubusercontent.com/fosskar/sand/main/install.sh | sh
# SAND_INSTALL_DIR: where to put sand, default ~/.local/bin
# SAND_RELEASE_URL: where to download the release assets from, default the latest github release
set -eu

repo_url=https://github.com/fosskar/sand
release_url=${SAND_RELEASE_URL:-$repo_url/releases/latest/download}
install_dir=${SAND_INSTALL_DIR:-$HOME/.local/bin}
# the highest GLIBC_ symbol version the release binary uses
min_glibc=2.34

die() {
	echo "sand install: $*" >&2
	exit 1
}

unsupported() {
	die "no prebuilt sand binary for $1; build it from source instead: cargo install --git $repo_url"
}

os=$(uname -s)
[ "$os" = Linux ] || unsupported "$os"
machine=$(uname -m)
case $machine in
x86_64 | amd64) ;;
*) unsupported "Linux $machine" ;;
esac
asset=sand-x86_64-linux.tar.gz

if command -v curl >/dev/null; then
	fetch() { curl -fsSL -o "$2" "$1"; }
elif command -v wget >/dev/null; then
	fetch() { wget -q -O "$2" "$1"; }
else
	die "curl or wget is required"
fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

fetch "$release_url/$asset" "$tmp/$asset"
fetch "$release_url/SHA256SUMS" "$tmp/SHA256SUMS"
(cd "$tmp" && grep "  $asset\$" SHA256SUMS | sha256sum -c -) >/dev/null ||
	die "checksum of $asset does not match SHA256SUMS"
tar -xzf "$tmp/$asset" -C "$tmp" sand
# checked before installing, so a binary that cannot run does not replace a working one
if ! output=$("$tmp/sand" --version 2>&1); then
	die "the release binary does not run on this system: $output
it is built on Ubuntu 22.04 and needs glibc $min_glibc or newer (ldd --version shows yours); build it from source instead: cargo install --git $repo_url"
fi

mkdir -p "$install_dir"
# a rename replaces a running sand, which writing over the file would not
cp "$tmp/sand" "$install_dir/.sand.new"
chmod 755 "$install_dir/.sand.new"
mv -f "$install_dir/.sand.new" "$install_dir/sand"

echo "installed sand to $install_dir/sand"
case ":$PATH:" in
*":$install_dir:"*) echo "$install_dir is on PATH" ;;
*) echo "$install_dir is not on PATH; add it to run sand by name" ;;
esac
