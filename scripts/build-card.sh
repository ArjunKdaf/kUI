#!/usr/bin/env bash
# Assemble the full fresh-install kUI card image from the OS payload + the
# curated system content pulled off the Hammer (no personal data).
# Run scripts/package.sh first (the payload is layer 1), with the device
# plugged in over adb. Produces dist/kUI-<ver>-card.zip.
set -e
cd "$(dirname "$0")/.."
VERSION=$(cat VERSION)
REPO=$(pwd)
STAGE="$REPO/dist/kUI-$VERSION-card"
SD=/mnt/SDCARD

[ -d "$REPO/dist/kUI-$VERSION" ] || {
	echo "dist/kUI-$VERSION missing - run scripts/package.sh first" >&2
	exit 1
}

rm -rf "$STAGE"
mkdir -p "$STAGE"

echo "[1/7] OS payload"
cp -a "$REPO/dist/kUI-$VERSION/." "$STAGE/"

echo "[2/7] cores"
adb pull -a "$SD/.system/tg5040/cores" "$STAGE/.system/tg5040/" >/dev/null
# dev leftovers on the curated card never ship (e.g. the pre-refresh
# picodrive kept as *.bak)
rm -f "$STAGE/.system/tg5040/cores/"*.bak

echo "[3/7] bootlogos (brick only, drop smartpro/lineage)"
mkdir -p "$STAGE/.system/tg5040/Bootlogo.pak"
adb pull -a "$SD/.system/tg5040/Bootlogo.pak/brick" "$STAGE/.system/tg5040/Bootlogo.pak/" >/dev/null
adb pull -a "$SD/.system/tg5040/Bootlogo.pak/launch.sh" "$STAGE/.system/tg5040/Bootlogo.pak/" >/dev/null 2>&1 || true

echo "[4/7] themes + overlays"
# kUI's own artbook themes at .system/themes - NOT the legacy Themes.pak
# downloader catalog, which stays off the image
adb pull -a "$SD/.system/themes" "$STAGE/.system/" >/dev/null
adb pull -a "$SD/Overlays" "$STAGE/" >/dev/null

echo "[5/7] fonts + carousel art"
adb pull -a "$SD/.system/res/font1.ttf" "$STAGE/.system/res/" >/dev/null
adb pull -a "$SD/.system/res/font2.ttf" "$STAGE/.system/res/" >/dev/null
adb pull -a "$SD/.system/res/carousel" "$STAGE/.system/res/" >/dev/null

echo "[6/7] empty Roms tree (folder names only) + empty user dirs"
mkdir -p "$STAGE/Roms"
# pipe ls through cat on the device: adb shell allocates a pty, and a
# bare ls colorizes onto it, baking color codes into the staged names.
# The (TAG)-suffix filter drops anything that isn't a platform folder.
adb shell "ls -1 /mnt/SDCARD/Roms | cat" | tr -d '\r' \
  | grep -E '\([A-Za-z0-9]+\)$' | while IFS= read -r f; do
  [ -n "$f" ] && mkdir -p "$STAGE/Roms/$f"
done
for d in Bios Saves Cheats Collections Screenshots; do mkdir -p "$STAGE/$d"; done

echo "[7/7] zip"
du -sm "$STAGE" | cut -f1 | xargs -I{} echo "staged size: {} MB"
echo "roms folders: $(ls "$STAGE/Roms" | wc -l)"
echo "cores: $(ls "$STAGE/.system/tg5040/cores/"*.so 2>/dev/null | wc -l)"
nix-shell -p zip --run "cd '$STAGE' && rm -f '../kUI-$VERSION-card.zip' && zip -Xr '../kUI-$VERSION-card.zip' . >/dev/null"
echo "built dist/kUI-$VERSION-card.zip"
sha256sum "$REPO/dist/kUI-$VERSION-card.zip"
