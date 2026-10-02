#!/bin/bash
# usage: smoke.sh <checkout> <label> [port]
# Builds the real server and dist/ from <checkout>, starts it on localhost against a throwaway config dir,
# drives the real page in headless Chrome, then sends SIGTERM WHILE THE PAGE IS STILL OPEN and times the drain.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"; TREE="${1:?tree}"; L="${2:?label}"; PORT="${3:-5310}"
export PATH="$HOME/.cargo/bin:$HOME/.bun/bin:/opt/homebrew/bin:$PATH"; export CARGO_TERM_COLOR=never
OUT="${TMPDIR:-/tmp}/deskmate-smoke/$L"; rm -rf "$OUT"; mkdir -p "$OUT/config" "$OUT/fw"
PID=; SP=
cleanup() {
    for pid in "$SP" "$PID"; do
        if [ -n "$pid" ]; then
            kill -KILL "$pid" 2>/dev/null || true
            wait "$pid" 2>/dev/null || true
        fi
    done
}
trap cleanup EXIT
( cd "$TREE/companion" && cargo build -p server > "$OUT/build.log" 2>&1 ) || { echo "BUILD FAILED" >&2; tail -5 "$OUT/build.log" >&2; exit 1; }
( cd "$TREE/companion/apps/deskmate" && bun run build > "$OUT/webbuild.log" 2>&1 ) || { echo "WEB BUILD FAILED" >&2; cat "$OUT/webbuild.log" >&2; exit 1; }
BIN="$TREE/companion/target/debug/server"; TOKEN=$(openssl rand -hex 32)
DESKMATE_PUBLIC_URL="http://localhost:$PORT" DESKMATE_FIRMWARE_VERSION="$(tr -d "[:space:]" < "$TREE/firmware/version.txt")" DESKMATE_SERVER_BIND=127.0.0.1:$PORT DESKMATE_ADMIN_TOKEN=$TOKEN DESKMATE_CONFIG_DIR="$OUT/config" DESKMATE_FIRMWARE_DIR="$OUT/fw" DESKMATE_WEB_DIR="$TREE/companion/apps/deskmate/dist" RUST_LOG=info "$BIN" > "$OUT/server.log" 2>&1 &
PID=$!; READY=false
for i in $(seq 1 50); do
    kill -0 "$PID" 2>/dev/null || break
    if curl -fs --max-time 1 -o /dev/null "http://localhost:$PORT/"; then READY=true; break; fi
    sleep 0.2
done
if [ "$READY" = false ] || ! kill -0 "$PID" 2>/dev/null; then
    echo "SERVER STARTUP FAILED (see $OUT/server.log)" >&2
    cat "$OUT/server.log" >&2
    exit 1
fi
echo "server pid=$PID bin=$(basename "$BIN") ready"
python3 "$HERE/smoke.py" "http://localhost:$PORT" "$TOKEN" "$OUT" 2> "$OUT/smoke.err" &
SP=$!
for i in $(seq 1 300); do
    [ -f "$OUT/page-open" ] && break
    kill -0 "$SP" 2>/dev/null || break
    sleep 0.2
done
if [ ! -f "$OUT/page-open" ]; then
    if kill -0 "$SP" 2>/dev/null; then
        echo "BROWSER TIMED OUT (see $OUT/smoke.err)" >&2
        exit 1
    fi
    result=0; wait "$SP" || result=$?; SP=
    cat "$OUT/smoke.err" >&2
    [ "$result" -ne 0 ] || result=1
    exit "$result"
fi
T0=$(python3 -c 'import time;print(time.monotonic())')
kill -TERM "$PID"
for i in $(seq 1 150); do kill -0 "$PID" 2>/dev/null || break; sleep 0.1; done
T1=$(python3 -c 'import time;print(time.monotonic())')
ALIVE=no
if kill -0 "$PID" 2>/dev/null; then ALIVE=yes; kill -KILL "$PID"; fi
server_result=0; wait "$PID" || server_result=$?; PID=
echo "GRACEFUL SHUTDOWN with the page (SSE) still open: $(python3 -c "print(round($T1-$T0,2))")s  still-alive-after-15s=$ALIVE"
touch "$OUT/server-stopped"
result=0; wait "$SP" || result=$?; SP=
cat "$OUT/smoke.err" >&2
[ "$result" -eq 0 ] || exit "$result"
[ "$ALIVE" = no ] && [ "$server_result" -eq 0 ] || exit 1
warnings=$(grep -c -i -E ' (ERROR|WARN) ' "$OUT/server.log" || true)
echo "server WARN+ERROR lines: $warnings"
