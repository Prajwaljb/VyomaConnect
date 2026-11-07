# VyomaConnect

Android client for the KDE Connect protocol (Kotlin + Compose), plus a future Rust Linux companion.
Package `dev.vyoma.connect` (debug: `.debug`). minSdk 31, target/compile 37. FOSS only (F-Droid): no GMS, Firebase,
or proprietary SDKs. Roadmap and architecture: `docs/PLAN.md`.

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
install -Dm755 desktop/companion/target/release/vyoma-companion ~/.local/bin/ && systemctl --user restart vyoma-companion
                                 # the installed systemd user unit runs ~/.local/bin/vyoma-companion; logs: journalctl --user -u vyoma-companion
```

## Modules
- `core/protocol`: pure JVM (no Android). Packets, identity, crypto/TLS, LanBackend, Device (pairing, plugins), DeviceHub.
  Keep it Android-free so it stays testable against kdeconnectd on the JVM; platform hooks go through `LanBackend.Config`
  and interfaces like `CredentialStore`/`TrustStore`.
- `core/designsystem`: theme, motion, One UI-style components.
- `app`: Application/AppContainer (manual lazy DI), `LinkService` (single connectedDevice FGS), UI.
- `desktop/companion`: Rust daemon beside stock kdeconnectd. Dials only phones kdeconnectd trusts (mDNS), pairs with
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
