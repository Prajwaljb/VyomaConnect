#!/bin/bash
# capture.sh OUTDIR NAME: a screenshot of the demo (status bar cropped off, so nothing personal)
# at full resolution plus measure.py's dp table beside it. Refuses unless the demo has focus.
set -e
out="$1"; name="$2"; dir="$(cd "$(dirname "$0")" && pwd)"
mkdir -p "$out"
adb shell dumpsys window | grep -m1 mCurrentFocus | grep -q "dev.vyoma.connect.demo/" || { echo "REFUSED: not the demo"; exit 3; }
adb exec-out screencap -p > "$out/_raw.png"
python3 - "$out/_raw.png" "$out/$name.png" <<'PY'
import sys
from PIL import Image
im = Image.open(sys.argv[1]); w, h = im.size
top = int(h * 0.047)  # the status bar
im.crop((0, top, w, h)).save(sys.argv[2])
PY
rm "$out/_raw.png"
(cd "$out" && python3 "$dir/measure.py" "$name" > "$name.txt")
echo "$out/$name.png"
