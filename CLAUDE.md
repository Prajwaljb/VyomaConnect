# VyomaConnect

Android client for the KDE Connect protocol (Kotlin + Compose), plus a future Rust Linux companion.
Package `dev.vyoma.connect` (debug: `.debug`). minSdk 31, target/compile 37. FOSS only (F-Droid): no GMS, Firebase,
or proprietary SDKs. Licence: GPL-3.0-or-later (`LICENSE`), as KDE Connect is GPL; new dependencies must be
compatible with it. Roadmap and architecture: `docs/PLAN.md`.

## Build
```bash
source scripts/env.sh            # user-local JDK 21 + Android SDK; required before gradlew
./gradlew :app:assembleDebug
./gradlew :core:protocol:test    # unit tests
VYOMA_INTEROP=1 ./gradlew :core:protocol:test --rerun   # live test vs this PC's kdeconnectd
scripts/deploy.sh debug          # install + launch on phone
./gradlew :app:assembleRelease   # signed with the release key in ~/.local/share/vyoma-signing/ (not in the repo; back it up, losing it means reinstalling)
./gradlew :app:assembleDemo      # demo build (.demo): simulated laptop + companion in-app over loopback, no network;
                                 # Settings > Generate log writes perf/device diagnostics to Downloads (app/.../demo/)
(cd desktop/companion && cargo build --release)   # Linux companion (Rust); run inside it for its .cargo linker fix
(cd desktop/companion && cargo test --release -p vyoma-core && cargo test --release)   # core + companion tests
(cd desktop/gui && cargo build --release && ./target/release/vyoma)   # the desktop window (talks to vyoma-desktop)
(cd desktop/daemon && cargo build --release && VYOMA_DIR=<scratch dir> VYOMA_ALLOW=<ids> VYOMA_ANNOUNCE=<IPs> ./target/release/vyoma-desktop)
                                 # standalone desktop app (in development, not installed); drive it with
                                 # busctl --user call dev.vyoma.Desktop /dev/vyoma/Desktop dev.vyoma.Desktop1 Devices|Pair|Accept|Reject|Unpair|Refresh|Browse|PairingCode.
                                 # Against this PC's kdeconnectd: allow only its id, announce to the PC's own global IPv6 (it ignores
                                 # loopback UDP), check codes with qdbus6 verificationKey, unpair at the end, rmdir ~/.config/kdeconnect/<dev id>
install -Dm755 desktop/companion/target/release/vyoma-companion ~/.local/bin/ && systemctl --user restart vyoma-companion
                                 # the installed systemd user unit runs ~/.local/bin/vyoma-companion; logs: journalctl --user -u vyoma-companion
```

## Modules
- `core/protocol`: pure JVM (no Android). Packets, identity, crypto/TLS, LanBackend, Device (pairing, plugins), DeviceHub.
  Keep it Android-free so it stays testable against kdeconnectd on the JVM; platform hooks go through `LanBackend.Config`
  and interfaces like `CredentialStore`/`TrustStore`.
- `core/designsystem`: theme, motion, One UI-style components.
- `app`: Application/AppContainer (manual lazy DI), `LinkService` (single connectedDevice FGS), UI.
- `desktop/core` (`vyoma-core`): the desktop side's shared protocol core (identity, links, payloads), no desktop
  calls, so it serves every desktop and macOS: discovery, links, pairing, trust store, the device hub (`device.rs`).
- `desktop/daemon` (`vyoma-desktop`): the standalone desktop app growing on that core (`docs/PLAN.md` section 10): pairing
  prompt as a freedesktop notification with Accept/Reject, `dev.vyoma.Desktop1` D-Bus control for the future GUI;
  stock plugins in `plugins.rs`, `clipboard.rs` (wl-clipboard), `share.rs` (files land in XDG Downloads), `media.rs`
  (MPRIS), `volume.rs` (pactl), `commands.rs`, `notifications.rs`, `telephony.rs` (calls: notification with Answer/Decline/Silence (`vyoma.call.request`), music paused for the call; answered here, a notification with Hang up and Use phone / Use this PC), `hfp.rs` (the call's sound on this PC: BlueZ hands-free with the PC as the phone's headset; PipeWire routes it to the default speaker and mic by itself), `messages.rs` (texts over the stock sms/contacts packets, in memory, for the window's Messages page: D-Bus `Conversations`, `Thread`, `TextThread`, `UnlockTexts`, signal `MessagesChanged`; behind the phone's texts lock (`vyoma.texts.*`, phone `platform/TextsLock.kt`): the page asks, the owner confirms with a fingerprint on the phone, which refuses texts until then; relocked (and dropped from memory) when the screen locks, after 10 min unused or when the phone goes), `presenter.rs` (the pointer dot: a layer-shell overlay, smithay-client-toolkit), `phonefiles.rs` (D-Bus `Browse`: the phone's SFTP mounted with sshfs under $XDG_RUNTIME_DIR/vyoma/<n>/<phone> (Vyoma Desktop), a new <n> each mount so no file manager shows a stale folder; Dolphin lists it under Remote by itself; GTK file managers (Thunar, Nautilus, Nemo, Caja...) browse it through GVfs instead (`gvfs.rs`: an ssh alias per phone in ~/.config/vyoma-desktop/ssh, one `Include` in ~/.ssh/config; `VYOMA_GVFS=1|0` overrides the choice), else a bookmark (`places.rs`), host key checked against its cert; sshfs runs as a transient `vyoma-files-<id>` user unit, as the app's own unit has NoNewPrivileges; over the cable through an adb forward on a Unix socket, which the binary's own `--relay` mode gives ssh as its ProxyCommand: adb's TCP listener keeps Nagle on, ~50 ms an open; the one-time password goes by SSH_ASKPASS, never sshfs's password_stdin, which gives up after 60 s), `input.rs` (RemoteDesktop portal), `menus.rs` ("Send to <phone>" on a right-clicked file or folder in Dolphin, Thunar, Nemo, Nautilus and Caja, rewritten as phones pair; runs `vyoma-desktop --send <id> <paths>` → D-Bus `Send`, with notifications; a folder goes as its files with a `folder` field, rebuilt on the phone; `--remove-menus` for `install.sh --undo`; not written when `VYOMA_DIR` is set), `update.rs` (once a day asks GitHub's latest release with curl, compares with `VERSION` at the repo root (the one version, the phone's versionName too), notifies with Update; This PC's Updates card, D-Bus `Update`/`SetUpdateChecks`, state in updates.json; Update runs `install.sh --update` from the clone `install.sh` notes in `source`, as transient unit `vyoma-update`: checks out the newest release tag, never back, reinstalls, restarts), `commands.rs` (runcommand; `commands.json` entries carry Vyoma's `fingerprint` and `everyTime` flags, which the phone honours (every time: asked on each run, past the app lock's grace, and never run by a rule); a phone's add/edit (`setup`) opens the window's Commands page through `window.rs`: the running window's `dev.vyoma.Window1.Show`, else `systemd-run --user` of `vyoma` beside the daemon). Per-phone permissions: `vyoma_companion::permits` (switches in the window; a refused request gets `vyoma.denied`). What phones did: `vyoma_companion::activity` (the window's Activity page). `VYOMA_PLAYERS=<bus suffix>` limits it to a test player.
  Own identity in `~/.config/vyoma-desktop`, separate from the companion's (`--import-kde` takes over kdeconnectd's instead).
  Every `vyoma.*` feature is the companion's own code: each paired phone gets a `vyoma_companion::session::serve`.
  The phone treats it as its own companion (`DeviceIdentity.featuresOf`). The cable, tethering, Bluetooth and the setup
  offer start only while kdeconnectd isn't running. `install.sh` takes over from KDE Connect (`--undo` gives it back): never run it unasked.
- `desktop/gui` (`vyoma-gui`, binary `vyoma`): the desktop app's window, Slint (GPL-3.0 option) in the phone app's look
  (`ui/app.slint`: the PC wallpaper as a soft canvas with its accent (`src/look.rs`, the phone's formulas, from the daemon's `Wallpaper`), glass cards whose rims and shadows follow the wallpaper's light, centred heroes, tiles, the floating glass sidebar, glass dialogs and status bubble, Inter; first run is the phone's setup steps, `--setup` or This PC > Run setup again shows it again, done is kept in ~/.config/vyoma-desktop/window.json; motion as the phone's: pages rise in, the title docks in a glass bubble once scrolled, pressed things dip, rings and bars ease; Up/Down walk the sidebar, Escape/Return answer dialogs; a phone's Clipboard card has the one-time adb Allow for background clipboard sync, D-Bus `AllowClipboardSync`, state in Details `clipboardSync` on|ask|plug; a Commands page adds and edits commands in a sheet, with Ask for fingerprint first and Every time; a phone's Messages tile opens its conversations and a thread with the reply box, `src/texts.rs`). App id `vyoma` (matches vyoma.desktop; the mirror window takes it too, `mirror.rs` `brand`, as do notifications' `desktop-entry`). Wrapped Text needs `min-height: self.preferred-height` (Slint 1.18 shrinks it otherwise) and long unbroken words go through `breakable()`. The command editor warns (`command_warning`: a program not on PATH, sudo, a name used twice) and Delete offers Undo. One window at a time (`src/instance.rs` holds `dev.vyoma.Window`; a second start, or `--page commands|this-pc|activity|<id>`, shows that page in the open one). `install.sh` installs it as ~/.local/bin/vyoma with a menu entry.. It only shows and asks: `src/daemon.rs` is the `Daemon`
  trait (D-Bus `dev.vyoma.Desktop1` today, a local socket on macOS later), rereading on the daemon's `Changed` signal.
  `vyoma --snapshot out.png [--dark] [--plain (no wallpaper)] [--size WxH (780x540 is the smallest window)] [--scroll PX] [--scene phones|pairing|nearby|away|unpair|add-done|notice|cable|pc|pc-setup|activity|activity-empty|activity-late|commands|commands-empty|commands-edit|commands-warn|messages|messages-thread|messages-silent|messages-loading|messages-asking|messages-declined|messages-locked|setup|setup-pairing|setup-search|setup-ready|setup-moment|empty|add|add-offline|down]` renders it offscreen
  with made-up phones (after the enter animations settle; `cable` shows the clipboard Allow): check the look with that, never by capturing the screen.
- `desktop/gnome-extension` (uuid `vyoma@vyoma.dev`, ESM, GNOME 45+): what GNOME keeps from apps, on the session
  bus as `dev.vyoma.Shell1` (client: `companion/src/shell.rs`): the clipboard (GNOME has no data-control protocol;
  `companion/src/clip.rs` picks it or wl-clipboard), the presenter dot (Cairo, drawn as presenter.rs draws it), the
  Overview steps and workspace switch, the screenshot (GNOME lets only its own parts capture silently; Spectacle on
  Plasma), screen sizes, the notification Reply dialog (GNOME has no inline reply; zenity on other such desktops), and the volume/brightness pop-up for changes the phone makes (`ShowOsd`; GNOME shows it only for its own keys).
  Bump its `VERSION` with `shell::VERSION` on any change: a running older one stays until the next login. It
  answers only vyoma-desktop/vyoma-companion (the caller's /proc/<pid>/exe). `install.sh` copies it (packaged:
  /usr/share/gnome-shell/extensions), enables it with gsettings and turns GSConnect off (back on with `--undo`);
  GNOME Shell loads it at the next login; This PC shows a "GNOME extension" check. GNOME without the extension:
  brightness via Mutter's DisplayConfig `Backlight`, monitors via ddcutil (`companion/src/ddc.rs`, never on
  Plasma, which does DDC itself), power via org.gnome.SessionManager/logind, wake via Mutter `PowerSaveMode`,
  wallpaper via gsettings. Test it headless: `dbus-run-session` + `gnome-shell --headless --wayland
  --virtual-monitor 1280x720` with XDG_CONFIG_HOME/XDG_DATA_HOME on a scratch dir (never the real dconf).
- X11 desktops (Cinnamon, MATE, Xfce, Plasma on X11, i3), picked when `XDG_SESSION_TYPE=x11` or DISPLAY is set without
  WAYLAND_DISPLAY: `companion/src/x11.rs` (x11rb) is the clipboard (its own thread owns CLIPBOARD on a hidden window;
  TARGETS then UTF8_STRING, INCR pieces, XFIXES for others' copies, password-manager copies skipped), input (XTEST;
  a keysym no key types goes on a spare keycode, given back on drop), RandR's primary monitor, DPMS wake, the EWMH
  workspace switch and the layout from `_XKB_RULES_NAMES`; `clip.rs`, `desktop.rs`, `pc.rs`, `screen.rs` (ximagesrc
  instead of the portal's PipeWire node) and `screenshot.rs` route to it; daemon `input.rs` (`Sink::X11`) and
  `presenter_x11.rs` (an override-redirect dot: ARGB with a compositor, Shape-cut without, clicks pass through).
  `install.sh` writes `~/.config/autostart/vyoma-desktop.desktop` (NotShowIn KDE;GNOME), which hands systemd the
  session's DISPLAY and starts the unit where graphical-session.target is never reached. Tests against Xvfb
  (`Xvfb :97 -screen 0 1280x720x24 -nolisten tcp`): `DISPLAY=:97 XDG_SESSION_TYPE=x11 VYOMA_X11_TEST=1 cargo test
  --release --lib x11::` in companion, `... cargo test --release presenter_x11` in daemon. Not yet run on a real
  Cinnamon/MATE/Xfce session; wlroots (Sway, Hyprland) input still has no path.
- `desktop/mac` (`vyoma-mac`): the Mac's PC side on vyoma-core (`docs/PLAN.md` section 10, macOS): pairs after a
  dialog with the code, serves ping, battery, find my phone, lock (display sleep), clipboard (pbcopy/pbpaste polled each
  second), share (files into ~/Downloads); `--install` makes it a LaunchAgent (`launchd.rs`; from Vyoma.app it runs the
  copy inside the app, which the window installs on first open or after the app moved). Vyoma.app
  (`packaging/bundle-mac.sh`: window `vyoma` as the app, `vyoma-mac` beside it, id `dev.vyoma.Vyoma`, icon
  `desktop/mac/dist/vyoma.icns` from `gen.py`, ad-hoc signed) is the CI artifact `Vyoma-mac`; inside it notifications go
  through UserNotifications under Vyoma's name (`notify.rs`, osascript outside the app or if macOS refuses). The window reaches it over
  `control.sock` in ~/Library/Application Support/Vyoma (mode 0600; `VYOMA_SOCKET` overrides): JSON lines, D-Bus method
  names, `Events` streams changes (`control.rs`; window side `gui/src/socket.rs` behind `daemon::connect()`); methods it
  lacks answer "Not on the Mac yet". macOS's own tools only
  (`mac.rs`), a device's text goes to `osascript` as argv after our own first word, never as script. Nobody has a
  Mac: `.github/workflows/macos.yml` builds and tests core, mac and the window on GitHub's runners and keeps Vyoma.app
  (universal) as an artifact; it compiles and tests on Linux too (`cd desktop/mac && cargo test --release`). Not yet run on a Mac.
- `desktop/icon/gen.py`: Vyoma's icon ("Moon": the PC as a white glass orb, the phone as a small orb in a gap of one ring, on a sky tile) for every place from one geometry: Android adaptive layers, monochrome, notification glyph, splash; the desktop's `dist/vyoma.svg` (installed into hicolor) and the window's `ui/icons/vyoma.svg`. Run `python3 desktop/icon/gen.py .` from the repo root after changing it.
- `desktop/companion`: lib + bin (the session loop is `session.rs`, shared with the desktop app); the bin is the Rust daemon beside stock kdeconnectd, built on `vyoma-core`. Dials only phones kdeconnectd trusts (mDNS), pairs with
  no prompt via `CompanionProof` (signed with kdeconnectd's key; phone verifies against the pinned PC cert), speaks
  `vyoma.*` packets. Phone folds it into its PC (`DeviceIdentity.companionOf`, `DeviceState.isCompanion`).
  Also watches sysfs for an adb USB interface and, for phones with Vyoma installed, runs `adb reverse tcp:1765 tcp:1716`
  (+ payload mirrors phone 1839..1864 → PC 1739..1764)
  plus the READ_LOGS / SYSTEM_ALERT_WINDOW grants, once the PC has allowed that APK's signing key (src/usb.rs). Installed as a systemd user unit.
  Installs its own runtime dependencies (src/setup.rs): notification with an Install button at start, or `--check` / `--setup [--dry-run]`; `install.sh` builds and installs it all. A new external tool it runs must be added to `NEEDS` there.

- `packaging/aur`: the AUR package `vyoma` (PKGBUILD + `.SRCINFO`, from the release tarball; binaries in /usr/bin, the
  KDE handoff left to `/usr/share/vyoma/install.sh`, which builds nothing when packaged; the update check then only
  says a release is out). `fastlane/metadata/android/en-US`: store listing for IzzyOnDroid/F-Droid (changelogs/<versionCode>.txt).
- `site/`: the website, https://prajwaljb.github.io/VyomaConnect/ (static, no build; `.github/workflows/pages.yml` publishes it on
  each push to main touching it, filling `@VERSION@`). The phone app's look: the first run's dunes with the draggable glass
  lens (`assets/site.js` paints the scene into a bitmap and bends it through the rim), glass cards, the Home tiles. Icons are
  Material Symbols Rounded (Apache-2.0) in `assets/icons.svg`; Inter subset in `assets/inter.woff2`. No third-party requests.
- Ready-made PC builds: `packaging/bundle-desktop.sh OUT` (both binaries + installer, extension, dist, VERSION, LICENSE in
  `vyoma-desktop-<arch>.tar.gz`), built on Debian 12 (oldest glibc) by `.github/workflows/desktop-release.yml` for x86_64
  and aarch64 on each `v*` tag, which also uploads `SHA256SUMS` and `packaging/get.sh` as the release's `install.sh`
  (its `@TAG@` filled in): the one-liner `curl -fsSL .../releases/latest/download/install.sh | sh` (wget too) checks the sum,
  unpacks into ~/.local/share/vyoma/app and runs its `desktop/daemon/install.sh`, which there is in `prebuilt` mode
  (`bin/vyoma-desktop` beside it, no Cargo.toml): installs missing runtime libraries (ldd), copies the binaries;
  `--update` fetches the newest release's install.sh. `VYOMA_DOWNLOADS=<url>` points get.sh at a local copy for tests.
- Pairing by QR code: the window shows `PairingCode` (the release page's link, the invite after `#`: id, name, cert
  fingerprint, IPv4 addresses, port, a one-time 10-minute secret; `vyoma_core::invite`), renewed every 5 min while shown,
  `EndPairingCode` when hidden. The phone (`PairScanActivity`: CameraX + ZXing; `service/CodePairing.kt`; protocol
  `crypto/QrInvite.kt`) dials those addresses, checks the fingerprint and sends the pair request with `vyomaInvite` =
  HMAC over both certs; the hub pairs on it with no prompt (one phone per secret), else prompts as usual.
  `vyoma://pair?...` opens the scanner too, asking first. Live check: `QrPairLiveTest` with `VYOMA_QR_CODE` (it unpairs after).
- Connection doctor: a PC away on the phone (Home's card, the Connection page) runs `service/ConnectionDoctor.kt`: Wi-Fi,
  VPN, local network permission, the PC's last address on this subnet, and a TCP knock on it (answers / refused = Vyoma
  not running / instant "no route" = its firewall / silence), each cause in plain words with its fix. The PC's Firewall
  check reads firewalld (`--query-port`) or ufw's rules file and offers Let phones through (D-Bus `OpenFirewall`, pkexec,
  `FirewallDone`), also on an away phone's page (`--scene away`).
- Releasing: bump `VERSION` and `versionCode`, add the changelog, push the tag `vX.Y.Z`: CI makes the draft release with the
  PC builds; build the APK and `gh release upload` `vyoma-X.Y.Z.apk` + `.sha256` and the notes to it (the user publishes
  the draft), then the PKGBUILD's pkgver and sha256 (`makepkg -g`) and `makepkg --printsrcinfo > .SRCINFO`.

## Conventions
- Protocol details and extension rules: the `protocol-ref` skill. Design rules: the `oneui-design` skill.
- Device/plugin state is mutated only on the device's serial dispatcher; never block in `Plugin.onPacket`.
- Widgets and tiles never do network I/O; they read cached state and send intents to the service.
- The user's real phone is paired with desktop KDE Connect; never unpair or alter it in tests.
- Since 2026-11-07 this PC runs `vyoma-desktop` (systemd user unit, identity taken over from kdeconnectd) instead of
  kdeconnectd + the companion; logs: `journalctl --user -u vyoma-desktop`; `desktop/daemon/install.sh --undo` goes back.
  The kdeconnectd interop test needs kdeconnectd running, so ask before switching back for it.
