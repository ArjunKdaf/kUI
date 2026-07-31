#!/bin/sh
# 0.27k one-shot migration: remove junk empty Roms folders whose names
# carry baked-in ls color-code text (e.g. "[1;34m3DO (3DO)[0m") -- an
# early card-curation script ran ls on a pty and its colored output got
# used as folder names. Only exact-pattern matches are touched, and only
# with rmdir, so a folder holding anything is never deleted. A no-op on
# the (clean) released 0.09k images; idempotent.

cd /mnt/SDCARD/Roms 2>/dev/null || exit 0
ESC=$(printf '\033')
for d in *; do
	[ -d "$d" ] || continue
	case "$d" in
	"[1;34m"*"[0m" | "$ESC["*"m"*"$ESC["*"m")
		rmdir "$d" 2>/dev/null && echo "removed junk Roms dir: $d"
		;;
	esac
done
exit 0
