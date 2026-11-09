<p align="center"><img src="desktop/daemon/dist/vyoma.svg" width="96" alt=""></p>

<h1 align="center">Vyoma</h1>

<p align="center">Your Android phone and your Linux PC, working as one.<br>
Open source, no account, no cloud. Speaks the KDE Connect protocol.</p>

<p align="center"><img src="docs/screenshots/desktop-phone.png" width="760" alt="Vyoma's PC window showing a connected phone: battery, connection, response time, and Send files, Phone files and Ring"></p>

Vyoma is two apps that work together: an app for your phone and an app for your PC. On the PC it
replaces KDE Connect and keeps your existing pairings. On the phone it also pairs with plain KDE
Connect or GSConnect, with fewer features.

## What it does

- **Files, both ways.** Send files and whole folders, browse your PC's files from the phone, and open
  the phone's storage in your PC's file manager. Transfers resume after a dropped connection.
- **Clipboard.** Copy on one side, paste on the other.
- **Notifications and calls** from the phone show up on the PC, with a Mute button for calls.
- **Your PC from the phone.** See and control its screen, take screenshots, take a photo with its
  camera, see its CPU, memory and temperature, change brightness and per-app volume, control
  music, lock it, or put it to sleep.
- **Your phone as a PC accessory.** A touchpad and keyboard, a presenter remote, a webcam and
  microphone, or a game controller with gyro aiming. Show the phone's screen in a window on the PC.
- **Commands and rules.** Run commands on the PC from the phone; sensitive ones can ask for your
  fingerprint first. Rules do things for you, like locking the PC when you walk away.
- **Widgets and quick settings tiles** for the things you do most.
- **Connects any way it can:** the same Wi-Fi, a USB cable (faster and steadier), Bluetooth when
  there's no Wi-Fi, or by IP address for Tailscale and other VPNs.

## Get it

Install the PC app first: it's what the phone pairs with.

### 1. On your PC

You need Linux with KDE Plasma (tested on Wayland). GNOME and others are planned.

```sh
git clone https://github.com/Prajwaljb/VyomaConnect && VyomaConnect/desktop/daemon/install.sh
```

This builds Vyoma (a few minutes the first time), installs it for your user with a menu entry,
starts it at login, and takes over from KDE Connect, keeping its pairings. It asks for your
password to install the packages it needs and to open the firewall for phones on Wi-Fi. Anything
missing later shows on **This PC** in Vyoma's window, with an Install button.

`install.sh --undo` gives the PC back to KDE Connect.

### 2. On your phone

Download `vyoma-<version>.apk` from [Releases](https://github.com/Prajwaljb/VyomaConnect/releases) and
open it. It needs Android 12 or newer.

To check the download is genuine, its signing certificate's SHA-256 fingerprint is:

```
7F:EB:FF:09:7C:EF:AA:F1:2C:CF:3A:C6:ED:2E:C8:EA:A0:C0:9A:7A:0A:77:83:8B:B8:2E:E5:3E:00:04:84:B1
```

(`apksigner verify --print-certs vyoma-<version>.apk` shows it.)

### 3. Pair

Open Vyoma on the phone and on the PC. Both show the same setup steps: pick your PC, then check
that the code on the phone matches the one on the PC. After that they find each other by
themselves.

<p align="center"><img src="docs/screenshots/desktop-setup.png" width="640" alt="The PC's setup: pairing, with the code to compare against the phone"></p>

## Good to know

**Already use KDE Connect or GSConnect?** The phone app pairs with them too. You get the basics
(files, clipboard, notifications, media, commands, touchpad and presenter); the rest needs Vyoma's
PC app.

**Clipboard while Vyoma is closed.** Android only lets the app on screen read the clipboard. To
send copies with Vyoma closed, plug the phone into the PC once with USB debugging on and click
**Allow** on the phone's page in Vyoma's window; you can turn USB debugging off afterwards. Android
then asks for log access each time Vyoma restarts (after an update or a phone restart): tap
**Allow one-time access**. Without this, copies sync while Vyoma is open, and the Clipboard page
has a Send button.

**Your data stays home.** Phones and PCs talk directly over your own network, cable or Bluetooth,
encrypted, with no account and no server. A phone works with a PC only after you compare codes on
both screens. In Vyoma's PC window you choose what each phone may do, and the phone can ask for
your fingerprint before it unlocks the PC, opens its files or shows its screen.

**Tested on** a Galaxy S24 FE (Android 17, One UI 9) with CachyOS and KDE Plasma 6 on Wayland. The
installer also supports Debian, Ubuntu and Fedora, but hasn't been tried there yet:
[an issue](https://github.com/Prajwaljb/VyomaConnect/issues) with what happened helps a lot.

<p align="center"><img src="docs/screenshots/desktop-commands-dark.png" width="640" alt="The PC's Commands page in dark mode, with commands that ask for a fingerprint"></p>

## Build it yourself

```sh
# Phone app (needs JDK 21 and the Android SDK)
./gradlew :app:assembleRelease

# PC app and its window (needs Rust)
(cd desktop/daemon && cargo build --release)
(cd desktop/gui && cargo build --release)
```

A demo build (`./gradlew :app:assembleDemo`) runs with a simulated PC, no network needed.

How it fits together is in [docs/PLAN.md](docs/PLAN.md).

## Licence

GPL-3.0-or-later, as KDE Connect is. Vyoma isn't affiliated with KDE; it speaks the KDE Connect
protocol so it works alongside it.
