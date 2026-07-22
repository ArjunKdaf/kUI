#!/bin/sh
# 0.09k one-shot settings import: carry a migrated 0.81a card's settings
# and history into kUI's own kui.cfg / kui/ store on the first boot after
# the 0.81a -> 0.09k bridge upgrade. This is the ONLY place the legacy
# minuisettings.txt / .ui_mode / .minui filenames are read; runs once,
# stamped by session.sh. No-op on a fresh install (nothing to import).

SHARED=/mnt/SDCARD/.userdata/shared
CFG="$SHARED/kui.cfg"

# only import onto a card that has no kui.cfg yet (a migrated 0.81a card)
[ -f "$CFG" ] && exit 0

# colors / brightness / font from the legacy settings file
SRC="$SHARED/minuisettings.txt"
if [ -f "$SRC" ]; then
	while IFS='=' read -r k v; do
		k=$(printf '%s' "$k" | tr -d ' \t\r')
		v=$(printf '%s' "$v" | tr -d ' \t\r' | sed 's/^0x//')
		case "$k" in
			color1) echo "theme.color1 = $v" >> "$CFG" ;;
			color2) echo "theme.color2 = $v" >> "$CFG" ;;
			color3) echo "theme.color3 = $v" >> "$CFG" ;;
			color4) echo "theme.color4 = $v" >> "$CFG" ;;
			color5) echo "theme.color5 = $v" >> "$CFG" ;;
			color6) echo "theme.color6 = $v" >> "$CFG" ;;
			brightness) echo "display.brightness = $v" >> "$CFG" ;;
			font) echo "theme.font = $v" >> "$CFG" ;;
		esac
	done < "$SRC"
fi

# ui mode flag file (0=lists, 1=covers, anything else=carousel)
if [ -f "$SHARED/.ui_mode" ]; then
	m=$(tr -d ' \t\r\n' < "$SHARED/.ui_mode")
	case "$m" in
		0) echo "ui.mode = lists" >> "$CFG" ;;
		1) echo "ui.mode = covers" >> "$CFG" ;;
		*) echo "ui.mode = carousel" >> "$CFG" ;;
	esac
fi

# history data files -> kUI's home (don't clobber if already present)
mkdir -p "$SHARED/kui"
[ -f "$SHARED/.minui/recent.txt" ] && [ ! -f "$SHARED/kui/recents.txt" ] \
	&& cp "$SHARED/.minui/recent.txt" "$SHARED/kui/recents.txt"
[ -f "$SHARED/.minui/pins.txt" ] && [ ! -f "$SHARED/kui/pins.txt" ] \
	&& cp "$SHARED/.minui/pins.txt" "$SHARED/kui/pins.txt"

sync
