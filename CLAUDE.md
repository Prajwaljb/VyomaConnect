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
(cd desktop/engine && cargo test --release)   # the engine (a fake Platform; every PC side runs it)
(cd desktop/windows && cargo test --release)  # the Windows agent: on Windows (or under Wine, see desktop/windows below)
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
  calls, so it serves every desktop and macOS: discovery, links, pairing, trust store, the device hub (`device.rs`), and
  what both PC sides share: `messages` (texts store + lock), `files`/`xfer` (phone file access, behind `files::Hooks`),
  `permits`, `activity`, `commands`, `telephony` (and `Line`: a phone's ringing/missed/pause steps), `away` (and `Rules`: the walk-away runner every PC side shares, its actions through `away::Hands`), `mirror` (the phone's screen in a PC window: Annex-B splitting, the sound's jitter buffer), `presenter` (the pointer dot: the packet as an order, its place, the dot drawn the same on every side), `usbip` (a USB device made in software and served over USB/IP: the import handshake, URBs, IN transfers held until a new report) and `xbox360` (a wired Xbox 360 pad as such a gadget: the real pad's descriptors, XInput's report, the rumble), `dualsense` (a USB DualSense as one: the real pad's report descriptor, reports streamed at 250 a second with counter and time, the feature reports SDL and Steam read, rumble from output reports), `release` (GitHub's newest release, `is_newer`), `updates` (the daily check of the Mac and Windows, which only say a release is out), `adb` (the USB cable for every PC
  side: when a phone with USB debugging appears (each side feeds a watch of plugged adb interfaces: sysfs on Linux,
  `companion/src/usb.rs`; IOKit on the Mac, `mac/src/usb.rs`) the reverse tunnel, payload ports both ways, and the clipboard
  grants once the person allows the APK's signing key (`set_asker`: a notification on Linux, a dialog on the Mac, a toast on Windows); `best_route` (the cable
  over Wi-Fi while plugged in) and `ClipboardSeen` (a phone's granted clipboard sync, remembered) for the Mac and Windows; adb runs without a console on Windows).
- `desktop/engine` (`vyoma-engine`, `docs/PLAN.md` section 11): the PC side once for every OS: the vyoma.* session
  (`session::serve<P: Platform>`: routing, permits, Activity, task lifetimes) and `platform::Platform`, one method per piece
  of work (default: not on this computer) plus `supports(Feature)`, from which `capabilities` makes the identity's lists.
  Linux's is `companion/src/platform.rs` (`Linux`, used by the daemon and the companion), the Mac's `mac/src/platform.rs`
  (`Mac`, `Mac::offers` before there's one; the wallpaper path is fed from the main thread). `stock` is the stock plugins once (ping, battery, find, lock, the clipboard from the phone, shares with `Arrivals` counting batches; each side's `Desk` shows, rings, locks, copies and opens links), `mousepad` reads the touchpad/keyboard/live-screen packets once into neutral acts each side maps to its own codes. `notifications` reads the phone's notifications and makes the reply/button/dismiss packets; `media` is MPRIS and the volume page's packets and requests (each side's players and outputs map onto it). `dispatch` routes every packet type to a `Route` each side's loop matches (and `Touchpad`, the touchpad's switch and Activity); `window` answers the window's requests once for the Mac and Windows (`Window<S: Side>`, the shared `Device` record; each side adds its own checks and pickers), `texting` is the Messages page's rules (Linux's D-Bus methods too); `ways` is This PC's "How this PC does it" rows (Linux's from `companion/src/ways.rs`, which asks what each part's chooser asks; `cargo test --release --lib ways::tests::reads_this_session -- --ignored --nocapture` in companion prints this session's); `commands` answers run command requests; `calls` is the Mac's and Windows' call task behind a `Ringer` (Linux's daemon keeps its fuller `telephony.rs` line) and the call packets. `gamepad::drive` is the phone's controller for every side (states in order, the fast routes, the pad remade for another kind, rumble back, the gyro mouse), each side's pads behind `gamepad::Pads` (uinput on Linux, USB/IP on Windows).
- `desktop/daemon` (`vyoma-desktop`): the standalone desktop app growing on that core (`docs/PLAN.md` section 10): pairing
  prompt as a freedesktop notification with Accept/Reject, `dev.vyoma.Desktop1` D-Bus control for the future GUI;
  stock plugins in `plugins.rs`, `clipboard.rs` (wl-clipboard), `share.rs` (files land in XDG Downloads), `media.rs`
  (MPRIS), `volume.rs` (pactl), `commands.rs`, `notifications.rs`, `telephony.rs` (calls: notification with Answer/Decline/Silence (`vyoma.call.request`), music paused for the call; answered here, a notification with Hang up and Use phone / Use this PC), `hfp.rs` (the call's sound on this PC: BlueZ hands-free with the PC as the phone's headset; PipeWire routes it to the default speaker and mic by itself), `messages.rs` (texts over the stock sms/contacts packets, in memory, for the window's Messages page: D-Bus `Conversations`, `Thread`, `TextThread`, `UnlockTexts`, `Contacts`, `SearchTexts` (names, numbers, words in loaded texts; each search loads 40 more conversations), `ThreadFor` (a contact's conversation, or a new one with a negative id until the phone's first text names its thread), `Call` (`vyoma.call.dial {number}`: only while Messages is unlocked, plain numbers only (`messages::dialable`, the phone's `Dialing.plain`: no * or # codes), the phone refuses emergency numbers, needs its per-PC Calls switch and CALL_PHONE; the window confirms in a dialog; on Linux the sound comes here over HFP as for a call answered here, on the Mac it stays on the phone), signal `MessagesChanged`; behind the phone's texts lock (`vyoma.texts.*`, phone `platform/TextsLock.kt`): the page asks, the owner confirms with a fingerprint on the phone, which refuses texts until then; relocked (and dropped from memory) when the screen locks, after 10 min unused or when the phone goes), `presenter.rs` (the pointer dot: a layer-shell overlay, smithay-client-toolkit), `phonefiles.rs` (D-Bus `Browse`: the phone's SFTP mounted with sshfs under $XDG_RUNTIME_DIR/vyoma/<n>/<phone> (Vyoma Desktop), a new <n> each mount so no file manager shows a stale folder; Dolphin lists it under Remote by itself; GTK file managers (Thunar, Nautilus, Nemo, Caja...) browse it through GVfs instead (`gvfs.rs`: an ssh alias per phone in ~/.config/vyoma-desktop/ssh, one `Include` in ~/.ssh/config; `VYOMA_GVFS=1|0` overrides the choice), else a bookmark (`places.rs`), host key checked against its cert; sshfs runs as a transient `vyoma-files-<id>` user unit, as the app's own unit has NoNewPrivileges; over the cable through an adb forward on a Unix socket, which the binary's own `--relay` mode gives ssh as its ProxyCommand: adb's TCP listener keeps Nagle on, ~50 ms an open; the one-time password goes by SSH_ASKPASS, never sshfs's password_stdin, which gives up after 60 s), `input.rs` (RemoteDesktop portal), `menus.rs` ("Send to <phone>" on a right-clicked file or folder in Dolphin, Thunar, Nemo, Nautilus and Caja, rewritten as phones pair; runs `vyoma-desktop --send <id> <paths>` → D-Bus `Send`, with notifications; a folder goes as its files with a `folder` field, rebuilt on the phone; `--remove-menus` for `install.sh --undo`; not written when `VYOMA_DIR` is set; `--send-any <paths>` (KDE's Share menu) sends to the one connected phone, else asks in a notification with a button per phone, `menus::pick`), `update.rs` (once a day asks GitHub's latest release with curl, compares with `VERSION` at the repo root (the one version, the phone's versionName too), notifies with Update; This PC's Updates card, D-Bus `Update`/`SetUpdateChecks`, state in updates.json; Update runs `install.sh --update` from the clone `install.sh` notes in `source`, as transient unit `vyoma-update`: checks out the newest release tag, never back, reinstalls, restarts), `commands.rs` (runcommand, the list itself in `vyoma_core::commands`, shared with the Mac; `commands.json` entries carry Vyoma's `fingerprint` and `everyTime` flags, which the phone honours (every time: asked on each run, past the app lock's grace, and never run by a rule); a phone's add/edit (`setup`) opens the window's Commands page through `window.rs`: the running window's `dev.vyoma.Window1.Show`, else `systemd-run --user` of `vyoma` beside the daemon). `power.rs` (battery: `vyoma.pc.leaving {why}` to phones before shutdown/sleep (logind delay inhibitor) and on quit (SIGTERM; the Mac agent too), and on start/wake an announcement to the broadcast and each phone's last address, `addresses.json`; the phone then stops dialling that PC (`DeviceHub.resting`) until it's back, the app opens or Wi-Fi returns, and backs off to 15 min with the screen off; the window's response-time echoes run only while the window is open). Per-phone permissions: `vyoma_companion::permits` (switches in the window; a refused request gets `vyoma.denied`). What phones did: `vyoma_companion::activity` (the window's Activity page). `VYOMA_PLAYERS=<bus suffix>` limits it to a test player.
  Own identity in `~/.config/vyoma-desktop`, separate from the companion's (`--import-kde` takes over kdeconnectd's instead).
  Every `vyoma.*` feature is the companion's own code: each paired phone gets a `vyoma_companion::session::serve`.
  The phone treats it as its own companion (`DeviceIdentity.featuresOf`). The cable, tethering, Bluetooth and the setup
  offer start only while kdeconnectd isn't running. `install.sh` takes over from KDE Connect (`--undo` gives it back): never run it unasked.
- `desktop/gui` (`vyoma-gui`, binary `vyoma`): the desktop app's window, Slint (GPL-3.0 option) in the phone app's look
  (`ui/app.slint`: the PC wallpaper as a soft canvas with its accent (`src/look.rs`, the phone's formulas, from the daemon's `Wallpaper`), glass cards whose rims and shadows follow the wallpaper's light, centred heroes, tiles, the floating glass sidebar, glass dialogs and status bubble, Inter; first run is the phone's setup steps, `--setup` or This PC > Run setup again shows it again, done is kept in ~/.config/vyoma-desktop/window.json; motion as the phone's: pages rise in, the title docks in a glass bubble once scrolled, pressed things dip, rings and bars ease; Up/Down walk the sidebar, Escape/Return answer dialogs; a phone's Clipboard card has the one-time adb Allow for background clipboard sync, D-Bus `AllowClipboardSync`, state in Details `clipboardSync` on|ask|plug; a Commands page adds and edits commands in a sheet, with Ask for fingerprint first and Every time; a phone's Messages tile opens its conversations and a thread with the reply box, `src/texts.rs`). App id `vyoma` (matches vyoma.desktop; the mirror window takes it too, `mirror.rs` `brand`, as do notifications' `desktop-entry`). Wrapped Text needs `min-height: self.preferred-height` (Slint 1.18+ shrinks it otherwise) and long unbroken words go through `breakable()`. The command editor warns (`command_warning`: a program not on PATH, sudo, a name used twice) and Delete offers Undo. One window at a time (`src/instance.rs` holds `dev.vyoma.Window`; on Windows `src/one_window.rs`, a named mutex and a message-only window taking the page as WM_COPYDATA; a second start, or `--page commands|this-pc|activity|<id>`, shows that page in the open one). `install.sh` installs it as ~/.local/bin/vyoma with a menu entry.. It only shows and asks: `src/daemon.rs` is the `Daemon`
  trait (D-Bus `dev.vyoma.Desktop1` today, a local socket on macOS later), rereading on the daemon's `Changed` signal.
  `vyoma --snapshot out.png [--dark] [--plain (no wallpaper)] [--size WxH (780x540 is the smallest window)] [--scroll PX] [--scene phones|pairing|nearby|away|unpair|add-done|notice|cable|pc|pc-setup|activity|activity-empty|activity-late|commands|commands-empty|commands-edit|commands-warn|messages|messages-thread|messages-search|messages-contacts|messages-call|messages-silent|messages-loading|messages-asking|messages-declined|messages-locked|setup|setup-pairing|setup-search|setup-ready|setup-moment|empty|add|add-offline|down]` renders it offscreen
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
- Hyprland (and other compositors with wlr's virtual pointer + virtual keyboard: Sway, river, labwc...), by probing, not
  names: `companion/src/wlr.rs` is input there (before the portal; daemon `input.rs` `Sink::Wlr`, `screen.rs` `Via::Wlr`);
  `companion/src/hypr.rs` talks to Hyprland's IPC socket (a listening `$XDG_RUNTIME_DIR/hypr/*/.socket.sock`; 0.57 takes
  dispatchers as Lua, `hl.dsp.*`, older the classic words; both tried): workspaces, dpms, log out, layout, hyprpaper/swww
  wallpaper. Screenshots with grim where there's no Spectacle or GNOME; kernel backlight through logind where neither Plasma
  nor Mutter dims. Test in a nested Hyprland inside `kwin_wayland --virtual` under `dbus-run-session`, with a short private
  XDG_RUNTIME_DIR (socket paths over 108 bytes fail) and XDG_CONFIG_HOME: `VYOMA_HYPR_TEST=1 VYOMA_WLR_TEST=1
  [VYOMA_WLR_OUTPUT=WxH VYOMA_HYPR_WALLPAPER=<path>] cargo test --release --lib -- hypr:: wlr::` in companion (the workspace test
  needs a window on workspaces 1 and 2: two `foot` there). The rest in the same stack: `VYOMA_SHOT_TEST=1 ... screenshot::`
  (grim: Spectacle comes first only on Plasma, `desktop::is_plasma`), `ways::tests::reads_this_session -- --ignored`, and the
  cast through xdg-desktop-portal-hyprland, `VYOMA_CAST_TEST=1 ... screen::tests::casts_through_the_portal`: start a private
  `pipewire`, `wireplumber` (a conf.d file with every `monitor.*` disabled, so no sound card is touched), then
  `/usr/lib/xdg-desktop-portal-hyprland` and `/usr/lib/xdg-desktop-portal` with XDG_CURRENT_DESKTOP=Hyprland; the picker is
  `screencopy { custom_picker_binary = <script> }` in $XDG_CONFIG_HOME/hypr/xdph.conf printing `[SELECTION]/screen:<monitor>`;
  Hyprland's portal sends a picture only once the screen changes, so the stream wakes it with an invisible layer-shell overlay until the first (`wlr::touch_screens`); `VYOMA_CAST_STILL=1` skips that, `VYOMA_CAST_SECS=<n>` counts longer, per second. Never
  `pgrep -x kwin_wayland` to stop it (the real KWin matches): kill by pid.
- `desktop/windows` (`vyoma-windows`, `docs/PLAN.md` section 11, Windows): the PC side on Windows on core and the engine
  (`platform.rs` `Windows::offers`): a notification-area agent (`tray.rs`: icon, Open/Quit, balloons where there are no toasts, the session
  ending), one per session (`system.rs`: named mutex, Run-key `--install`, power, wake; `--quit` closes the window and stops the agent
  for the installer), the window's named pipe `\\.\pipe\vyoma-<user>` (`control.rs`, this user's SID only, no remote clients; the window's
  `socket.rs` opens it as a file), stock plugins (`win.rs`: clipboard, battery, lock, Downloads, only http(s) links opened, HKCU registry
  helpers; `input.rs`: SendInput, the live screen's absolute touches), media (`media.rs`: Windows' media controls as MPRIS players, covers
  as payloads, Core Audio's default speaker; pause/resume/mute helpers for walk-away and calls), toasts (`toast.rs`: `AppUserModelId\dev.vyoma.Vyoma` + a COM activator class in HKCU, so
  buttons and the reply box work, `-Embedding` when Windows starts the agent for one; the pairing question, phone notifications with Reply
  and actions, Send to's phone choice; not registered when `VYOMA_DIR` is set unless `VYOMA_TOASTS=1`), engine features (stats, specs,
  screenshots in `screen.rs`, the live screen in `cast.rs`: desktop duplication (a rotated display turned upright, `Turn`) or GDI, scaled and made NV12 on the graphics card's video processor (`gpu.rs`, the pointer drawn with GDI on the card's copy; else the CPU's `to_nv12`), the
  graphics card's H.264 encoder (an async Media Foundation transform) else the software one, the Mac's framing; files, power, wake, Task
  View, paste, brightness and app volumes in `pc.rs` (WMI for a laptop's screen, DDC/CI for monitors, Core Audio's sessions), the
  wallpaper's colours in `wallpaper.rs` (WIC), walk-away in `away.rs` (core `Rules`), the phone's screen in a window in `mirror.rs`
  (`--mirror-window` child: Media Foundation's H.264 decoder into GDI, Windows keys, sound as PCM through WASAPI; core `mirror` shares the
  splitting and jitter buffer with the Mac), a photo in `camera.rs` (MF source reader, WIC JPEG, a notification each time, This PC's Camera
  check on Windows' privacy switches; tests only count cameras unless `VYOMA_CAMERA_TEST=1`), the presenter dot in `presenter.rs` (a layered,
  click-through topmost window drawn once with core's dot), the phone as a game controller in `gamepad.rs` (core's `xbox360` or `dualsense`, as the phone asks, served over USB/IP on 127.0.0.1:1767-1770 and attached through usbip-win2's driver
  (BSD, signed; its VHCI interface's IOCTLs, open to every user, each release's struct layout tried in turn), unplugged before its connection closes so the driver doesn't
  retry; This PC's Game controller check, Get driver = the pinned usbip-win2 installer from GitHub, its SHA-256 checked, opened; without the driver the phone is told
  so; Wine has no driver, so the attach is untested here), the USB cable in `usb.rs` (Android's adb device interface listed every 2 s through
  cfgmgr32 feeds core `adb`; the grants' question as a toast with Allow / Not now; This PC's USB cable check, Get adb = Google's platform tools
  into Vyoma's folder with Windows' own curl and tar)), calls (`calls.rs`, core `Line`: ringing toast with
  Silence and Decline, missed calls, music paused), the daily update check (`update.rs`, core `updates`, releases with
  `Vyoma-setup-*.exe`), Explorer's Send to (`sendto.rs`: "Phone (Vyoma)" in the SendTo folder, written by `--install`, removed by
  `--uninstall`), the firewall check (`firewall.rs`, Let phones through = `--open-firewall` elevated: TCP 1714-1764, UDP 1716 and the controller's 1750-1764). Icon `dist/vyoma.ico`
  (from `gen.py`) in both programs via `build.rs` (embed-resource). Installer `packaging/windows/vyoma.nsi` (NSIS, per user; CI signs
  the programs, installer and uninstaller when the repo has `WINDOWS_SIGN_PFX`/`WINDOWS_SIGN_PASSWORD`, else unsigned). Built and
  tested as Windows builds under Wine here and on GitHub's runners (`.github/workflows/windows.yml`, artifact `Vyoma-windows`: the
  installer and a zip; releases get `Vyoma-setup-<ver>.exe` through desktop-release.yml); not yet run on a real Windows PC. Test switches as
  the daemon's (`VYOMA_DIR`, `VYOMA_ALLOW`, `VYOMA_ANNOUNCE`) plus `VYOMA_PIPE`, `VYOMA_UDP_PORT`, `VYOMA_TCP_PORTS`; log in
  `%LOCALAPPDATA%\Vyoma\vyoma.log`. Here: a scratch rustup with `x86_64-pc-windows-gnu`, mingw-w64 as linker/CC/RC
  (`CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc`, `CC_/CXX_x86_64_pc_windows_gnu`, `RC_x86_64_pc_windows_gnu=x86_64-w64-mingw32-windres`),
  `CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUNNER=wine` with a scratch WINEPREFIX, `CARGO_TARGET_DIR` on disk (the build is ~5 GB; /tmp is a
  small tmpfs), and DISPLAY on a private `Xvfb :97` with WAYLAND_DISPLAY unset (never the real screen: GDI would capture it); then
  `cargo test --release --target x86_64-pc-windows-gnu` in core, engine, windows, gui. Wine has no WinRT XML, toasts, media sessions or
  firewall (those tests skip); its H.264 encoder works, so the live screen runs (`VYOMA_CAST_DUMP=<file>` in `cast::tests` keeps 2 s to decode).
- `desktop/mac` (`vyoma-mac`): the Mac's PC side on vyoma-core (`docs/PLAN.md` section 10, macOS): pairs in Vyoma's
  window (inside Vyoma.app a request opens it, with a notification; the bare binary asks in a dialog with the code), serves ping, battery, find my phone, lock (display sleep), clipboard (pbcopy/pbpaste polled each
  second), share (files into ~/Downloads); `--install` makes it a LaunchAgent (`launchd.rs`; from Vyoma.app it runs the
  copy inside the app, which the window installs on first open, after the app moved, or when the running agent is older than the window (Health `version`: replacing the app leaves the old agent running); one agent at a time, `agent.lock`). Vyoma.app
  (`packaging/bundle-mac.sh`: window `vyoma` as the app, `vyoma-mac` beside it, id `dev.vyoma.Vyoma`, icon
  `desktop/mac/dist/vyoma.icns` from `gen.py`, ad-hoc signed) is the CI artifact `Vyoma-mac`, and on a `v*` tag desktop-release.yml has macos.yml attach it to the release as `Vyoma-mac-<version>.zip` + `.sha256` (by hand: `gh workflow run macos.yml -f release=vX.Y.Z`); inside it notifications go
  through UserNotifications under Vyoma's name (`notify.rs`, osascript outside the app or if macOS refuses). The window reaches it over
  `control.sock` in ~/Library/Application Support/Vyoma (mode 0600; `VYOMA_SOCKET` overrides): JSON lines, D-Bus method
  names, `Events` streams changes (`control.rs`; window side `gui/src/socket.rs` behind `daemon::connect()`); methods it
  lacks answer "Not on the Mac yet". Also: Music and Spotify plus the output volume (`media.rs`, fixed AppleScripts, pgrep
  first so nothing is launched, read every 3 s while a phone listens), touchpad/keyboard (`input.rs`, CGEvent FFI; text as
  Unicode strings, shortcuts as US keys with the phone's Ctrl as Command; asks for Accessibility once), the phone's
  notifications (replaced/withdrawn by id inside the app), run commands (core `commands`; the agent's PATH adds Homebrew).
  Screen (`screen.rs`): screenshots (`screencapture`, PNG) and the live screen on the phone: ScreenCaptureKit (dlopened,
  macOS 12.3+, so older Macs still start) into VideoToolbox's real-time H.264, framed as Linux's (avc4, parameter sets before
  keyframes, VYTS stamps), frames skipped before encoding while the link is behind; the phone's touches as absolute
  CGEvents on that display (`input::touch_acts`, `Act::At`); needs Screen Recording (a This Mac check). Mirror (`mirror.rs`):
  the phone's screen in a window, `vyoma-mac --mirror-window <name>` as a child (AppKit window, AVSampleBufferDisplayLayer
  decoding the Annex-B stdin, mouse/trackpad/keys as JSON lines on stdout: Command+C/X/A/V, Option/Command+arrows, Escape or
  right click Back, middle click Home, Command+W closes); the phone's sound as PCM (it says `sound: []`) through an
  AudioQueue with a small jitter buffer; copy onto the Mac's clipboard, volume keys via osascript. Paste (`vyoma.paste`,
  the old clipboard back after 1.2 s) and the three-finger gestures (Control+Up Mission Control, Control+Left/Right Spaces,
  with the arrow keys' fn/number-pad flags). The USB cable (`usb.rs`): IOKit notifications feed core `adb`; adb from
  Android Studio, Homebrew, or the USB cable check's Get adb (Google's platform tools into Vyoma's folder); the window's
  Clipboard card and `via: cable` as on Linux. Camera (`camera.rs`): a photo for the phone's Camera tile through AVFoundation's photo output (JPEG, after a short warm-up), a notification each time; macOS's camera Allow needs Vyoma.app (`NSCameraUsageDescription`), the bare binary refuses; stats say `camera`/`cameraLid` (clamshell from ioreg); a This Mac check. Calls (`calls.rs`, core `telephony`): who is ringing and missed calls as notifications, Music and Spotify paused for the call; answering stays on the phone. Walk-away rules (`away.rs`, core `away`): pause, mute, display off. Brightness of the built-in display (`brightness.rs`, DisplayServices dlopened; `vyoma.pc` offered only where there is one, no per-app volumes). The presenter dot (`presenter.rs`: `vyoma-mac --presenter-window` as a child, an AppKit window above everything, lines on stdin). The window's wallpaper look (`Wallpaper`). Switches: screen, control, files, camera, power. Type-check it on Linux with a
  scratch rustup (`rustup target add aarch64-apple-darwin`, `cargo check --target aarch64-apple-darwin`; ring's C needs a stub
  TargetConditionals.h and `-nostdlibinc` with tiny string.h/assert.h stubs in CFLAGS_aarch64_apple_darwin).
  Vyoma's extras (`vyoma.rs`): stats, specs, power, wake, wallpaper; the phone's files (~/Library off limits) and resumable
  sends; the Messages page (texts lock again when the screen locks); per-phone permits and Activity; a daily update check
  (`update.rs`: tells when a release carries `Vyoma-mac*.zip`; the window's Download opens the release page, as
  Health `update.download`). Finder's "Send to Phone" Quick Action (`finder.rs`: an Automator workflow in
  ~/Library/Services written at start from Vyoma.app, never from a translocated copy, removed by `--uninstall`) runs
  `vyoma-mac --send-any`, which hands the files to the agent's `SendAny` (core `share::pick`, a choose-from-list for
  several phones, notifications for the result; opens Vyoma first if the agent isn't up). The phone shows only the tiles a PC offers (`PcFeature`, from phone 0.10). macOS's own tools only
  (`mac.rs`), a device's text goes to `osascript` as argv after our own first word, never as script. Nobody has a
  Mac: `.github/workflows/macos.yml` builds and tests core, mac and the window on GitHub's runners and keeps Vyoma.app
  (universal) as an artifact; it compiles and tests on Linux too (`cd desktop/mac && cargo test --release`). Not yet run on a Mac.
- `desktop/icon/gen.py`: Vyoma's icon ("Moon": the PC as a white glass orb, the phone as a small orb in a gap of one ring, on a sky tile) for every place from one geometry: Android adaptive layers, monochrome, notification glyph, splash; the desktop's `dist/vyoma.svg` (installed into hicolor) and the window's `ui/icons/vyoma.svg`. Run `python3 desktop/icon/gen.py .` from the repo root after changing it.
- `desktop/companion`: lib + bin (the session loop is the engine's, `session.rs` re-exports it; `platform.rs` is Linux's side); the bin is the Rust daemon beside stock kdeconnectd, built on `vyoma-core`. Dials only phones kdeconnectd trusts (mDNS), pairs with
  no prompt via `CompanionProof` (signed with kdeconnectd's key; phone verifies against the pinned PC cert), speaks
  `vyoma.*` packets. Phone folds it into its PC (`DeviceIdentity.companionOf`, `DeviceState.isCompanion`).
  Also watches sysfs for an adb USB interface and, for phones with Vyoma installed, runs `adb reverse tcp:1765 tcp:1716`
  (+ payload mirrors phone 1839..1864 → PC 1739..1764)
  plus the READ_LOGS / SYSTEM_ALERT_WINDOW grants, once the PC has allowed that APK's signing key (src/usb.rs). Installed as a systemd user unit.
  Installs its own runtime dependencies (src/setup.rs): notification with an Install button at start, or `--check` / `--setup [--dry-run]`; `install.sh` builds and installs it all. A new external tool it runs must be added to `NEEDS` there.

- `desktop/purpose`: "Send to phone" in KDE's Share menu (Purpose: Dolphin, Spectacle, Gwenview, Okular): a small C++
  plugin (Qt 6 + KF6 Purpose, CMake/ECM) that runs `vyoma-desktop --send-any` with the local files (`VYOMA_DESKTOP_BIN`
  overrides for tests). Purpose runs it out of process and drops a job's error, so it says failures with notify-send itself.
  `install.sh` builds it where KDE's Purpose is and puts it in ~/.local/lib/qt6/plugins with
  ~/.config/plasma-workspace/env/vyoma-share.sh adding that to QT_PLUGIN_PATH (next login); the release bundle carries one
  built on Debian 13 (`share-plugin` job, Qt 6.8+); the AUR package installs it into /usr/lib/qt6/plugins. Test with a
  Purpose::AlternativesModel harness (pluginType Export, `urls`) and a fake send binary, never the real phone.
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
  PC builds and the Mac zip; build the APK and `gh release upload` `vyoma-X.Y.Z.apk` + `.sha256` and the notes to it (the user publishes
  the draft), then the PKGBUILD's pkgver and sha256 (`makepkg -g`) and `makepkg --printsrcinfo > .SRCINFO`.

## Conventions
- Protocol details and extension rules: the `protocol-ref` skill. Design rules: the `oneui-design` skill.
- Device/plugin state is mutated only on the device's serial dispatcher; never block in `Plugin.onPacket`.
- Widgets and tiles never do network I/O; they read cached state and send intents to the service.
- The user's real phone is paired with desktop KDE Connect; never unpair or alter it in tests.
- Since 2026-11-07 this PC runs `vyoma-desktop` (systemd user unit, identity taken over from kdeconnectd) instead of
  kdeconnectd + the companion; logs: `journalctl --user -u vyoma-desktop`; `desktop/daemon/install.sh --undo` goes back.
  The kdeconnectd interop test needs kdeconnectd running, so ask before switching back for it.
