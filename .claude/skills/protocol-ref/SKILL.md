---
name: protocol-ref
description: Reference for the KDE Connect wire protocol (v7/v8) as implemented in core/protocol, plus VyomaConnect's vyoma.* extension rules. Read before adding a plugin, a packet type, a transport, or touching handshake/pairing code.
---

# KDE Connect protocol, as we implement it

## Framing
One JSON object per line, `\n` terminated: `{"id": <ms long>, "type": "...", "body": {...}}`
plus `payloadSize` and `payloadTransferInfo: {"port": N}` when a binary payload follows on a separate TLS socket.
Peers mix up number and string types, so always use the lenient `NetworkPacket.string/int/long/bool` accessors.

## Discovery & link (LanBackend)
- UDP 1716: identity broadcast (also unicast to manual hosts, which is how Tailscale works).
- TCP 1716..1764: first free port, advertised as `tcpPort` in identity.
- Whoever **receives** the UDP connects over TCP, sends its identity in plaintext, then is the **TLS server**.
  The acceptor reads that line (byte-by-byte; never over-read) and is the **TLS client**.
- Mutual TLS with self-signed RSA-2048 certs, CN = deviceId, O=KDE, OU=Kde connect. The TLS layer accepts any
  cert; trust = exact pin of the cert stored at pairing (DeviceHub.onLink).
- v8: after TLS both sides resend identity; only the TLS copy is trusted; CN must equal deviceId.
- Device ids: we generate UUID with `_` (36 chars). We accept `[a-zA-Z0-9_-]{16,64}` from peers.
- Names: strip `"',;:.!?()[]<>`, max 32 chars.
- USB = adb reverse tunnel (phone dials 127.0.0.1:1765 → PC 1716) or a tether interface. Payloads over the tunnel:
  the companion mirrors PC 1739..1764 at phone 1839..1864 (`LanPayloads.USB_PAYLOAD_SHIFT`), clear of the phone's own
  payload ports; the other way it `adb forward`s PC 1939..1964 → phone 1739..1764 and, on a loopback link, the phone
  announces its payload port + 200 (`USB_FORWARD_SHIFT`), so kdeconnectd's connect-back reaches the phone (before,
  nothing sent from the phone over the cable ever arrived); `Device.openPayload` falls back to another link if one can't reach the sender. kdeconnectd keeps only its
  **newest** connection per device and ignores broadcasts from devices it's linked to, so while on USB we never dial
  Wi-Fi, and refuse its incoming Wi-Fi *before TLS* if our cable link is < 10 s old (a race). An incoming Wi-Fi call
  on an older cable link means the tunnel died on its side: accept it and drop the USB link (`LanBackend.Config.refuses`,
  `DeviceHub.onLink`). Unplug (USB_STATE) closes USB links at once and refreshes; the companion rechecks the
  tunnel every 12 s (an adb server restart kills it while the phone's end stays half-open).
- Cable in (or app start with it in): Wi-Fi is held back 4 s so the tunnel wins the race. Never dial a paired
  device's address while a device at that address is on the cable, and never dial companions (no listener; their
  address:1716 is kdeconnectd, which would swap our cable link out).

## Pairing (Device)
- Request: `kdeconnect.pair {pair:true, timestamp:<unix seconds>}`; accept: `{pair:true}`; reject/unpair: `{pair:false}`.
- Requests from v8 peers must carry a timestamp within 30 min of ours, else `{pair:false}`: it goes into the code, so a free one lets a sender search for a matching code.
- All live links of a device show one certificate (`Device.admits`), as ids are public: paired, only the pinned one; a pairing on screen, only the one whose code is shown; otherwise the newest replaces the others at once. Limit: when the phone asks, any `pair:true` from whoever holds that id completes it, so the user must see the PC really showed the prompt.
- Anyone on the network: 24 handshakes at a time, 4 per address, 20 s each in all (`HandshakeGate`, Dispatchers.IO has 64 threads and runs every link); discovery dials only to 1716..1764, at most 16; 8 unpaired devices (`makeRoomForStranger`); 64 KiB per packet until paired.
- 30 s timeout. Verification key = first 8 hex chars (uppercase) of SHA-256(larger SPKI ‖ smaller SPKI ‖ decimal timestamp),
  compared unsigned. This is verified against kdeconnectd over D-Bus.
- Paired peer re-requests → we re-confirm with `pair:true`. Packets from unpaired peer → reply `pair:false` once.

## Plugins
`PluginSpec(id, incoming, outgoing, create)`. Loaded when paired+reachable and
`incoming ∩ peer.outgoing ≠ ∅ || outgoing ∩ peer.incoming ≠ ∅`. Our identity advertises the union of all specs.
`onPacket` runs on the device's single-threaded dispatcher: never block there; launch heavy work in `ctx.scope`.
Stock types live in `PacketTypes`. Check the upstream kdeconnect-kde / kdeconnect-android plugin source for exact
body fields before implementing one.

## Presenter (stock, no companion)
Slides are `kdeconnect.mousepad.request` keys: Page Down/Up (next/previous), F5, Esc, `key:"b"` (black screen). The pointer
is `kdeconnect.presenter {dx, dy}` (KDE's overlay, screen -1..1 across, it multiplies dy by the aspect) then `{stop:true}`;
keep sending while held, as KDE drops a dot it stops hearing about. `PresenterPlugin`, app presenter/PointerGyro.kt.

## vyoma.* extensions (companion and our own features)
- Namespace `vyoma.<feature>[.<verb>]`, advertised in capabilities like any packet, so stock peers just don't load them.
- Never change the meaning of a stock `kdeconnect.*` packet; extend with extra optional body fields only if stock
  peers ignore unknown fields (they do).
- Real-time streams (gamepad, video) don't go through the JSON link: negotiate over a vyoma packet, then use a
  dedicated socket (UDP with AEAD for input, TLS/TCP for video).

### Companion packets (desktop/companion ⇄ CompanionPlugin)
- `vyoma.stats.request` → 12 s lease of `vyoma.stats` every 2 s: `cpu, memUsed, memTotal, cpuTemp?, gpu?, uptime?,
  handoff?` (`handoff` = title of a web video the phone could continue; the phone shows "Continue on phone" only
  when it matches the playing title, so local players like mpv never offer it).
  `gpu` = amdgpu `gpu_busy_percent`, else Intel `1 − Δrc6_residency/Δt` (no root; never wake the NVIDIA dGPU).
- `vyoma.screenshot.request` → `vyoma.screenshot {format:"webp"}` + payload (companion listens on 1739..1764, is the
  TLS server, and only serves the pinned phone cert), or `{error}` with no payload. Phone saves to Pictures/Vyoma.
  Any companion payload uses this same flow (`payload.rs`).
  The payload port is open to the network, so on either side a connection that's wrong or stalls is only dropped: at most 4 prove themselves at once, 5 s each, and the wait goes on (15 s PC, `timeoutMs` phone). The PC shows a notification for each screenshot (`notice.rs`).
- `vyoma.camera.request` → `vyoma.camera {format:"jpg"}` + payload, or `{error}`. ffmpeg grabs one frame from the
  first V4L2 node offering MJPEG (skips IR cams), largest size, after 15 warm-up frames. The PC always shows a
  notification ("<phone> took a photo with this PC's camera"). Phone: `CompanionPlugin.photo` (same `Capture` as
  `screenshot`), saved to Pictures/Vyoma.
- `vyoma.handoff.request {pause?=true}` (only players with an http(s) URL count) → `vyoma.handoff {url, title, position(ms)|null, exact}` or `{error}`; the
  companion pauses that player. Position: MPRIS `Position` when nonzero, else our own clock from signals (track
  change → 0 or the URL's `t=`, counts while Playing, reset on `Seeked`); null if we attached mid-play and the player
  never said. Phone opens `Handoff.at(url, pos)` (YouTube `watch?v=ID&t=Ns`). Phone→PC: sharing a YouTube link
  while YouTube plays here appends the MediaSession position and pauses the phone (`Handoff.continueOnPc`).
- `vyoma.files.list {path}` → `vyoma.files.listing {path, entries:[{name, dir, size, modified(ms)}], truncated, error?}`.
  `path` is relative to the PC user's home (`""` = home); `..`/absolute components are refused and the canonical
  path (symlinks resolved) must stay inside home, with no dot-named part: hidden files can't be listed, fetched or written (`.config/autostart` would run code on the PC). Folders first, max 5000 entries.
- `vyoma.files.get {path}` → `vyoma.files.file {path, name}` + payload, or `{path, error}`. The PC says so in a notification, at most one per 30 s. Phone saves to
  Downloads/Vyoma via `ShareSink.createFile`, tracks it in `Transfers`, and opens it.
- `vyoma.files.put {path(folder), name}` + payload (phone serves it; the companion connects as TLS client with its
  cert and pins the phone's) → `vyoma.files.saved {path, name, error?}`. Written as `.name.vyoma-part`, renamed when
  complete, never overwrites (`name (2).ext`); the PC then shows the same "Received" notification as Send files. A refusal before connecting makes the phone stop waiting.
- The companion rechecks `trusted_devices` every 5 s and ends a session once the phone is unpaired or its certificate changed.
- `vyoma.pc.request` → `vyoma.pc {displays:[{id, label, internal, value 0..1}], apps:[{id, name, binary?, title?, volume, muted}]}`
  (`pc.rs`). Displays come from Plasma's `org.kde.ScreenBrightness` (panels and DDC monitors; ids are its D-Bus names,
  checked as `[A-Za-z0-9_]` since they become a path); apps are `pactl -f json list sink-inputs` minus event sounds,
  `volume` = loudest channel / 100% (can pass 1 if boosted). Either part is empty if the PC can't report it.
  `vyoma.pc.set {display, brightness}` | `{app, volume?, muted?}` → a fresh `vyoma.pc`. Brightness is floored at 1%
  (never black from the phone); app volume clamps to 150%. The companion handles these in order on one worker per
  link and collapses a burst of sets for the same slider to the newest; the phone also sends at most one per target
  every 60 ms (`CompanionPlugin.queueSet`) and re-requests every 3 s while the controls are on screen.
- `vyoma.pc.wake` (no reply), sent with every unlock from the phone: kdeconnectd unlocks via logind but a screen that
  idled off stayed dark. The companion runs `kscreen-doctor --dpms on`, ScreenSaver `SimulateUserActivity`, then logind
  `Session.Unlock` on `session/auto`.
- `vyoma.away {lock, pause, mute, after, undo}` (no reply), the phone's walk-away rules merged (enabled "PC goes away"
  rules; `after` = the shortest wait in seconds). Sent on every companion link (`onStart`/`onLinkAdded`) and whenever the
  rules change (`CompanionPlugin.away`). When all of that phone's links close and it stays gone `max(after, 10)` s
  (plus the ~30 s keepalive), the companion pauses every playing MPRIS player, mutes the default sink (`pactl`) and
  locks via logind `Session.Lock` on `session/auto` (desktop/companion/src/away.rs); with `undo`, its next link unmutes
  and resumes what it paused. A returning link bumps a generation counter that cancels the pending timer.
- `vyoma.stats` also carries `camera: bool`: false when the only cameras are built in (`removable=fixed` /
  `connect_type=hardwired`) and the lid is closed (`/proc/acpi/button/lid/*/state`). The phone dims its Camera tile
  (NoPhotography icon) and a tap says why; the companion also refuses such a capture.
- `vyoma.stats` also carries `ac: bool` (absent on a desktop): a Mains or USB-C supply is online. Stock battery packets
  only say `isCharging`, false on a charger at a charge limit or full, so the phone shows "Plugged in" from this.
- Bluetooth fallback (`bluetooth.rs`, app `service/BluetoothTunnel.kt`), not stock's beta backend: two BlueZ RFCOMM
  services for bonded phones. `…f801` (channel 26; BlueZ won't listen without one) pipes to kdeconnectd 127.0.0.1:1716,
  and the phone links through it as TLS server after 15 s offline. Once that's the PC's only link, `…f802` (channel 27)
  bridges to a one-time local port the companion dials, with `LanBackend.acceptTunnel` answering, so stats, controls and
  the controller (link packets, no UDP) work and the walk-away rule sees a phone still here. Links have `payloads = null`:
  the Dashboard's file/picture tiles say "Needs Wi-Fi or USB". Any other link to the PC closes both (`DeviceHub.onLink`).
  `…f803` (channel 28) takes the controller's 51-byte frames whatever the link is on (order: cable, Bluetooth, UDP; only
  with the Settings switch on). The cable, Bluetooth and UDP echo the seq (4 bytes, big-endian) of every 32nd frame: the
  phone logs "controller round trip over …" every 30 s, the companion "controller frames: … gaps over 150 ms" at close.
  `vyoma.bluetooth {address}` on every companion link tells the phone where to dial, per PC.
- Companion over the cable: it dials the phone's listener through `adb forward`, which arrives from loopback, so
  `LanBackend`'s "another app on this device" check only applies to identities with no `vyomaHost`.
- Typing: kdeconnectd on Wayland presses the key a keysym is on at any level, without Shift, and drops what no key
  types (✓, emoji: "failed to convert keysym"). `MousePadPlugin.type` sets `shift` on runs that are shifted on a US layout.
  What isn't printable ASCII goes by clipboard (`paste.rs`): `vyoma.paste {text}` → `vyoma.paste.ready {error?}` once it's on
  the PC's clipboard, the phone presses Ctrl+V through mousepad, and 1.2 s after the last one the companion restores the old
  content unless something new was copied. The phone ignores stock clipboard packets for 3 s meanwhile (their echo). The
  typing bar queues everything, counts by character (`characterCount`, `commonEnds`) and, with no companion, refuses such
  text, so its field always equals what the PC got. The PC screen keyboard does the same paste with portal keys, ignores the echo too, and on an unasked `vyoma.paste.ready {error}` (text that failed) clears its field.
- USB tethering (no adb): the companion's `tether.rs` finds the phone on a `rndis_host`/`cdc_ncm`/`cdc_ether` interface
  (its default gateway there from /proc/net/route, else the one ARP neighbour) and dials port 1716 every 5 s while a
  trusted phone is unlinked, accepting whichever trusted certificate answers (`link::connect_trusted`).
- `vyoma.pc.power {action: sleep|restart|shutdown|logout}` → `vyoma.pc.powered {action, error?}`. Sleep is
  Solid's `suspendToRam`; the others are `org.kde.Shutdown` (`logoutAndReboot`/`logoutAndShutdown`/`logout`), i.e.
  graceful, apps asked to close. The phone confirms every action first and treats no reply in 5 s as done.
  **Never send these to the real PC in tests.**
- Resumable sends, phone → PC (`xfer.rs`, `CompanionUploads`): used for shares and picks whenever the companion is up
  (else stock `kdeconnect.share`). `vyoma.xfer.begin {key, name, size, folder?, batch?}` → `vyoma.xfer.ready {key,
  offset | error}`; `vyoma.xfer.data {key, offset}` + payload of the rest → `vyoma.xfer.done {key, name | error}`;
  `vyoma.xfer.end {batch, count}` after a folder (one PC notification). Lands in kdeconnectd's `incoming_path`, else
  XDG Downloads, as `.name.vyoma-part` + `.name.vyoma-key`, renamed when complete, never overwriting; parts untouched
  for a day are cleared. A folder's top gets a free name once per `batch` (resumes write into the same one); hidden
  names are allowed inside a folder only. The phone retries with the same key after any break and gives up after 5 min
  without progress; a new companion link (it restarted) or onStop fails waiting sends at once. Files whose size the
  provider won't give go the stock way. Payloads to a companion name `usbPort` (= port + 200) in
  `payloadTransferInfo` when the PC's cable tunnel is up; the companion tries 127.0.0.1:usbPort first (cert-pinned,
  so another plugged-in phone is refused) — ~70 MB/s vs this home's 2.4 GHz Wi-Fi at <1 MB/s.
  Measured here: parallel streams didn't help on lossy Wi-Fi (4 streams 0.53 MB/s vs 1 stream 0.88), so none.
- `vyoma.screen.request {maxFps?}` (the most frames a second the phone shows and decodes; PC caps at 120, 60 if absent) → `vyoma.screen {codec:"h264", framing:"avc4", width, height}` + an open-ended payload
  (`payloadSize` -1), or `{error}`. The companion casts one monitor via the ScreenCast portal (persist_mode 2; restore
  token in `~/.config/vyoma-companion/screencast-token`, so Plasma asks once), asks KWin for ≤60 fps (`max-framerate`
  caps; videorate can't cap the variable-rate capture), encodes with `vah264enc` (VBR 8 Mbps, no B-frames, one slice,
  ≤1920 wide, 600 kbit CPB so a full-screen change can't make a burst frame, IDR only every 1024 frames, with SPS/PPS). Framing: 4-byte big-endian length + NAL; before each slice a stamp NAL
  `06 'VYTS' <u64 µs epoch>` (13 bytes) the phone strips and uses for its latency log. Small buffers end to end (16 KiB
  pipe and relay, `TCP_NOTSENT_LOWAT` 16 KiB) so a slow link drops raw frames at the leaky queue instead of queueing
  seconds. The phone fetches a companion's payloads through the PC's USB tunnel mirror when the PC is linked over USB
  (`DeviceEnvironment.usbTunnelUp`), holds a low-latency Wi-Fi lock while viewing, waits up to 60 s (portal dialog),
  decodes with MediaCodec (`KEY_LOW_LATENCY`) into a TextureView and shows only the newest of a burst.
  `vyoma.screen.stop` (or closing the payload) ends it; a new request replaces a running cast.
  `VYOMA_SCREEN_TEST=1` on the companion casts a moving test pattern, no portal (safe to screenshot).
- `vyoma.gamepad.state {b, lx, ly, rx, ry, lt, rt, kind, mx, my, sid, seq}` (phone → companion): the phone's
  controller. `b` bitmask in `GamepadButton` order (A B X Y LB RB Back Start Guide LS RS Up Down Left Right), sticks
  -32768..32767 y down, triggers 0..255, `kind` "xbox"|"dualsense", `mx`/`my` running totals (wrapping i32) of
  gyro-mouse pixels (the companion moves a separate uinput REL mouse by the difference, made on first motion),
  `sid` random per stream + `seq` counting (older ones dropped; a new sid takes over). Sent on every change over the
  normal link (TCP_NODELAY both ends, so it works over the USB tunnel too) and resent each second; the companion's
  first one creates a uinput pad (gamepad.rs, raw ioctls, user needs /dev/uinput access: input group/ACL): Xbox 360
  045e:028e as xpad, or DualSense 054c:0ce6 version 0x8111 as hid-playstation over USB (sticks 0..255, triangle =
  BTN_NORTH 0x133, square = BTN_WEST 0x134, BTN_TL2/TR2 with the triggers; SDL3 reads it as "PS5 Controller",
  checked with a C probe). A different `kind` replaces the pad. Removed on `vyoma.gamepad.close`, 4 s of silence or
  link loss (every button released first). `vyoma.gamepad.rumble {strong, weak, ms}` (companion → phone) from the
  game's FF_RUMBLE effects, or `{error}` when the pad can't be made. SDL takes a trigger's first value as its rest.
- Touchpad over Bluetooth (2026-11-05, Settings > Steadier over Bluetooth, which also sends the controller over `…f803`; off by default, separate from the Bluetooth backup link's switch; old settings carry over if either was on; never on the cable): RFCOMM service `…f804` channel 29. Phone opens the radio, then `vyoma.pointer.request` → `vyoma.pointer.ready {key | error}` (base64 32 bytes; waits up to 60 s for Plasma's allow-control dialog; an error stops retries until the page reopens); `vyoma.pointer.close` when the page closes. 33-byte frames: "VYP1", seq u32, kind u8 (0 keep-awake every 100 ms, 1 move, 2 scroll, 3 press, 4 release), dx, dy f32 (button index in dx), HMAC-SHA256 of those cut to 16; every 32nd echoed (4 bytes). Companion pointer.rs: own RemoteDesktop session (pointer only, token `pointer-token`), same NotifyPointer* calls as kdeconnectd. Write stuck > 500 ms: back to the link.
- `vyoma.gamepad.udp {port, key}` (companion → phone, once the pad is up): a UDP port in 1750..1764 (inside the
  KDE Connect firewall range, UDP 1714..1764) and a base64 32-byte key. The phone (if it reaches the PC over a
  network, not the USB tunnel: `Device.datagramTo`, routed like payloads past a VPN) then sends each state as a
  51-byte datagram, big-endian: "VYG1", sid, seq, b (u32), lx ly rx ry (i16), lt rt (u8), mx my (i32), kind (u8:
  0 xbox, 1 dualsense), HMAC-SHA256 of those 35 bytes cut to 16. It repeats the latest every 100 ms (a lost one
  heals fast) and still sends the link state once a second. The companion logs "controller input arriving over
  UDP" on the first good datagram. Wired: usb.rs reverses phone tcp:1866 → companion 127.0.0.1:1766; the phone
  tries that first (and each second while on UDP) and writes the same 51-byte frames down the TCP stream; the first
  frame's key picks the controller. Measured with continuous motion: Wi-Fi (2.4 GHz, busy) 33 gaps >60 ms / 25 s,
  max 242 ms; after PC Wi-Fi power save off (user's NM connection, powersave=2) 5, max 242; cable 1, max 87. GamepadActivity follows the current companion link (a relink restarts the stream).
  Cable without USB debugging (Android Open Accessory, accessory.rs + app gamepad/UsbAccessoryLink.kt): if the
  1866 socket fails, the phone writes the same frames to an open "Vyoma"/"Companion" USB accessory; if it has none and
  USB_STATE says a computer is plugged in, it sends `vyoma.usb.accessory {open: bool}` (at most every 15 s). The
  companion switches every AOA-speaking non-hub device (GET_PROTOCOL 51, strings 52, START 53) to 18d1:2d00/2d01,
  retries open() up to 3 s (udev's ACL lags), claims interface 0 and pipes bulk-in reads to 127.0.0.1:1766.
  `open:false` while already an accessory = the app lost its permission (reinstall): reset and switch again so
  Android re-attaches it (dialog once, "Always" sticks). NEVER reset on controller close: on the S24 FE a reset while
  the app held the accessory wedged the phone's USB (error -71) until replugged; it stays an accessory until unplugged.
  Phone writes go through a newest-frame writer thread; a write pending > 500 ms fails over to Wi-Fi. Access without
  debugging: desktop/companion/udev/70-vyoma-usb.rules (sudo, once). Verified 2026-11-03: switch ~2 s, A/B/X/Y arrive.
- Phone mirroring + control from the PC (app mirror/, companion mirror.rs, 2026-11-04): phone → PC `vyoma.mirror
  {w, h}` + live payload of H.264 Annex-B (MediaProjection, whole screen only via createConfigForDefaultDisplay on
  34+; one VirtualDisplay AUTO_MIRROR per grant; long side <= 1920, multiples of 8, 60 fps, 8 Mbit/s; a turn makes a
  new encoder and so a new SPS in the same stream). Shared H264Encoder + companionLink() in app webcam/H264Encoder.kt.
  PC → phone `vyoma.mirror.state {open}` / `{error}` (open:false after open = the user closed the window: phone stops;
  the payload can break first, so the phone waits 1.5 s for it) and `vyoma.mirror.input {kind, x, y (0..1 of the
  picture), dx, dy, text, key}`: kinds down/move/up (left button), scroll, text, key (backspace/delete/enter/left/
  right), back (right click, Esc, mouse back), home (middle click), recents. Companion runs itself as a child
  (`--mirror-window`, GST_GL_WINDOW=x11 because GStreamer 1.28's Wayland windows have no keyboard): fdsrc ! h264parse
  ! vah264dec ! vapostproc ! caps <= 85% of the smallest screen ! glimagesink; navigation events caught by a probe on
  the capsfilter's src pad, JSON lines on stdout. X11 window quirks: keys come unshifted and without modifiers (track
  Shift/Ctrl/Alt/Caps ourselves, shift via libxkbcommon's level 1 in kxkbrc's layout), the wheel is buttons 4-7,
  closing is the error "Quit requested". Phone: MirrorAccessibility (user enables once) + MirrorControl: tap unless
  past touch slop or held 300 ms, then a live stroke of continueStroke pieces sent one after another; wheel = 8% of
  the height drag then 60 ms still (no fling); typing via ACTION_SET_TEXT + SET_SELECTION on the input focus, Enter
  via ACTION_IME_ENTER. LinkService adds FGS type mediaProjection (ACTION_MIRROR) before getMediaProjection.
  Verified by the user 2026-11-04 (typing with Shift, wheel, drag, close-to-stop).
- Mirror extras (2026-11-04): `vyoma.mirror.audio {rate, channels}` + live s16le stereo 48 kHz (AudioPlaybackCapture
  on the same projection: MEDIA/GAME/UNKNOWN; RECORD_AUDIO); PC plays with pacat behind a jitter buffer (10 ms
  pieces, target 4 judged by the window's low point over 2 s, max 30; a hard 4 KiB drop-when-full pipe crackled).
  While it plays the phone's media volume is 0 (restored after, saved in prefs "mirror"/restoreVolume) and the
  accessibility key filter turns volume keys into `vyoma.mirror.volume {up}` (wpctl 5%, limit 1.0). Video is held
  back by the default sink's PipeWire Latency (pw-dump, BT AAC ~190 ms) + 60 ms, but only while sound is audible
  (1.5 s) and no input came in 2 s. Keys: Ctrl+V/Shift+Insert = "paste" (wl-paste → text), key kinds carry
  ctrl/shift (word steps, selection, home/end), selectall/copy/cut; `vyoma.mirror.copy {text}` → wl-copy.
- PC screen touch control (2026-11-04): the cast is a RemoteDesktop portal session (keyboard+pointer, its own
  restore token remotedesktop-token; falls back to ScreenCast watch-only) and `vyoma.screen` says `control`.
  Touches: `vyoma.screen.touch` live payload of JSON lines (rides the cable; Wi-Fi link spikes 50-280 ms made drags
  stutter), else `vyoma.screen.input` packets: {kind click|right|down|move|up|scroll|scrollend, x, y 0..1, dx, dy px}
  → NotifyPointerMotionAbsolute/Button/Axis sent NoReplyExpected; runs of moves collapse to the newest.
  Phone: tap click, hold 450 ms right-click, drag = left drag, two fingers scroll at 1x (zoom/pan when zoomed),
  tap on the black bars or two-finger tap shows the buttons. Gamepad + viewer use requestUnbufferedDispatch.
- Phone as the PC's webcam/mic (app webcam/PhoneWebcam.kt, companion webcam.rs, 2026-11-04): phone → PC
  `vyoma.webcam {w, h, rotate}` + live payload (payloadSize -1) of H.264 Annex-B (MediaCodec surface encoder,
  30 fps, 4/8 Mbit/s, I-frame 2 s, SPS/PPS before every keyframe, repeat-previous 100 ms); `vyoma.webcam.mic
  {rate, channels}` + live s16le PCM (VOICE_COMMUNICATION, 48 kHz mono, 10 ms reads); `vyoma.webcam.turn {rotate}`
  (clockwise degrees: back (sensor+held)%360, front (sensor-held+360)%360, held from OrientationEventListener
  snapped with 30 deg give). PC → phone `vyoma.webcam.state {camera|mic: true}` or `{error}` (error = stop and show).
  Companion: v4l2loopback labelled "Vyoma Phone Camera" (desktop/companion/webcam/, sudo once, video_nr=20,
  exclusive_caps=1), opened once per stream with VIDIOC_S_FMT YUYV; `gst-launch-1.0 fdsrc ! h264parse ! vah264dec !
  vapostproc video-direction=.. add-borders=true ! YUY2 wxh ! fdsink` (no caps filter before h264parse: it failed
  "caps unfixed"); a turn restarts only gst. Mic: pactl module-pipe-source "Vyoma Phone Mic" through a FIFO,
  O_NONBLOCK + F_SETPIPE_SZ 4096 and 10 ms writes dropped when full (blocking queued 1.5 s of lag, measured);
  stale modules removed on start. Either stream silent 4 s = phone gone. Payloads ride the USB forward when the
  cable is in. Verified: 30 fps at 720p (front), audio at real-time rate, background with the screen off.
  Camera/mic switches stay inside the stream (one encoder + payload; the camera feeding the encoder is swapped,
  the mic payload comes and goes; size = one both cameras give): a restart made the PC device vanish and call apps
  dropped the camera (user). Measured: a front/back switch is a <1 s freeze, the reader never disconnects.
  Only a quality change restarts. The preview joins the capture session as a second target (rebuilt on
  attach/detach; detach stops repeating synchronously from surfaceDestroyed); zoom (CONTROL_ZOOM_RATIO, capped 8x)
  and torch only re-issue the repeating request.
 (tether.rs, polled every 5 s while a trusted phone is unlinked): USB tethering
  (rndis/cdc gateway or neighbour) and any Wi-Fi gateway, i.e. the PC on the phone's hotspot (verified 2026-11-03,
  during a power cut) and the cable (usb.rs `adb forward tcp:0 tcp:1716` per plugged phone, so the companion links
  with Wi-Fi off); all dialled with `connect_trusted` (only kdeconnectd-trusted certs), gateway refusals at debug.
- `vyoma.wallpaper.request` → `vyoma.wallpaper {cols, rows, colors:[[r,g,b], ...]}` (row-major, top-left first) or
  `{error}`. The companion reads Plasma's wallpaper (`Image=` under `[Wallpaper][org.kde.image][General]` in
  `plasma-org.kde.plasma.desktop-appletsrc`; a package resolves to its largest image; unset = Next) and has ffmpeg
  average it to 4x4. The phone asks on link and on plugin start and caches the last grid ("pc_wallpaper" in the ui
  prefs) so the background is right before the PC connects. The companion also pushes `vyoma.wallpaper` unasked
  when it changes (polls the wallpaper path, image mtime and appletsrc mtime every 3 s, sends 400 ms after a change).
- `vyoma.desktop.step {up}` → `vyoma.desktop.stepped {view: desktop|windows|overview|grid}` or `{error}`: the
  touchpad's vertical three-finger swipe on Plasma, one rung of desktop ↔ windows ↔ Overview ↔ Grid (desktop.rs).
  It reads KWin first (`/Effects activeEffects` has "overview"; `/KWin showingDesktop`), then invokes kglobalaccel
  shortcuts ("Overview" steps Grid → Overview → closed, "Grid View", "Show Desktop" toggles). KWin's
  `showDesktop(bool)` does nothing on Plasma 6 Wayland. Without the companion the phone keeps its own ladder guess.

## Local access rules (not on the wire)
- USB grants (`usb.rs`): READ_LOGS and the overlay appop are real power and any app can take our package name, so they go only to an APK whose signing certificate is in `~/.config/vyoma-companion/allowed-apps`; otherwise the PC asks once by notification (Allow / Not now). The digest comes from the APK's signing block (`dumpsys` only has a forgeable 32-bit hash), and every signer in every scheme must agree, because Android skips signers meant for other versions. Nothing is fetched or asked when the grants are already there. Tunnels need no such check: every link and payload proves its certificate.
- Shares (`ShareActivity`): `file:` and Vyoma's own providers are refused (`maySend`), as anything named there is read with Vyoma's access. A shared link waits for a yes, since it opens on the PC.
- Fingerprint lock (`AppLock`): unlocking the PC, PC files, its screen, screenshots, its camera and power actions, plus any command with its switch on in the Commands sheet (per command and PC; holds even with the main lock off, and turning one off always asks). One pass lasts until the phone's screen goes off or 5 min away from the app; PC files and the screen viewer close then. Turning it off always asks. On by default, off in the demo. Rules run unattended and aren't asked, but choosing a marked command for a rule asks once, in the editor, and so does Test now on a rule with a marked command, Sleep PC or Screenshot.
