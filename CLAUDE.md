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
                                 # busctl --user call dev.vyoma.Desktop /dev/vyoma/Desktop dev.vyoma.Desktop1 Devices|Pair|Accept|Reject|Unpair|Refresh|Browse.
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
  (MPRIS), `volume.rs` (pactl), `commands.rs`, `notifications.rs`, `telephony.rs` (calls: notification with Mute, music paused for the call), `presenter.rs` (the pointer dot: a layer-shell overlay, smithay-client-toolkit), `phonefiles.rs` (D-Bus `Browse`: the phone's SFTP mounted with sshfs under $XDG_RUNTIME_DIR/vyoma/<n>/<phone> (Vyoma Desktop), a new <n> each mount so no file manager shows a stale folder; Dolphin lists it under Remote by itself; GTK file managers (Thunar, Nautilus, Nemo, Caja...) browse it through GVfs instead (`gvfs.rs`: an ssh alias per phone in ~/.config/vyoma-desktop/ssh, one `Include` in ~/.ssh/config; `VYOMA_GVFS=1|0` overrides the choice), else a bookmark (`places.rs`), host key checked against its cert; sshfs runs as a transient `vyoma-files-<id>` user unit, as the app's own unit has NoNewPrivileges; over the cable through an adb forward on a Unix socket, which the binary's own `--relay` mode gives ssh as its ProxyCommand: adb's TCP listener keeps Nagle on, ~50 ms an open; the one-time password goes by SSH_ASKPASS, never sshfs's password_stdin, which gives up after 60 s), `input.rs` (RemoteDesktop portal), `commands.rs` (runcommand; `commands.json` entries carry Vyoma's `fingerprint` and `everyTime` flags, which the phone honours (every time: asked on each run, past the app lock's grace, and never run by a rule); a phone's add/edit (`setup`) opens the window's Commands page through `window.rs`: the running window's `dev.vyoma.Window1.Show`, else `systemd-run --user` of `vyoma` beside the daemon). Per-phone permissions: `vyoma_companion::permits` (switches in the window; a refused request gets `vyoma.denied`). What phones did: `vyoma_companion::activity` (the window's Activity page). `VYOMA_PLAYERS=<bus suffix>` limits it to a test player.
  Own identity in `~/.config/vyoma-desktop`, separate from the companion's (`--import-kde` takes over kdeconnectd's instead).
  Every `vyoma.*` feature is the companion's own code: each paired phone gets a `vyoma_companion::session::serve`.
  The phone treats it as its own companion (`DeviceIdentity.featuresOf`). The cable, tethering, Bluetooth and the setup
  offer start only while kdeconnectd isn't running. `install.sh` takes over from KDE Connect (`--undo` gives it back): never run it unasked.
- `desktop/gui` (`vyoma-gui`, binary `vyoma`): the desktop app's window, Slint (GPL-3.0 option) in the phone app's look
  (`ui/app.slint`: the PC wallpaper as a soft canvas with its accent (`src/look.rs`, the phone's formulas, from the daemon's `Wallpaper`), glass cards whose rims and shadows follow the wallpaper's light, centred heroes, tiles, the floating glass sidebar, glass dialogs and status bubble, Inter; first run is the phone's setup steps, `--setup` or This PC > Run setup again shows it again, done is kept in ~/.config/vyoma-desktop/window.json; motion as the phone's: pages rise in, the title docks in a glass bubble once scrolled, pressed things dip, rings and bars ease; Up/Down walk the sidebar, Escape/Return answer dialogs; a phone's Clipboard card has the one-time adb Allow for background clipboard sync, D-Bus `AllowClipboardSync`, state in Details `clipboardSync` on|ask|plug; a Commands page adds and edits commands in a sheet, with Ask for fingerprint first and Every time). One window at a time (`src/instance.rs` holds `dev.vyoma.Window`; a second start, or `--page commands|this-pc|activity|<id>`, shows that page in the open one). `install.sh` installs it as ~/.local/bin/vyoma with a menu entry.. It only shows and asks: `src/daemon.rs` is the `Daemon`
  trait (D-Bus `dev.vyoma.Desktop1` today, a local socket on macOS later), rereading on the daemon's `Changed` signal.
  `vyoma --snapshot out.png [--dark] [--plain (no wallpaper)] [--scene phones|pairing|nearby|unpair|notice|cable|pc|pc-setup|activity|activity-empty|commands|commands-empty|commands-edit|setup|setup-pairing|setup-search|setup-ready|setup-moment|empty|down]` renders it offscreen
  with made-up phones (after the enter animations settle; `cable` shows the clipboard Allow): check the look with that, never by capturing the screen.
- `desktop/companion`: lib + bin (the session loop is `session.rs`, shared with the desktop app); the bin is the Rust daemon beside stock kdeconnectd, built on `vyoma-core`. Dials only phones kdeconnectd trusts (mDNS), pairs with
  no prompt via `CompanionProof` (signed with kdeconnectd's key; phone verifies against the pinned PC cert), speaks
  `vyoma.*` packets. Phone folds it into its PC (`DeviceIdentity.companionOf`, `DeviceState.isCompanion`).
  Also watches sysfs for an adb USB interface and, for phones with Vyoma installed, runs `adb reverse tcp:1765 tcp:1716`
  (+ payload mirrors phone 1839..1864 → PC 1739..1764)
  plus the READ_LOGS / SYSTEM_ALERT_WINDOW grants, once the PC has allowed that APK's signing key (src/usb.rs). Installed as a systemd user unit.
  Installs its own runtime dependencies (src/setup.rs): notification with an Install button at start, or `--check` / `--setup [--dry-run]`; `install.sh` builds and installs it all. A new external tool it runs must be added to `NEEDS` there.

## Conventions
- Protocol details and extension rules: the `protocol-ref` skill. Design rules: the `oneui-design` skill.
- Device/plugin state is mutated only on the device's serial dispatcher; never block in `Plugin.onPacket`.
- Widgets and tiles never do network I/O; they read cached state and send intents to the service.
- The user's real phone is paired with desktop KDE Connect; never unpair or alter it in tests.
- Since 2026-11-07 this PC runs `vyoma-desktop` (systemd user unit, identity taken over from kdeconnectd) instead of
  kdeconnectd + the companion; logs: `journalctl --user -u vyoma-desktop`; `desktop/daemon/install.sh --undo` goes back.
  The kdeconnectd interop test needs kdeconnectd running, so ask before switching back for it.
