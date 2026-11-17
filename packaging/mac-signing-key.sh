#!/bin/sh
# One time: makes Vyoma's own Mac signing key (self-signed, no Apple account) and gives it to GitHub's
# macOS build as the secrets MAC_SIGN_P12 and MAC_SIGN_PASSWORD. Every Vyoma.app is then signed by the
# same key, so macOS keeps the Accessibility Allow (the touchpad and keyboard) across updates; ad-hoc
# signing is a new identity each build, which macOS treats as a different app.
# The key stays in ~/.local/share/vyoma-signing beside the Android one: back it up, a new key means
# every Mac allows Vyoma once more.
#   packaging/mac-signing-key.sh
set -eu
dir=$HOME/.local/share/vyoma-signing
p12=$dir/vyoma-mac.p12
umask 077
mkdir -p "$dir"
if [ ! -e "$p12" ]; then
	tmp=$(mktemp -d)
	trap 'rm -rf "$tmp"' EXIT
	openssl req -x509 -newkey rsa:2048 -nodes -days 7300 -subj "/CN=Vyoma Mac Signing" \
		-addext "keyUsage=critical,digitalSignature" -addext "extendedKeyUsage=critical,codeSigning" \
		-addext "basicConstraints=critical,CA:false" -keyout "$tmp/key.pem" -out "$tmp/cert.pem" 2>/dev/null
	openssl rand -hex 24 > "$p12.password"
	# -legacy: macOS's security tool reads only the older PKCS#12 ciphers.
	openssl pkcs12 -export -legacy -inkey "$tmp/key.pem" -in "$tmp/cert.pem" -name "Vyoma Mac Signing" \
		-passout "file:$p12.password" -out "$p12"
	echo "made $p12"
fi
base64 -w0 "$p12" | gh secret set MAC_SIGN_P12
gh secret set MAC_SIGN_PASSWORD < "$p12.password"
echo "GitHub's macOS build now signs Vyoma.app with it"
