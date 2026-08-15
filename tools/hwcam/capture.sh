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

# avfoundation device indices shift as cameras come and go; resolve by name,
# stopping before the audio-device section (the OBSBOT microphone also matches).
# The -list_devices invocation always exits nonzero by design (there is no real
# input to open), so its status must be tolerated explicitly under pipefail.
device_list=$(ffmpeg -hide_banner -f avfoundation -list_devices true -i "" 2>&1 || true)
device_index=$(printf '%s\n' "$device_list" |
    awk '/audio devices:/ { exit }
         /OBSBOT/ { if (match($0, /\[[0-9]+\]/)) { print substr($0, RSTART + 1, RLENGTH - 2); exit } }')

if [ -z "$device_index" ]; then
    echo "error: no OBSBOT camera among avfoundation video devices:" >&2
    printf '%s\n' "$device_list" | sed -n '/video devices/,/audio devices/p' >&2
    exit 1
fi

mkdir -p "$session_dir"
stamp=$(date -u +%Y%m%dT%H%M%SZ)
frame="$session_dir/$stamp${label:+-$label}.jpg"

if ! ffmpeg -hide_banner -loglevel error -f avfoundation -framerate 30 \
    -video_size 3840x2160 -i "$device_index" \
    -vf "select='gte(n,75)'" -frames:v 1 -y "$frame" >&2; then
    echo "error: capture failed" >&2
    exit 1
fi
if [ ! -s "$frame" ]; then
    echo "error: empty capture: $frame" >&2
    exit 1
fi
echo "$frame"
