# Vyoma for your PC

Vyoma's app for Linux PCs, in place of KDE Connect. Your phone pairs with it as it would with KDE
Connect (a phone already paired with KDE Connect stays paired), and Vyoma on the phone gets
everything it can do with a PC: the PC's screen on the phone, the phone as the PC's webcam, game
controller and touchpad, the PC's stats and power, files both ways, clipboard, notifications,
calls, media and commands. Its window shows your phones and lets you choose what each may do here.

## Install

```sh
git clone https://github.com/Prajwaljb/VyomaConnect && VyomaConnect/desktop/daemon/install.sh
```

Install it before pairing: it's what Vyoma on your phone pairs with. When it's done, its window
opens on the same setup steps as the phone, so you pair from both screens at once.

This builds it, installs it for your user with a menu entry ("Vyoma"), starts it at login, and
takes over from KDE Connect, keeping its pairings. It asks for your password to install the
packages it needs and to open the firewall for phones on Wi-Fi. Anything missing later shows on
This PC in Vyoma's window, with an Install button.

`install.sh --undo` gives the PC back to KDE Connect.

## Clipboard sync when the phone app is closed

Android only lets a computer grant this. Plug the phone in once with USB debugging on, allow it on
the phone, then click Allow on the phone's page in Vyoma's window (or in its notification). You
can turn USB debugging off afterwards.
