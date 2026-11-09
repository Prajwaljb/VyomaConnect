// Vyoma's GNOME Shell extension. GNOME gives apps no way to follow the clipboard, draw over the
// desktop or open the Overview, so Vyoma's PC app (vyoma-desktop) asks the shell for those here,
// on the session bus: dev.vyoma.Shell at /dev/vyoma/Shell (see desktop/companion/src/shell.rs).
// It answers only Vyoma's own programs, so other apps can't read the clipboard through it.
//
// SPDX-License-Identifier: GPL-3.0-or-later

import Cairo from 'cairo';
import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Meta from 'gi://Meta';
import Shell from 'gi://Shell';
import St from 'gi://St';

import * as Dialog from 'resource:///org/gnome/shell/ui/dialog.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as ModalDialog from 'resource:///org/gnome/shell/ui/modalDialog.js';
import * as ShellEntry from 'resource:///org/gnome/shell/ui/shellEntry.js';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';

const BUS = 'dev.vyoma.Shell';
const PATH = '/dev/vyoma/Shell';
const VERSION = 3;

const IFACE = `<node>
  <interface name="dev.vyoma.Shell1">
    <method name="GetClipboard"><arg type="s" direction="out"/></method>
    <method name="SetClipboard"><arg type="s" direction="in"/></method>
    <signal name="ClipboardChanged"/>
    <method name="Point"><arg type="d" direction="in"/><arg type="d" direction="in"/></method>
    <method name="HidePointer"/>
    <method name="Step"><arg type="b" direction="in"/><arg type="s" direction="out"/></method>
    <method name="SwitchWorkspace"><arg type="b" direction="in"/></method>
    <method name="Screenshot"><arg type="ay" direction="out"/></method>
    <method name="Screens"><arg type="a(ii)" direction="out"/></method>
    <method name="AskReply">
      <arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="s" direction="in"/>
      <arg type="s" direction="out"/>
    </method>
    <property name="Version" type="u" access="read"/>
  </interface>
</node>`;

// The programs allowed to call, by file name.
const CALLERS = ['vyoma-desktop', 'vyoma-companion'];
// Password managers (KeePassXC, KWallet) mark what they copy with this; it's never read.
const SECRET_HINT = 'x-kde-passwordManagerHint';
// The phone keeps pointing while it's held; this long without a move, it stopped.
const QUIET_MS = 1000;
// The dot's square, a 9 px core with a white rim in a soft glow, as presenter.rs draws it on Plasma.
const DOT = 48;
// Apple's system red, the colour a presenter's laser is expected to be.
const RED = [1.0, 0.231, 0.188];

export default class VyomaExtension extends Extension {
    enable() {
        this._clipboard = St.Clipboard.get_default();
        this._pointer = null;
        this._spot = null;
        this._quiet = 0;
        this._asking = new Set();

        this._object = Gio.DBusExportedObject.wrapJSObject(IFACE, this);
        this._object.export(Gio.DBus.session, PATH);
        this._name = Gio.bus_own_name_on_connection(Gio.DBus.session, BUS, Gio.BusNameOwnerFlags.NONE, null, null);

        // Each copy, in any app. The signal carries no text: any app can listen to it.
        this._selection = global.display.get_selection();
        this._ownerChanged = this._selection.connect('owner-changed', (_selection, type) => {
            if (type === Meta.SelectionType.SELECTION_CLIPBOARD)
                this._object.emit_signal('ClipboardChanged', null);
        });
    }

    disable() {
        this._selection.disconnect(this._ownerChanged);
        this._selection = null;
        Gio.bus_unown_name(this._name);
        this._object.unexport();
        this._object = null;
        this._hidePointer();
        for (const close of this._asking)
            close();
        this._clipboard = null;
    }

    get Version() {
        return VERSION;
    }

    // Who's calling, by the program behind the bus name.
    async _check(invocation) {
        const reply = await Gio.DBus.session.call(
            'org.freedesktop.DBus', '/org/freedesktop/DBus', 'org.freedesktop.DBus',
            'GetConnectionUnixProcessID', new GLib.Variant('(s)', [invocation.get_sender()]),
            new GLib.VariantType('(u)'), Gio.DBusCallFlags.NONE, 1000, null);
        const [pid] = reply.deepUnpack();
        // A program replaced on disk while running (an update) reads "<path> (deleted)".
        const exe = GLib.file_read_link(`/proc/${pid}/exe`).replace(/ \(deleted\)$/, '');
        if (!CALLERS.includes(GLib.path_get_basename(exe)))
            throw new Error('only Vyoma may call this');
    }

    // Runs [work] for Vyoma's calls, refusing everyone else's.
    async _answer(invocation, work) {
        try {
            await this._check(invocation);
        } catch (e) {
            invocation.return_dbus_error('org.freedesktop.DBus.Error.AccessDenied', `${e.message ?? e}`);
            return;
        }
        try {
            invocation.return_value(await work());
        } catch (e) {
            invocation.return_dbus_error('org.freedesktop.DBus.Error.Failed', `${e.message ?? e}`);
        }
    }

    GetClipboardAsync(_params, invocation) {
        this._answer(invocation, async () => {
            const types = this._clipboard.get_mimetypes(St.ClipboardType.CLIPBOARD);
            if (types.includes(SECRET_HINT))
                return new GLib.Variant('(s)', ['']);
            const text = await new Promise(resolve =>
                this._clipboard.get_text(St.ClipboardType.CLIPBOARD, (_c, t) => resolve(t ?? '')));
            return new GLib.Variant('(s)', [text]);
        });
    }

    SetClipboardAsync([text], invocation) {
        this._answer(invocation, async () => {
            this._clipboard.set_text(St.ClipboardType.CLIPBOARD, text);
            return null;
        });
    }

    // Moves the presenter's pointer by [dx, dy] (the screen is 2 across, dy scaled by its aspect,
    // as kdeconnectd does). The first move shows it in the middle.
    PointAsync([dx, dy], invocation) {
        this._answer(invocation, async () => {
            this._point(dx, dy);
            return null;
        });
    }

    HidePointerAsync(_params, invocation) {
        this._answer(invocation, async () => {
            this._hidePointer();
            return null;
        });
    }

    // One three-finger swipe, as GNOME's own: up goes windows → Overview → apps, down comes back.
    StepAsync([up], invocation) {
        this._answer(invocation, async () => new GLib.Variant('(s)', [this._step(up)]));
    }

    // A reply to a phone's notification, in GNOME's own dialog: GNOME's notifications have no
    // reply field. "" if cancelled.
    AskReplyAsync([heading, message, placeholder], invocation) {
        this._answer(invocation, async () => new GLib.Variant('(s)', [await this._askReply(heading, message, placeholder)]));
    }

    _askReply(heading, message, placeholder) {
        return new Promise(resolve => {
            const dialog = new ModalDialog.ModalDialog({destroyOnClose: true});
            const shown = message.length > 400 ? `${message.slice(0, 400)}…` : message;
            const content = new Dialog.MessageDialogContent({title: heading, description: shown});
            dialog.contentLayout.add_child(content);
            const entry = new St.Entry({
                style_class: 'prompt-dialog-password-entry',
                hint_text: placeholder,
                style: 'min-width: 20em;',
                can_focus: true,
                x_expand: true,
            });
            ShellEntry.addContextMenu(entry);
            dialog.contentLayout.add_child(entry);

            let done = false;
            const finish = text => {
                if (done)
                    return;
                done = true;
                this._asking.delete(cancel);
                dialog.close();
                resolve(text);
            };
            const cancel = () => finish('');
            this._asking.add(cancel);
            entry.clutter_text.connect('activate', () => finish(entry.get_text()));
            dialog.setButtons([
                {label: 'Cancel', action: cancel, key: Clutter.KEY_Escape},
                {label: 'Send', action: () => finish(entry.get_text()), default: true},
            ]);
            dialog.setInitialKeyFocus(entry.clutter_text);
            if (!dialog.open())
                finish('');
        });
    }

    // Three fingers sideways: the next workspace that way, as GNOME's own swipe and shortcut go.
    SwitchWorkspaceAsync([right], invocation) {
        this._answer(invocation, async () => {
            const current = global.workspace_manager.get_active_workspace();
            const next = current.get_neighbor(right ? Meta.MotionDirection.RIGHT : Meta.MotionDirection.LEFT);
            if (next !== current)
                Main.wm.actionMoveWorkspace(next);
            return null;
        });
    }

    // The whole desktop as PNG, for the phone's Screenshot: GNOME lets only its own tools capture
    // without asking, and the shell is one.
    ScreenshotAsync(_params, invocation) {
        this._answer(invocation, async () => {
            const stream = Gio.MemoryOutputStream.new_resizable();
            await new Promise((resolve, reject) => {
                new Shell.Screenshot().screenshot(false, stream, (shooter, result) => {
                    try {
                        shooter.screenshot_finish(result);
                        resolve();
                    } catch (e) {
                        reject(e);
                    }
                });
            });
            stream.close(null);
            return new GLib.Variant('(ay)', [stream.steal_as_bytes().toArray()]);
        });
    }

    // Each screen's size in logical pixels (scaled, turned), for sizing windows.
    ScreensAsync(_params, invocation) {
        this._answer(invocation, async () =>
            new GLib.Variant('(a(ii))', [Main.layoutManager.monitors.map(m => [m.width, m.height])]));
    }

    _step(up) {
        const overview = Main.overview;
        const apps = overview.dash.showAppsButton;
        const open = overview.visibleTarget;
        if (up) {
            if (!open) {
                overview.show();
                return 'overview';
            }
            apps.checked = true;
            return 'grid';
        }
        if (open && apps.checked) {
            apps.checked = false;
            return 'overview';
        }
        if (open)
            overview.hide();
        return 'windows';
    }

    _point(dx, dy) {
        if (!Number.isFinite(dx) || !Number.isFinite(dy))
            return;
        if (!this._pointer) {
            const monitor = Main.layoutManager.currentMonitor ?? Main.layoutManager.primaryMonitor;
            if (!monitor)
                return;
            this._monitor = monitor;
            this._spot = {x: 0, y: 0};
            this._pointer = new St.DrawingArea({width: DOT, height: DOT, reactive: false, can_focus: false});
            this._pointer.connect('repaint', paintDot);
            // Above everything, full-screen windows too.
            Main.layoutManager.uiGroup.add_child(this._pointer);
            Main.layoutManager.uiGroup.set_child_above_sibling(this._pointer, null);
        }
        const m = this._monitor;
        const aspect = m.height > 0 ? m.width / m.height : 1;
        const clamp = v => Math.min(1, Math.max(-1, v));
        this._spot = {x: clamp(this._spot.x + dx), y: clamp(this._spot.y + dy * aspect)};
        this._pointer.set_position(
            Math.round(m.x + (this._spot.x + 1) / 2 * (m.width - DOT)),
            Math.round(m.y + (this._spot.y + 1) / 2 * (m.height - DOT)));

        if (this._quiet)
            GLib.source_remove(this._quiet);
        this._quiet = GLib.timeout_add(GLib.PRIORITY_DEFAULT, QUIET_MS, () => {
            this._quiet = 0;
            this._hidePointer();
            return GLib.SOURCE_REMOVE;
        });
    }

    _hidePointer() {
        if (this._quiet) {
            GLib.source_remove(this._quiet);
            this._quiet = 0;
        }
        this._pointer?.destroy();
        this._pointer = null;
    }
}

function paintDot(area) {
    const cr = area.get_context();
    const [w] = area.get_surface_size();
    const unit = w / DOT;
    const c = w / 2;
    const [r, g, b] = RED;
    // The glow fades as the square of the distance past the rim, from 0.35 to nothing at the edge.
    const glow = new Cairo.RadialGradient(c, c, 6 * unit, c, c, c);
    for (let i = 0; i <= 8; i++) {
        const t = i / 8;
        glow.addColorStopRGBA(t, r, g, b, 0.35 * (1 - t) ** 2);
    }
    cr.setSource(glow);
    cr.arc(c, c, c, 0, 2 * Math.PI);
    cr.fill();
    cr.setSourceRGBA(1, 1, 1, 1);
    cr.arc(c, c, 6 * unit, 0, 2 * Math.PI);
    cr.fill();
    cr.setSourceRGBA(r, g, b, 1);
    cr.arc(c, c, 4.5 * unit, 0, 2 * Math.PI);
    cr.fill();
    cr.$dispose();
}
