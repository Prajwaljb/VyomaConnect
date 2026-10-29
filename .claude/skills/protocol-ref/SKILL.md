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
