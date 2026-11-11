#!/bin/sh
# Builds Vyoma's desktop app, installs it for this user and lets it take over from KDE Connect:
# it becomes this PC as kdeconnectd was (same id and pairings), kdeconnectd is kept from starting,
# and the companion, whose work it now does, is stopped. `install.sh --undo` puts KDE Connect back.
# It asks for your password only to install packages and open the firewall.
# `install.sh --update` moves this copy to the newest release and installs that, pairings kept.
# Installed by a package (as /usr/share/vyoma/install.sh) it builds nothing: it only takes over.
# From a release's ready-made build (what `curl ... install.sh | sh` unpacks into
# ~/.local/share/vyoma/app, with its programs in bin/) it builds nothing either, and updates by
# downloading the newest one.
set -eu
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH= cd -- "$here/../.." && pwd)
repo=Prajwaljb/VyomaConnect
packaged=
prebuilt=
bin="$HOME/.local/bin"
if [ ! -f "$here/Cargo.toml" ] && [ -x "$root/bin/vyoma-desktop" ]; then
    prebuilt=1
elif [ ! -f "$here/Cargo.toml" ] && [ -x /usr/bin/vyoma-desktop ]; then
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
# Starts Vyoma at login where the desktop doesn't reach systemd's graphical-session.target
# (Cinnamon, MATE, Xfce, i3...), with the session's display, which systemd may not have been given.
starter="$config/autostart/vyoma-desktop.desktop"

# KDE's Share menu (Purpose: Dolphin, Spectacle, Gwenview, Okular...) gets "Send to phone" from a
# small plugin (desktop/purpose). Qt finds plugins only on its own path, so for this user's copy
# Plasma is told where at login.
share_plugins="$HOME/.local/lib/qt6/plugins"
share_env="$config/plasma-workspace/env/vyoma-share.sh"
has_purpose() {
    for d in /usr/lib/qt6/plugins/kf6/purpose /usr/lib64/qt6/plugins/kf6/purpose /usr/lib/*-linux-gnu*/qt6/plugins/kf6/purpose; do
        [ -d "$d" ] && return 0
    done
    return 1
}
share_note=
put_share_plugin() {
    install -Dm755 "$1" "$share_plugins/kf6/purpose/vyomaplugin.so"
    mkdir -p "$(dirname "$share_env")"
    cat >"$share_env" <<'EOF'
# Vyoma: "Send to phone" in KDE's Share menu; its plugin is in ~/.local/lib/qt6/plugins.
case ":${QT_PLUGIN_PATH:-}:" in
    *":$HOME/.local/lib/qt6/plugins:"*) ;;
    *) export QT_PLUGIN_PATH="$HOME/.local/lib/qt6/plugins${QT_PLUGIN_PATH:+:$QT_PLUGIN_PATH}" ;;
esac
EOF
    case ":${QT_PLUGIN_PATH:-}:" in
        *":$share_plugins:"*) ;;
        *) share_note="\"Send to phone\" shows in KDE's Share menu after you log out and back in once." ;;
    esac
}
# From source: built where KDE's Share menu is, with what that needs; nothing else waits on it.
build_share_plugin() {
    has_purpose || return 0
    src="$here/../purpose"
    configure() { cmake -S "$src" -B "$src/build" -DCMAKE_BUILD_TYPE=Release >/dev/null 2>&1; }
    if ! command -v cmake >/dev/null 2>&1 || ! configure; then
        if command -v pacman >/dev/null 2>&1; then
            as_root pacman -S --needed --noconfirm cmake extra-cmake-modules || true
        elif command -v apt-get >/dev/null 2>&1; then
            as_root apt-get install -y cmake extra-cmake-modules qt6-base-dev libkf6purpose-dev libkf6coreaddons-dev || true
        elif command -v dnf >/dev/null 2>&1; then
            as_root dnf install -y cmake extra-cmake-modules qt6-qtbase-devel kf6-purpose-devel kf6-kcoreaddons-devel || true
        fi
        configure || { echo "Skipped \"Send to phone\" in KDE's Share menu: its build tools aren't here (cmake, extra-cmake-modules, KDE's Purpose)."; return 0; }
    fi
    if cmake --build "$src/build" >/dev/null 2>&1; then
        put_share_plugin "$src/build/bin/kf6/purpose/vyomaplugin.so"
    else
        echo "Skipped \"Send to phone\" in KDE's Share menu: it didn't build (cmake --build $src/build says why)."
    fi
}

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

if [ "${1:-}" = "--update" ] && [ -n "$prebuilt" ]; then
    echo "Getting the newest release..."
    # GitHub sends .../releases/latest on to the newest release's tag.
    tag=$(curl -fsSLI -o /dev/null -w '%{url_effective}' "https://github.com/$repo/releases/latest")
    tag=${tag##*/}
    case "$tag" in v[0-9]*) ;; *) echo "No release found." >&2; exit 1 ;; esac
    current=$(cat "$root/VERSION" 2>/dev/null || echo 0)
    if [ "$(printf '%s\n%s\n' "$current" "${tag#v}" | sort -V | tail -n 1)" = "$current" ]; then
        echo "Vyoma $current is already the newest."
        exit 0
    fi
    get=$(mktemp)
    curl -fsSL -o "$get" "https://github.com/$repo/releases/download/$tag/install.sh"
    # That release's own download and installer from here on.
    trap - EXIT
    exec sh "$get" --updating
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
    rm -f "$autostart" "$activation" "$starter"
    [ -n "$packaged" ] || rm -f "$share_plugins/kf6/purpose/vyomaplugin.so" "$share_env"
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

if [ -n "$prebuilt" ]; then
# Ready-made: only the libraries the programs load, where the desktop didn't bring them.
if ldd "$root/bin/vyoma-desktop" "$root/bin/vyoma" 2>/dev/null | grep -q 'not found'; then
    echo "Installing the libraries it needs..."
    if command -v pacman >/dev/null 2>&1; then
        as_root pacman -S --needed --noconfirm gstreamer gst-plugins-base-libs fontconfig libxkbcommon
    elif command -v apt-get >/dev/null 2>&1; then
        as_root sh -c 'apt-get update && apt-get install -y libgstreamer1.0-0 libgstreamer-plugins-base1.0-0 libgstreamer-gl1.0-0 libfontconfig1 libxkbcommon0'
    elif command -v dnf >/dev/null 2>&1; then
        as_root dnf install -y gstreamer1 gstreamer1-plugins-base fontconfig libxkbcommon
    elif command -v zypper >/dev/null 2>&1; then
        as_root zypper --non-interactive install gstreamer gstreamer-plugins-base libgstgl-1_0-0 fontconfig libxkbcommon0
    fi
    if ldd "$root/bin/vyoma-desktop" "$root/bin/vyoma" 2>/dev/null | grep 'not found' >&2; then
        echo "Vyoma needs these libraries, which this installer couldn't add. Install them with your package manager, then run this again." >&2
        exit 1
    fi
fi
install -Dm755 "$root/bin/vyoma-desktop" "$bin/vyoma-desktop"
install -Dm755 "$root/bin/vyoma" "$bin/vyoma"
mkdir -p "$config/vyoma-desktop" "$data/applications"
# Where updates start from.
printf '%s\n' "$root" >"$config/vyoma-desktop/source"
sed "s|@BIN@|$bin/vyoma|" "$here/dist/vyoma.desktop" >"$data/applications/vyoma.desktop"
install -Dm644 "$here/dist/vyoma.svg" "$data/icons/hicolor/scalable/apps/vyoma.svg"
# Built on Debian 13 (Qt 6.8); an older KDE just doesn't load it.
if has_purpose && [ -f "$root/lib/qt6/plugins/kf6/purpose/vyomaplugin.so" ]; then
    put_share_plugin "$root/lib/qt6/plugins/kf6/purpose/vyomaplugin.so"
fi
elif [ -z "$packaged" ]; then
# The newest Rust any of the crates asks for (cargo names the crate when that moves up).
min_rust=1.96
rust_ok() {
    v=$(cargo --version 2>/dev/null | awk '{print $2}')
    [ -n "$v" ] && [ "$(printf '%s\n%s\n' "$min_rust" "$v" | sort -V | head -n1)" = "$min_rust" ]
}
# The C libraries the build links (the mirror window's video, the window's fonts, the keyboard), by pkg-config name.
libs_ok() {
    command -v cc >/dev/null 2>&1 && command -v pkg-config >/dev/null 2>&1 &&
        pkg-config --exists gstreamer-1.0 gstreamer-base-1.0 gstreamer-video-1.0 gstreamer-gl-1.0 gio-2.0 fontconfig xkbcommon
}
if ! libs_ok || ! rust_ok; then
    echo "Installing what's needed to build it..."
    if command -v pacman >/dev/null 2>&1; then
        as_root pacman -S --needed --noconfirm rust gcc pkgconf gstreamer gst-plugins-base-libs fontconfig libxkbcommon
    elif command -v apt-get >/dev/null 2>&1; then
        as_root sh -c 'apt-get update && apt-get install -y build-essential pkg-config libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev libfontconfig-dev libxkbcommon-dev'
        # Debian's and Ubuntu's own Rust is older than the build needs: theirs only when it's new enough.
        rust_ok || as_root apt-get install -y cargo || true
    elif command -v dnf >/dev/null 2>&1; then
        as_root dnf install -y gcc pkgconf-pkg-config gstreamer1-devel gstreamer1-plugins-base-devel fontconfig-devel libxkbcommon-devel
        rust_ok || as_root dnf install -y cargo || true
    elif ! libs_ok; then
        echo "No known package manager: install a C compiler, pkg-config and the GStreamer (with its base plugins) and fontconfig and xkbcommon development files, then run this again." >&2
        exit 1
    fi
fi
# Still too old (or none): a current Rust for this user from rustup, which the distro packages.
if ! rust_ok; then
    if ! command -v rustup >/dev/null 2>&1 && ! command -v rustup-init >/dev/null 2>&1; then
        if command -v apt-get >/dev/null 2>&1; then as_root apt-get install -y rustup || true
        elif command -v dnf >/dev/null 2>&1; then as_root dnf install -y rustup || true
        fi
    fi
    if command -v rustup >/dev/null 2>&1; then
        rustup toolchain install stable --profile minimal && rustup default stable
    elif command -v rustup-init >/dev/null 2>&1; then
        rustup-init -y --no-modify-path --profile minimal
    fi
    PATH="$HOME/.cargo/bin:$PATH"
    if ! rust_ok; then
        echo "Vyoma needs Rust $min_rust or newer to build. Install it from https://rustup.rs, then run this again." >&2
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
build_share_plugin
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
cat >"$starter" <<'EOF'
[Desktop Entry]
Type=Application
Name=Vyoma
Comment=Starts Vyoma's desktop app with this session
Exec=sh -c "systemctl --user import-environment DISPLAY XAUTHORITY WAYLAND_DISPLAY XDG_CURRENT_DESKTOP XDG_SESSION_TYPE 2>/dev/null; systemctl --user start vyoma-desktop"
NotShowIn=KDE;GNOME;
NoDisplay=true
EOF
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
echo "Done. Open Vyoma from your app menu to pair your phone."
echo "(To undo: $0 --undo)"
[ -z "$gnome_note" ] || echo "$gnome_note"
[ -z "$share_note" ] || echo "$share_note"
# Its window, on its first-run setup: pairing with the phone, side by side with the phone's own steps.
if [ -z "$updating" ] && [ -n "${WAYLAND_DISPLAY:-}${DISPLAY:-}" ]; then
    (setsid "$bin/vyoma" >/dev/null 2>&1 &)
fi
