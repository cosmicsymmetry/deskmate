#!/bin/bash
# usage: smoke.sh <checkout> <label> [port]
# Builds the real server and dist/ from <checkout>, starts it on localhost against a throwaway config dir,
# drives the real page in headless Chrome, then sends SIGTERM WHILE THE PAGE IS STILL OPEN and times the drain.
HERE="$(cd "$(dirname "$0")" && pwd)"; TREE="${1:?tree}"; L="${2:?label}"; PORT="${3:-5310}"
export PATH="$HOME/.cargo/bin:$HOME/.bun/bin:/opt/homebrew/bin:$PATH"; export CARGO_TERM_COLOR=never
OUT="${TMPDIR:-/tmp}/deskmate-smoke/$L"; rm -rf "$OUT"; mkdir -p "$OUT/config" "$OUT/fw"
( cd "$TREE/companion" && cargo build -p server > "$OUT/build.log" 2>&1 ) || { echo "BUILD FAILED"; tail -5 "$OUT/build.log"; exit 1; }
( cd "$TREE/companion/apps/deskmate" && bun run build > "$OUT/webbuild.log" 2>&1 ) || { echo "WEB BUILD FAILED"; exit 1; }
BIN=$(ls -t "$TREE"/companion/target/debug/deskmate-server "$TREE"/companion/target/debug/server 2>/dev/null | head -1); TOKEN=$(openssl rand -hex 32)
DESKMATE_FIRMWARE_VERSION="$(tr -d "[:space:]" < "$TREE/firmware/version.txt")" DESKMATE_SERVER_BIND=127.0.0.1:$PORT DESKMATE_ADMIN_TOKEN=$TOKEN DESKMATE_CONFIG_DIR="$OUT/config" DESKMATE_FIRMWARE_DIR="$OUT/fw" DESKMATE_WEB_DIR="$TREE/companion/apps/deskmate/dist" RUST_LOG=info "$BIN" > "$OUT/server.log" 2>&1 &
PID=$!; for i in $(seq 1 50); do curl -s -o /dev/null "http://localhost:$PORT/" && break; sleep 0.2; done
echo "server pid=$PID bin=$(basename $BIN) up=$(curl -s -o /dev/null -w '%{http_code}' http://localhost:$PORT/)"
python3 "$HERE/smoke.py" "http://localhost:$PORT" "$TOKEN" "$OUT" 2> "$OUT/smoke.err" &
SP=$!; for i in $(seq 1 300); do [ -f "$OUT/page-open" ] && break; kill -0 $SP 2>/dev/null || break; sleep 0.2; done
T0=$(python3 -c 'import time;print(time.time())'); kill -TERM $PID; for i in $(seq 1 150); do kill -0 $PID 2>/dev/null || break; sleep 0.1; done
T1=$(python3 -c 'import time;print(time.time())'); ALIVE=no; kill -0 $PID 2>/dev/null && { ALIVE=yes; kill -KILL $PID; }
echo "GRACEFUL SHUTDOWN with the page (SSE) still open: $(python3 -c "print(round($T1-$T0,2))")s  still-alive-after-15s=$ALIVE"
touch "$OUT/server-stopped"; wait $SP 2>/dev/null; tail -3 "$OUT/smoke.err" | cut -c1-300
grep -c -i -E ' (ERROR|WARN) ' "$OUT/server.log" | sed 's/^/server WARN+ERROR lines: /'
