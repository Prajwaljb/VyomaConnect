#!/bin/sh
# Builds the Vyoma companion, installs it for this user and starts it. Run from anywhere; it asks for
# your password only to install packages. Afterwards it keeps itself supplied (see src/setup.rs).
set -eu
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)

as_root() {
    if command -v pkexec >/dev/null 2>&1; then pkexec "$@"; else sudo "$@"; fi
}

if ! command -v cargo >/dev/null 2>&1 || ! command -v cc >/dev/null 2>&1 || ! command -v pkg-config >/dev/null 2>&1; then
    echo "Installing what's needed to build the companion..."
    if command -v pacman >/dev/null 2>&1; then as_root pacman -S --needed --noconfirm rust gcc pkgconf
    elif command -v apt-get >/dev/null 2>&1; then as_root sh -c 'apt-get update && apt-get install -y cargo build-essential pkg-config'
    elif command -v dnf >/dev/null 2>&1; then as_root dnf install -y cargo gcc pkgconf-pkg-config
    else
        echo "No known package manager: install Rust (cargo) and a C compiler, then run this again." >&2
        exit 1
    fi
fi

echo "Building (a few minutes the first time)..."
(cd "$here" && cargo build --release)
install -Dm755 "$here/target/release/vyoma-companion" "$HOME/.local/bin/vyoma-companion"

"$HOME/.local/bin/vyoma-companion" --setup

install -Dm644 "$here/dist/vyoma-companion.service" "${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user/vyoma-companion.service"
systemctl --user daemon-reload
systemctl --user enable vyoma-companion
systemctl --user restart vyoma-companion
echo "Done. On your phone, Vyoma now shows this PC's extras. Logs: journalctl --user -u vyoma-companion"
