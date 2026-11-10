<p align="center"><img src="desktop/daemon/dist/vyoma.svg" width="140" alt="Vyoma"></p>

<h1 align="center">Vyoma</h1>

<p align="center"><b>Your Android phone and your Linux PC, working as one.</b><br>
Set it up once. After that it just works: no fiddling, no developer switches left on.<br>
Open source. No account. No cloud.</p>

<p align="center"><a href="https://github.com/Prajwaljb/VyomaConnect/releases/latest"><b>Download</b></a> ·
<a href="https://prajwaljb.github.io/VyomaConnect/">Website</a> ·
<a href="#get-started">Get started</a> ·
<a href="docs/FEATURES.md">All features</a> ·
<a href="https://github.com/Prajwaljb/VyomaConnect/issues">Report an issue</a></p>

<p align="center"><img src="docs/screenshots/desktop-phone.png" width="760" alt="Vyoma's PC window showing a connected phone: battery, connection, response time, and Send files, Phone files and Ring"></p>

Vyoma is a phone app and a PC app made as one product. Scan a code once and they belong together:
from then on they find each other, pick the best way to connect and keep going, while your PC stays
yours, behind your fingerprint.

## Why Vyoma

**Set up once, then forget about it.** Pairing is one scan of the code in the PC's window: nothing
to type, nothing to compare. After that the phone and PC connect by themselves, every time. Wi-Fi
at home, the USB cable the moment you plug it in, Bluetooth when there's no Wi-Fi: Vyoma picks the
best route and switches on its own. The PC app updates itself with one click and keeps your
pairings. And if they ever can't reach each other, the phone says why in plain words (not on
Wi-Fi, a VPN in the way, the PC's firewall, Vyoma not running there) with the button that fixes it.

**No developer switches left on.** Vyoma doesn't need root or Shizuku, and doesn't ask you to keep
USB debugging on. Everything works over Wi-Fi without it, the cable works through USB tethering,
and the game controller works over USB as a plain accessory. The one thing Android lets only a
computer unlock, reading the clipboard while Vyoma is closed, is a one-time grant from your PC:
plug in once, click Allow, and switch USB debugging off again. Copies keep syncing.

**Your fingerprint guards your PC.** An unlocked phone in someone else's hand isn't a way into
your PC. With the fingerprint lock on, unlocking the PC, opening its files, seeing its screen,
using its camera, its power buttons, the touchpad, keyboard and presenter, and your clipboard
history all ask for your fingerprint first, once per use, not at every tap. Commands that matter
can ask every time, and that's set on the PC, so it can't be switched off from the phone.

**Secure without thinking about it.** Phone and PC talk directly and encrypted, and each remembers
the other's certificate from the moment they pair. The pairing code works once, for one phone, for
ten minutes, and names the PC's certificate, so a phone can't be tricked into pairing with a
different PC. The PC decides what each phone may do and keeps a log of what it did. Copies your
password manager marks as private never leave the phone, and the PC always tells you when its
camera was used.

**Made with care.** One design across phone and PC, in your wallpaper's colours, light or dark,
with motion and haptics that answer every touch. Widgets, Quick Settings tiles and lock-screen
controls for what you use most. On the PC it fits into the desktop you already have: right-click
any file and choose **Send to** your phone, find the phone in your file manager's sidebar, and see
its calls and notifications in your desktop's own pop-ups.

**Private by design.** No account, no server, no tracking. The only thing that goes online is the
PC app's daily look for a new version on GitHub, which you can turn off.

## What you get

<p align="center">
<img src="docs/screenshots/phone-welcome.png" width="240" alt="Vyoma's welcome screen on the phone">
<img src="docs/screenshots/phone-home.png" width="240" alt="The phone's home screen: the PC's battery and stats, its music, volume and display brightness">
<img src="docs/screenshots/phone-setup.png" width="240" alt="The phone's setup steps">
</p>

**Files, both ways.** Send files and whole folders in either direction: on the PC, right-click and
pick **Send to** your phone. Browse your PC from the phone, and open the phone's storage right in
your PC's file manager. A dropped connection picks up
where it left off.

**One clipboard.** Copy on one, paste on the other.

**Calls and notifications on the PC.** See who's calling and mute the ringer from your desk.

**Your PC in your pocket.** View and control its screen, grab a screenshot, snap a photo with its
camera, watch CPU, memory and temperature, set brightness and per-app volume, control music, lock
it or put it to sleep.

**Your phone as a PC accessory.** Touchpad and keyboard, presenter remote, webcam and microphone,
or a game controller with gyro aiming. Put the phone's screen in a window on the PC.

**Commands and rules.** Run your own commands on the PC from the phone, with a fingerprint check
for the sensitive ones. Rules handle the routine, like locking the PC when you walk away.

**Widgets and quick settings tiles** for everything you use most.

**Always connected.** Wi-Fi, a USB cable for top speed, Bluetooth when there's no Wi-Fi, or an IP
address for Tailscale and other VPNs. Vyoma picks the best route on its own.

**And a lot more:** see [everything Vyoma does](docs/FEATURES.md).

## Get started

Set up the PC first, then the phone.

### 1. Your PC

Vyoma runs on Linux with KDE Plasma or GNOME. Support for other desktops and macOS is coming in
future releases.

Paste this into a terminal:

```sh
curl -fsSL https://github.com/Prajwaljb/VyomaConnect/releases/latest/download/install.sh | sh
```

No curl (Ubuntu's desktop has wget instead)?

```sh
wget -qO- https://github.com/Prajwaljb/VyomaConnect/releases/latest/download/install.sh | sh
```

It downloads the ready-made build for your PC (x86_64 or ARM64), checks it against the release's
checksums, adds Vyoma to your app menu, starts it at login and takes over from KDE Connect with your
pairings intact. Nothing is compiled. It asks for your password once, to install the libraries it
needs and let phones through the firewall. Prefer KDE Connect after all? Run the same line with
`sh -s -- --undo` in place of the last `sh` to put it back.

On GNOME, the installer also adds Vyoma's small GNOME Shell extension, which clipboard sync, the
presenter's pointer, screenshots, the touchpad's swipes and replying to notifications need. Log out
and back in once to turn it on; Vyoma's window shows whether it's on under This PC. GSConnect is
switched off while Vyoma runs, as both use the same port. Locking the PC from the phone needs GNOME's
own login screen (GDM).

When a new version is out, Vyoma tells you and updates itself with one click, your pairings kept
(or run the install line again).

**Rather build it from source?** `git clone https://github.com/Prajwaljb/VyomaConnect && VyomaConnect/desktop/daemon/install.sh`
does the same from a clone, building it first (Arch, Debian, Ubuntu and Fedora).

**On Arch or CachyOS?** Build it as a package instead, so pacman keeps track of it:

```sh
git clone https://github.com/Prajwaljb/VyomaConnect && cd VyomaConnect/packaging/aur && makepkg -si
/usr/share/vyoma/install.sh
```

### 2. Your phone

Download the APK from [Releases](https://github.com/Prajwaljb/VyomaConnect/releases/latest) and
open it. Android 12 or newer.

To get every update automatically, add this repository to
[Obtainium](https://github.com/ImranR98/Obtainium):
`https://github.com/Prajwaljb/VyomaConnect`.

The signing certificate's SHA-256 fingerprint, for anyone who checks
(`apksigner verify --print-certs`):

```
7F:EB:FF:09:7C:EF:AA:F1:2C:CF:3A:C6:ED:2E:C8:EA:A0:C0:9A:7A:0A:77:83:8B:B8:2E:E5:3E:00:04:84:B1
```

### 3. Pair

Open Vyoma on both. The PC's window shows a QR code: in the phone app, tap **Scan code** and point
the camera at it. That's it, with nothing to compare or accept, and it works even on Wi-Fi that
hides devices from each other. (Pointed at the code, a phone without Vyoma opens the download.)
You can also pick your PC from the list and confirm a code matches on both screens. From then on
they find each other automatically.

<p align="center"><img src="docs/screenshots/desktop-setup.png" width="640" alt="The PC's setup: the code to scan with Vyoma on the phone"></p>

## Good to know

**Works with KDE Connect and GSConnect too.** The phone app pairs with either one and covers the
essentials: files, clipboard, notifications, media, commands, touchpad and presenter. Vyoma's PC
app unlocks everything else.

**Clipboard with Vyoma closed (optional).** Android lets only the app on screen read the
clipboard, and only a computer can lift that. So it's a one-time step: plug the phone into the PC
with USB debugging on and click **Allow** on the phone's page in Vyoma's window. Then turn USB
debugging off again; copies keep reaching the PC with Vyoma closed. After Vyoma restarts or
updates, Android asks once for log access: tap **Allow one-time access**. Without this step,
clipboard sync works whenever Vyoma is open, and the Clipboard page has a Send button.

**Controlling the phone from the PC** (its screen in a window on the PC, clicked and typed into)
uses Android's accessibility service, asked for only when you first use it. Nothing else needs it.

<p align="center"><img src="docs/screenshots/desktop-commands-dark.png" width="640" alt="The PC's Commands page in dark mode, with commands that ask for a fingerprint"></p>

## Something not right?

[Open an issue](https://github.com/Prajwaljb/VyomaConnect/issues) and say what you did, what
happened, and which phone, distro and desktop you're on. Vyoma is built on KDE Plasma 6 and GNOME
with Wayland; the installer also supports Debian, Ubuntu and Fedora, and reports from those are
especially welcome.

## Build it yourself

```sh
# Phone app (JDK 21 and the Android SDK)
./gradlew :app:assembleRelease

# PC app and its window (Rust)
(cd desktop/daemon && cargo build --release)
(cd desktop/gui && cargo build --release)
```

`./gradlew :app:assembleDemo` builds a demo with a simulated PC, no network needed. The
architecture is in [docs/PLAN.md](docs/PLAN.md).

## Licence

GPL-3.0-or-later, like KDE Connect. Vyoma is an independent project, not affiliated with KDE.
