# Usage: python3 scripts/gen_widget_previews.py app/src/main/res
# Writes app/src/main/res/layout/widget_preview_<widget>_<size>.xml for One UI's per-size picker
# slots (tiny 1x1, small 2x1, wideSmall 4x1, medium 2x2, large 4x2; canvas sizes from SESL).
import sys, os
RES = sys.argv[1]
A = 'xmlns:android="http://schemas.android.com/apk/res/android"'
SIZES = {'tiny': (52, 52), 'small': (125, 52), 'wide_small': (270, 52), 'medium': (125, 130), 'large': (270, 130)}
INK, SOFT = '@color/widget_ink', '@color/widget_ink_soft'

def root(size, body, orient='horizontal', pad=12, bg='@drawable/widget_preview_bg', gravity='center_vertical'):
    w, h = SIZES[size]
    return ('<?xml version="1.0" encoding="utf-8"?>\n'
            '<!-- Generated widget-picker preview (One UI %s slot). Static sample content. -->\n'
            '<LinearLayout %s\n    android:id="@android:id/background"\n'
            '    android:layout_width="%ddp" android:layout_height="%ddp"\n'
            '    android:background="%s" android:orientation="%s" android:gravity="%s" android:padding="%ddp">\n%s</LinearLayout>\n'
            ) % (size, A, w, h, bg, orient, gravity, pad, body)

def btn(icon, size, accent=False, tint=None):
    bg = '@drawable/widget_preview_accent_button' if accent else '@drawable/widget_preview_button'
    tint = tint or ('@android:color/white' if accent else INK)
    return ('<ImageView android:layout_width="%ddp" android:layout_height="%ddp" android:background="%s" '
            'android:padding="%ddp" android:src="@drawable/%s" android:tint="%s" />\n') % (size, size, bg, round(size * 0.26), icon, tint)

def glass_circle(size):
    return ('<ImageView android:layout_width="%ddp" android:layout_height="%ddp" android:background="@drawable/widget_preview_glass_circle" '
            'android:padding="%ddp" android:src="@drawable/ic_w_upload" android:tint="@android:color/white" />\n') % (size, size, round(size * 0.26))

def text(t, sp, color=INK, bold=False, extra=''):
    return ('<TextView android:layout_width="wrap_content" android:layout_height="wrap_content" android:text="%s" '
            'android:textColor="%s" android:textSize="%ssp"%s android:maxLines="1" android:ellipsize="end"%s />\n'
            ) % (t, color, sp, ' android:textStyle="bold"' if bold else '', extra)

def col(body, weight=True, gravity=None, extra=''):
    w = 'android:layout_width="0dp" android:layout_weight="1"' if weight else 'android:layout_width="wrap_content"'
    g = ' android:gravity="%s"' % gravity if gravity else ''
    return '<LinearLayout %s android:layout_height="wrap_content" android:orientation="vertical"%s%s>\n%s</LinearLayout>\n' % (w, g, extra, body)

def row(body, gravity='center_vertical', extra=''):
    return '<LinearLayout android:layout_width="match_parent" android:layout_height="wrap_content" android:gravity="%s"%s>\n%s</LinearLayout>\n' % (gravity, extra, body)

def gap(w=0, h=0):
    return '<FrameLayout android:layout_width="%ddp" android:layout_height="%ddp" />\n' % (max(w, 1) if w else 1, h or 1)

FLEX_H = '<FrameLayout android:layout_width="0dp" android:layout_height="1dp" android:layout_weight="1" />\n'
FLEX_V = '<FrameLayout android:layout_width="1dp" android:layout_height="0dp" android:layout_weight="1" />\n'

def ring(label, size, sp):
    return ('<TextView android:layout_width="%ddp" android:layout_height="%ddp" android:gravity="center" android:background="@drawable/widget_preview_ring" '
            'android:text="%s" android:textColor="%s" android:textSize="%ssp" android:textStyle="bold" />\n') % (size, size, label, INK, sp)

def hairline():
    return '<FrameLayout android:layout_width="match_parent" android:layout_height="1dp" android:background="@color/widget_button" />\n'

def clip(t, first):
    return ('<TextView android:layout_width="match_parent" android:layout_height="wrap_content" android:paddingVertical="6dp" android:text="%s" '
            'android:textColor="%s" android:textSize="11sp" android:maxLines="1" android:ellipsize="end" />\n') % (t, INK if first else SOFT)

def spaced(items):
    out = FLEX_H
    for i in items: out += i + FLEX_H
    return out

ONLINE = text('● Online', 10, '@color/widget_online')
L = {}
# ---- status
L['status', 'tiny'] = root('tiny', ring('82', 40, 12), pad=6, gravity='center')
L['status', 'small'] = root('small', ring('82', 36, 11) + gap(8) + col(text('My PC', 13, bold=True) + ONLINE), pad=8)
L['status', 'wide_small'] = root('wide_small', ring('82', 36, 11) + gap(8) + col(text('My PC', 13, bold=True) + ONLINE)
    + '<ImageView android:layout_width="14dp" android:layout_height="14dp" android:src="@drawable/ic_w_phone" android:tint="%s" />\n' % INK
    + gap(3) + text('64%', 12, bold=True) + gap(10) + btn('ic_pause', 34, accent=True), pad=8)
L['status', 'medium'] = root('medium', text('My PC', 14, bold=True) + ONLINE + FLEX_V
    + row(ring('82', 52, 15), gravity='center') + row(text('PC', 10, SOFT), gravity='center', extra=' android:layout_marginTop="2dp"') + FLEX_V,
    orient='vertical', gravity='top')
L['status', 'large'] = root('large', row(col(text('My PC', 15, bold=True) + ONLINE)
    + col(ring('82', 40, 12) + text('PC', 9, SOFT, extra=' android:layout_marginTop="2dp"'), weight=False, gravity='center_horizontal')
    + gap(10) + col(ring('64', 40, 12) + text('Phone', 9, SOFT, extra=' android:layout_marginTop="2dp"'), weight=False, gravity='center_horizontal'))
    + FLEX_V + row(spaced([btn(i, 34) for i in ('ic_w_clipboard', 'ic_w_upload', 'ic_w_mouse', 'ic_w_ring')] + [btn('ic_play', 34, accent=True)])[len(FLEX_H):-len(FLEX_H)]),
    orient='vertical', gravity='top')
# ---- actions
L['actions', 'small'] = root('small', spaced([btn('ic_w_clipboard', 34), btn('ic_w_upload', 34), btn('ic_play', 34, accent=True)]), pad=6)
L['actions', 'wide_small'] = root('wide_small', spaced([btn(i, 36) for i in ('ic_w_clipboard', 'ic_w_upload', 'ic_w_mouse', 'ic_w_ring')] + [btn('ic_play', 36, accent=True)]), pad=8)
L['actions', 'medium'] = root('medium', FLEX_V + row(spaced([btn('ic_w_clipboard', 42), btn('ic_w_upload', 42)])) + FLEX_V
    + row(spaced([btn('ic_w_mouse', 42), btn('ic_play', 42, accent=True)])) + FLEX_V, orient='vertical', pad=10)
def labelled(icon, label, accent=False):
    return col(btn(icon, 42, accent) + text(label, 10, SOFT, extra=' android:layout_marginTop="4dp"'), gravity='center_horizontal')
L['actions', 'large'] = root('large', row(labelled('ic_w_clipboard', 'Clipboard') + labelled('ic_w_upload', 'Files')
    + labelled('ic_w_mouse', 'Touchpad') + labelled('ic_play', 'Play', True)), orient='vertical', gravity='center_vertical')
# ---- media
ART = lambda s: ('<ImageView android:layout_width="%ddp" android:layout_height="%ddp" android:background="@drawable/widget_preview_art" '
                 'android:padding="%ddp" android:src="@drawable/ic_w_music" android:tint="@android:color/white" />\n') % (s, s, round(s * 0.28))
SONG = lambda sp: text('Song title', sp, bold=True)
ARTIST = lambda sp: text('Artist', sp, SOFT)
L['media', 'small'] = root('small', gap(4) + col(SONG(12) + ARTIST(10)) + gap(4) + btn('ic_pause', 34, accent=True), pad=8)
L['media', 'wide_small'] = root('wide_small', ART(36) + gap(10) + col(SONG(13) + ARTIST(10))
    + btn('ic_skip_previous', 30) + gap(4) + btn('ic_pause', 36, accent=True) + gap(4) + btn('ic_skip_next', 30), pad=8)
L['media', 'medium'] = root('medium', ART(44) + gap(0, 6) + SONG(12) + ARTIST(10) + FLEX_V
    + row(btn('ic_skip_previous', 30) + gap(4) + btn('ic_pause', 34, accent=True) + gap(4) + btn('ic_skip_next', 30), gravity='center'),
    orient='vertical', gravity='top', pad=10)
L['media', 'large'] = root('large', row(ART(56) + gap(12) + col(SONG(14) + text('Artist · on My PC', 11, SOFT))) + FLEX_V
    + row(btn('ic_skip_previous', 34) + gap(10) + btn('ic_pause', 42, accent=True) + gap(10) + btn('ic_skip_next', 34), gravity='center'),
    orient='vertical', gravity='top')
# ---- clipboard
L['clipboard', 'tiny'] = root('tiny', btn('ic_w_send', 40, accent=True), pad=6, gravity='center')
L['clipboard', 'small'] = root('small', col(text('Send', 12, bold=True) + text('clipboard', 10, SOFT)) + btn('ic_w_send', 34, accent=True), pad=8)
L['clipboard', 'wide_small'] = root('wide_small', col(text('Clipboard', 10, SOFT) + text('https://kde.org/connect', 12, bold=True))
    + gap(8) + btn('ic_w_send', 36, accent=True), pad=8)
HEAD = lambda sp, b: row(col(text('Clipboard', sp, bold=True)) + btn('ic_w_send', b, accent=True))
L['clipboard', 'medium'] = root('medium', HEAD(13, 28) + gap(0, 2) + clip('https://kde.org', True) + hairline() + clip('Meeting at 4:30', False),
    orient='vertical', gravity='top', pad=10)
L['clipboard', 'large'] = root('large', HEAD(14, 30) + gap(0, 2) + clip('https://kde.org/connect', True) + hairline() + clip('Meeting at 4:30', False),
    orient='vertical', gravity='top')
# ---- drop
ACC = '@drawable/widget_accent'
L['drop', 'tiny'] = root('tiny', glass_circle(36), pad=8, bg=ACC, gravity='center')
L['drop', 'small'] = root('small', glass_circle(36) + gap(10) + col(text('Send', 12, '@android:color/white', True) + text('files', 10, '#B3FFFFFF')), pad=8, bg=ACC)
L['drop', 'medium'] = root('medium', glass_circle(46) + text('Send files', 11, '@android:color/white', True, ' android:layout_marginTop="8dp"'),
    orient='vertical', gravity='center', bg=ACC)

for (w, s), xml in L.items():
    open(os.path.join(RES, 'layout', 'widget_preview_%s_%s.xml' % (w, s)), 'w').write(xml)
print(len(L))
