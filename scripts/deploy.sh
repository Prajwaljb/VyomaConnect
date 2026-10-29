#!/usr/bin/env bash
# Build, install and launch on the connected phone, then tail our logs.
# Usage: scripts/deploy.sh [debug|release] [--no-logs]
set -euo pipefail
cd "$(dirname "$0")/.."
source scripts/env.sh
variant="${1:-debug}"
case "$variant" in
  debug)   task=":app:installDebug";   pkg="dev.vyoma.connect.debug" ;;
  release) task=":app:installRelease"; pkg="dev.vyoma.connect" ;;
  *) echo "unknown variant $variant" >&2; exit 1 ;;
esac
if ! adb get-state >/dev/null 2>&1; then
  echo "No device. Plug in USB (allow debugging) or: adb pair <ip:port> && adb connect <ip:port>" >&2
  exit 2
fi
./gradlew "$task" --console=plain -q
adb shell am start -n "$pkg/dev.vyoma.connect.ui.MainActivity" >/dev/null
echo "Launched $pkg"
if [[ "${2:-}" != "--no-logs" ]]; then
  pid=""
  for _ in $(seq 1 20); do pid="$(adb shell pidof "$pkg" || true)"; [[ -n "$pid" ]] && break; sleep 0.25; done
  adb logcat -v color,brief --pid="$pid" 'Vyoma/*:V' 'AndroidRuntime:E' '*:S'
fi
