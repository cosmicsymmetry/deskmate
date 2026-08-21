#!/bin/bash
# Capture one 4K frame of the Deskmate board from the OBSBOT webcam.
#
# Usage: capture.sh <session-dir> [label]
# Prints the produced frame path on stdout. Diagnostics go to stderr.
#
# The first 75 frames (~2.5s at 30fps) are discarded so autofocus and
# exposure settle; validated 2026-08-15 as the difference between an
# unreadable and a fully readable panel.
set -euo pipefail

usage() {
    echo "usage: $0 <session-dir> [label]" >&2
    exit 2
}
[ $# -ge 1 ] && [ $# -le 2 ] || usage
session_dir=$1
label=${2:-}

# Capture geometry is overridable because the camera that is reachable depends
# on how the OBSBOT software is running. When its app is open it holds the
# physical "OBSBOT Meet 2 StreamCamera" and exposes a processed feed as
# "OBSBOT Virtual Camera", which offers 1920x1080@60 only -- asking the held
# physical device for 4K fails with an I/O error, not a resolution error.
device_name=${HWCAM_DEVICE:-OBSBOT}
video_size=${HWCAM_VIDEO_SIZE:-3840x2160}
framerate=${HWCAM_FRAMERATE:-30}
# ~2.5s of warmup at the chosen rate; validated 2026-08-15 at 30fps.
warmup_frames=${HWCAM_WARMUP_FRAMES:-$((framerate * 5 / 2))}

# avfoundation device indices shift as cameras come and go; resolve by name,
# stopping before the audio-device section (the OBSBOT microphone also matches).
# The -list_devices invocation always exits nonzero by design (there is no real
# input to open), so its status must be tolerated explicitly under pipefail.
device_list=$(ffmpeg -hide_banner -f avfoundation -list_devices true -i "" 2>&1 || true)
device_index=$(printf '%s\n' "$device_list" |
    awk -v want="$device_name" '/audio devices:/ { exit }
         index($0, want) { if (match($0, /\[[0-9]+\]/)) { print substr($0, RSTART + 1, RLENGTH - 2); exit } }')

if [ -z "$device_index" ]; then
    echo "error: no camera matching '$device_name' among avfoundation video devices:" >&2
    printf '%s\n' "$device_list" | sed -n '/video devices/,/audio devices/p' >&2
    exit 1
fi

mkdir -p "$session_dir"
stamp=$(date -u +%Y%m%dT%H%M%SZ)
frame="$session_dir/$stamp${label:+-$label}.jpg"

if ! ffmpeg -hide_banner -loglevel error -f avfoundation -framerate "$framerate" \
    -video_size "$video_size" -i "$device_index" \
    -vf "select='gte(n,$warmup_frames)'" -frames:v 1 -y "$frame" >&2; then
    echo "error: capture failed from [$device_index] at $video_size@${framerate}fps" >&2
    echo "hint: if the OBSBOT app is running it holds the physical camera; try" >&2
    echo "      HWCAM_DEVICE='OBSBOT Virtual' HWCAM_VIDEO_SIZE=1920x1080 HWCAM_FRAMERATE=60" >&2
    exit 1
fi
if [ ! -s "$frame" ]; then
    echo "error: empty capture: $frame" >&2
    exit 1
fi
echo "$frame"
