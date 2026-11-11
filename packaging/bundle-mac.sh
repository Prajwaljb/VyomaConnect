#!/bin/sh
# Vyoma.app for macOS: the window (vyoma, what opens) and the background part (vyoma-mac, which
# the window sets up as a login agent on first open) in one app, with Vyoma's icon and name.
# Unsigned for now (ad-hoc signed where codesign exists), so the first open needs macOS's Allow in
# System Settings > Privacy & Security.
#   packaging/bundle-mac.sh <vyoma binary> <vyoma-mac binary> <out dir>
set -eu
[ $# -eq 3 ] || { echo "usage: $0 <vyoma> <vyoma-mac> <out dir>" >&2; exit 2; }
window=$1 daemon=$2 out=$3
root=$(cd "$(dirname "$0")/.." && pwd)
version=$(tr -d '[:space:]' < "$root/VERSION")
app="$out/Vyoma.app"

rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$window" "$app/Contents/MacOS/vyoma"
cp "$daemon" "$app/Contents/MacOS/vyoma-mac"
chmod 755 "$app/Contents/MacOS/vyoma" "$app/Contents/MacOS/vyoma-mac"
cp "$root/desktop/mac/dist/vyoma.icns" "$app/Contents/Resources/vyoma.icns"
cp "$root/LICENSE" "$app/Contents/Resources/LICENSE"
cat > "$app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleName</key>
	<string>Vyoma</string>
	<key>CFBundleDisplayName</key>
	<string>Vyoma</string>
	<key>CFBundleIdentifier</key>
	<string>dev.vyoma.Vyoma</string>
	<key>CFBundleExecutable</key>
	<string>vyoma</string>
	<key>CFBundleIconFile</key>
	<string>vyoma</string>
	<key>CFBundlePackageType</key>
	<string>APPL</string>
	<key>CFBundleShortVersionString</key>
	<string>$version</string>
	<key>CFBundleVersion</key>
	<string>$version</string>
	<key>LSMinimumSystemVersion</key>
	<string>11.0</string>
	<key>LSApplicationCategoryType</key>
	<string>public.app-category.utilities</string>
	<key>NSHighResolutionCapable</key>
	<true/>
	<key>NSAppleEventsUsageDescription</key>
	<string>Vyoma plays, pauses and skips in Music and Spotify, and restarts, shuts down or logs out this Mac, when you ask from your phone.</string>
	<key>NSLocalNetworkUsageDescription</key>
	<string>Vyoma finds your phone and talks to it over your Wi-Fi.</string>
	<key>NSHumanReadableCopyright</key>
	<string>Free software under the GPL, version 3 or later.</string>
</dict>
</plist>
EOF

if command -v plutil >/dev/null; then plutil -lint "$app/Contents/Info.plist"; fi
# Ad-hoc: no Developer ID, but the app is sealed as one (Apple silicon runs nothing unsigned).
if command -v codesign >/dev/null; then
	codesign --force --sign - "$app/Contents/MacOS/vyoma-mac"
	codesign --force --sign - "$app"
	codesign --verify --strict --verbose=2 "$app"
fi
echo "$app"
