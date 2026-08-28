#!/bin/sh
# Install CtxC from a GitHub release.
#
#   curl -fsSL https://raw.githubusercontent.com/nirajgirixd/ctxc/main/install.sh | sh
#
# Detects the platform, downloads the matching archive, checks it against the
# release's SHA256SUMS, and puts the binary somewhere on PATH. No Rust and no
# Node needed — that is the whole point of this script.
#
# Environment:
#   CTXC_VERSION  a tag to install, e.g. v0.2.0. Default: the latest release.
#   CTXC_BIN_DIR  where to put the binary. Default: ~/.local/bin.

set -eu

REPO="nirajgirixd/ctxc"
BIN_DIR="${CTXC_BIN_DIR:-$HOME/.local/bin}"

say() { printf '%s\n' "$*"; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }

need() {
    command -v "$1" >/dev/null 2>&1 || die "$1 is required but not installed"
}

# A downloader that is on essentially every machine, one way or the other.
fetch() {
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL "$1" -o "$2"
    elif command -v wget >/dev/null 2>&1; then
        wget -qO "$2" "$1"
    else
        die "neither curl nor wget is installed"
    fi
}

target_triple() {
    os="$(uname -s)"
    arch="$(uname -m)"

    case "$os" in
        Linux)
            case "$arch" in
                x86_64|amd64) echo "x86_64-unknown-linux-gnu" ;;
                *) die "no prebuilt binary for Linux on $arch; build from source (see USAGE.md)" ;;
            esac
            ;;
        Darwin)
            case "$arch" in
                arm64|aarch64) echo "aarch64-apple-darwin" ;;
                x86_64) echo "x86_64-apple-darwin" ;;
                *) die "no prebuilt binary for macOS on $arch" ;;
            esac
            ;;
        *)
            die "no prebuilt binary for $os; on Windows use install.ps1"
            ;;
    esac
}

# The tag of the newest release, read from the API rather than guessed.
latest_version() {
    tmp="$(mktemp)"
    fetch "https://api.github.com/repos/$REPO/releases/latest" "$tmp"
    version="$(sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' "$tmp" | head -n 1)"
    rm -f "$tmp"
    [ -n "$version" ] || die "could not determine the latest release"
    echo "$version"
}

# Fail loudly on a bad download rather than installing it. A checksum tool is
# present on every supported platform; not finding one is a broken environment,
# not a reason to skip the check.
verify() {
    archive="$1"
    sums="$2"
    name="$(basename "$archive")"

    expected="$(grep " \*\{0,1\}$name\$" "$sums" | awk '{print $1}' | head -n 1)"
    [ -n "$expected" ] || die "$name is not listed in SHA256SUMS"

    if command -v sha256sum >/dev/null 2>&1; then
        actual="$(sha256sum "$archive" | awk '{print $1}')"
    elif command -v shasum >/dev/null 2>&1; then
        actual="$(shasum -a 256 "$archive" | awk '{print $1}')"
    else
        die "no sha256sum or shasum available to verify the download"
    fi

    [ "$actual" = "$expected" ] || die "checksum mismatch for $name"
}

need tar
need uname

target="$(target_triple)"
version="${CTXC_VERSION:-$(latest_version)}"
number="${version#v}"
name="ctxc-${number}-${target}.tar.gz"
base="https://github.com/$REPO/releases/download/$version"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT INT TERM

say "Downloading ctxc $version for $target..."
fetch "$base/$name" "$work/$name"
fetch "$base/SHA256SUMS" "$work/SHA256SUMS"

say "Verifying..."
verify "$work/$name" "$work/SHA256SUMS"

tar -xzf "$work/$name" -C "$work"
mkdir -p "$BIN_DIR"
install -m 755 "$work/ctxc-${number}-${target}/ctxc" "$BIN_DIR/ctxc"

say ""
say "Installed $BIN_DIR/ctxc"

case ":$PATH:" in
    *":$BIN_DIR:"*)
        say ""
        say "Next: cd into a project and run"
        say "  ctxc init"
        ;;
    *)
        # Installing something the shell cannot find is half an install, and
        # the missing half is the one nobody thinks to check.
        say ""
        say "$BIN_DIR is not on your PATH. Add it:"
        say "  export PATH=\"\$PATH:$BIN_DIR\""
        say ""
        say "Then: cd into a project and run \`ctxc init\`."
        ;;
esac
