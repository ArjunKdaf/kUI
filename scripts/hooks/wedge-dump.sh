#!/bin/sh
# wedge-dump.sh — pre-sleep hook: snapshot kernel/input/USB/thermal state
# to SD card before suspend/poweroff wipes tmpfs logs. Debug tool for the
# input+USB wedge (2026-07-12) and the CPU2 tick-death watchdog crashes
# (2026-07-25: RCU stalls at 79°C — thermal columns added so the next
# wedge shows whether the kernel was throttling).
# Install: /mnt/SDCARD/.userdata/tg5040/.hooks/pre-sleep.d/wedge-dump.sh (chmod +x)

OUT="/mnt/SDCARD/.userdata/tg5040/logs/wedge-dumps/$(date +%Y%m%d-%H%M%S)"
mkdir -p "$OUT" || exit 0

dmesg > "$OUT/dmesg.txt" 2>&1
cat /var/log/messages > "$OUT/messages.txt" 2>&1
ps w > "$OUT/ps.txt" 2>&1
cat /proc/interrupts > "$OUT/interrupts.txt" 2>&1
cat /proc/bus/input/devices > "$OUT/input-devices.txt" 2>&1
{ uptime; free; } > "$OUT/state.txt" 2>&1

# thermal + cpufreq snapshot, labeled: zone temps, throttle step, clocks
{
	for z in /sys/class/thermal/thermal_zone*; do
		echo "$z: $(cat "$z/type" 2>/dev/null) $(cat "$z/temp" 2>/dev/null)"
	done
	for c in /sys/class/thermal/cooling_device*; do
		echo "$c: $(cat "$c/type" 2>/dev/null) state $(cat "$c/cur_state" 2>/dev/null)/$(cat "$c/max_state" 2>/dev/null)"
	done
	CPU="/sys/devices/system/cpu/cpufreq/policy0"
	echo "governor: $(cat "$CPU/scaling_governor" 2>/dev/null)"
	echo "freq cur/max: $(cat "$CPU/scaling_cur_freq" 2>/dev/null)/$(cat "$CPU/scaling_max_freq" 2>/dev/null)"
	echo "gpu clk: $(cat /sys/kernel/debug/clk/gpu/clk_rate 2>/dev/null)"
} > "$OUT/thermal.txt" 2>&1

# kernel stacks of D-state (uninterruptible) threads — the smoking gun for
# a stalled workqueue/I2C transaction
for pid in $(ps w | awk '$4 ~ /^D/ {print $1}'); do
	echo "=== pid $pid $(cat /proc/$pid/comm 2>/dev/null) ==="
	cat /proc/$pid/stack 2>/dev/null
done > "$OUT/dstate-stacks.txt" 2>&1

# is the input daemon alive, and is adbd alive?
{ pidof trimui_inputd && echo trimui_inputd:ALIVE || echo trimui_inputd:DEAD
  pidof adbd && echo adbd:ALIVE || echo adbd:DEAD
} > "$OUT/daemons.txt" 2>&1

# keep only the 15 newest dumps (busybox head may not support negative -n)
cd /mnt/SDCARD/.userdata/tg5040/logs/wedge-dumps 2>/dev/null && {
	total=$(ls -1d 2*/ 2>/dev/null | wc -l)
	[ "$total" -gt 15 ] && ls -1d 2*/ | head -n $((total - 15)) | while read -r d; do rm -rf "$d"; done
}

exit 0
