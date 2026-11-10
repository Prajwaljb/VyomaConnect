# Vyoma for your PC

Vyoma's app for Linux PCs, in place of KDE Connect. Your phone pairs with it as it would with KDE
Connect (a phone already paired with KDE Connect stays paired), and Vyoma on the phone gets
everything it can do with a PC: the PC's screen on the phone, the phone as the PC's webcam, game
controller and touchpad, the PC's stats and power, files both ways, clipboard, notifications,
calls, media and commands. Its window shows your phones and lets you choose what each may do here.

## Install

Paste this into a terminal on the PC:

```sh
curl -fsSL https://github.com/Prajwaljb/VyomaConnect/releases/latest/download/install.sh | sh
```

No curl? `wget -qO- https://github.com/Prajwaljb/VyomaConnect/releases/latest/download/install.sh | sh`

It downloads the ready-made build for your PC, checks it, adds Vyoma to your app menu, starts it at
login and takes over from KDE Connect, keeping its pairings. Nothing is compiled. It asks for your
password once, for the libraries it needs and to let phones through the firewall. Anything missing
later shows on This PC in Vyoma's window, with an Install button.

Then open Vyoma's window: it shows a code. In Vyoma on your phone, tap **Scan code**, and they're
paired, for good. They find each other by themselves from then on.

Rather build it yourself? `git clone https://github.com/Prajwaljb/VyomaConnect && VyomaConnect/desktop/daemon/install.sh`
does the same from source. `install.sh --undo` gives the PC back to KDE Connect.

## Clipboard sync when the phone app is closed (optional)

Android lets only a computer grant this, once. Plug the phone in with USB debugging on, allow it on
the phone, then click Allow on the phone's page in Vyoma's window (or in its notification). Then
turn USB debugging off again: copies keep syncing. Nothing else in Vyoma needs USB debugging.
