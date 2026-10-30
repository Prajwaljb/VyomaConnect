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
  payload ports; `Device.openPayload` falls back to another link if one can't reach the sender. kdeconnectd keeps only its
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
- 30 s timeout. Verification key = first 8 hex chars (uppercase) of SHA-256(larger SPKI ‖ smaller SPKI ‖ decimal timestamp),
  compared unsigned. This is verified against kdeconnectd over D-Bus.
- Paired peer re-requests → we re-confirm with `pair:true`. Packets from unpaired peer → reply `pair:false` once.

## Plugins
`PluginSpec(id, incoming, outgoing, create)`. Loaded when paired+reachable and
`incoming ∩ peer.outgoing ≠ ∅ || outgoing ∩ peer.incoming ≠ ∅`. Our identity advertises the union of all specs.
`onPacket` runs on the device's single-threaded dispatcher: never block there; launch heavy work in `ctx.scope`.
Stock types live in `PacketTypes`. Check the upstream kdeconnect-kde / kdeconnect-android plugin source for exact
body fields before implementing one.

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
  path (symlinks resolved) must stay inside home. Dotfiles hidden, folders first, max 5000 entries.
- `vyoma.files.get {path}` → `vyoma.files.file {path, name}` + payload, or `{path, error}`. Phone saves to
  Downloads/Vyoma via `ShareSink.createFile`, tracks it in `Transfers`, and opens it.
- `vyoma.files.put {path(folder), name}` + payload (phone serves it; the companion connects as TLS client with its
  cert and pins the phone's) → `vyoma.files.saved {path, name, error?}`. Written as `.name.vyoma-part`, renamed when
  complete, never overwrites (`name (2).ext`). A refusal before connecting makes the phone stop waiting.

