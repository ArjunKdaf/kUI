#!/usr/bin/env bash
# Record the Hammer's screen over adb into an mp4.
#
#   nix-shell -p android-tools ffmpeg --run "./scripts/screenrec.sh [secs] [out.mp4]"
#
# The device's own ffmpeg grabs /dev/fb0 (which mirrors the GLES screen)
# and encodes MJPEG on-device; the host then transcodes to a clean H.264
# mp4. 15fps is the honest ceiling - the framebuffer is uncached memory
# and reading it tops out around 45-60MB/s regardless of encoder or
# governor - so we ask for exactly 15 to get clean constant-frame-rate
# output. Recording starts immediately; navigate while it runs.
set -e
SECS=${1:-10}
OUT=${2:-capture-$(date +%Y%m%d-%H%M%S).mp4}
TMP=$(mktemp --suffix=.avi)

echo "recording ${SECS}s..."
adb shell "ffmpeg -hide_banner -loglevel error -y -f fbdev -framerate 15 -i /dev/fb0 -t $SECS -c:v mjpeg -q:v 3 /tmp/kui-cap.avi"
adb pull /tmp/kui-cap.avi "$TMP" >/dev/null
adb shell rm -f /tmp/kui-cap.avi
ffmpeg -hide_banner -loglevel error -y -i "$TMP" \
	-c:v libx264 -preset slow -crf 18 -pix_fmt yuv420p \
	-movflags +faststart "$OUT"
rm -f "$TMP"
echo "saved $OUT"
