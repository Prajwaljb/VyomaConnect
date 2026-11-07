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
- Next: the stock plugins, in the order above. Each is checked against kdeconnectd, then against the phone's debug build.

### GUI (decided 2026-11-07)
Slint, in Vyoma's own look, on every desktop. First version: pairing and phones (the code to
compare, how each is connected, unpair), setup and health (missing tools, firewall, udev, what's
broken and why), per-phone permissions (screen view/control, files, camera, power, webcam,
controller), and an activity log (screenshots, photos, files, power actions). Built once the daemon
can pair on its own.

### Open
Packaging (AUR, Flatpak, Homebrew), the app's desktop name, and whether KDE users keep the option of
running beside kdeconnectd.

