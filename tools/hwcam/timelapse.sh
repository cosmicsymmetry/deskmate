#!/bin/bash
# Periodic Deskmate panel captures for soak monitoring. Runs until killed.
#
# Usage: timelapse.sh <session-dir> <interval-seconds> [label]
# interval-seconds is the sleep BETWEEN shots; each shot itself holds the
# camera for ~5s, so the true period is roughly interval + 5s.
#
# A failed shot is logged and the loop continues: transient camera
# exclusivity (e.g. a video call) must not kill a soak.
set -euo pipefail

usage() {
    echo "usage: $0 <session-dir> <interval-seconds> [label]" >&2
    exit 2
}
[ $# -ge 2 ] && [ $# -le 3 ] || usage
session_dir=$1
interval=$2
label=${3:-}
script_dir=$(cd "$(dirname "$0")" && pwd)

case $interval in
    '' | *[!0-9]*) usage ;;
esac
if [ "$interval" -lt 15 ]; then
    echo "error: interval must be >= 15s (each shot holds the camera ~5s)" >&2
    exit 2
fi

mkdir -p "$session_dir"
log="$session_dir/timelapse.log"
echo "$(date -u +%Y%m%dT%H%M%SZ) start interval=${interval}s label=${label:-none}" >>"$log"
trap 'echo "$(date -u +%Y%m%dT%H%M%SZ) stop" >>"$log"' EXIT

while :; do
    if ! "$script_dir/capture.sh" "$session_dir" ${label:+"$label"} >>"$log" 2>&1; then
        echo "$(date -u +%Y%m%dT%H%M%SZ) capture FAILED (continuing)" >>"$log"
    fi
    sleep "$interval"
done
