#!/bin/sh
# kUI wifi radio bring-up/teardown. Sysfs rfkill only — no helper bins.

IFACE="wlan0"
CONF="/etc/wifi/wpa_supplicant.conf"

# rfkill node for the wifi radio (index is not stable across boots)
rfk_wifi() {
	for d in /sys/class/rfkill/rfkill*; do
		[ "$(cat "$d/type" 2>/dev/null)" = "wlan" ] && { echo "$d"; return; }
	done
}

start() {
	RFK=$(rfk_wifi)
	[ -n "$RFK" ] && echo 0 > "$RFK/soft"

	# ctrl_interface path is load-bearing: the launcher talks to
	# wpa_supplicant with wpa_cli -p /etc/wifi/sockets
	if [ ! -f "$CONF" ]; then
		mkdir -p "$(dirname "$CONF")"
		printf 'ctrl_interface=/etc/wifi/sockets\ndisable_scan_offload=1\nupdate_config=1\n' > "$CONF"
	fi

	# Wake-on-WLAN is asked for at suspend time and the XR819's support
	# for it is not trustworthy on this firmware; we now tear the radio
	# down before sleeping instead, so the trigger buys nothing and is a
	# suspect in the resume-time firmware death. Strip it from configs
	# written by older kUI builds -- in place, keeping the network blocks.
	# NEVER edit this file in place. It holds every saved SSID + PSK, and
	# the rootfs overlay can sit at 100% full (a stale Port Forge swapfile
	# did exactly that) -- an in-place `sed -i` then renames a ZERO-BYTE
	# temp over it and the user's networks are gone. Build the copy first,
	# keep it only if it survived the write with content.
	if grep -q '^wowlan_triggers=' "$CONF" 2>/dev/null; then
		if grep -v '^wowlan_triggers=' "$CONF" > "$CONF.tmp" 2>/dev/null &&
			[ -s "$CONF.tmp" ]; then
			mv -f "$CONF.tmp" "$CONF"
		else
			rm -f "$CONF.tmp"
		fi
	fi

	if [ -x /etc/init.d/wpa_supplicant ]; then
		/etc/init.d/wpa_supplicant start
	elif ! pidof wpa_supplicant >/dev/null 2>&1; then
		wpa_supplicant -B -i "$IFACE" -c "$CONF" 2>/dev/null
	fi

	pidof udhcpc >/dev/null 2>&1 || udhcpc -i "$IFACE" -b 2>/dev/null
}

stop() {
	if [ -x /etc/init.d/wpa_supplicant ]; then
		/etc/init.d/wpa_supplicant stop
	else
		killall wpa_supplicant 2>/dev/null
	fi

	RFK=$(rfk_wifi)
	[ -n "$RFK" ] && echo 1 > "$RFK/soft"

	killall udhcpc 2>/dev/null
}

case "$1" in
start|"") start ;;
stop) stop ;;
*)
	echo "Usage: $0 {start|stop}"
	exit 1
	;;
esac
