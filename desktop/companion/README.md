# Vyoma companion

A small daemon for your Linux PC that runs beside KDE Connect and gives Vyoma on your phone what KDE
Connect can't: the PC's live screen, the phone as its webcam and game controller, its stats and
power, and files both ways. It only talks to phones KDE Connect already trusts, so there's nothing
new to pair.

## Install

```sh
git clone https://github.com/Prajwaljb/VyomaConnect && VyomaConnect/desktop/companion/install.sh
```

This builds it, installs it for your user, starts it at login, and installs the packages it needs
(pacman, apt or dnf). It asks for your password once.

If something goes missing later, the companion shows a notification with an Install button.
By hand: `vyoma-companion --check` lists what's missing, `vyoma-companion --setup` installs it, and
`vyoma-companion --setup --dry-run` prints what would run as root.

## Clipboard sync when the app is closed

Android only lets a computer grant this. Plug the phone in once with USB debugging on, allow it on
the phone, then click Allow on the PC's notification. You can turn USB debugging off afterwards.
