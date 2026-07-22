#!/bin/sh
# kUI CPU governor helper. Compat entry for paks that call
# $SYSTEM_PATH/bin/governor.sh <mode>; kUI's own boot/launch paths set
# the governor inline. Min freq is always pinned to the floor so idle
# states stay reachable.

CPU="/sys/devices/system/cpu/cpufreq/policy0"
[ -d "$CPU" ] || exit 0

FREQS=$(cat "$CPU/scaling_available_frequencies" 2>/dev/null)
LO=$(printf '%s\n' $FREQS | sort -n | head -n1)
HI=$(printf '%s\n' $FREQS | sort -n | tail -n1)
# second-highest for 'auto' (never overclock by default)
HI2=$(printf '%s\n' $FREQS | sort -n | tail -n2 | head -n1)
# rough midpoint for 'powersave'
MID=$(printf '%s\n' $FREQS | sort -n | awk '{a[NR]=$0} END{print a[int((NR+1)/2)]}')

case "$1" in
	performance)
		echo performance > "$CPU/scaling_governor" 2>/dev/null
		[ -n "$HI" ] && echo "$HI" > "$CPU/scaling_max_freq" 2>/dev/null
		;;
	powersave)
		echo conservative > "$CPU/scaling_governor" 2>/dev/null
		[ -n "$MID" ] && echo "$MID" > "$CPU/scaling_max_freq" 2>/dev/null
		;;
	auto|"")
		echo schedutil > "$CPU/scaling_governor" 2>/dev/null
		[ -n "$HI2" ] && echo "$HI2" > "$CPU/scaling_max_freq" 2>/dev/null
		;;
	*)
		echo "usage: governor.sh {performance|auto|powersave}" >&2
		exit 1
		;;
esac
[ -n "$LO" ] && echo "$LO" > "$CPU/scaling_min_freq" 2>/dev/null
