#!/bin/bash
# Record a short clip of the Deskmate panel, for observations that are about
# *when* something happened rather than what it shows.
#
# Usage: record.sh <session-dir> <seconds> [label]
# Prints the produced clip path on stdout. Diagnostics go to stderr.
#
# Why a clip and not stills: the only honest way to measure tap latency is to
# see the finger and the panel change in the same timebase. A still cannot do
# that, and comparing a Mac timestamp against the server's journal measures the
# two hosts' clock offset as much as the device. At 60fps one frame is 16.7 ms,
# which is finer than the quantity being measured.
#
# Extract frames from the clip with:
#   ffmpeg -i <clip> -vf fps=60 -q:v 3 <dir>/%05d.jpg
# Latency is (frame of first panel change - frame of finger contact) / 60.
set -euo pipefail

usage() {
    echo "usage: $0 <session-dir> <seconds> [label]" >&2
    exit 2
}
[ $# -ge 2 ] && [ $# -le 3 ] || usage
session_dir=$1
seconds=$2
label=${3:-}

case $seconds in
    '' | *[!0-9]*) usage ;;
esac

# Same device-resolution rules as capture.sh: see its comments for why the
# virtual camera is usually the reachable one, and why 4K fails when the
# OBSBOT app holds the physical device.
device_name=${HWCAM_DEVICE:-OBSBOT}
video_size=${HWCAM_VIDEO_SIZE:-1920x1080}
framerate=${HWCAM_FRAMERATE:-60}

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
clip="$session_dir/$stamp${label:+-$label}.mp4"

echo "recording ${seconds}s at ${framerate}fps -- tap when you see 'press any key' scroll past" >&2
# -t bounds the recording; -c:v libx264 -preset ultrafast keeps the writer ahead
# of a 60fps 1080p feed without dropping frames, which is the whole point.
if ! ffmpeg -hide_banner -loglevel error -f avfoundation -framerate "$framerate" \
    -video_size "$video_size" -i "$device_index" -t "$seconds" \
    -c:v libx264 -preset ultrafast -crf 18 -pix_fmt yuv420p -y "$clip" >&2; then
    echo "error: recording failed from [$device_index] at $video_size@${framerate}fps" >&2
    echo "hint: if the OBSBOT app is running it holds the physical camera; try" >&2
    echo "      HWCAM_DEVICE='OBSBOT Virtual'" >&2
    exit 1
fi
if [ ! -s "$clip" ]; then
    echo "error: empty recording: $clip" >&2
    exit 1
fi
echo "$clip"
