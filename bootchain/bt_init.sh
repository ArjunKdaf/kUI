#!/bin/sh
# kUI bluetooth radio bring-up/teardown. Sysfs rfkill only.
# The controller sits on ttyS1 behind hciattach (xradio line discipline
# on this firmware); firmware load is handled by the vendor driver once
# the uart is attached.

rfk_bt() {
	for d in /sys/class/rfkill/rfkill*; do
		[ "$(cat "$d/type" 2>/dev/null)" = "bluetooth" ] && { echo "$d"; return; }
	done
}

attach_hci() {
	killall hciattach 2>/dev/null
	# let the controller drive the uart wake line
	[ -e /proc/bluetooth/sleep/btwrite ] && echo 1 > /proc/bluetooth/sleep/btwrite
	# power-cycle the radio so the firmware load starts clean
	if [ -n "$RFK" ]; then
		echo 1 > "$RFK/soft"; sleep 1
		echo 0 > "$RFK/soft"; sleep 1
	fi
	hciattach -n ttyS1 xradio >/dev/null 2>&1 &

	# hci0 appears once the firmware is up; give it ~7s
	waited=0
	while [ ! -d /sys/class/bluetooth/hci0 ]; do
		waited=$((waited + 1))
		[ "$waited" -ge 70 ] && return 1
		usleep 100000
	done
}

start() {
	RFK=$(rfk_bt)
	[ -n "$RFK" ] && echo 0 > "$RFK/soft"

	[ -d /sys/class/bluetooth/hci0 ] || attach_hci || exit 1

	if ! pidof bluetoothd >/dev/null 2>&1; then
		if [ -x /etc/bluetooth/bluetoothd ]; then
			/etc/bluetooth/bluetoothd start
		else
			bluetoothd 2>/dev/null &
		fi
		sleep 1
	fi

	# a2dp source: audio routing (kuid's .asoundrc) rides on bluealsa
	if ! pidof bluealsa >/dev/null 2>&1; then
		bluealsa -p a2dp-source 2>/dev/null &
	fi

	bluetoothctl power on >/dev/null 2>&1
}

stop() {
	killall bluealsa 2>/dev/null
	if pidof bluetoothd >/dev/null 2>&1; then
		bluetoothctl power off >/dev/null 2>&1
		killall bluetoothd 2>/dev/null
		sleep 1
	fi
	hciconfig hci0 down 2>/dev/null
	killall hciattach 2>/dev/null
	[ -e /proc/bluetooth/sleep/btwrite ] && echo 0 > /proc/bluetooth/sleep/btwrite

	RFK=$(rfk_bt)
	[ -n "$RFK" ] && echo 1 > "$RFK/soft"
}

case "$1" in
start|"") start ;;
stop) stop ;;
*)
	echo "Usage: $0 {start|stop}"
	exit 1
	;;
esac
