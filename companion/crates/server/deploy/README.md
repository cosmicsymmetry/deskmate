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
sudo $EDITOR /etc/deskmate/server.env   # set DESKMATE_ADMIN_TOKEN for real
```

**`DESKMATE_ADMIN_TOKEN` is the whole V2 auth story for the Mac-facing
surface** -- the explicit stand-in for V3's accounts (spec §5.2). Generate it
with `openssl rand -hex 32` and paste it straight into the file; never type it
on a command line where it lands in shell history, and never commit
`/etc/deskmate/server.env` (it is deliberately outside this repo). The 0600
mode above is load-bearing.

### Device identity persistence

Provisioned identities survive normal process and host restarts. The registry
lives at `$DESKMATE_CONFIG_DIR/device-identities.json`, alongside the per-device
schema-v4 config files. It is a versioned JSON document containing each
`dev-NNNN` id and the lowercase SHA-256 digest of that device's bearer token.
Its historical schema-v1 field `next_sequence` stores the **last issued**
sequence; minting adds one. That counterintuitive field name is retained so an
already-written schema-v1 store remains compatible. The plaintext bearer token
is never written there and cannot be recovered from the file; the server only
returns it from `POST /v1/devices` at mint time.

The server creates the registry with mode `0600` and replaces it atomically via
a private temporary file, so a crash during a mint cannot leave a half-written
registry. The file is capped at 64 KiB (roughly 580 devices); an oversized file
is treated as invalid rather than read without a bound.

If the file is unreadable, corrupt, or truncated at startup, the server starts
with no authenticated identities so it can still serve the admin surface. It
does **not** silently overwrite the failed input: before the first replacement
mint, it renames the existing file to
`device-identities.json.corrupt-<unix-timestamp>` (adding a collision suffix if
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

The device and the Mac app both need a publicly-trusted TLS certificate --
that's what makes `esp_https_ota` and the WSS link work without embedding a
private CA on the device. Cloudflare Tunnel is the spec's chosen way to get
one without opening a port or managing certificates by hand.

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
