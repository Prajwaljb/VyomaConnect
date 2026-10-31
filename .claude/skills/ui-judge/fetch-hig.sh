#!/bin/bash
# Refreshes hig/: Apple's Human Interface Guidelines chapters the judge reads, as Markdown.
# Apple's text, so hig/ stays out of git (see .gitignore); run this to get it on a new checkout.
set -e
dir="$(cd "$(dirname "$0")" && pwd)"
raw="$(mktemp -d)"
for p in materials layout buttons color typography motion playing-haptics charts gauges toggles sliders \
         lists-and-tables tab-bars toolbars accessibility feedback consistency dark-mode sf-symbols icons widgets \
         disclosure-controls progress-indicators segmented-controls menus alerts sheets onboarding branding; do
    curl -sf -o "$raw/$p.json" "https://developer.apple.com/tutorials/data/design/human-interface-guidelines/$p.json" || echo "skipped $p"
done
mkdir -p "$dir/hig"
python3 -I "$dir/hig2md.py" "$raw"/*.json "$dir/hig"
rm -rf "$raw"
ls "$dir/hig"
