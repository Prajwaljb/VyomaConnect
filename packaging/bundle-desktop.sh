#!/bin/sh
# bundle-desktop.sh OUT_DIR: builds Vyoma's PC app and packs it, ready-made, as
# OUT_DIR/vyoma-desktop-<arch>.tar.gz: its two programs in bin/, and the installer with what it
# installs (the menu entry, icon, systemd unit and GNOME extension) where the installer looks for them,
# and KDE's Share menu plugin when VYOMA_SHARE_PLUGIN names one built (on Debian 13: KDE 6 is newer
# than Debian 12).
# packaging/get.sh downloads and unpacks it. The release build runs this on Debian 12, so the
# programs run on any distro as new as that (glibc 2.36, GStreamer 1.22).
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
mkdir -p "$1"
out=$(CDPATH= cd -- "$1" && pwd)
arch=$(uname -m)

(cd "$root/desktop/daemon" && cargo build --release --locked)
(cd "$root/desktop/gui" && cargo build --release --locked)

stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
mkdir -p "$stage/bin" "$stage/desktop/daemon"
install -m755 "$root/desktop/daemon/target/release/vyoma-desktop" "$root/desktop/gui/target/release/vyoma" "$stage/bin/"
install -m644 "$root/VERSION" "$root/LICENSE" "$stage/"
install -m755 "$root/desktop/daemon/install.sh" "$stage/desktop/daemon/"
cp -r "$root/desktop/daemon/dist" "$stage/desktop/daemon/"
cp -r "$root/desktop/gnome-extension" "$stage/desktop/"
if [ -n "${VYOMA_SHARE_PLUGIN:-}" ]; then
    install -Dm755 "$VYOMA_SHARE_PLUGIN" "$stage/lib/qt6/plugins/kf6/purpose/vyomaplugin.so"
fi
tar -czf "$out/vyoma-desktop-$arch.tar.gz" --owner=0 --group=0 -C "$stage" .
echo "$out/vyoma-desktop-$arch.tar.gz"
