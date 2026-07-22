#!/bin/sh
# 0.09k one-shot migration: delete the legacy C stack on the first boot
# after the 0.81a -> 0.09k bridge upgrade. Everything below is fully
# replaced by native kUI (kuid, kui-power, the launcher's Control Panel,
# the native frontend). Runs once, stamped by session.sh; idempotent
# (rm -f), so a no-op on a fresh 0.09k install with nothing to clean.
#
# Named -99- so it sorts LAST among migrations: the cleanup must run only
# after every kUI piece is in place -- settings imported (-01-), and the
# shims/libs it copies from (kui-shim, libmsettings.so) already on the card.

SYS=/mnt/SDCARD/.system/tg5040
RES=/mnt/SDCARD/.system/res
UD=/mnt/SDCARD/.userdata
TOOLS=/mnt/SDCARD/Tools/tg5040

# helper binaries with NO third-party callers: delete outright.
# keys/battery/audio -> kuid, power -> kui-power, hooks -> native.
for b in keymon.elf batmon.elf audiomon.elf show2.elf rfkill.elf \
	setterm nextui.elf run_hooks.sh poweroff_next reboot_next; do
	rm -f "$SYS/bin/$b"
done

# helper CLIs paks STILL call by name: replace, don't remove.
# nextval/syncsettings/gametimectl -> the kUI multi-call shim (FAT32 has
# no symlinks, so copy the one binary under each name).
rm -f "$SYS/bin/nextval.elf" "$SYS/bin/syncsettings.elf" "$SYS/bin/gametimectl.elf"
if [ -x "$SYS/bin/kui-shim" ]; then
	cp "$SYS/bin/kui-shim" "$SYS/bin/nextval.elf"
	cp "$SYS/bin/kui-shim" "$SYS/bin/syncsettings.elf"
	cp "$SYS/bin/kui-shim" "$SYS/bin/gametimectl.elf"
	# some paks call bare nextval (no .elf); cover both
	cp "$SYS/bin/kui-shim" "$SYS/bin/nextval"
	chmod +x "$SYS/bin/nextval.elf" "$SYS/bin/syncsettings.elf" \
		"$SYS/bin/gametimectl.elf" "$SYS/bin/nextval" 2>/dev/null
fi

# governor.sh -> kUI's own script (Music Player calls it unguarded)
rm -f "$SYS/bin/governor.sh"
[ -f "$SYS/bin/kui-governor.sh" ] && cp "$SYS/bin/kui-governor.sh" "$SYS/bin/governor.sh"
chmod +x "$SYS/bin/governor.sh" 2>/dev/null

# suspend -> kui-power sleep (a bare 'echo mem' freezes this hardware)
rm -f "$SYS/bin/suspend"
printf '#!/bin/sh\nexec /mnt/SDCARD/kui-power sleep\n' > "$SYS/bin/suspend"
chmod +x "$SYS/bin/suspend" 2>/dev/null

# minarch.elf compat: third-party paks may exec it directly, and the
# native frontend is argv-compatible (<core.so> <rom>). FAT32 has no
# symlinks, so a two-line wrapper stands in.
rm -f "$SYS/bin/minarch.elf"
printf '#!/bin/sh\nexec /mnt/SDCARD/kui-frontend "$@"\n' > "$SYS/bin/minarch.elf"
chmod +x "$SYS/bin/minarch.elf" 2>/dev/null

# their private libs; the generic core deps (libchdr, liblz4, ...) stay.
# libmsettings.so is NOT deleted — kUI ships a clean-room replacement of
# the same name so third-party paks keep linking (the OTA overlay has
# already dropped ours over the legacy one by the time this runs).
for l in libbatmondb.so libgametimedb.so; do
	rm -f "$SYS/lib/$l"
done

# emu wrapper paks: the native frontend launches every libretro
# platform. Only wrappers go - a pak that ships its own emulator stays.
for pak in "$SYS/paks/Emus"/*.pak; do
	[ -f "$pak/launch.sh" ] || continue
	grep -q "minarch.elf" "$pak/launch.sh" && rm -rf "$pak"
done

# the old launcher pak; Bootlogo.pak (Arjun's art) stays
rm -rf "$SYS/paks/MinUI.pak"

# the nine legacy tool paks - every one is native in Control Panel now
# (incl. Updater: it only existed to deliver kUI rusted from the old
# generation). User-installed paks are untouched.
for t in "Battery" "Files" "Game Tracker" "Input" "PakDek" \
	"RA PreFetch" "Scraper & Patcher" "Settings" "Updater"; do
	rm -rf "$TOOLS/$t.pak"
done

# superseded state (gametime + battery history already migrated to
# kui/ logs; msettings.bin was the deleted libmsettings store)
rm -f "$UD/shared/battery_logs.sqlite" "$UD/shared/game_logs.sqlite" \
	"$UD/tg5040/msettings.bin"

# res: only what kUI reads stays - font1/font2, carousel/, the icon
# sheet (assets@2x.png, until the kUI-own sheet replaces it), cacert.pem
for r in background.png assets.svg assets@1x.png assets@3x.png \
	assets@4x.png logo.png minuisettings.defaults.txt \
	font1.backup.ttf font2.backup.ttf BPreplayBold-unhinted.otf \
	BPreplayBold-unhinted_b.otf charging-640-480.png; do
	rm -f "$RES/$r"
done
rm -f "$RES"/grid-*.png "$RES"/line-*.png "$RES"/Collections@*.png \
	"$RES"/Games@*.png "$RES"/Recents@*.png "$RES"/Tools@*.png

sync
