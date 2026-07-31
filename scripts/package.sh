#!/usr/bin/env bash
# Assemble the card-drop release zip: three binaries + boot hook + docs.
set -e
cd "$(dirname "$0")/.."
VERSION=$(cat VERSION)
nix-shell --run 'cargo build --release --target aarch64-unknown-linux-gnu'

T=target/aarch64-unknown-linux-gnu/release
DIST="dist/kUI-$VERSION"
rm -rf "$DIST"
mkdir -p "$DIST"
cp "$T/kui-launcher" "$T/kui-frontend" "$DIST/"
cp "$T/kui-daemon" "$DIST/kuid"
cp "$T/kui-power" "$DIST/kui-power"
# clean-room libmsettings.so: third-party paks link it; lives in the
# system lib dir where their LD_LIBRARY_PATH resolves DT_NEEDED
mkdir -p "$DIST/.system/tg5040/lib"
cp "$T/libmsettings.so" "$DIST/.system/tg5040/lib/libmsettings.so"
# pak-compat helpers: the cleanup migration copies these under the
# legacy CLI names (nextval/syncsettings/gametimectl -> kui-shim;
# governor.sh -> kui-governor.sh) so paks calling them keep working
mkdir -p "$DIST/.system/tg5040/bin"
cp "$T/kui-shim" "$DIST/.system/tg5040/bin/kui-shim"
cp bootchain/governor.sh "$DIST/.system/tg5040/bin/kui-governor.sh"
chmod +x "$DIST/.system/tg5040/bin/kui-shim" \
	"$DIST/.system/tg5040/bin/kui-governor.sh"
# the kUI bootchain (Phase 2): .tmp_update owns boot once installed.
# Scripts are read at boot only - safe to overlay while running.
mkdir -p "$DIST/.tmp_update"
cp bootchain/updater bootchain/tg5040.sh bootchain/session.sh \
   bootchain/wifi_init.sh bootchain/bt_init.sh "$DIST/.tmp_update/"
chmod +x "$DIST/.tmp_update/"* "$DIST/kui-power"
# one-shot migrations: session.sh runs each exactly once per card
mkdir -p "$DIST/.system/tg5040/migrations"
cp bootchain/migrations/*.sh "$DIST/.system/tg5040/migrations/"
# license notices ship on the card
mkdir -p "$DIST/.system/res"
cp -r LICENSES "$DIST/LICENSES"
mkdir -p "$DIST/.system/res" "$DIST/Shaders"
cp shaders/*.glsl "$DIST/Shaders/"
cp assets/cacert.pem "$DIST/.system/res/cacert.pem"
cp assets/iconsheet/assets@2x.png "$DIST/.system/res/assets@2x.png"
# default collection background art: new in 0.27k, so updaters coming
# from 0.09k need it delivered in the payload (cards built fresh get it
# here too - the card image is assembled on top of this payload)
cp -r assets/collections "$DIST/.system/res/collections"
# one-off core refresh: PicoDrive rebuilt from upstream master + the SMS
# FM serialize_size fix (picodrive PR #266) - fixes SMS save states
# resetting the game (found via Asterix). Pulled off the curated card
# (md5-matched to the device-verified build), not built by this script.
if [ ! -f vendor/cores/picodrive_libretro.so ]; then
	echo "missing vendor/cores/picodrive_libretro.so (pull it off the card)" >&2
	exit 1
fi
mkdir -p "$DIST/.system/tg5040/cores"
cp vendor/cores/picodrive_libretro.so "$DIST/.system/tg5040/cores/"
cp scripts/INSTALL.txt "$DIST/README.txt"

# OS update payload: one flat zip in SD-root layout. It is kUI itself
# (binaries + boot chain + migrations + libs) -- NOT cores or art, save
# for the one-off PicoDrive refresh and the default collection art
# above. Serves every UPDATE/overlay path:
#   - the in-OS Updater  (unzip auto-detects no top-level dir to strip)
#   - the 0.81a bridge   (its C updater does `cd /mnt/SDCARD && unzip -o`)
#   - a manual overlay onto a card that already has cores + art
# A fresh, blank-SD install needs the full card image (OS + cores + art +
# overlays + bootlogos), assembled separately from a curated card.
nix-shell -p zip --run "cd 'dist/kUI-$VERSION' && rm -f '../kUI-$VERSION.zip' && zip -Xr '../kUI-$VERSION.zip' . >/dev/null"
echo "built dist/kUI-$VERSION.zip"
sha256sum "dist/kUI-$VERSION.zip"
