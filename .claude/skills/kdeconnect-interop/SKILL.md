---
name: kdeconnect-interop
description: Verify VyomaConnect against the real KDE Connect daemon (kdeconnectd 26.x) on this Plasma machine via the JVM interop test, kdeconnect-cli and D-Bus. Use after touching anything in core/protocol, or to check pairing/plugin behaviour from the desktop side.
---

# Interop with the desktop KDE Connect

The user's PC runs KDE Connect (Plasma 6, Wayland). Their real phone ("Galaxy S24 FE") is already
paired with it via official KDE Connect; never unpair or modify that device.

## Automated JVM interop test (no phone needed)
```bash
source scripts/env.sh && VYOMA_INTEROP=1 ./gradlew :core:protocol:test --rerun --console=plain 2>&1 | grep -E 'PASSED|FAILED|[DIWE]/'
```
It runs our protocol stack as a fake phone ("Vyoma Interop Test", state in core/protocol/build/interop)
and checks discovery, both TLS roles, v8 identity, pairing both ways with matching verification
keys, ping, unpair. It always unpairs itself at the end.

Notes:
- kdeconnectd owns UDP 1716, so the test doesn't listen on UDP; it broadcasts to the LAN IP and uses TCP 1717+.
- kdeconnectd sends the peer device id as TLS SNI; the JDK server rejects '_' in hostnames, so the
  Gradle test task sets `jdk.tls.server.disableExtensions=server_name`. Android (Conscrypt) is fine.
  The Rust companion (rustls) must also tolerate this SNI.

## Desktop-side tools
```bash
kdeconnect-cli -l                     # devices + pair state
kdeconnect-cli --my-id                # desktop device id
kdeconnect-cli -d <id> --ping-msg hi  # send a ping to us
kdeconnect-cli -d <id> --share <file> # send a file
kdeconnect-cli -d <id> --send-clipboard
D=org.kde.kdeconnect; P=/modules/kdeconnect/devices/<id>
qdbus6 $D $P                          # list members
qdbus6 $D $P org.kde.kdeconnect.device.verificationKey
qdbus6 $D $P org.kde.kdeconnect.device.acceptPairing
qdbus6 $D $P/<plugin>                 # per-plugin objects, e.g. /mprisremote, /clipboard
journalctl --user --since -2min | grep kdeconnect   # daemon logs
```
For a phone-side check, combine with the `android-deploy` skill's logcat dump.
