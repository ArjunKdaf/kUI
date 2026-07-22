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
cp scripts/INSTALL.txt "$DIST/README.txt"

# Release payload: one flat zip in SD-root layout. It serves every path:
#   - the in-OS Updater  (unzip auto-detects no top-level dir to strip)
#   - the 0.81a bridge   (its C updater does `cd /mnt/SDCARD && unzip -o`)
#   - a manual install   (unzip straight onto the card root)
# A full fresh-install card image (OS + cores + art + overlays) is
# assembled separately at release time from a curated card.
nix-shell -p zip --run "cd 'dist/kUI-$VERSION' && rm -f '../kUI-$VERSION.zip' && zip -Xr '../kUI-$VERSION.zip' . >/dev/null"
echo "built dist/kUI-$VERSION.zip"
sha256sum "dist/kUI-$VERSION.zip"
