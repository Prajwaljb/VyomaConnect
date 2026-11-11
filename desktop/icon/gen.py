# Vyoma's "Moon" icon (draft B, sky tile) for every place it shows: Android adaptive layers and
# notification glyph, the desktop's hicolor SVG. One geometry, in a 100-unit tile.
import math, sys
R, W, BIG, SMALL, ANGLE, GAP = 30, 6, 18, 7.2, -42, 3.4
ROOT = sys.argv[1]
half = math.degrees(2 * math.asin((SMALL + GAP) / (2 * R)))
moon = (50 + R * math.cos(math.radians(ANGLE)), 50 + R * math.sin(math.radians(ANGLE)))
f = lambda v: f"{v:.3f}".rstrip("0").rstrip(".")

def geo(s, o):
    P = lambda x, y: (o + x * s, o + y * s)
    a1, a2 = ANGLE + half, ANGLE - half + 360
    p1 = P(50 + R * math.cos(math.radians(a1)), 50 + R * math.sin(math.radians(a1)))
    p2 = P(50 + R * math.cos(math.radians(a2)), 50 + R * math.sin(math.radians(a2)))
    ring = f"M{f(p1[0])},{f(p1[1])} A{f(R*s)},{f(R*s)} 0 1 1 {f(p2[0])},{f(p2[1])}"
    return dict(ring=ring, w=W * s, big=(*P(50, 50), BIG * s), small=(*P(*moon), SMALL * s), s=s)

def circle(x, y, r):
    return f"M{f(x-r)},{f(y)} a{f(r)},{f(r)} 0 1 0 {f(2*r)},0 a{f(r)},{f(r)} 0 1 0 {f(-2*r)},0 Z"

# ---- Android vectors ---------------------------------------------------------------------------
A = 'xmlns:android="http://schemas.android.com/apk/res/android"'
AAPT = 'xmlns:aapt="http://schemas.android.com/aapt"'

def radial(cx, cy, r, stops):
    items = "".join(f'<item android:offset="{o}" android:color="{c}"/>' for o, c in stops)
    return (f'<aapt:attr name="android:fillColor"><gradient android:type="radial" android:centerX="{f(cx)}" '
            f'android:centerY="{f(cy)}" android:gradientRadius="{f(r)}">{items}</gradient></aapt:attr>')

def orb_vd(x, y, r, s, big):
    out = []
    # A soft shadow under it (vectors have no blur).
    sr = r + 2.4 * s
    out.append(f'<path android:pathData="{circle(x, y + 1.6 * s, sr)}">{radial(x, y + 1.6*s, sr, [(0, "#590A3F8F"), (f(r/sr*0.9), "#590A3F8F"), (1, "#000A3F8F")])}</path>')
    stops = [(0, "#FFFFFFFF"), (0.55, "#FFEEF8FF"), (1, "#FFB9DEFC")] if big else [(0, "#FFFFFFFF"), (1, "#FFDFF1FF")]
    out.append(f'<path android:pathData="{circle(x, y, r)}">{radial(x - 0.14*r, y - 0.4*r, 1.6*r, stops)}</path>')
    out.append(f'<path android:pathData="{circle(x, y, r - 0.5*s)}" android:fillColor="#00000000" android:strokeColor="#E6FFFFFF" android:strokeWidth="{f(s)}"/>')
    if big:
        hx, hy = x - 0.32 * r, y - 0.42 * r
        out.append(f'<group android:rotation="-28" android:pivotX="{f(hx)}" android:pivotY="{f(hy)}">'
                   f'<path android:pathData="M{f(hx-0.38*r)},{f(hy)} a{f(0.38*r)},{f(0.2*r)} 0 1 0 {f(0.76*r)},0 a{f(0.38*r)},{f(0.2*r)} 0 1 0 {f(-0.76*r)},0 Z" android:fillColor="#D9FFFFFF"/></group>')
    return "\n    ".join(out)

def vector(size, vp, body, aapt=False):
    return (f'<?xml version="1.0" encoding="utf-8"?>\n<!-- Vyoma\'s icon (desktop/icon/gen.py makes every size from one geometry). -->\n'
            f'<vector {A}{" " + AAPT if aapt else ""}\n    android:width="{size}dp" android:height="{size}dp"\n'
            f'    android:viewportWidth="{vp}" android:viewportHeight="{vp}">\n    {body}\n</vector>\n')

g = geo(0.72, 54 - 50 * 0.72)
fg = [f'<path android:pathData="{g["ring"]}" android:fillColor="#00000000" android:strokeColor="#F2FFFFFF" android:strokeWidth="{f(g["w"])}"/>',
      orb_vd(*g["big"], g["s"], True), orb_vd(*g["small"], g["s"], False)]
open(f"{ROOT}/app/src/main/res/drawable/ic_launcher_foreground.xml", "w").write(vector(108, 108, "\n    ".join(fg), aapt=True))

def solid(g, color):
    return "\n    ".join([
        f'<path android:pathData="{g["ring"]}" android:fillColor="#00000000" android:strokeColor="{color}" android:strokeWidth="{f(g["w"])}"/>',
        f'<path android:pathData="{circle(*g["big"])}" android:fillColor="{color}"/>',
        f'<path android:pathData="{circle(*g["small"])}" android:fillColor="{color}"/>'])
open(f"{ROOT}/app/src/main/res/drawable/ic_launcher_monochrome.xml", "w").write(vector(108, 108, solid(g, "#FFFFFFFF")))
open(f"{ROOT}/app/src/main/res/drawable/ic_launcher_background.xml", "w").write(vector(108, 108,
    '<path android:pathData="M0,0h108v108h-108z"><aapt:attr name="android:fillColor"><gradient android:type="linear" '
    'android:startX="0" android:startY="0" android:endX="43.2" android:endY="108">'
    '<item android:offset="0" android:color="#FF5CCBFF"/><item android:offset="1" android:color="#FF1872EA"/></gradient></aapt:attr></path>', aapt=True))
n = 24 / 70
open(f"{ROOT}/app/src/main/res/drawable/ic_notification.xml", "w").write(vector(24, 24, solid(geo(n, 12 - 50 * n), "#FFFFFFFF")))

# ---- Desktop SVG ---------------------------------------------------------------------------------
def svg(tile=True, mac=False):
    # macOS draws no mask: the tile sits on Apple's 1024 grid (824 body, 100 margin) with its own shadow.
    s, o = (0.8047, 9.765) if mac else (0.92, 4) if tile else (1, 0)
    g = geo(s, o)
    def orb(x, y, r, big, i):
        stops = ('<stop offset="0" stop-color="#fff"/><stop offset=".55" stop-color="#eef8ff"/><stop offset="1" stop-color="#b9defc"/>' if big
                 else '<stop offset="0" stop-color="#fff"/><stop offset="1" stop-color="#dff1ff"/>')
        hi = ""
        if big:
            hx, hy = x - 0.32 * r, y - 0.42 * r
            hi = f'<ellipse cx="{f(hx)}" cy="{f(hy)}" rx="{f(0.38*r)}" ry="{f(0.2*r)}" transform="rotate(-28 {f(hx)} {f(hy)})" fill="#fff" fill-opacity=".85"/>'
        return (f'<radialGradient id="o{i}" cx=".36" cy=".3" r=".8">{stops}</radialGradient>'
                f'<circle cx="{f(x)}" cy="{f(y)}" r="{f(r)}" fill="url(#o{i})" filter="url(#sh)"/>'
                f'<circle cx="{f(x)}" cy="{f(y)}" r="{f(r - 0.5*s)}" fill="none" stroke="#fff" stroke-opacity=".9" stroke-width="{f(s)}"/>{hi}')
    return (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100" width="256" height="256">'
            f'<defs><linearGradient id="bg" x1="0" y1="0" x2=".4" y2="1"><stop offset="0" stop-color="#5ccbff"/><stop offset="1" stop-color="#1872ea"/></linearGradient>'
            f'<filter id="sh" x="-30%" y="-30%" width="160%" height="170%"><feDropShadow dx="0" dy="{f(1.6*s)}" stdDeviation="{f(1.6*s)}" flood-color="#0a3f8f" flood-opacity=".35"/></filter>'
            + (f'<filter id="tile" x="-20%" y="-20%" width="140%" height="150%"><feDropShadow dx="0" dy="1" stdDeviation="1.2" flood-color="#000" flood-opacity=".3"/></filter>' if mac else '')
            + f'</defs>'
            f'<rect x="{o}" y="{o}" width="{f(100*s)}" height="{f(100*s)}" rx="{f(22.4*s if mac else 23*s)}" fill="url(#bg)"{' filter="url(#tile)"' if mac else ''}/>'
            f'<path d="{g["ring"]}" fill="none" stroke="#fff" stroke-opacity=".95" stroke-width="{f(g["w"])}"/>'
            f'{orb(*g["big"], True, 1)}{orb(*g["small"], False, 2)}</svg>\n')
open(f"{ROOT}/desktop/daemon/dist/vyoma.svg", "w").write(svg())
open(f"{ROOT}/desktop/gui/ui/icons/vyoma.svg", "w").write(svg())

# ---- macOS: Vyoma.app's icon (icns of PNGs, rendered with resvg) ---------------------------------
import os, struct, subprocess, tempfile
mac_svg = f"{ROOT}/desktop/mac/dist/vyoma.svg"
os.makedirs(os.path.dirname(mac_svg), exist_ok=True)
open(mac_svg, "w").write(svg(mac=True))
# Each PNG once per size, in every slot that size fills (1x and the half size's 2x).
SLOTS = [("icp4", 16), ("ic11", 32), ("icp5", 32), ("ic12", 64), ("ic07", 128), ("ic13", 256), ("ic08", 256), ("ic14", 512), ("ic09", 512), ("ic10", 1024)]
with tempfile.TemporaryDirectory() as tmp:
    pngs = {}
    for size in sorted({n for _, n in SLOTS}):
        out = f"{tmp}/{size}.png"
        subprocess.run(["resvg", "-w", str(size), "-h", str(size), mac_svg, out], check=True)
        pngs[size] = open(out, "rb").read()
body = b"".join(kind.encode() + struct.pack(">I", 8 + len(pngs[n])) + pngs[n] for kind, n in SLOTS)
open(f"{ROOT}/desktop/mac/dist/vyoma.icns", "wb").write(b"icns" + struct.pack(">I", 8 + len(body)) + body)
