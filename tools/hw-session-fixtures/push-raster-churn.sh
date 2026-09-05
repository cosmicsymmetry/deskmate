#!/bin/bash
# Task 7 Step 10: drive >=20 raster revisions through the negotiated operator route.
# Run from the VM (or anywhere that reaches the server) with:
#   TOKEN=... BASE=http://192.168.8.20:8443 DEVICE=dev-0005 ./push-raster-churn.sh [count] [interval]
# Values are visibly distinct (AQI counts up) so every accepted frame is tellable
# apart on the panel. Timestamps print per push; pair them with the server journal's
# AssetBegin/PushScene lines for the floor evidence.
set -euo pipefail
: "${TOKEN:?set TOKEN}"; : "${BASE:?set BASE}"; : "${DEVICE:?set DEVICE}"
count=${1:-20}
interval=${2:-31}
for i in $(seq 1 "$count"); do
    aqi=$((40 + i * 7))
    printf '%s push %02d aqi=%d -> ' "$(date -u +%FT%TZ)" "$i" "$aqi"
    curl -s -X POST "$BASE/v1/devices/$DEVICE/scene" \
        -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
        -d "{\"card_id\":\"svg-aqi-card\",\"template\":\"plugin\",\"plugin_id\":\"svg-aqi\",\"data\":{\"current\":{\"aqi\":$aqi,\"category\":\"churn\"}},\"stale\":false,\"error\":null}"
    echo
    [ "$i" -lt "$count" ] && sleep "$interval"
done
