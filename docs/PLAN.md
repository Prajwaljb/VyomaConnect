# VyomaConnect — Architecture & Roadmap

A fast, battery-friendly, open-source Android client for the KDE Connect protocol.
Fully compatible with KDE Connect (Plasma), GSConnect and Valent (GNOME), with an
optional Linux companion that unlocks features stock KDE Connect can't do.

Primary target: Galaxy S24 FE, Android 17 / One UI 9. Supported: Android 12+ (API 31).
Distribution: F-Droid / FOSS — **no Google Play Services, Firebase, or proprietary SDKs.**

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
  Not yet: phone audio to the PC, clipboard paste into the phone from the window.
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
Next: setup and health, per-phone permissions, the activity log; then the macOS socket behind the same `Daemon` trait.

### Open
Packaging (AUR, Flatpak, Homebrew), the app's desktop name, and whether KDE users keep the option of
running beside kdeconnectd.

