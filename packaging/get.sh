#!/bin/sh
# Installs Vyoma's PC app from a release, ready-made (nothing to build):
#   curl -fsSL https://github.com/Prajwaljb/VyomaConnect/releases/latest/download/install.sh | sh
# or, where there's no curl (Ubuntu's desktop has wget instead):
#   wget -qO- https://github.com/Prajwaljb/VyomaConnect/releases/latest/download/install.sh | sh
# It downloads this PC's build, checks it against the release's SHA256SUMS, unpacks it into
# ~/.local/share/vyoma/app and runs the installer in it, which sets up the rest (the libraries it
# needs, the firewall, taking over from KDE Connect) and asks for your password when it must.
# End the command with `sh -s -- --undo` instead of `sh` to give KDE Connect back.
# Published as each release's install.sh; this release's own build is the one it fetches.
set -eu

repo=Prajwaljb/VyomaConnect
# Filled in when the release is made, so each install.sh fetches its own release.
tag=@TAG@

main() {
    data=${XDG_DATA_HOME:-$HOME/.local/share}
    app="$data/vyoma/app"
    # Giving KDE Connect back needs nothing new.
    if [ "${1:-}" = "--undo" ] && [ -f "$app/desktop/daemon/install.sh" ]; then
        exec sh "$app/desktop/daemon/install.sh" "$@" </dev/null
    fi

    case "$(uname -s)" in Linux) ;; *) fail "Vyoma's PC app runs on Linux for now." ;; esac
    case "$(uname -m)" in
        x86_64 | amd64) arch=x86_64 ;;
        aarch64 | arm64) arch=aarch64 ;;
        *) fail "There's no ready-made Vyoma for $(uname -m) yet: build it from source (see the README)." ;;
    esac
    for tool in tar sha256sum; do
        command -v "$tool" >/dev/null 2>&1 || fail "Vyoma's installer needs $tool: install it, then run this again."
    done
    command -v curl >/dev/null 2>&1 || command -v wget >/dev/null 2>&1 || fail "Vyoma's installer needs curl or wget: install one, then run this again."

    base=${VYOMA_DOWNLOADS:-https://github.com/$repo/releases/download/$tag}
    file="vyoma-desktop-$arch.tar.gz"
    tmp=$(mktemp -d)
    trap 'rm -rf "$tmp"' EXIT
    echo "Downloading Vyoma ${tag#v} for this PC..."
    fetch "$base/$file" "$tmp/$file" progress || fail "Couldn't download Vyoma. Check the connection and run this again."
    fetch "$base/SHA256SUMS" "$tmp/SHA256SUMS" || fail "Couldn't download the release's checksums."
    (cd "$tmp" && grep " $file\$" SHA256SUMS | sha256sum -c --quiet -) || fail "The download doesn't match the release's checksum. Nothing was installed."

    mkdir -p "$tmp/app"
    tar -xzf "$tmp/$file" -C "$tmp/app"
    [ -x "$tmp/app/bin/vyoma-desktop" ] || fail "The download isn't a Vyoma build."
    # Swapped in whole, so a failed download never leaves half of one.
    mkdir -p "$data/vyoma"
    rm -rf "$app.old"
    [ ! -e "$app" ] || mv "$app" "$app.old"
    mv "$tmp/app" "$app"
    rm -rf "$app.old"

    # Not reading on from this pipe: the installer asks for nothing on it.
    sh "$app/desktop/daemon/install.sh" "$@" </dev/null
}

# fetch URL FILE [progress]: with curl, or wget where there's no curl. The progress bar only
# on a terminal: piped into a log, wget's would be thousands of lines of dots.
fetch() {
    [ -t 2 ] || set -- "$1" "$2"
    if command -v curl >/dev/null 2>&1; then
        if [ -n "${3:-}" ]; then curl -fSL --progress-bar -o "$2" "$1"; else curl -fsSL -o "$2" "$1"; fi
    elif [ -n "${3:-}" ]; then
        wget -q --show-progress -O "$2" "$1"
    else
        wget -q -O "$2" "$1"
    fi
}

fail() {
    echo "$1" >&2
    exit 1
}

# All of it runs only once the whole script has arrived.
main "$@"
