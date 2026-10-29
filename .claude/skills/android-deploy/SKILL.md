---
name: android-deploy
description: Build, install, launch and read logs of the VyomaConnect Android app on the user's Galaxy S24 FE (or any adb device). Use whenever you need to run the app, verify a change on-device, read logcat, or take a screenshot.
---

# Deploying VyomaConnect to the phone

Toolchain is user-local (no sudo): JDK 21 at `~/.local/share/jdks/temurin-21`, SDK at `~/Android/Sdk`.
Always `source scripts/env.sh` first (sets JAVA_HOME, ANDROID_HOME, adds adb to PATH).

## Check a device is attached
```bash
source scripts/env.sh && adb devices -l
```
None? Ask the user to either plug in USB and allow debugging, or use wireless debugging:
Developer options → Wireless debugging → Pair device with pairing code, then
`! adb pair <ip>:<pair-port>` and `! adb connect <ip>:<port>` (the user runs these; the pairing code is theirs).

## Build + install + launch
```bash
scripts/deploy.sh debug --no-logs     # package dev.vyoma.connect.debug
scripts/deploy.sh release --no-logs   # R8-minified, dev.vyoma.connect
```
Both variants can be installed side by side.

## Logs (non-blocking)
Never run the follow-mode logcat in the foreground; dump instead:
```bash
adb logcat -d -v brief 'Vyoma/*:V' 'AndroidRuntime:E' '*:S' | tail -100
```
All protocol logs use the `Vyoma/<Tag>` tag (routed from `VLog`).

## Screenshot to verify UI
```bash
adb exec-out screencap -p > "$SCRATCH/screen.png"   # then Read the png
```
Use the session scratchpad directory, not the repo.

## Useful
- Grant runtime perms without UI: `adb shell pm grant <pkg> android.permission.POST_NOTIFICATIONS`
- Clipboard-sync grant (Phase 1 clipboard): `adb shell pm grant <pkg> android.permission.READ_LOGS` and
  `adb shell appops set <pkg> SYSTEM_ALERT_WINDOW allow`
- Force-stop: `adb shell am force-stop <pkg>`
