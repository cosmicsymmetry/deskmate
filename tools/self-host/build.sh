#!/usr/bin/env bash
# Build a native public bundle. Run on the machine/architecture that will serve it.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
DEST="${1:?usage: build.sh /absolute/new/bundle-directory}"
[[ "$DEST" = /* && ! -e "$DEST" ]] || { echo 'Destination must be absolute and must not exist.' >&2; exit 1; }
for tool in python3 curl cargo bun; do
    command -v "$tool" >/dev/null || { echo "build.sh needs $tool on PATH" >&2; exit 1; }
done
python3 "$ROOT/tools/self-host/fetch-components.py"
(cd "$ROOT/companion" && cargo build --target-dir "$ROOT/companion/target" --locked --release -p server -p deskmate-cli)
(cd "$ROOT/companion/apps/deskmate" && bun install --frozen-lockfile && env -u VITE_DESKMATE_MOCK bun run build)
mkdir -p "$DEST/bin" "$DEST/web" "$DEST/faces"
cp "$ROOT/companion/target/release/server" "$DEST/bin/deskmate-server"
cp "$ROOT/companion/target/release/deskmate-cli" "$DEST/bin/"
cp -R "$ROOT/companion/apps/deskmate/dist/." "$DEST/web/"
# Keep the package separate and install its native dependencies for THIS host.
cp -R "$ROOT/companion/faces/src" "$ROOT/companion/faces/plugins" "$ROOT/companion/faces/assets" "$DEST/faces/"
cp "$ROOT/companion/faces/package.json" "$ROOT/companion/faces/bun.lock" "$DEST/faces/"
(cd "$DEST/faces" && bun install --frozen-lockfile --production && bun run src/main.ts describe > /dev/null)
cp "$ROOT/tools/self-host/instance.py" "$DEST/"
cp "$ROOT/firmware/version.txt" "$DEST/firmware-version.txt"
printf '%s\n' 'Built. Next: python3 <bundle>/instance.py init /absolute/new/instance'
