# VyomaConnect — Architecture & Roadmap

A fast, battery-friendly, open-source Android client for the KDE Connect protocol.
Fully compatible with KDE Connect (Plasma), GSConnect and Valent (GNOME), with an
optional Linux companion that unlocks features stock KDE Connect can't do.

Primary target: Galaxy S24 FE, Android 17 / One UI 9. Supported: Android 12+ (API 31).
Distribution: Google Play, F-Droid and GitHub (section 11) — **no Google Play Services, Firebase, or proprietary SDKs** in any build.

---

## 1. Design principles

1. **Protocol-correct first.** Speak KDE Connect v7 + v8 exactly, so stock peers just work.
   Our extensions use a `vyoma.*` packet namespace that stock peers ignore.
2. **One process, one service, zero polling.** Everything is event-driven (sockets,
   callbacks, flows). No wake-locks except during active transfers/streams.
3. **Widgets never do I/O.** They show cached state and fire intents. A widget can
   never hang, crash, or show stale "loading" forever.
4. **Graceful degradation.** Every extra feature detects its capability (companion
   present? Shizuku? permission?) and hides or falls back cleanly.
5. **Motion is a feature.** Spring physics, shared-element transitions, predictive
   back, and haptics on every meaningful interaction. Nothing janky, nothing purple.

---

## 2. System overview

```
┌──────────────────────── Android (VyomaConnect) ─────────────────────────┐
│  UI (Compose)   Widgets (Glance)   QS Tiles   Share targets   Shortcuts │
│        │               │               │            │            │      │
│        └───────────── StateStore (DataStore/Room, StateFlows) ───┘      │
│                                  │                                      │
│  LinkService (FGS: connectedDevice | +mediaProjection/camera/mic)       │
│   ├─ DeviceManager ── Device ── PluginHost ── [plugins…]                │
│   ├─ Transports: LAN (UDP/mDNS + TLS/TCP) · Manual IP/Tailscale ·       │
│   │              USB (adb reverse) · Bluetooth RFCOMM (fallback)        │
│   ├─ PayloadEngine (resumable, parallel chunks)                         │
│   └─ StreamEngine (low-latency video/audio/input channels)              │
└─────────────────────────────────────────────────────────────────────────┘
            │ KDE Connect protocol (TLS)        │ vyoma.* extensions + streams
            ▼                                   ▼
   KDE Connect / GSConnect / Valent     vyoma-companion (Rust daemon, Linux)
   (stock baseline features)            PipeWire · portals · uinput · v4l2loopback
```

### Companion ↔ KDE Connect coexistence
*(Today's model; being replaced by the standalone desktop app, section 10.)*
The companion is a **separate peer** on the same host, on its own port range. It
advertises `linkedKdeConnectId` so the app merges it into the same device card.
On Plasma it reads `~/.config/kdeconnect/` to auto-trust the phone certificate
KDE Connect already paired, so there's **no second pairing step**. If that fails
(e.g. GSConnect), it falls back to its own pairing using the same verification-key flow.

---

## 3. Android tech stack

| Concern | Choice | Why |
|---|---|---|
| Language | Kotlin 2.x, coroutines/Flow | Structured concurrency for sockets |
| UI | Jetpack Compose + Material 3 Expressive, custom theme | Springs, shape morphing, shared elements |
| Widgets | Jetpack Glance (responsive sizes) | Native Android 12+ widget features |
| DI | Manual app container (lazy) | Fastest cold start, no annotation processing |
| Storage | DataStore (proto) for state/settings · Room for history (transfers, clipboard, SMS cache) | |
| Crypto | BouncyCastle (cert gen) + Android Keystore | RSA-2048 self-signed certificates, for compatibility with older peers |
| Media | MediaCodec (HW H.264/HEVC, low-latency mode), Camera2, AudioRecord | No FFmpeg on the phone |
| Images | Coil 3 | High-res album art with memory/disk cache |
| Build | Gradle KTS, version catalog, R8 full mode, Baseline Profiles | Startup < 300 ms target |

### Module layout
```
app/                   entry, navigation, theming
core/protocol          packet model, JSON codec, identity, v7/v8 handshake
core/crypto            cert generation, TLS contexts, verification key
core/transport         lan, manual/tailscale, usb, bluetooth
core/payload           resumable chunked transfers
core/stream            low-latency media/input channels
core/data              DataStore, Room, StateStore
plugins/<name>         one module per plugin (see §4)
feature/<screen>       Compose screens
widgets/               Glance widgets + actions
desktop/companion/     Rust daemon (Phase 2)
```

---

## 4. Feature inventory

### Phase 1: Core parity, polished (stock KDE Connect on the PC)
- **Discovery & pairing:** UDP broadcast on 1716, mDNS `_kdeconnect._udp`, manual IP list
  (Tailscale/VPN), hotspot-aware, pairing verification key shown with animation.
- **Clipboard:** PC→phone automatic. Phone→PC uses the one-time ADB grant
  (`READ_LOGS` + `SYSTEM_ALERT_WINDOW` logcat trick), with Shizuku optional and the
  tile/widget/notification button as a fallback. A clipboard **history UI** is shared across devices.
- **Notifications:** mirroring with icons, inline replies, dismiss sync, per-app filters.
- **Share:** files, text and URLs both ways; multi-file; Live Update progress in the Now Bar.
- **Media control (MPRIS):** **high-quality art.** Full-resolution art is sent and requested
  (up to 1024 px WebP), not thumbnails. YouTube art is upgraded to `maxresdefault` by the companion later.
- **Remote input:** touchpad with gestures, keyboard, presenter mode (done 2026-11-05: Next/Previous + volume keys, start/end with a timer, black screen, hold to point with the gyro; pointer not yet tried on the real PC).
- **Telephony / SMS:** conversation sync, send SMS, call notifications, mute ringer.
  Contacts with photos. (RCS isn't accessible to third-party apps; that's an Android limitation.)
- **Run commands, find my phone / ring PC, battery, connectivity, system volume, lock PC.**
- **Widgets:** device status · quick actions grid · PC media · transfers/file drop ·
  clipboard · quick input (mouse/keyboard launcher).
- **System integration:** QS tiles, Live Updates (Android 16+ promoted ongoing
  notifications, which show in the One UI Now Bar), direct-share targets, app shortcuts, and
  intents usable from Samsung Modes & Routines.

### Phase 2: Companion + power features
- Rust daemon (`tokio`, `rustls`, `ashpd` portals, `pipewire`, `evdev`/uinput), systemd user unit, AUR package.
- **PC screenshot → phone** instantly (portal Screenshot), one widget tap.
- **Laptop camera photo → phone** (PipeWire camera portal / V4L2).
- **Live PC screen view** on the phone (ScreenCast portal + VAAPI encode → MediaCodec decode).
- **PC stats & control:** CPU/GPU/RAM/temps, per-app volume, brightness, sleep/shutdown.
- **Fast transfers:** parallel chunks, resume after a drop, folder send, two-way folder sync.
- **Two-way file browser** with thumbnails and streaming.
- **Handoff / continuity:** links, tabs, media position. **YouTube resume at timestamp.**
  PC→phone: the browser's MPRIS `xesam:url` plus position opens the YouTube app at `t=`. Phone→PC:
  the shared URL plus the position from the YouTube MediaSession, with `&t=` appended.
- **Automation rules (done 2026-11-03):** When / Only if / Do rules with "undo when it ends" (app/automation/):
  PC connected or away, call, charging, battery, PC locked/playing/plugged in, headphones, home Wi-Fi (learned from
  the PC's network, no location permission), times and periods → PC lock/mute/volume/media/brightness/command/
  clipboard/message/screenshot/wake/sleep and phone notify/vibrate/pause. Walk-away actions run on the companion
  (`vyoma.away`). Not done: "app opened" (needs usage access).
- **USB mode:** the companion runs `adb reverse`, and also automates the clipboard grant.

### Phase 3: Real-time features
- **Gamepad (done 2026-11-03):** virtual Xbox 360 or DualSense pad, floating sticks, ABXY/D-pad slides, rumble,
  gyro aim (mouse or right stick), analog triggers, layout editor with per-game profiles; signed UDP frames on
  Wi-Fi, and over the USB cable via adb (debugging on) or Android accessory mode (debugging off; needs the
  companion's udev rule once). Still untested: real games, Steam with the DualSense. Plan as first written: uinput virtual **Xbox** (045e:028e-style) and **DualSense-style** pads
  (Steam Input recognises both). Gyro → right stick or mouse with deadzone and smoothing,
  haptics on press, PC rumble → phone vibrator (VibratorManager compositions), and a
  drag-and-drop layout editor with per-game profiles. Inspired by Steam Input, Backbone and Moonlight.
  Transport: UDP state snapshots with AEAD (ChaCha20-Poly1305, session key from TLS), <10 ms on LAN/USB.
- **Phone mirroring + control from PC (done 2026-11-04):** MediaProjection → H.264 → a companion window
  (GStreamer via XWayland); mouse, wheel and keyboard come back through a user-enabled accessibility service.
  Since then: the phone's sound plays on the PC (`vyoma.mirror.audio`), and Ctrl+V / Shift+Insert in the window
  types the PC's clipboard into the phone.
- **Phone as PC webcam/mic (done 2026-11-04):** Camera2 → H.264 → companion → v4l2loopback "Vyoma Phone
  Camera"; mic → PipeWire source "Vyoma Phone Mic". Preview, zoom (incl. ultra-wide), torch and a notification
  Stop are in. Not yet: tried in real call apps.
- **Bluetooth fallback transport** when Wi-Fi and the cable are absent: a tunnel for kdeconnectd and one for the
  companion (notifications, clipboard, stats, touchpad, controller). No files or pictures over it.

---

## 5. Reliability & battery on One UI

- A single foreground service of type `connectedDevice` (allowed to start from boot), with
  extra types (`mediaProjection`, `camera`, `microphone`) added only while those features run.
- `NotificationListenerService` plus the FGS keep the process alive. Onboarding walks
  through One UI "Never sleeping apps" and unrestricted battery.
- Socket keep-alive tuned to avoid radio wakeups. Reconnects are triggered by
  `ConnectivityManager` network callbacks, not timers.
- **Android 17 watch-list:** the local-network permission, logcat clipboard-trick
  viability, FGS rule changes. All are tested on the S24 FE first.
- Widget actions use user-initiated PendingIntents, which are exempt from background FGS
  start limits, so a tap always works even if the service was killed.

## 6. Visual design

- One UI–style structure: big collapsing titles, content in the bottom half for
  one-handed reach, grouped rounded cards (~26 dp), generous spacing.
- ColorOS / iOS-style polish: frosted surfaces (RenderEffect blur on Android 12+),
  fluid springs, morphing shapes, a live "connection orb" on the device card.
- Themes: accent from the **PC wallpaper** (default, via the companion), the phone wallpaper, or curated fixed palettes
  (Graphite, Ocean, Ember, Mint, Sand) ; dark mode is **AMOLED true black** apart from dim accent glows at the top. No purple default.
- Haptics are tied to motion (`performHapticFeedback` + Vibrator primitives).

## 7. Security

User choice: **all features enabled by default** for paired devices. TLS with pinned
peer certificates (inherent to the protocol). Per-plugin toggles still exist in settings
and can be turned off at any time.

Fingerprint lock: per-command switches and "asks when chosen for a rule" live on the phone for
now. They may move to the planned PC companion GUI, along with allowed phones and signing keys.

## 8. Testing

- Protocol: unit tests against captured KDE Connect / GSConnect packet fixtures.
- Interop matrix: KDE Connect (Plasma 6, your machine), GSConnect & Valent (GNOME VM).
- Device: S24 FE (Android 17) plus an emulator on API 31 / 34 / 36.
- Perf: Macrobenchmark for startup and scroll jank; battery measured over 24 h idle.

## 9. Phase 0: setup (next step)
1. Install the JDK 21 / Android SDK cmdline-tools / platform-tools (adb) on CachyOS.
2. `git init`, Gradle skeleton, module layout, version catalog, theming foundation.
3. Protocol core + discovery + pairing with your Plasma machine → first green link.

- Automation connections (2026-11-05): triggers/conditions for one paired Bluetooth device (ACL broadcasts + audio profile proxies for what's already connected), how the PC is connected (best transport, settled 3 s), and a named Wi-Fi (added while on it; fingerprint of subnet/router/DHCP, no location permission, so two networks with identical addressing can't be told apart).

## 10. Desktop app: standalone, every desktop (decided 2026-11-07)

The companion grows into Vyoma's whole desktop side: it speaks the full KDE Connect protocol and
pairs on its own, so no kdeconnectd, GSConnect or Valent is needed. One install behaves the same on
KDE, GNOME, Cinnamon, COSMIC and macOS. The phone keeps working with stock KDE Connect desktops for
the basics; the extras need Vyoma's.

### Shape
- **Core** (pure Rust, no desktop calls): identity and certificates, discovery (UDP, mDNS, manual,
  cable, Bluetooth), TLS links, pairing with the verification code, payloads, the plugin host, the
  trust store. Tested like core/protocol: unit tests plus a live test against the phone app.
- **Platform layer**: one trait per thing the desktop does (clipboard, notifications, media, input,
  screen capture, volume, power, lock, wallpaper, brightness, files, camera), with a common
  freedesktop implementation (portals, MPRIS, PipeWire, logind, notifications) and small per-desktop
  parts chosen at run time (`XDG_CURRENT_DESKTOP`): KDE, GNOME, Cinnamon (X11), COSMIC; then macOS.
- **Daemon** (systemd user unit on Linux, a LaunchAgent on macOS) owns the links. The **GUI** is a
  separate process that talks to it (D-Bus on Linux, a local socket on macOS), so closing or crashing
  it never drops a link. Cross-platform toolkit in Vyoma's own look (not Plasma-native); no reliance
  on a tray, since GNOME has none by default.

### What it takes over from kdeconnectd
By the phone's plugins, in the order to build them:
1. Pairing (prompt with the code, on the desktop), ping, battery, lock, find my phone, connectivity.
2. Clipboard both ways, share (files, links, text) both ways.
3. Media (the PC's players for the phone, via MPRIS / the macOS equivalent), system volume.
4. Touchpad, keyboard and presenter (portal RemoteDesktop; X11 on Cinnamon; macOS Accessibility).
5. Notifications from the phone: shown, replied to, actioned, dismissed.
6. Run commands (the PC's list, edited from the phone as today).
7. Later: calls, SMS and contacts (a messaging window), browsing the phone's files (sftp).
The vyoma.* extras (screen, files, camera, controller, webcam, mirror, specs) move over as they are.

### Phases
1. **Split and stand alone on KDE** (this PC): core + platform layer out of today's companion, own
   pairing, the stock features above, checked one by one against kdeconnectd. Development must not
   touch the real phone's pairing with KDE Connect: a separate identity and the debug/demo phone
   builds until it's ready to replace kdeconnectd here.
2. **GNOME** (Wayland, portals; most users). Tested in a VM the user sets up.
3. **Cinnamon** (X11 input and capture, thin portals).
4. **COSMIC** (its portals; remote input may not be supported yet).
5. **macOS**: same core; ScreenCaptureKit, CGEvent (Accessibility), AVFoundation, VideoToolbox.
   No virtual controller, phone webcam or mic at first (a camera extension and audio driver need
   signing); Gatekeeper wants a paid Apple Developer ID. Needs a Mac to build and test on.

Phase 1 so far:
- Done: the core is its own crate, `desktop/core` (vyoma-core), shared by the companion.
- Done: the core links and pairs on its own, checked live against kdeconnectd.
  - Discovery shares UDP 1716 and listens on the first free TCP port, IPv6 and IPv4.
  - Links are made both ways, unpaired certificates must name their id, and there are handshake limits.
  - Pairing follows the phone's rules, with the code, timeouts and clock check; there is a trust store.
- Done: the daemon, `desktop/daemon` (vyoma-desktop), checked live against kdeconnectd in both directions, plus reject and ping.
  - The core's `device.rs` hub keeps each device's state across links.
  - The code is shown in a freedesktop notification with Accept and Reject.
  - `dev.vyoma.Desktop1` on the session bus serves the GUI (Devices, Pair, Accept, Reject, Unpair, Refresh).
  - It has its own identity, and an allowlist for development.
- Milestone 4, stock plugins. Each is checked against kdeconnectd first, then against the phone's debug build.
  - Done (`desktop/daemon` plugins.rs and system.rs), checked against kdeconnectd:
    - Ping.
    - Battery both ways, from sysfs.
    - The PC's lock state through logind, without lock requests outward.
    - Find my phone both ways, ringing with pw-play/paplay/canberra and a "Found it" button.
    - The phone's signal report, stored and shown in `Details`.
  - Checked with the phone's debug build (2026-11-07): pairing from the phone, battery both ways, signal,
    ping, ringing the phone, and lock then unlock of the PC from the phone (the tile follows logind).
  - Clipboard (`clipboard.rs`): wl-clipboard's `wl-paste --watch`, so KWin, wlroots and COSMIC; GNOME has no
    data-control and needs its own way. Each change goes out; a linking device gets the current one with its
    time (`clipboard.connect`), and a device's newer one replaces ours. Our own echo is ignored, password-manager
    copies (`x-kde-passwordManagerHint`) are never sent, and nothing over 1 MiB goes out.
  - Share (`share.rs`), both ways: files into XDG Downloads under a free name, never over a file; text as a
    notification with Copy; http(s) links open at once, and any other scheme is shown as text. D-Bus
    `SendFiles(id, paths)` and `SendText(id, text)`; a lone link goes as `url`. The core's `payload::Offer`
    lets the announcement go through the hub.
  - Checked with the phone's debug build (2026-11-07): files both ways (hash checked, empty file, a second copy
    landed as `name (2)`), text both ways, and the clipboard both ways: on linking, a live copy, the phone's
    Send clipboard, our own echo ignored, and an older copy from the phone declined.
  - Media (`media.rs`): every MPRIS player by its Identity (" [2]" on a second of the same name), its state
    out on each change (not on position alone), actions, volume, seek, loop and shuffle, and the cover of a
    local file as a payload: only the cover the player itself shows, never a file a device names, 8 MiB at most.
  - Volume (`volume.rs`): pactl, so PulseAudio or PipeWire; outputs, volume (100% is the slider's top, 150%
    the most a device may set), mute, the default; only outputs we listed are touched.
  - Run commands (`commands.rs`): `commands.json` in the daemon's folder, D-Bus `AddCommand`, `RemoveCommand`
    and `Commands`; the phone sends only a key.
  - The phone's notifications (`notifications.rs`): shown here with their buttons, KDE's inline reply, and the
    app icon cached by hash; dismissing here dismisses on the phone when it's clearable. The backlog the
    phone sends when asked (`silent`) waits for the GUI's list.
  - Checked with the phone's debug build (2026-11-07), against a fake player and a null sink so nothing of
    the user's was touched (`VYOMA_PLAYERS` limits the players for tests): player list, state, Pause and
    Next from the phone, the cover (7.8 KB), the output's level as on the PC, a command run from the
    phone, and a phone notification shown with its icon. Live tests: `cargo test --release -- --ignored`.
  - Input (`input.rs`): the touchpad and keyboard through the RemoteDesktop portal (asked once, then a restore
    token), characters pressed as the real key of the desktop's layout (portal keysyms doubled and lost keys).
    Checked with the phone's debug build: the user allowed it, the pointer moved and typing arrived. Over this
    home's Wi-Fi (22% ping loss, 0.4 s round trips) it lags badly, in bursts up to 5 s apart: TCP holds what
    follows a lost packet. The release app is on the cable, which the standalone app can't use yet. No
    presenter pointer yet (stock draws it as a KDE overlay).

### Milestone 5: everything the companion setup does (asked 2026-11-07)
The standalone app must match kdeconnectd plus the companion, "Bluetooth and all". The companion's
feature code is reused, not rewritten:
1. **Companion as a library.** `desktop/companion` becomes lib + bin: its session loop (main.rs `serve`)
   takes packets from a channel instead of a link, so both the companion and the daemon run it. The core's
   `link::Sender` gets a second form that queues packets (into the daemon's hub) besides the TLS writer.
2. **The daemon hosts it**: each paired phone's `vyoma.*` packets go to a session; the daemon advertises
   the vyoma capabilities, and its own identity carries what the phone needs to know it isn't a companion
   beside kdeconnectd (no `vyomaHost`, no proof: it paired itself).
3. **Phone: a PC can be its own companion.** Every `companionOf == pcId` lookup (about 15, app and
   service) goes through one helper that also takes the PC itself when it speaks vyoma.*; `pcId` in
   CompanionPlugin falls back to the device's own id.
4. Each feature checked with the debug build: stats, screenshot, camera, files (list/get/put), resumable
   sends (xfer), PC controls (brightness, app volume, power, wake), walk-away, handoff, paste, specs,
   wallpaper, desktop gestures, screen view and touch control, mirror, webcam and mic, controller (uinput,
   UDP, rumble, cable accessory), the Bluetooth touchpad.
5. **Transports the companion gave**, owned by the daemon once it replaces kdeconnectd: the USB cable
   (`adb reverse` 1765 → its own TCP port, payload mirrors, the READ_LOGS / overlay grants), USB tethering,
   and the Bluetooth backup link (RFCOMM `…f801` piped to its own port, `…f802`–`…f804`).
6. **Setup**: one systemd unit, its runtime dependencies (`setup.rs` NEEDS), the firewall, and taking
   over from kdeconnectd (stop and mask it, keep its pairings via the trust store, or pair afresh).
Transports can only be tried for real once kdeconnectd is off on this PC: the user decides when.

Progress (2026-11-07): steps 1–3, 5 and 6 are written, and tests pass.
- The companion is lib + bin. The daemon runs `session::serve` per paired phone, fed by its hub, with packets carrying their source address.
- The phone finds features through `DeviceIdentity.featuresOf`.
- Cable, tethering (unicast announce), Bluetooth (`…f801` to the daemon's port; no `…f802`, nothing to bridge), the wired controller and the setup offer all start only while kdeconnectd isn't running.
- `--import-kde` copies kdeconnectd's id, key and pairings into an empty folder. `install.sh` builds and sets up, opens the firewall, imports, keeps kdeconnectd from starting (autostart Hidden, D-Bus activation to /bin/false), stops the companion and enables `vyoma-desktop.service`. `--undo` reverses it.

Checked with the debug build over Wi-Fi: stats and PC controls on Home, and a resumable send (vyoma.xfer, 3 MB, hash matched).

Done 2026-11-07/08: this PC took over (`install.sh`); on the real phone with the release build, step 4 in full
(the user tried every feature, webcam and camera included) and the transports for real (cable; Bluetooth only:
touchpad, wake and unlock, controller). Over Bluetooth alone, everything that moves a file or a picture says
"Needs Wi-Fi or USB" (Home tiles, the Files tab's sends, opening or saving PC files, Screenshot); listing PC
folders works, and album art isn't asked for. Settings says the PC is paired over Bluetooth (Android's own page
stops saying so once audio and calls are off for it); unpaired, it says so even with the switch off, and a tap
starts Android's pairing with the PC's address (`BluetoothFallback.pair`), a code to confirm on both. The PC
side doesn't pair by itself.

Done 2026-11-08: the phone's files in the PC's file manager over stock SFTP (`kdeconnect.sftp`, as KDE's app):
off until Settings' "Browse this phone from your PC" (asks for All files access); the desktop app mounts it with
sshfs under $XDG_RUNTIME_DIR/vyoma and checks the phone's host key against its pinned certificate. Asked for over
D-Bus (`Browse`) until the GUI has a button.
Loose ends, 2026-11-08: a mount survives the link moving between the cable and Wi-Fi (remounted quietly at the
same folder), and the mirror's sound goes as Opus (128 kbit/s instead of 1.5 Mbit/s PCM) to a PC that can play it.

Still to do:
- The portal touchpad (`pointer-token`) and the daemon's input (`remotedesktop-token`) are separate sessions, so Plasma may ask twice. Each asks once only (its token is kept), so left as is.

### GUI (decided 2026-11-07)
Slint, in Vyoma's own look, on every desktop. First version: pairing and phones (the code to
compare, how each is connected, unpair), setup and health (missing tools, firewall, udev, what's
broken and why), per-phone permissions (screen view/control, files, camera, power, webcam,
controller), and an activity log (screenshots, photos, files, power actions). Built once the daemon
can pair on its own.

Started 2026-11-08 (`desktop/gui`): phones in a sidebar (paired, then nearby), each phone's page with how it's
connected, battery, Ring / Browse files / Send files, pairing with the code to compare (Accept/Reject only when the
phone asked) and Unpair asked in place. The daemon gained a `Changed` signal and `via`/`can`/`asking` in Details.
Setup and health (2026-11-08): This PC in the sidebar (a dot when something needs the user), its page lists each
feature ready or what it needs (needing first), checks no package fixes (firewall: on, and whether a phone got through
on Wi-Fi; KDE Connect running beside) and advice (VA-API), with Install (pkexec, once) when it can help. Daemon:
`Health` (JSON), `Install`, `SetupDone(ok)`.
Per-phone permissions (2026-11-08): each paired phone's page has "Allow this phone to" switches (screen, touchpad and
controller, this PC's files, camera, webcam, power), all on by default, only offs kept in `permissions.json`, dropped on
unpair. Off refuses with `vyoma.denied` (the phone shows why where it was waiting, else a toast) and stops what was
running. Daemon: `Permissions(id)` (JSON), `SetPermission(id, key, on)`.
Activity (2026-11-08): an Activity page under This PC lists what phones did here by day (screenshots, photos, the live
screen, files both ways, shares, power and wake, webcam, touchpad and controller, refusals, commands run), the newest
500 kept in `activity.json` (`vyoma_companion::activity`), Clear asked in place. Daemon: `Activity()` (JSON),
`ClearActivity()`, signal `ActivityChanged`.

macOS (started 2026-11-11, phase 5): nobody on the project has a Mac yet, so macOS code is built and
tested only on GitHub's macOS runners (`.github/workflows/macos.yml`), and nothing counts as working
until it has run on a real Mac. Builds are unsigned at first (no Developer ID): the install steps
explain macOS's one-time Allow in Privacy & Security.
1. Done: vyoma-core builds and passes its tests on macOS (per-OS keepalive options, SO_REUSEPORT on
   the shared discovery port).
2. Done (built and tested on CI, not run on a Mac): `desktop/mac` (`vyoma-mac`, a LaunchAgent) on the
   same core: pairing with a prompt, ping, battery (`pmset`), find my phone, lock, clipboard both
   ways, shares into ~/Downloads, notifications (osascript, shown under Script Editor's name until
   it's an app bundle). Sharing and the clipboard's rules moved into vyoma-core first.
3. Done (built and tested on CI, not run on a Mac): the window talks to vyoma-mac over
   `control.sock` in its folder (this user only), one JSON line per request with the D-Bus method
   names, plus an `Events` stream (`gui/src/socket.rs`, `mac/src/control.rs`). The Mac answers
   phones, pairing (Accept and Reject in the window too), ring, send files, the QR code; the rest
   says "Not on the Mac yet" until step 4. Vyoma.app holds both (`packaging/bundle-mac.sh`,
   universal, ad-hoc signed, CI artifact): opening it sets up the login agent from inside the app,
   and notifications go through UserNotifications under Vyoma's name and icon. The pairing
   question is still an osascript dialog.
4. Built and tested on CI, not run on a Mac: Music and Spotify and the output volume (AppleScript,
   no public "now playing" for every app), touchpad and keyboard (CGEvent, asks for Accessibility
   once), the phone's notifications on the Mac, run commands (the list moved into vyoma-core) with
   the window's Commands page.
5. Next: screen viewing (ScreenCaptureKit and VideoToolbox into the phone's viewer), the pairing
   question in the window instead of a dialog, then a Developer ID if the Mac side gets users.

X11 (2026-11-11, phase 3 started early): one X11 backend for every X11 desktop rather than Cinnamon alone
(`companion/src/x11.rs`, daemon `presenter_x11.rs`): clipboard, touchpad and keyboard (XTEST), screen view and
screenshots (ximagesrc), the presenter's dot, wake, workspace switch and keyboard layout; wallpaper from Cinnamon's,
MATE's and Xfce's settings; a login autostart where systemd's graphical session isn't reached. Checked against
Xvfb only; next is a real Cinnamon or Xfce session (a VM).

### Open
Packaging (AUR, Flatpak, Homebrew), the app's desktop name, and whether KDE users keep the option of
running beside kdeconnectd.

## 11. Direction (decided 2026-11-19): Play Store, Windows, Hyprland, one engine

Four goals: the phone app on Google Play beside F-Droid and GitHub; a Windows PC side; Hyprland (and
the other compositors built the same way); and one engine underneath, so every PC side behaves the
same however the person's machine is set up.

### One engine (first, since Windows and Hyprland both build on it)
Today the feature logic lives twice: Linux has the daemon's stock plugins plus the companion's
`session.rs`; the Mac agent's `main.rs` is its own copy of both. Windows would make a third.
- **Engine** (on `vyoma-core`): the session loop, what every packet does, permits, activity,
  throttles, and the window's control API as one method table (served over D-Bus on Linux, the
  socket on the Mac, a named pipe on Windows).
- **Backends** behind one trait per capability: clipboard; input (pointer, keys, absolute touch);
  media; volume and per-app volume; notifications (show, replace, withdraw, actions, reply);
  lock/sleep/power; brightness; screenshot; screen capture and encoding; the presenter overlay;
  workspaces and overview steps; wallpaper; gamepad; camera; file manager menus; autostart; firewall.
- **Probing, not desktop names**: each capability picks its backend at start from what's there
  (Wayland globals, D-Bus names, portals, programs) and probes again when the session changes, so a
  compositor nobody tested still gets every protocol it speaks. This PC in the window lists each
  capability, the backend in use, and the fix when none works.
- **OS crates become thin**: linux (today's daemon), mac, windows: `main`, install/autostart, backends.
- Done when Linux and the Mac behave exactly as before (unit tests, emulator runs, snapshots).

Progress (2026-11-19):
1. Done: `desktop/engine` (`vyoma-engine`) holds the vyoma.* session (moved from the companion) as
   `serve<P: Platform>`; `Platform` has a method per piece of work with a "not on this computer"
   default, and `supports(Feature)` decides what the identity offers (`capabilities`). Linux's
   `companion/src/platform.rs` calls today's modules, which keep their per-desktop choices; the daemon
   and the companion both use it, offering exactly the packet lists they did before.
2. Done: the Mac runs the same sessions (`mac/src/platform.rs`); its own screen, camera, mirror,
   stats/specs/power/files and paste/Spaces routing is gone. Changes a Mac user notices: a new
   wallpaper reaches the phone without it asking, and the phone's name in Activity is cleaned as on
   Linux.
3. The stock plugins into the engine (2026-11-20, under way): `engine::stock` (ping, battery both
   ways, ringing this PC, its lock: what a packet means, through a `Desk` each side implements with
   its own notifications, ringer and lock; a lock that fails now tells the phone why on the Mac and
   Windows too) and `engine::mousepad` (the touchpad, keyboard and live-screen touches read once, in
   the phone's terms: buttons, named keys, the phone's modifiers, shortcuts by character, text; each
   side maps them to its own key codes, Linux through the layout). Linux, the Mac and Windows all use
   both; each side's own input tests pass unchanged on top. Then the clipboard from the phone and
   shares: `stock` copies (the `Desk`'s clipboard), opens web links, offers or copies shared text
   (Linux a Copy button, the Mac and Windows copy at once), and hands files to the side to fetch
   (`stock::fetch` into Downloads); `Arrivals` counts each phone's batch, so the activity log and
   the Mac's and Windows' notification say a batch once it's all in ("2 of 3 files are in
   Downloads") instead of a line per file. Then the phone's notifications (`notifications`: what a
   packet asks, the note in plain fields, the reply/button/dismiss packets; each side shows it its
   own way) and the players and volume (`media`: the track as the phone is told it, a request read
   into steps in the protocol's order with only its words, outputs and their changes; each side
   finds and drives its players). Linux's players now show seeking only with a length, as Windows'
   and the Mac's did. Then run command (`commands`: list, the phone's "add command", running one
   and its activity line; each side opens its own editor and says a command that couldn't start)
   and calls (`calls`: the Mac's and Windows' call task once, behind a `Ringer` each side gives
   its notifications and pausing; the packets a PC sends about a call; Linux keeps its fuller line
   for answering here and the call's sound over Bluetooth). The presenter dot and the Messages page
   were already shared through core (`presenter`, `messages::Store`). Then one dispatch
   (`dispatch::route`: every packet type to a `Route` each side's loop matches exhaustively, so a
   plugin added to the engine can't go unhandled on one OS; `dispatch::Touchpad`, the touchpad's
   switch, its refusals and its stretches of use in Activity, now on Linux too). Then the
   window's method table (`window::Window<S: Side>`: the Mac's and Windows' requests answered
   once, pairing, ringing, sending, the pairing code, commands, Activity, switches, Messages,
   the phone's details; each `Side` adds its checks, wallpaper and firewall, its notifications,
   its phone picker; one `Device` record) and the Messages page's rules once (`texting::Texting`,
   which Linux's D-Bus methods use too; Linux still reads texts already loaded while the phone's
   away). Linux's other D-Bus methods keep their own table: its devices carry more (files,
   Bluetooth, the companion). Then This PC's "How this PC does it" (`ways::Way`; Linux's
   `companion/src/ways.rs` asks the session what each part's own chooser asks, one row per
   capability with the way found or why there's none; the screenshot's choice is one function
   both use). The Mac and Windows send theirs too (2026-11-24): the Mac's never vary but for
   ScreenCaptureKit (macOS 12.3), Windows' live screen row says the last cast's source and
   encoder (`cast::way`), before the first the graphics card's encoder it will try first. Then Hyprland checked again, nested and in a real session (below). Next: the Play
   Store build.
4. Hyprland and Windows were taken first (user, 2026-11-19); then the window's method table, then
   probing per capability and This PC's report.

Linux, the order each capability tries (as the code does it, and This PC's report says):

| Capability | Tried in order |
|---|---|
| Input | X11 XTEST (an X11 session) → wlr virtual pointer + virtual keyboard (Hyprland, Sway, river) → RemoteDesktop portal (KDE, GNOME) |
| Clipboard | X11 selections → GNOME extension → ext/wlr data-control (KWin, Hyprland, Sway, COSMIC) |
| Overlay | X11 override-redirect → GNOME extension → layer-shell |
| Workspaces | EWMH → Hyprland IPC → Sway IPC → GNOME extension → KWin |
| Screen | ScreenCast portal (xdg-desktop-portal-hyprland too) → X11 ximagesrc |
| Screenshot | X11 (ximagesrc) → GNOME extension → Spectacle → grim (wlr screencopy) |
| Lock | logind lock-session (the session's locker: KDE, GNOME, hyprlock/swaylock through their idle daemon) |
| Autostart | graphical-session.target (KDE, GNOME, uwsm) → XDG autostart → Hyprland's exec-once, offered, never written unasked |

### Hyprland
- xdg-desktop-portal-hyprland has ScreenCast and Screenshot but no RemoteDesktop (late 2026), so input
  goes over the wlr virtual pointer/keyboard protocols, which Hyprland speaks. Clipboard
  (data-control), the layer-shell dot, MPRIS, pactl and freedesktop notifications already work.
- Hyprland's own: workspaces through its IPC socket; locking through hypridle/hyprlock or loginctl;
  often no polkit agent (the Firewall check's pkexec).
- Hyprland 0.57 is installed on the dev PC: tests run in a nested Hyprland with a scratch config.

Done (2026-11-19), checked against Hyprland 0.57.2 nested in a headless KWin (private runtime dir and
bus): `companion/src/wlr.rs` (virtual pointer + keyboard, the layout's keymap from xkbcommon, a lone
keymap for symbols no key types; used wherever the compositor offers both, before the portal) for the
touchpad, keyboard, Bluetooth touchpad and cast touches (an output-bound pointer for absolute moves);
`companion/src/hypr.rs` (IPC socket, found by probing a listening socket; 0.57 takes dispatchers as Lua,
`hl.dsp.focus({ workspace = "e+1" })`, older ones the classic words, both tried) for workspaces, dpms on
wake, log out, the keyboard layout (`[[EMPTY]]` is unset) and the wallpaper (hyprctl hyprpaper
listactive: hyprpaper 0.8's socket is binary; swww query); grim screenshots; the kernel backlight
through logind (any desktop without Plasma's or Mutter's); restart/shut down through logind where no
session manager answers. Hyprland 0.57 starts hyprland-session.target (binds graphical-session.target)
and hands systemd its environment, so the unit starts with no config line.

Checked again (2026-11-20) in the same nested stack plus a private PipeWire/WirePlumber (no devices)
and xdg-desktop-portal with xdg-desktop-portal-hyprland, a scripted picker in place of its dialog: This
PC's report reads Hyprland's virtual pointer, data-control, the ScreenCast portal with touches through
the virtual pointer, grim, layer-shell and Hyprland's workspaces; the clipboard round-trips through
wl-clipboard; a cast runs through the portal into the encoder (DMA-BUF, ~20 frames in 3 s with a clock
on screen). Fixed: Spectacle was picked for screenshots wherever installed, so a person who moved from
Plasma got failing screenshots; it now comes first only on Plasma, grim first elsewhere. Then in a
real session (Hyprland 0.57.2 with Caelestia, xdph 1.5, Intel graphics driving the screen): the
report, a grim screenshot and the daemon itself (it took Hyprland's environment from systemd at
login; the phone linked, the clipboard flowed) all fine. The cast: Hyprland's portal sends a
picture only once part of the screen changes (wlr screencopy's copy_with_damage), nested or not,
and the pointer moving doesn't count; after that pipewiresrc only repeats the last picture each
second. Measured with the screen still: nothing for 4 s. So a still desktop left the phone's viewer
empty. Fixed: until the first picture the stream marks the screens changed with an invisible 1×1
layer-shell overlay that takes no clicks (`wlr::touch_screens`, every 300 ms, at most 10 times);
the first picture then came after 0.8 s. Still so: the pointer moving alone sends no new picture
(a touch from the phone changes what's under it, which does). Not yet tried: hyprexpo's overview
(plugin not installed; its Lua dispatcher name is a guess).

### Sway
Done (2026-11-24), checked against Sway 1.12 (wlroots 0.20, unpacked from the distro's signed
packages into a scratch folder) nested in a headless KWin: `companion/src/sway.rs` speaks Sway's
IPC socket (i3's protocol; `$SWAYSOCK`, else the only listening `sway-ipc.<uid>.*.sock`) for the
workspaces (Hyprland's swipe rules: past the last to a new number, nothing past the first), the
displays' power (`output * power on`, `dpms` before 1.8), log out, the keyboard layout (the
config's `xkb_layout`, else XKB_DEFAULT_*) and swaybg's wallpaper (the config's last `output ... bg`,
else swww). Input, data-control, the layer-shell dot, grim and the cast's wake were already the
wlroots ways. This PC reads "Sway's virtual pointer and keyboard" and "Sway's workspaces". Not
done: the swipe up (Sway has no overview), Sway's lock state (logind's LockedHint only).

### Windows
- `vyoma-windows` on the engine; the window reaches it over a named pipe. Backends: Win32 clipboard,
  SendInput, the system media controls (every player), Core Audio (per-app volume too), toasts
  (Start menu shortcut + AppUserModelID), LockWorkStation, WMI/DDC brightness, Windows.Graphics.Capture
  + Media Foundation H.264, ViGEmBus for the gamepad (a real Xbox 360/DualShock 4 pad; the driver is
  installed once and no longer maintained), cmd/PowerShell commands, Run-key autostart.
- Core changes: interface addresses, TCP keep-alive through socket2, owner-only files, the send-buffer tweak.
- Catches: the firewall asks an admin on first run; SmartScreen warns about an unsigned app
  (SignPath Foundation signs open-source projects for free); crates `windows`/`windows-sys` (MIT/Apache).
- CI `windows.yml` builds and tests on GitHub's runners; real testing needs a Windows PC.

Started (2026-11-19), `desktop/windows`, checked on this PC as Windows builds under Wine (mingw cross
build, headless Xvfb display, private ports and pipe) and next on GitHub's runners:
- Core builds and passes on Windows: socket2 keep-alive, GetAdaptersAddresses for the pairing code's
  addresses (virtual adapters skipped by name), owner-only files by the profile's ACL, run commands
  through `cmd /C` (9009 = not found).
- The agent: one per session (named mutex), Run-key autostart (`--install`, also what the window's
  first open runs), the notification-area icon with Open/Quit, phones told it's leaving when the session ends; pairing with a dialog
  showing the code (stale answers ignored) or in the window; ping, battery both ways, find my PC
  (alert sound), lock, clipboard both ways (password managers' copies skipped), share (Downloads;
  only http(s) links opened, other schemes copied instead), touchpad and keyboard (SendInput: Unicode
  typing, shortcuts on the layout's keys), phone notifications, texts, run commands. The engine's
  Windows platform: stats, About this PC (registry SMBIOS names, never the serial), screenshots
  (GDI, per-monitor DPI), files, power (ExitWindowsEx/SetSuspendState, never forced), wake, Task
  View and the desktops (Win+Tab, Ctrl+Win+arrows), paste.
- The window builds for Windows: the named pipe in `socket.rs`, one window per session, the agent
  started on first open. Slint 1.19 (1.18's winit throttle divided by a monitor's 0 Hz refresh).
- End to end (`windows/tests/agent.rs`): a vyoma-core phone links, the window pairs it over the
  pipe, the command list, About this PC, stats, a screenshot, the players and the speaker come back,
  the window unpairs it, and `Quit` (the installer's `--quit`) stops the agent.
- Then (2026-11-19, the user's "first 4"):
  - Media and volume (`media.rs`): every app in Windows' media controls (system media transport
    controls: Spotify, browsers, Media Player...) as MPRIS players, with covers as payloads, shuffle
    and repeat; the default speaker's volume and mute through Core Audio. Players have no volume of
    their own here (theirs is the speaker's).
  - Toasts (`toast.rs`): registered for this user only (`AppUserModelId\dev.vyoma.Vyoma` + a COM
    activator class, no shortcut, no administrator), every notification a toast where Windows has
    them, balloons otherwise. The pairing question with Accept/Reject (a dialog if toasts are off),
    the phone's notifications with Reply and up to three of their actions, taken down when the phone
    dismisses them and dismissed on the phone when closed here, Send to's choice of phone as buttons.
  - The live screen (`cast.rs`): desktop duplication of the main display (GDI copies where there's
    none), the pointer drawn in, NV12 into Media Foundation's software H.264 encoder (low latency,
    30 fps unless asked for up to 60), the Mac's framing; touches as absolute input on that display.
    Under Wine (GDI plus Wine's encoder) two seconds decode cleanly with ffmpeg.
  - The icon in both programs (`dist/vyoma.ico` from `gen.py`, embed-resource), the firewall check
    (`firewall.rs`: blocks win over allows, per network profile; Let phones through runs the agent
    elevated as `--open-firewall`, which replaces Vyoma's rules with two for its ports), and the
    NSIS installer (`packaging/windows/vyoma.nsi`: per user, no administrator, Start menu entry,
    Settings > Apps uninstall entry; built on CI and attached to releases as `Vyoma-setup-<ver>.exe`).
- Then (same day, "fix the worth knowing parts, then add more features"):
  - The live screen uses the graphics card's H.264 encoder (an asynchronous Media Foundation
    transform: Intel, NVIDIA, AMD) when one takes pictures from memory, and goes on with the
    software one if it refuses mid-stream; a display turned to portrait is turned upright instead
    of falling back to GDI copies (Microsoft's Desktop Duplication sample's mapping, in a test).
  - Signing on CI when the repo has a certificate (`WINDOWS_SIGN_PFX`, `WINDOWS_SIGN_PASSWORD`):
    both programs, the installer and its uninstaller (NSIS `!finalize`/`!uninstfinalize`). Only a
    certificate authority's certificate (or SignPath's free programme for open source) stops
    SmartScreen's warning; without one it builds unsigned.
  - Brightness and app volumes (`pc.rs`), the wallpaper's colours for the phone and the window's
    look (`wallpaper.rs`), walk-away rules, calls (ringing toast with Silence and Decline where the
    phone allows, missed calls, music paused), the daily update check, Explorer's Send to.
  - Shared now instead of copied per side: walk-away's runner (`vyoma_core::away::Rules`, Linux and
    the Mac switched to it), a phone's call steps (`telephony::Line`), the update check
    (`vyoma_core::updates`, the Mac switched to it).
- Then ("do the mirror and camera next"):
  - The phone's screen in a window (`mirror.rs`): the agent runs itself as a child
    (`--mirror-window`), as Linux and the Mac do; Windows' own H.264 decoder (Media Foundation, low
    latency) into GDI, aspect kept; the mouse and keys as on Windows (Ctrl+C/X/A/V, Ctrl+arrows by
    word, Escape / right click / the mouse's back button for Back, middle click Home, Ctrl+W closes);
    the phone's sound as PCM through WASAPI; its copies onto the clipboard, its volume keys as the
    speaker's own steps.
  - A photo with the PC's camera (`camera.rs`): Media Foundation's source reader at the largest
    picture up to 1080p, a moment to expose, a JPEG from WIC, a notification each time; the per-phone
    Camera switch; This PC's Camera check reads Windows' privacy switches and opens their page.
  - Shared now: the stream splitting and the sound's jitter buffer (`vyoma_core::mirror`, the Mac
    switched to it).
  - Checked under Wine: the decoder turns the encoder's clip back into pictures, the window shows
    them and closes at the stream's end, and the agent opens and closes it for a phone (end-to-end
    test). Not checked: the sound (Wine would play it on this PC's speakers), a real camera.
- Then (after committing, "proceed building further"):
  - The presenter's pointer: a layered, click-through window above everything with the same dot as
    Linux and the Mac (`vyoma_core::presenter` now draws it for all three and reads the packets).
  - The USB cable: Android's adb device interface watched through cfgmgr32 feeds core's `adb`, so a
    phone plugged in with USB debugging links over adb's tunnel; the grants' question as a toast;
    the window's Clipboard card Allow; This PC's USB cable check with Get adb (Google's platform
    tools, fetched and unpacked with Windows' own curl and tar). Not run with a phone: none here may
    be used, and adb itself can't be run under Wine without running a download.
  - The window's `--page` reaches an open window (WM_COPYDATA to a message-only window), so the
    agent's "open the command editor" works while it's open.
- The gamepad (2026-11-19; ViGEmBus is retired upstream, and the other drivers are paid or
  DirectInput-only): an Xbox 360 pad made in software (core `usbip` + `xbox360`: our own USB/IP
  server and the real pad's descriptors) that usbip-win2's signed driver (BSD) plugs in, so Windows'
  own Xbox driver binds to it and XInput games, Steam and SDL see it; rumble comes back. The loop
  itself moved into the engine (`gamepad::drive`), Linux's uinput pad now behind the same `Pads`.
  This PC's Game controller check gets the pinned usbip-win2 installer (SHA-256 checked); without
  it the phone says how. Tested against a played driver side only: Wine loads no kernel drivers, so
  the attach and a real game wait for a Windows PC.
- The DualSense on Windows: core `dualsense`, a HID gadget with the real pad's report descriptor,
  reports streamed at 250 a second, the calibration / address / firmware feature reports SDL and
  Steam read, rumble from output reports. Same limits: tested against a played host only.
- Colour conversion on the graphics card (2026-11-20, `windows/src/gpu.rs`): Direct3D 11's video
  processor scales the duplicated desktop and makes NV12 (BT.601, video range), the pointer drawn
  with GDI on a copy on the card; only the NV12 picture is read back (3 MB at 1080p instead of the
  desktop's BGRA, and no per-pixel pass on the CPU). A turned display, a driver without a video
  processor (Wine), or one that fails mid-stream goes through the CPU as before. Untested: GitHub's
  runners have only Windows' software card, which has no video processor (the test checks nothing
  there), and Wine's claims one it can't make; it waits for a PC with a graphics card.
- Next: the NV12 picture handed to the hardware encoder as a texture (no read-back at all; needs the
  encoder's Direct3D manager), the mirror drawn with Direct3D if GDI's scaling proves slow.

### Google Play
- Started 2026-11-20: a `play` build type (not a flavor, so every existing task keeps its name)
  built as an AAB (`bundlePlay`); the GitHub/F-Droid build stays as it is. The checklist, declaration
  texts and Data safety notes are in `docs/PLAY.md`; the privacy policy is `site/privacy.html`; the
  mirror screen asks before opening Accessibility settings (Play's prominent disclosure).
- Signing: uploading the existing release key to Play App Signing keeps one signature everywhere
  (people switch channels without reinstalling; the PC's cable allow-list goes by signing key). It
  can't be undone: the owner's call.
- Account: personal accounts made after 13 Nov 2023 need a closed test with 12 testers for 14 days
  before production; organisation accounts (D-U-N-S number) don't.
- Declarations, each reviewed case by case; the Play build drops what's refused:

| Permission | Play's terms |
|---|---|
| READ_SMS, SEND_SMS, READ_CALL_LOG | Exception "cross-device synchronization or transfer of SMS or calls" |
| MANAGE_EXTERNAL_STORAGE (the PC browsing phone files) | File managers, backup, documents only; if refused, folder grants (SAF) in the Play build |
| Accessibility service (mirror control) | Prominent in-app disclosure plus a declaration |
| Foreground service types camera, microphone, mediaProjection, connectedDevice | Console declarations with videos |
| USE_FULL_SCREEN_INTENT | Calling and alarm apps only: check each use |
| REQUEST_IGNORE_BATTERY_OPTIMIZATIONS | A justified use |

- Also: a privacy policy page on the site; Data safety (no servers; data only goes to the person's
  own PCs); the store listing from fastlane metadata. No self-update code to remove.

### Open (2026-11-19)
Play account type and signing; who tests on Windows and on Hyprland.
