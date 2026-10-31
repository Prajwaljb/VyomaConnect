#!/usr/bin/env python3
"""Measures the demo screen on the phone in dp, so the judge reads sizes instead of guessing them.

Usage: measure.py NAME   (writes NAME.json and prints a table; the demo build must have focus)

Dumps the accessibility tree (uiautomator), converts every node's bounds to dp with the display's
density, and flags what a designer would measure first: touch targets under 48 dp, near-miss
left edges (1-4 dp apart: almost aligned reads as a mistake), and tappable nodes with no label.
Only Vyoma's demo package is read; anything else on screen is refused.
"""
import json, re, subprocess, sys

PKG = "dev.vyoma.connect.demo"


def adb(*args):
    return subprocess.run(["adb", *args], capture_output=True, text=True, check=True).stdout


def main():
    name = sys.argv[1] if len(sys.argv) > 1 else "screen"
    focus = adb("shell", "dumpsys", "window")
    line = next((l for l in focus.splitlines() if "mCurrentFocus" in l), "")
    if PKG + "/" not in line:
        sys.exit("REFUSED: the demo isn't in focus")
    density = int(re.search(r"(\d+)", adb("shell", "wm", "density").splitlines()[-1]).group(1)) / 160
    adb("shell", "uiautomator", "dump", "/sdcard/judge.xml")
    xml = adb("shell", "cat", "/sdcard/judge.xml")
    adb("shell", "rm", "-f", "/sdcard/judge.xml")

    nodes = []
    for m in re.finditer(r"<node [^>]*>", xml):
        n = m.group(0)
        if f'package="{PKG}"' not in n:
            continue
        a = dict(re.findall(r'(\w[\w-]*)="([^"]*)"', n))
        x1, y1, x2, y2 = map(int, re.findall(r"\d+", a["bounds"]))
        if x2 <= x1 or y2 <= y1:
            continue
        label = a.get("text") or a.get("content-desc") or ""
        nodes.append({
            "label": label,
            "class": a.get("class", "").rsplit(".", 1)[-1],
            "clickable": a.get("clickable") == "true",
            "x": round(x1 / density, 1), "y": round(y1 / density, 1),
            "w": round((x2 - x1) / density, 1), "h": round((y2 - y1) / density, 1),
        })

    # The window's extent: nodes cut by its edges (scrolled half out) aren't measured for size.
    bottom = max((n["y"] + n["h"] for n in nodes), default=0)

    def inside(a, b):
        return a is not b and b["x"] >= a["x"] - 0.5 and b["y"] >= a["y"] - 0.5 and \
            b["x"] + b["w"] <= a["x"] + a["w"] + 0.5 and b["y"] + b["h"] <= a["y"] + a["h"] + 0.5

    flags = []
    for n in nodes:
        if not n["clickable"]:
            continue
        # A button's words usually sit on a child node: it's labelled if anything inside it is.
        if not n["label"]:
            n["label"] = next((m["label"] for m in nodes if m["label"] and inside(n, m)), "")
        cut = n["y"] <= 1 or n["y"] + n["h"] >= bottom - 1
        if not cut and min(n["w"], n["h"]) < 48:
            flags.append(f"small target {n['w']}x{n['h']} dp: {n['label'] or n['class']}")
        if not n["label"]:
            flags.append(f"unlabelled tappable at ({n['x']},{n['y']}) {n['w']}x{n['h']} dp")
    texts = sorted({n["x"] for n in nodes if n["label"] and n["class"] == "TextView"})
    for a, b in zip(texts, texts[1:]):
        if 0 < b - a <= 4:
            flags.append(f"near-miss text edges: x={a} and x={b} dp")

    out = {"density": density, "nodes": nodes, "flags": flags}
    with open(f"{name}.json", "w") as f:
        json.dump(out, f, indent=1)
    print(f"{name}: {len(nodes)} nodes at {density}x")
    for n in nodes:
        if n["label"] or n["clickable"]:
            tap = "tap " if n["clickable"] else "    "
            print(f"  {tap}{n['x']:6} {n['y']:6} {n['w']:6}x{n['h']:<6} {n['class'][:12]:12} {n['label'][:40]}")
    for fl in flags:
        print("  ! " + fl)


if __name__ == "__main__":
    main()
