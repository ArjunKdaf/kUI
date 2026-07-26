#!/bin/sh
# kUI boot, stage 3: the session. Owns the console from here until
# poweroff. There is no fallback UI — kUI IS the UI.

BOOT_DIR="$(cd "$(dirname "$0")" && pwd)"

# ---- pak contract -----------------------------------------------------
# These exports ARE the interface third-party paks are written against.
# Do not rename, drop, or reorder semantics without a migration.
export PLATFORM="tg5040"
export SDCARD_PATH="/mnt/SDCARD"
export BIOS_PATH="$SDCARD_PATH/Bios"
export ROMS_PATH="$SDCARD_PATH/Roms"
export SAVES_PATH="$SDCARD_PATH/Saves"
export CHEATS_PATH="$SDCARD_PATH/Cheats"
export SYSTEM_PATH="$SDCARD_PATH/.system/$PLATFORM"
export CORES_PATH="$SYSTEM_PATH/cores"
export USERDATA_PATH="$SDCARD_PATH/.userdata/$PLATFORM"
export SHARED_USERDATA_PATH="$SDCARD_PATH/.userdata/shared"
export LOGS_PATH="$USERDATA_PATH/logs"
export HOOKS_PATH="$USERDATA_PATH/.hooks"
export DATETIME_PATH="$SHARED_USERDATA_PATH/datetime.txt"
export HOME="$USERDATA_PATH"
export LD_LIBRARY_PATH="$SYSTEM_PATH/lib:/usr/trimui/lib:$LD_LIBRARY_PATH"
export PATH="$SYSTEM_PATH/bin:/usr/trimui/bin:$PATH"

export TRIMUI_MODEL=$(strings /usr/trimui/bin/MainUI | grep ^Trimui)
if [ "$TRIMUI_MODEL" = "Trimui Brick" ]; then
	export DEVICE="brick"
else
	export DEVICE="smartpro"
fi

# legacy compat flag: paks probe it to pick codepaths, so it stays
export IS_NEXT="yes"

mkdir -p "$BIOS_PATH" "$ROMS_PATH" "$SAVES_PATH" "$CHEATS_PATH" \
	"$USERDATA_PATH" "$LOGS_PATH" "$HOOKS_PATH" "$SHARED_USERDATA_PATH"

# ---- gpio -------------------------------------------------------------
gpio_pin() { # num direction [value]
	[ -d "/sys/class/gpio/gpio$1" ] || echo "$1" > /sys/class/gpio/export 2>/dev/null
	echo -n "$2" > "/sys/class/gpio/gpio$1/direction"
	[ -n "$3" ] && echo -n "$3" > "/sys/class/gpio/gpio$1/value"
}
gpio_pin 107 out 1  # PD11: 5v rail on
gpio_pin 227 out 0  # PH3: rumble motor idle
gpio_pin 243 in     # PH19: DIP switch, read-only

# ---- time zone --------------------------------------------------------
# kui.cfg stores the UTC offset; POSIX TZ sign is inverted
CFG="$SHARED_USERDATA_PATH/kui.cfg"
cfg_get() { sed -n "s/^$1[[:space:]]*=[[:space:]]*//p" "$CFG" 2>/dev/null; }
TZOFF=$(cfg_get tz.utc)
[ -n "$TZOFF" ] && export TZ=$(awk -v o="$TZOFF" 'BEGIN{printf "UTC%+d", -o}')

# ---- one-shot per-release migrations ----------------------------------
# The OTA updater is a plain overlay — it adds/overwrites but can never
# delete or move. Each release ships its cleanup as migrations/<version>.sh;
# every script runs exactly once (stamped in userdata), so OTA users get
# restructures applied on the first boot after updating.
MIGRATIONS_DIR="$SYSTEM_PATH/migrations"
MIGRATIONS_STAMPS="$SHARED_USERDATA_PATH/.migrations"
if [ -d "$MIGRATIONS_DIR" ]; then
	mkdir -p "$MIGRATIONS_STAMPS"
	for m in "$MIGRATIONS_DIR"/*.sh; do
		[ -f "$m" ] || continue
		MSTAMP="$MIGRATIONS_STAMPS/$(basename "$m" .sh)"
		[ -f "$MSTAMP" ] && continue
		sh "$m" >> "$LOGS_PATH/migrations.txt" 2>&1
		touch "$MSTAMP"
	done
fi

# ---- daemons ----------------------------------------------------------
# stock gpio input daemon; kuid consumes its events
trimui_inputd &

# kuid: the kUI system daemon (keys, battery, audio routing, LEDs)
if [ -x "$SDCARD_PATH/kuid" ] && ! pidof kuid >/dev/null 2>&1; then
	"$SDCARD_PATH/kuid" >> "$LOGS_PATH/kuid.txt" 2>&1 &
fi

# ---- radios -----------------------------------------------------------
# our radio scripts double as the /etc init scripts: kuid and the UI
# toggles call those paths (/etc is a persistent overlay on this device)
mkdir -p /etc/wifi /etc/bluetooth 2>/dev/null
cp -f "$BOOT_DIR/wifi_init.sh" /etc/wifi/wifi_init.sh
cp -f "$BOOT_DIR/bt_init.sh" /etc/bluetooth/bt_init.sh
chmod +x /etc/wifi/wifi_init.sh /etc/bluetooth/bt_init.sh 2>/dev/null

# The stock rootfs auto-starts wpa_supplicant at boot (rc.d/S96, procd-
# managed so a kill just respawns), lighting wifi up for ~20s before we can
# tear it down. Neuter that hook once — /etc persists — so radios come up
# only on request. On-demand wifi still works: wifi_init.sh calls
# /etc/init.d/wpa_supplicant, which stays in place.
[ -e /etc/rc.d/S96wpa_supplicant ] \
	&& mv -f /etc/rc.d/S96wpa_supplicant /etc/rc.d/.dis_S96wpa_supplicant

# kui.cfg is the only authority; absent key means off
[ "$(cfg_get radio.wifi)" = "on" ] && "$BOOT_DIR/wifi_init.sh" start >/dev/null 2>&1 &
[ "$(cfg_get radio.bluetooth)" = "on" ] && "$BOOT_DIR/bt_init.sh" start >/dev/null 2>&1 &

# safety net for the ONE first boot after install, when the stock hook ran
# before we neutered it: a single delayed teardown if the user isn't opted in
(
	sleep 25
	[ "$(cfg_get radio.wifi)" = "on" ] || /etc/wifi/wifi_init.sh stop >/dev/null 2>&1
	[ "$(cfg_get radio.bluetooth)" = "on" ] || /etc/bluetooth/bt_init.sh stop >/dev/null 2>&1
) &

# The stock rootfs auto-starts sshd at boot (rc.d/S50sshd, procd-managed) and
# generates host keys on first boot -- slow, and SSH is meant to be opt-in.
# Neuter that hook once (/etc persists) and stop the server unless the user
# opted in; on-demand start via /etc/init.d/sshd still works for the toggle.
if [ -e /etc/rc.d/S50sshd ]; then
	mv -f /etc/rc.d/S50sshd /etc/rc.d/.dis_S50sshd
	[ -f "$SHARED_USERDATA_PATH/.ssh_on_boot" ] || /etc/init.d/sshd stop >/dev/null 2>&1
fi

# Settings -> Developer -> "Start SSH on boot"
if [ -f "$SHARED_USERDATA_PATH/.ssh_on_boot" ]; then
	(/etc/init.d/sshd start >/dev/null 2>&1 || /etc/init.d/S50sshd start >/dev/null 2>&1) &
fi

# pak-contract boot task: paks may install a one-per-boot script.
# (The hook contract - boot.d, pre/post-launch.d, sleep pair - lives
# entirely inside the launcher; nothing to run here.)
[ -f "$USERDATA_PATH/auto.sh" ] && sh "$USERDATA_PATH/auto.sh" &

# ---- helpers ----------------------------------------------------------
perf_governor() {
	CPU="/sys/devices/system/cpu/cpufreq/policy0"
	[ -d "$CPU" ] || return
	FREQ_MIN=$(printf '%s\n' $(cat "$CPU/scaling_available_frequencies" 2>/dev/null) | sort -n | head -n1)
	FREQ_MAX=$(printf '%s\n' $(cat "$CPU/scaling_available_frequencies" 2>/dev/null) | sort -n | tail -n1)
	echo performance > "$CPU/scaling_governor" 2>/dev/null
	[ -n "$FREQ_MAX" ] && echo "$FREQ_MAX" > "$CPU/scaling_max_freq" 2>/dev/null
	[ -n "$FREQ_MIN" ] && echo "$FREQ_MIN" > "$CPU/scaling_min_freq" 2>/dev/null
}

# ---- main loop --------------------------------------------------------
NEXT_PATH="/tmp/next"

cd "$SDCARD_PATH"
while true; do
	echo "=== kui-launcher start $(date)" >> "$LOGS_PATH/kui.txt"
	"$SDCARD_PATH/kui-launcher" >> "$LOGS_PATH/kui.txt" 2>&1
	echo "=== kui-launcher exit rc=$? $(date)" >> "$LOGS_PATH/kui.txt"

	# launched paks default to full speed; they may drop it themselves
	perf_governor

	if [ -f "$NEXT_PATH" ]; then
		CMD=$(cat "$NEXT_PATH")
		# session log: append, pruned each launch so it never grows
		# unbounded but a crash always leaves the last session on card
		FELOG="$LOGS_PATH/kui-frontend.txt"
		if [ -f "$FELOG" ] && [ "$(wc -c < "$FELOG")" -gt 524288 ]; then
			tail -c 262144 "$FELOG" > "$FELOG.tmp" && mv -f "$FELOG.tmp" "$FELOG"
		fi
		echo "=== session start $(date): $CMD" >> "$FELOG"
		eval "$CMD" >> "$FELOG" 2>&1
		echo "=== session exit rc=$? $(date)" >> "$FELOG"
		rm -f "$NEXT_PATH"
		# back to full speed for the launcher; it resets auto if it wants
		perf_governor
	fi

	[ -f /tmp/poweroff ] && exec "$SDCARD_PATH/kui-power" off
	[ -f /tmp/reboot ] && exec "$SDCARD_PATH/kui-power" reboot
done
