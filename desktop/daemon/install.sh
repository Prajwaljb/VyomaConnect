#!/bin/sh
# Builds Vyoma's desktop app, installs it for this user and lets it take over from KDE Connect:
# it becomes this PC as kdeconnectd was (same id and pairings), kdeconnectd is kept from starting,
# and the companion, whose work it now does, is stopped. `install.sh --undo` puts KDE Connect back.
# It asks for your password only to install packages and open the firewall.
# `install.sh --update` moves this copy to the newest release and installs that, pairings kept.
# Installed by a package (as /usr/share/vyoma/install.sh) it builds nothing: it only takes over.
set -eu
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH= cd -- "$here/../.." && pwd)
packaged=
bin="$HOME/.local/bin"
if [ ! -f "$here/Cargo.toml" ] && [ -x /usr/bin/vyoma-desktop ]; then
    packaged=1
    bin=/usr/bin
fi
# Rust from rustup, which a systemd unit's PATH (the window's Update) doesn't have.
if [ -d "$HOME/.cargo/bin" ]; then PATH="$HOME/.cargo/bin:$PATH"; fi
config=${XDG_CONFIG_HOME:-$HOME/.config}
data=${XDG_DATA_HOME:-$HOME/.local/share}
# Keeps kdeconnectd from starting: Plasma's autostart and D-Bus activation both start it.
autostart="$config/autostart/org.kde.kdeconnect.daemon.desktop"
activation="$data/dbus-1/services/org.kde.kdeconnect.service"

as_root() {
    if command -v pkexec >/dev/null 2>&1; then pkexec "$@"; else sudo "$@"; fi
}

# GNOME: Vyoma's Shell extension does what GNOME keeps from apps (following the clipboard, the
# presenter's dot, the Overview). GSConnect, GNOME's own KDE Connect, would hold the same port.
extension=vyoma@vyoma.dev
gsconnect=gsconnect@andyholmes.github.io
has_gnome() { command -v gnome-shell >/dev/null 2>&1 && gsettings list-keys org.gnome.shell >/dev/null 2>&1; }
shell_list() { gsettings get org.gnome.shell "$1" | sed 's/^@as //'; }
shell_add() {
    current=$(shell_list "$1")
    case "$current" in
        *"'$2'"*) return 0 ;;
        "[]") gsettings set org.gnome.shell "$1" "['$2']" ;;
        *) gsettings set org.gnome.shell "$1" "${current%]}, '$2']" ;;
    esac
}
# Fails when it wasn't there.
shell_remove() {
    current=$(shell_list "$1")
    case "$current" in *"'$2'"*) ;; *) return 1 ;; esac
    item=$(printf '%s' "$2" | sed 's/[.]/\\./g')
    gsettings set org.gnome.shell "$1" "$(printf '%s' "$current" | sed "s/'$item', //; s/, '$item'//; s/'$item'//")"
}

# An update from the window runs with no terminal: say so if it fails.
updating=
if [ "${1:-}" = "--update" ] || [ "${1:-}" = "--updating" ]; then
    updating=1
    finish() {
        [ "$1" -eq 0 ] || notify-send --app-name=Vyoma --icon=vyoma --hint=string:desktop-entry:vyoma \
            "Vyoma's update didn't finish" "Run $here/install.sh --update in a terminal to see why." 2>/dev/null || true
    }
    trap 'finish $?' EXIT
fi

if [ "${1:-}" = "--update" ] && [ -n "$packaged" ]; then
    trap - EXIT
    echo "Vyoma came from your package manager: update it there."
    exit 0
fi

if [ "${1:-}" = "--update" ]; then
    echo "Getting the newest release..."
    git -C "$root" fetch --quiet --tags --force origin
    tag=$(git -C "$root" tag --list 'v[0-9]*' --sort=-v:refname | head -n 1)
    [ -n "$tag" ] || { echo "No release found." >&2; exit 1; }
    # Never back: a copy ahead of the release (built from main) stays as it is.
    current=$(cat "$root/VERSION" 2>/dev/null || echo 0)
    if [ "$(printf '%s\n%s\n' "$current" "${tag#v}" | sort -V | tail -n 1)" = "$current" ]; then
        echo "Vyoma $current is already the newest."
        exit 0
    fi
    git -C "$root" -c advice.detachedHead=false checkout --quiet "$tag"
    # The release's own installer from here on.
    trap - EXIT
    exec sh "$here/install.sh" --updating
fi

if [ "${1:-}" = "--undo" ]; then
    systemctl --user disable --now vyoma-desktop 2>/dev/null || true
    "$bin/vyoma-desktop" --remove-menus 2>/dev/null || true
    rm -f "$autostart" "$activation"
    [ -n "$packaged" ] || rm -f "$data/applications/vyoma.desktop" "$data/icons/hicolor/scalable/apps/vyoma.svg"
    if has_gnome; then
        shell_remove enabled-extensions "$extension" || true
        [ -n "$packaged" ] || rm -rf "$data/gnome-shell/extensions/$extension"
        if [ -e "$config/vyoma-desktop/gsconnect-was-on" ]; then
            shell_add enabled-extensions "$gsconnect"
            rm -f "$config/vyoma-desktop/gsconnect-was-on"
        fi
    fi
    [ -f "$config/systemd/user/vyoma-companion.service" ] && systemctl --user enable --now vyoma-companion
    (setsid kdeconnectd >/dev/null 2>&1 &)
    echo "KDE Connect is back. Vyoma's desktop app keeps its folder ($config/vyoma-desktop) until you remove it."
    exit 0
fi

if [ -z "$packaged" ]; then
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
# Where updates build from.
mkdir -p "$config/vyoma-desktop"
printf '%s\n' "$root" >"$config/vyoma-desktop/source"
# The window (desktop/gui), beside it: the daemon opens it when a phone asks for the command editor.
(cd "$here/../gui" && cargo build --release)
install -Dm755 "$here/../gui/target/release/vyoma" "$HOME/.local/bin/vyoma"
mkdir -p "$data/applications"
sed "s|@BIN@|$HOME/.local/bin/vyoma|" "$here/dist/vyoma.desktop" >"$data/applications/vyoma.desktop"
install -Dm644 "$here/dist/vyoma.svg" "$data/icons/hicolor/scalable/apps/vyoma.svg"
fi
"$bin/vyoma-desktop" --setup

# KDE Connect's ports, as its own packages open them.
if command -v firewall-cmd >/dev/null 2>&1 && firewall-cmd --state >/dev/null 2>&1; then
    firewall-cmd --query-service=kdeconnect >/dev/null 2>&1 || as_root sh -c 'firewall-cmd --permanent --add-service=kdeconnect && firewall-cmd --reload'
elif command -v ufw >/dev/null 2>&1 && systemctl is-active --quiet ufw; then
    grep -q "allow udp 1714:1764" /etc/ufw/user.rules 2>/dev/null && grep -q "allow tcp 1714:1764" /etc/ufw/user.rules 2>/dev/null ||
        as_root sh -c 'ufw allow 1714:1764/udp && ufw allow 1714:1764/tcp'
fi

# Phones paired with this PC stay paired: its identity is kdeconnectd's.
if [ ! -e "$config/vyoma-desktop/device_id" ] && [ -e "$config/kdeconnect/privateKey.pem" ]; then
    "$bin/vyoma-desktop" --import-kde
fi

mkdir -p "$(dirname "$autostart")" "$(dirname "$activation")"
printf '[Desktop Entry]\nType=Application\nName=KDE Connect\nHidden=true\n' >"$autostart"
printf '[D-BUS Service]\nName=org.kde.kdeconnect\nExec=/bin/false\n' >"$activation"
systemctl --user disable --now vyoma-companion 2>/dev/null || true
pkill -x kdeconnectd 2>/dev/null || true

gnome_note=
if has_gnome; then
    if [ -z "$packaged" ]; then
        rm -rf "$data/gnome-shell/extensions/$extension"
        mkdir -p "$data/gnome-shell/extensions"
        cp -r "$here/../gnome-extension" "$data/gnome-shell/extensions/$extension"
    fi
    shell_remove disabled-extensions "$extension" || true
    shell_add enabled-extensions "$extension"
    if shell_remove enabled-extensions "$gsconnect"; then
        mkdir -p "$config/vyoma-desktop"
        touch "$config/vyoma-desktop/gsconnect-was-on"
        echo "GSConnect is off while Vyoma runs, as both use the same port; --undo turns it back on."
    fi
    # GNOME Shell picks up a new extension when it starts.
    case "${XDG_CURRENT_DESKTOP:-}" in
        *GNOME*) gdbus call --session --dest org.freedesktop.DBus --object-path /org/freedesktop/DBus \
            --method org.freedesktop.DBus.NameHasOwner dev.vyoma.Shell 2>/dev/null | grep -q true ||
            gnome_note="Log out and back in once to turn on Vyoma's GNOME extension (clipboard sync and the presenter's pointer need it)." ;;
    esac
fi

if [ -n "$packaged" ]; then
    # The package's unit, not one a source install left behind.
    rm -f "$config/systemd/user/vyoma-desktop.service"
else
    install -Dm644 "$here/dist/vyoma-desktop.service" "$config/systemd/user/vyoma-desktop.service"
fi
systemctl --user daemon-reload
systemctl --user enable vyoma-desktop
systemctl --user restart vyoma-desktop
echo "Done: Vyoma's desktop app is this PC's KDE Connect now. Logs: journalctl --user -u vyoma-desktop. Undo: $0 --undo"
[ -z "$gnome_note" ] || echo "$gnome_note"
# Its window, on its first-run setup: pairing with the phone, side by side with the phone's own steps.
if [ -z "$updating" ] && [ -n "${WAYLAND_DISPLAY:-}${DISPLAY:-}" ]; then
    (setsid "$bin/vyoma" >/dev/null 2>&1 &)
fi
