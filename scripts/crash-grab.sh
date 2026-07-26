#!/usr/bin/env bash
# Grab crash evidence from the Hammer over ADB BEFORE relaunching any emulator.
# Recreated 2026-07-25 (original lived in ~/dev/KdafUI, repo since deleted).
# Usage: nix-shell -p android-tools --run './crash-grab.sh [label]'
set -euo pipefail

LABEL="${1:-crash}"
OUT="$HOME/dev/kUI/crash-reports/$(date +%Y%m%d-%H%M%S)-$LABEL"
mkdir -p "$OUT"
echo "grabbing to $OUT"

adb shell "ls -la /sys/fs/pstore/ 2>&1"                          > "$OUT/pstore-ls.txt"
adb shell "cat /sys/fs/pstore/* 2>/dev/null"                     > "$OUT/pstore-contents.txt"
adb shell "dmesg 2>&1"                                           > "$OUT/dmesg-current-boot.txt"
adb shell "cat /var/log/messages 2>&1"                           > "$OUT/var-log-messages.txt"
adb shell "uptime; date; cat /proc/loadavg; free"                > "$OUT/state.txt"
adb shell "for z in /sys/class/thermal/thermal_zone*; do echo \$z: \$(cat \$z/type) \$(cat \$z/temp); done 2>&1" > "$OUT/thermal-now.txt"
adb shell "cat /proc/sys/kernel/panic; cat /sys/class/watchdog/watchdog0/timeout 2>/dev/null" > "$OUT/sysinfo.txt"
adb shell "ls -la /mnt/SDCARD/.userdata/tg5040/logs/ 2>&1"       > "$OUT/logs-dir-ls.txt"

# launcher + daemon logs (append-mode across reboots, safe but grab anyway)
adb pull /mnt/SDCARD/.userdata/tg5040/logs/kui.txt  "$OUT/kui.txt"  || true
adb pull /mnt/SDCARD/.userdata/tg5040/logs/kuid.txt "$OUT/kuid.txt" || true

# newest two wedge-dumps (pre-sleep hook output)
for d in $(adb shell "ls /mnt/SDCARD/.userdata/tg5040/logs/wedge-dumps/" | tr -d '\r' | sort | tail -2); do
  adb pull "/mnt/SDCARD/.userdata/tg5040/logs/wedge-dumps/$d" "$OUT/" || true
done

echo "done. evidence in $OUT"
