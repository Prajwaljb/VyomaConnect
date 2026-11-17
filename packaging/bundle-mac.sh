#!/bin/sh
# Vyoma.app for macOS: the window (vyoma, what opens) and the background part (vyoma-mac, which
# the window sets up as a login agent on first open) in one app, with Vyoma's icon and name.
# No Developer ID (signed with Vyoma's own key, else ad-hoc), so the first open needs macOS's Allow in
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
	<key>NSCameraUsageDescription</key>
	<string>Vyoma takes a photo with this Mac's camera when you ask from your phone, and tells you here each time.</string>
	<key>NSLocalNetworkUsageDescription</key>
	<string>Vyoma finds your phone and talks to it over your Wi-Fi.</string>
	<key>NSHumanReadableCopyright</key>
	<string>Free software under the GPL, version 3 or later.</string>
</dict>
</plist>
EOF

if command -v plutil >/dev/null; then plutil -lint "$app/Contents/Info.plist"; fi
# No Developer ID. VYOMA_SIGN_IDENTITY (Vyoma's own self-signed key, packaging/mac-signing-key.sh) keeps
# the same identity every build, so macOS keeps the Accessibility Allow across updates; without it,
# ad-hoc (Apple silicon runs nothing unsigned), a new identity each build.
if command -v codesign >/dev/null; then
	sign() {
		codesign --force --sign "$1" --identifier dev.vyoma.Vyoma.agent "$app/Contents/MacOS/vyoma-mac" &&
			codesign --force --sign "$1" "$app"
	}
	identity=${VYOMA_SIGN_IDENTITY:--}
	if ! sign "$identity"; then
		echo "warning: couldn't sign as $identity, signing ad-hoc" >&2
		sign -
	fi
	codesign --verify --strict --verbose=2 "$app"
	codesign --display --requirements - "$app/Contents/MacOS/vyoma-mac" 2>&1 | tail -1
fi
echo "$app"
