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
- **Remote input:** touchpad with gestures, keyboard, presenter mode.
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
- **Gamepad:** uinput virtual **Xbox** (045e:028e-style) and **DualSense-style** pads
  (Steam Input recognises both). Gyro → right stick or mouse with deadzone and smoothing,
  haptics on press, PC rumble → phone vibrator (VibratorManager compositions), and a
  drag-and-drop layout editor with per-game profiles. Inspired by Steam Input, Backbone and Moonlight.
  Transport: UDP state snapshots with AEAD (ChaCha20-Poly1305, session key from TLS), <10 ms on LAN/USB.
- **Phone mirroring + control from PC** (MediaProjection + HW encode; control via accessibility gestures).
- **Phone as PC webcam/mic** (Camera2 → companion → v4l2loopback; mic → PipeWire virtual source).
- **Bluetooth fallback transport** for notifications/clipboard when Wi-Fi is absent.

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
- Themes: **wallpaper dynamic color** (default) or curated fixed palettes
  (Graphite, Ocean, Ember, Mint, Sand) ; dark mode is **AMOLED true black** apart from dim accent glows at the top. No purple default.
- Haptics are tied to motion (`performHapticFeedback` + Vibrator primitives).

## 7. Security

User choice: **all features enabled by default** for paired devices. TLS with pinned
peer certificates (inherent to the protocol). Per-plugin toggles still exist in settings
and can be turned off at any time.

## 8. Testing

- Protocol: unit tests against captured KDE Connect / GSConnect packet fixtures.
- Interop matrix: KDE Connect (Plasma 6, your machine), GSConnect & Valent (GNOME VM).
- Device: S24 FE (Android 17) plus an emulator on API 31 / 34 / 36.
- Perf: Macrobenchmark for startup and scroll jank; battery measured over 24 h idle.

## 9. Phase 0: setup (next step)
1. Install the JDK 21 / Android SDK cmdline-tools / platform-tools (adb) on CachyOS.
2. `git init`, Gradle skeleton, module layout, version catalog, theming foundation.
3. Protocol core + discovery + pairing with your Plasma machine → first green link.
