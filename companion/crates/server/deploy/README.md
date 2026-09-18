# Deploying the Deskmate server

Per the design spec's §5.4: "a single binary plus `cloudflared`, run under
systemd or launchd." This directory holds the unit files for both, plus this
runbook. A server that only ever runs from a developer shell is not deployed
-- these files, and a reproducible tunnel hostname, are what make it one.

## 1. Build the binary

```sh
cd companion
cargo build --release -p server
```

The binary is `companion/target/release/server`. Copy it to
`/usr/local/bin/deskmate-server` on the machine that will run it (both unit
files below assume that path):

```sh
sudo install -m 0755 target/release/server /usr/local/bin/deskmate-server
```

## 2. Configuration: the environment file

Bind address, firmware directory, device-config directory, and admin token are
read from environment variables (`src/main.rs`), never from a config file the
binary parses itself. Both unit files source them from **one env file at
`/etc/deskmate/server.env`** -- see `deskmate-server.env.example` in this
directory for the exact keys and comments.

```sh
sudo install -d -m 0755 /etc/deskmate
sudo install -m 0600 deskmate-server.env.example /etc/deskmate/server.env
sudo $EDITOR /etc/deskmate/server.env   # set token and published firmware version
```

`DESKMATE_FIRMWARE_VERSION` is required. Copy the exact value from
`firmware/version.txt` that corresponds to the image published in
`DESKMATE_FIRMWARE_DIR`; a mismatch can make the bidirectional catalog offer a
downgrade. The supplied systemd unit and launchd job already source this environment
file, so no second unit-local value should be added.

**`DESKMATE_ADMIN_TOKEN` controls the browser-facing operator surface.** Generate it
with `openssl rand -hex 32` and paste it straight into the file; never type it
on a command line where it lands in shell history, and never commit
`/etc/deskmate/server.env` (it is deliberately outside this repo). The 0600
mode above is load-bearing.

**`RUST_LOG` is load-bearing too, and its absence is silent.**
`tracing_subscriber::fmt::init()` defaults its `EnvFilter` to **ERROR** when
`RUST_LOG` is unset, so an unset value discards every device-link diagnostic --
including `device link established`, `device link refused: owner already live`,
and the unknown-token warnings. The startup line you still see comes from a
`println!`, so logging *looks* alive while reporting nothing. The launchd plist
sets `RUST_LOG` in its own `EnvironmentVariables`; the systemd unit reads this
file only, so a Linux deployment that omits it runs blind. A live server was
found in exactly that state on 2026-08-19, which made accepted connections
impossible to observe. Keep `info`
unless you have a reason not to; `info,server=debug` additionally logs each
device's firmware check, which is useful while diagnosing updates.

### Device identity persistence

Provisioned identities survive normal process and host restarts. The registry
lives at `$DESKMATE_CONFIG_DIR/device-identities.json`, alongside the per-device
config files (currently [schema v10](../../../../docs/config/v10.md); v4-v9 files
are migrated on load). It is a versioned JSON document containing each
`dev-NNNN` id and the lowercase SHA-256 digest of that device's bearer token.
Its historical schema-v1 field `next_sequence` stores the **last issued**
sequence; minting adds one. That counterintuitive field name is retained so an
already-written schema-v1 store remains compatible. The plaintext bearer token
is never written there and cannot be recovered from the file; the server only
returns it from `POST /v1/devices` at mint time.

The server creates the registry with mode `0600` and replaces it atomically via
a private temporary file, so a crash during a mint cannot leave a half-written
registry. The file is capped at 64 KiB, which is 492 devices -- 492 encode to 65,505 bytes
and 493 to 65,638, so mint 493 fails on the encoded-size check rather than at
any rounder number. An oversized file is treated as invalid rather than read
without a bound.

If the file is unreadable, corrupt, or truncated at startup, the server starts
with no authenticated identities so it can still serve the admin surface. It
does **not** silently overwrite the failed input: before the first replacement
mint, it renames the existing file to
`device-identities.json.corrupt-<unix-time-in-nanoseconds>` (adding a collision suffix if
needed). If that rename fails, minting fails and leaves the original untouched.
Only after the archive succeeds does the server atomically commit a replacement
store. Inspect or copy the archived bytes before deciding they are irreparable.

Because the failed store's sequence cannot be trusted, the server scans
canonical `dev-NNNN.json` config filenames and assigns the replacement above
their high-water mark. A replacement therefore cannot inherit an earlier
device's id-keyed playlist. Both unknown-token warning variants -- ordinary
unknown token and identity-store load failure -- share the same process-wide
one-warning-per-minute limiter, and neither includes any token bytes.

Back up `device-identities.json` with the rest of the state directory. Losing
it does not expose the bearer tokens, but it does invalidate every existing
device identity: mint a replacement identity and provision it over USB. There
is no token-recovery procedure because only digests are stored.

Create the read-only firmware directory the env file points at:

```sh
sudo install -d -m 0755 /var/lib/deskmate/firmware
```

`DESKMATE_CONFIG_DIR` defaults to `/var/lib/deskmate/configs`; both it and
`DESKMATE_FIRMWARE_DIR` must be absolute paths. Keep both under
`/var/lib/deskmate` when using the supplied systemd unit. Do not pre-create
`configs` as a root-owned directory for the `DynamicUser`: systemd gives the
service ownership of the `StateDirectory` parent, and the server creates its
state subdirectory on the first successful identity or config write. For
launchd, point `DESKMATE_CONFIG_DIR` at an absolute directory writable by the
account running the agent/daemon.

### Server-rendered data cards (optional)

If this deployment draws its own weather, RSS or token faces, it also needs a spec
file. `data-cards.json.example` in this directory is a starting point;
`docs/images/server-rendered-cards.md` is the reference.

```sh
# One image source per card. Keep the id; the token is only needed by an
# external producer, and the server pushes to its own store directly.
curl -sX POST https://deskmate.rodi.one/v1/images \
  -H "Authorization: Bearer $DESKMATE_ADMIN_TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"name": "Weather"}'

sudo install -m 0644 data-cards.json.example \
  /var/lib/deskmate/configs/data-cards.json
sudo $EDITOR /var/lib/deskmate/configs/data-cards.json
```

Then add one `picture` card per source in the browser companion, naming the same
`source_id`, and restart the server.

Two consequences worth stating before you enable this:

- **The server makes outbound HTTP again.** Only to Open-Meteo, CoinGecko and whatever
  feed URLs the specs name, and every request goes through the SSRF guard in
  `crates/server/src/egress.rs` -- scheme checks, the RFC1918/loopback/link-local/
  metadata deny list, resolve-then-pin against DNS rebinding, per-hop re-validation
  across redirects, a body cap and a wall-clock budget. But the surface is non-zero
  again, where between `e137294` and `feat/server-side-cards` it was zero.
- **A malformed spec file fails the start.** Deliberately: a server that came up with
  cards silently missing presents as "the panel stopped updating" with nothing in the
  log. Unknown fields are refused too, so a typo does not quietly take a default.

## 3a. Run under systemd (Linux)

```sh
sudo install -m 0644 deskmate-server.service /etc/systemd/system/deskmate-server.service
sudo systemctl daemon-reload
sudo systemctl enable --now deskmate-server
sudo systemctl status deskmate-server
journalctl -u deskmate-server -f
```

The unit runs as a `DynamicUser` with `ProtectSystem=strict`; its only
writable path is `/var/lib/deskmate` (via `StateDirectory=deskmate`), which is
where `/etc/deskmate/server.env`'s `DESKMATE_FIRMWARE_DIR` and
`DESKMATE_CONFIG_DIR` should live.

## 3b. Run under launchd (macOS)

launchd has no `EnvironmentFile=` equivalent, so the plist's
`ProgramArguments` shells out to source `/etc/deskmate/server.env` before
exec'ing the binary (see the comment in the plist itself):

```sh
# Per-user agent (runs at login as your user):
cp com.deskmate.server.plist ~/Library/LaunchAgents/
launchctl load ~/Library/LaunchAgents/com.deskmate.server.plist

# Or system-wide daemon (runs at boot as root):
sudo cp com.deskmate.server.plist /Library/LaunchDaemons/
sudo launchctl load /Library/LaunchDaemons/com.deskmate.server.plist
```

Logs go to `/usr/local/var/log/deskmate-server.log` (create the directory
first: `sudo install -d -m 0755 /usr/local/var/log`, or a LaunchDaemon running
as root will fail to open the log and launchd will just keep restarting it).

To stop and unload: `launchctl unload <path-to-plist>`.

## 4. Expose it: the Cloudflare Tunnel

The device's HTTPS/WSS connections and the browser companion need a
publicly-trusted TLS certificate. For the device, that keeps `esp_https_ota` and
the WSS link working without embedding a private CA. Cloudflare Tunnel is the
deployment's chosen way to provide one without opening a port or managing
certificates by hand.

Install `cloudflared` (`brew install cloudflared` on macOS, or the equivalent
package for your Linux distribution), then create a **named** tunnel -- named,
not `cloudflared tunnel --url ...`, because a named tunnel's hostname is
stable across restarts, which is the whole point of this runbook:

```sh
cloudflared tunnel login
cloudflared tunnel create deskmate-server
```

Note the tunnel UUID it prints, then create `/etc/cloudflared/config.yml` (or
`~/.cloudflared/config.yml` for a per-user launchd agent):

```yaml
tunnel: deskmate-server
credentials-file: /etc/cloudflared/<tunnel-uuid>.json

ingress:
  - hostname: deskmate.example.com   # your real hostname
    service: http://127.0.0.1:8443   # must match DESKMATE_SERVER_BIND
  - service: http_status:404
```

Route the DNS record once:

```sh
cloudflared tunnel route dns deskmate-server deskmate.example.com
```

Run the tunnel itself as its own service -- `cloudflared` ships its own
`cloudflared service install` command, which installs a systemd unit or
launchd job for the tunnel the same way this directory's files do for the
server:

```sh
sudo cloudflared service install
sudo systemctl enable --now cloudflared   # Linux
# or, on macOS, cloudflared service install registers a launchd daemon directly
```

`DESKMATE_SERVER_BIND` should stay on loopback (`127.0.0.1:8443`, the example
default) -- the tunnel is the only thing that ever needs to reach the server
from outside, so there is no reason to bind a public interface at all.

## 5. Verify

```sh
curl -i https://deskmate.example.com/v1/device/firmware?current=0.0.0 \
  -H "Authorization: Bearer <a-token-minted-for-a-test-device>"
```

A `200` with a `{"version": ..., "url": ...}` body (or a `204` if `0.0.0`
somehow matches `DESKMATE_FIRMWARE_VERSION`) confirms the whole chain --
`cloudflared` tunnel, TLS, and the running binary -- end to end.

## 6. Redeploying an already-running server

**Use `deploy.sh` in this directory.** It does everything below, refuses to run
against a dirty tree (the export is from `HEAD`, so a dirty tree deploys code nobody
can reproduce), skips the restart when the binary came out byte-identical, and
checks the public endpoint afterwards.

```sh
companion/crates/server/deploy/deploy.sh              # binary + UI
companion/crates/server/deploy/deploy.sh --ui-only    # just the companion
companion/crates/server/deploy/deploy.sh --dry-run    # build, do not install
```

Measured on the live deployment, 2026-09-18:

| | |
|---|---|
| UI-only change | **1.3 s** — no Rust build, no restart |
| No-op deploy | **3.8 s** — build finishes in 0.12 s, restart skipped |
| Real server change | **23 s** — 17 s of that is the container build |
| The same no-op, before this | **96 s** |

### Why it is fast, and what breaks if you hand-roll it

The prose recipe below was correct and still took **96 seconds to produce a
byte-identical binary**, because the container it launched kept `CARGO_HOME` and
`RUSTUP_HOME` inside the image: `--rm` threw the crate registry away after every
run, so each deploy re-downloaded 229 crates and re-synced the toolchain in order
to compile nothing. Mounting both from the VM makes that **0.7 seconds**.

`target/` was already preserved, which is why this looked solved and was not: it
caches compilation *output*, while the registry and the dependency sources live in
`CARGO_HOME`. Half the cache was configured; the missing half was 90% of the time.

Two things must stay true or the speed goes away:

- **`--exclude 'target/'` on the companion rsync.** It is a 1.6 GB build cache and
  `--delete` will take it.
- **`--checksum` on every rsync.** `git archive` stamps each file with the commit
  time, so a fresh export after any commit gives every file a new mtime and cargo
  rebuilds all seven workspace crates regardless of content. Checksum mode skips
  files whose content matches, and a skipped file keeps its old mtime.

The persistent caches live at `~/deskmate-build/.cargo` and `~/deskmate-build/.rustup`
on the VM, seeded once from the image. If they are ever lost, the next build
re-creates them at the cost of one slow run; if the toolchain pin in
`companion/rust-toolchain.toml` moves, delete `.rustup` so the new one is fetched.

### The hand-rolled version


This is the recipe the live deployment at `deskmate.rodi.one` actually uses. There is no
Rust toolchain on the VM: the binary is cross-built in a throwaway container over an
rsync'd source export.

**Reach the VM over Tailscale** (`docker-vm`, `100.93.166.123`). `~/.ssh/config` pins its
LAN address (`192.168.8.20`), which is unreachable from any other network, so use the
Tailscale address explicitly.

**Export from `git archive HEAD`, never from the working tree.** A dirty tree deploys code
nobody can reproduce.

**The payload is `companion/` plus part of `firmware/`, and nothing the server compiles
may live outside it.** An `include_bytes!` path that climbs out of the payload builds
fine on the Mac and then fails to compile on the VM, where that path does not exist.
This has happened three times: twice for the bundled Inter faces (now at
`companion/crates/server/assets/fonts/`), and once when the card preview moved into the
server -- `lvgl-sim` compiles the firmware's own LVGL, scene decoder and fonts, which
live under `firmware/`.

**`firmware/managed_components/` is gitignored**, so it cannot come from `git archive`:
those are third-party sources the ESP-IDF component manager fetches, pinned by the
tracked `firmware/dependencies.lock`. They are the one part of the payload copied from
the working tree, and they are copied without `tests/`, `demos/`, `docs/`, `scripts/`
and `examples/`, which are 121 MB of the 180 MB and none of it compiled. If the VM's copy
is ever lost or suspect, `idf.py -C firmware reconfigure` refetches exactly what the lock
file names.

To check before deploying, build the export in isolation:

```sh
rm -rf /tmp/deskmate-exportcheck && mkdir -p /tmp/deskmate-exportcheck
git archive HEAD companion firmware | tar -x -C /tmp/deskmate-exportcheck
rsync -a --exclude 'tests/' --exclude 'demos/' --exclude 'docs/' --exclude 'scripts/' \
  --exclude 'examples/' firmware/managed_components/ \
  /tmp/deskmate-exportcheck/firmware/managed_components/
(cd /tmp/deskmate-exportcheck/companion && cargo build --release -p server)
```

```sh
# On the Mac, from the repository root:
rm -rf /tmp/deskmate-deploy && mkdir -p /tmp/deskmate-deploy
git archive HEAD companion firmware | tar -x -C /tmp/deskmate-deploy

# Sources only. `target/` on the VM is root-owned and left by the previous deploy:
# keeping it turns a cold build into roughly 40 seconds, so the exclude below is what
# protects it from --delete. Never drop it.
rsync -a --delete --exclude 'target/' \
  /tmp/deskmate-deploy/companion/ rodion@100.93.166.123:~/deskmate-build/companion/

# The tracked firmware sources. `managed_components/` is excluded here because it is
# not in the export at all -- it is synced separately, below.
rsync -a --delete --exclude 'managed_components/' \
  /tmp/deskmate-deploy/firmware/ rodion@100.93.166.123:~/deskmate-build/firmware/

# The component-manager sources, from the working tree. Only needed when they change,
# which is when firmware/dependencies.lock changes.
rsync -a --delete --exclude 'tests/' --exclude 'demos/' --exclude 'docs/' \
  --exclude 'scripts/' --exclude 'examples/' \
  firmware/managed_components/lvgl__lvgl/ \
  rodion@100.93.166.123:~/deskmate-build/firmware/managed_components/lvgl__lvgl/
rsync -a --delete firmware/managed_components/espressif__cbor/ \
  rodion@100.93.166.123:~/deskmate-build/firmware/managed_components/espressif__cbor/
```

Build and install on the VM. The container image must match
`companion/rust-toolchain.toml` (`1.98.0`). **The bin target is `server`, not
`deskmate-server`** -- the installed file is renamed on the way in, and
`-p server --bin deskmate-server` fails with "no bin target named".

```sh
ssh rodion@100.93.166.123
cd ~/deskmate-build
sudo -n docker run --rm -v "$PWD":/work -w /work/companion rust:1.98-bookworm \
  cargo build --release -p server
sudo -n cp -a /usr/local/bin/deskmate-server "/usr/local/bin/deskmate-server.bak-$(date -u +%Y%m%dT%H%M%SZ)"
sudo -n install -m 0755 companion/target/release/server /usr/local/bin/deskmate-server
```

The backup name carries a **UTC timestamp, not just a date**, because more than one
redeploy a day is now normal and `cp -a` overwrites silently: a date-only name meant the
second deploy of a day destroyed the first one's rollback target while reporting success.
It also stamps UTC deliberately — the VM runs UTC while the Mac driving the deploy may not,
so a local date can name a backup for the wrong day. Note the two same-day names already on
the box (`.bak-20260910`, `.bak-20260910-rle`) are the scar from that.

### The browser companion

The UI is a directory of built assets, not something compiled into the binary, so
shipping a UI change is a copy and needs no Rust build and no restart -- the server
reads each file per request.

```sh
# On the Mac:
(cd companion/apps/deskmate && bun run build)
rsync -a --delete companion/apps/deskmate/dist/ rodion@100.93.166.123:/tmp/deskmate-web/

# On the VM. The service runs under DynamicUser, so its StateDirectory really lives at
# /var/lib/private/deskmate and the files must be owned by deskmate-server:
sudo -n rsync -a --delete --chown=deskmate-server:deskmate-server \
  /tmp/deskmate-web/ /var/lib/private/deskmate/web/
```

`DESKMATE_WEB_DIR=/var/lib/deskmate/web` in `/etc/deskmate/server.env` is what mounts it.
The server refuses to start if that path is set and has no `index.html`, rather than
serving 404s that look like a routing bug.

**A binary change still needs the install and restart below. A UI-only change does not.**

Restart and read the service log:

```sh
ssh rodion@100.93.166.123
sudo -n systemctl restart deskmate-server
sudo -n journalctl -u deskmate-server -n 5 --no-pager
```

**Redeploy whenever the config schema moves.** The server
compiles its own `CURRENT_SCHEMA_VERSION` in, so a schema bump on the app side does
nothing to a live deployment until the binary is replaced; the symptom is a typed
"schema version N is not supported; expected M" on the first save, which reads like a
config problem rather than a deploy problem.

### Picture-card rollout

The order is load-bearing:

1. **Redeploy the server binary first.** It compiles its own
   `CURRENT_SCHEMA_VERSION`, so until the binary moves every v9 save is refused with the
   typed `schema version 9 is not supported; expected 8` error.
2. Mint the image source and capture its plaintext token. It is shown once.
3. Point the producer at the picture webhook and send its PNG.
4. Save the v9 config that names the source.
