#!/bin/sh
# kUI boot, stage 2 (tg5040). Minimal by design: everything that can
# live in the session script does. This stage only handles what must
# happen before ANY kUI binary runs.

SDCARD_PATH="/mnt/SDCARD"

# OTA staging swap: a running binary can't be overwritten in place
# (ETXTBSY), so updates drop replacements beside it as *.new at the SD
# root. Nothing is running yet, so swap them in now.
for staged in "$SDCARD_PATH"/*.new; do
	[ -f "$staged" ] || continue
	mv -f "$staged" "${staged%.new}"
	chmod +x "${staged%.new}"
done
sync

# Full speed for the rest of boot. Min freq always pinned to the floor
# so idle states stay reachable.
CPU="/sys/devices/system/cpu/cpufreq/policy0"
if [ -d "$CPU" ]; then
	FREQ_MIN=$(printf '%s\n' $(cat "$CPU/scaling_available_frequencies" 2>/dev/null) | sort -n | head -n1)
	FREQ_MAX=$(printf '%s\n' $(cat "$CPU/scaling_available_frequencies" 2>/dev/null) | sort -n | tail -n1)
	echo performance > "$CPU/scaling_governor" 2>/dev/null
	[ -n "$FREQ_MAX" ] && echo "$FREQ_MAX" > "$CPU/scaling_max_freq" 2>/dev/null
	[ -n "$FREQ_MIN" ] && echo "$FREQ_MIN" > "$CPU/scaling_min_freq" 2>/dev/null
fi

cd "$(dirname "$0")" && exec ./session.sh
