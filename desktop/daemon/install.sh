#!/bin/sh
# Builds Vyoma's desktop app, installs it for this user and lets it take over from KDE Connect:
# it becomes this PC as kdeconnectd was (same id and pairings), kdeconnectd is kept from starting,
# and the companion, whose work it now does, is stopped. `install.sh --undo` puts KDE Connect back.
# It asks for your password only to install packages and open the firewall.
set -eu
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
config=${XDG_CONFIG_HOME:-$HOME/.config}
data=${XDG_DATA_HOME:-$HOME/.local/share}
# Keeps kdeconnectd from starting: Plasma's autostart and D-Bus activation both start it.
autostart="$config/autostart/org.kde.kdeconnect.daemon.desktop"
activation="$data/dbus-1/services/org.kde.kdeconnect.service"

as_root() {
    if command -v pkexec >/dev/null 2>&1; then pkexec "$@"; else sudo "$@"; fi
}

if [ "${1:-}" = "--undo" ]; then
    systemctl --user disable --now vyoma-desktop 2>/dev/null || true
    rm -f "$autostart" "$activation"
    [ -f "$config/systemd/user/vyoma-companion.service" ] && systemctl --user enable --now vyoma-companion
    (setsid kdeconnectd >/dev/null 2>&1 &)
    echo "KDE Connect is back. Vyoma's desktop app keeps its folder ($config/vyoma-desktop) until you remove it."
    exit 0
fi

if ! command -v cargo >/dev/null 2>&1 || ! command -v cc >/dev/null 2>&1 || ! command -v pkg-config >/dev/null 2>&1; then
    echo "Installing what's needed to build it..."
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
install -Dm755 "$here/target/release/vyoma-desktop" "$HOME/.local/bin/vyoma-desktop"
"$HOME/.local/bin/vyoma-desktop" --setup

# KDE Connect's ports, as its own packages open them.
if command -v firewall-cmd >/dev/null 2>&1 && firewall-cmd --state >/dev/null 2>&1; then
    firewall-cmd --query-service=kdeconnect >/dev/null 2>&1 || as_root sh -c 'firewall-cmd --permanent --add-service=kdeconnect && firewall-cmd --reload'
elif command -v ufw >/dev/null 2>&1 && systemctl is-active --quiet ufw; then
    grep -q "allow udp 1714:1764" /etc/ufw/user.rules 2>/dev/null && grep -q "allow tcp 1714:1764" /etc/ufw/user.rules 2>/dev/null ||
        as_root sh -c 'ufw allow 1714:1764/udp && ufw allow 1714:1764/tcp'
fi

# Phones paired with this PC stay paired: its identity is kdeconnectd's.
if [ ! -e "$config/vyoma-desktop/device_id" ] && [ -e "$config/kdeconnect/privateKey.pem" ]; then
    "$HOME/.local/bin/vyoma-desktop" --import-kde
fi

mkdir -p "$(dirname "$autostart")" "$(dirname "$activation")"
printf '[Desktop Entry]\nType=Application\nName=KDE Connect\nHidden=true\n' >"$autostart"
printf '[D-BUS Service]\nName=org.kde.kdeconnect\nExec=/bin/false\n' >"$activation"
systemctl --user disable --now vyoma-companion 2>/dev/null || true
pkill -x kdeconnectd 2>/dev/null || true

install -Dm644 "$here/dist/vyoma-desktop.service" "$config/systemd/user/vyoma-desktop.service"
systemctl --user daemon-reload
systemctl --user enable vyoma-desktop
systemctl --user restart vyoma-desktop
echo "Done: Vyoma's desktop app is this PC's KDE Connect now. Logs: journalctl --user -u vyoma-desktop. Undo: $0 --undo"
