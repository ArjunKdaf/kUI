#!/bin/sh
# kUI CPU governor helper. Compat entry for paks that call
# $SYSTEM_PATH/bin/governor.sh <mode>; kUI's own boot/launch paths set
# the profile via the HAL (same mapping as here). schedutil everywhere so
# an idle SoC always clocks down; profiles differ only by the frequency
# ceiling (thermal defense — the A133P has wedged a core under sustained
# heat). Min freq is always pinned to the floor so idle states stay
# reachable.

CPU="/sys/devices/system/cpu/cpufreq/policy0"
[ -d "$CPU" ] || exit 0

FREQS=$(cat "$CPU/scaling_available_frequencies" 2>/dev/null)
LO=$(printf '%s\n' $FREQS | sort -n | head -n1)
HI=$(printf '%s\n' $FREQS | sort -n | tail -n1)
# highest step at or below the target cap, mirroring the HAL
cap() {
	printf '%s\n' $FREQS | sort -n | awk -v t="$1" '$0 <= t {c=$0} END{print c}'
}
AUTO=$(cap 1416000)
SAVE=$(cap 1008000)

case "$1" in
	performance)
		echo schedutil > "$CPU/scaling_governor" 2>/dev/null
		[ -n "$HI" ] && echo "$HI" > "$CPU/scaling_max_freq" 2>/dev/null
		;;
	powersave)
		echo schedutil > "$CPU/scaling_governor" 2>/dev/null
		[ -n "$SAVE" ] && echo "$SAVE" > "$CPU/scaling_max_freq" 2>/dev/null
		;;
	auto|"")
		echo schedutil > "$CPU/scaling_governor" 2>/dev/null
		[ -n "$AUTO" ] && echo "$AUTO" > "$CPU/scaling_max_freq" 2>/dev/null
		;;
	*)
		echo "usage: governor.sh {performance|auto|powersave}" >&2
		exit 1
		;;
esac
[ -n "$LO" ] && echo "$LO" > "$CPU/scaling_min_freq" 2>/dev/null
