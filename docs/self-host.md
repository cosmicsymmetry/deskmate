# Run your own Deskmate server

The public repository builds the complete self-host runtime: accounts and first-run
setup, panel claiming, the web companion, card previews, clock/pomodoro/picture cards,
weather/Hacker News/RSS/token faces, external PNG producers, and local sandboxed plugins.
The bundle includes `bin/deskmate-server`, `bin/deskmate-cli`, `web/` (the built SPA),
and `faces/` (TypeScript, plugins and native dependencies). Bun remains installed on
this host. Build on the operating system and architecture where you will run it.

Hosted allowance tables, paid-policy code, billing UI/services and the managed reviewed
plugin directory are absent. There is no licence key or activation request. Install
plugins locally instead; there is no directory button or checkout stub to configure.
Self-host has no commercial panel cap. Existing safety bounds remain: eight cards per
panel, eight image sources per account, one current frame per source, 60-second minimum
scheduled refresh, and the instance's 32 concurrent-link guard. These are capacity
limits, not an invitation to upgrade. Proposed hosted CPU budgets are not implemented
by this package. Keep registration closed unless you intend to operate a shared service.

## 1. Prepare a machine

Use an existing Linux or macOS machine; no cloud account, tunnel, payment service or
paid API is required to reach setup. The instructions below use a native build, not
Docker. Allow several GB of free disk for the Rust build and dependency downloads.
Build time and runtime capacity depend on your machine; this is not a fleet capacity
promise.

On Debian/Ubuntu install the build prerequisites:

```sh
sudo apt-get update
sudo apt-get install -y build-essential pkg-config libudev-dev libssl-dev python3 curl git unzip ca-certificates
```

On macOS install Apple's command-line tools (`xcode-select --install`) and Python 3.
On either platform install Rust through [rustup](https://rustup.rs/) and
[Bun 1.3.8](https://bun.com/docs/installation). The repository pins Rust in
`companion/rust-toolchain.toml`; rustup installs that version on the first build.
For Bun's installer, pass the version explicitly: `bash install.sh bun-v1.3.8` after
downloading and inspecting `https://bun.com/install` as `install.sh`.

```sh
export PATH="$HOME/.cargo/bin:$HOME/.bun/bin:$PATH"
git clone https://github.com/cosmicsymmetry/deskmate.git
cd deskmate
tools/self-host/build.sh "$HOME/deskmate-release"
python3 "$HOME/deskmate-release/instance.py" init "$HOME/deskmate-instance"
python3 "$HOME/deskmate-release/instance.py" run "$HOME/deskmate-instance"
```

The bundle and instance destinations must be new absolute paths. Build downloads the
pinned LVGL and CBOR preview sources from the public Espressif registry, verifies their
content hashes, builds with Cargo's lockfile, builds the real SPA (even if the mock
variable was inherited), and installs the faces dependencies with Bun's lockfile.
No ESP-IDF installation, firmware build, private crate or existing managed-components
folder is needed. A mismatched existing component directory is refused; use a clean
checkout or move it aside explicitly. Do not copy native `node_modules` between hosts.

## 2. Reach first-run setup

Keep the terminal open. Visit **http://localhost:8443** in Chrome or Edge on the same
machine. You should see **Set up this server**. Copy the setup code from the server's
terminal, enter your email, and press **Set up**. The next screen is **Add your panel**.
No SMTP, Google client or physical panel is required for these steps. Subsequent email
sign-in links appear in the server log until SMTP is configured. Protect that log as
an authentication secret; do not publish it. `Ctrl-C` stops the server gracefully.

If the machine is remote, an SSH local forward allows this initial browser check:
`ssh -L 8443:127.0.0.1:8443 user@your-server`, then browse localhost on your computer.
This is only the setup check: a physical panel cannot reach your server at localhost.
Configure the HTTPS address below before claiming one.

A minimal generated `server.env` has this shape (the helper fills real absolute paths
and generates the random token; do not use the placeholder):

```sh
RUST_LOG=info
DESKMATE_SERVER_BIND=127.0.0.1:8443
DESKMATE_PUBLIC_URL=http://localhost:8443
DESKMATE_ADMIN_TOKEN=<random-64-hex-characters>
DESKMATE_CONFIG_DIR=/home/you/deskmate-instance/configs
DESKMATE_FIRMWARE_DIR=/home/you/deskmate-instance/firmware
DESKMATE_FIRMWARE_VERSION=v2.1.0-proto2
DESKMATE_WEB_DIR=/home/you/deskmate-release/web
DESKMATE_FACES_DIR=/home/you/deskmate-release/faces
DESKMATE_BUN=/home/you/.bun/bin/bun
DESKMATE_SIGNUPS=closed
```

`instance.py run` parses this as data, not as shell code. Use one `KEY=value` per line;
quote values with spaces. Shell expansion, `export`, command substitution and variable
references are not supported. The file must be mode 0600. The server binary itself
does not read `.env` files; use the helper or your service manager's environment-file
facility. Initialization refuses an existing destination so rerunning it cannot reset
your token or database.

## 3. Serve a panel over HTTPS

Use an existing domain you control that resolves to this machine, with TCP 80/443
reachable, and an installed [Caddy](https://caddyserver.com/docs/install) server.
Do not buy a domain or service just for the localhost check. A minimal Caddyfile is:

```caddyfile
desk.example.org {
    reverse_proxy 127.0.0.1:8443
}
```

Caddy handles WebSocket upgrades and [automatic HTTPS](https://caddyserver.com/docs/automatic-https). Set
`DESKMATE_PUBLIC_URL=https://desk.example.org` in your private `server.env`, restart
Deskmate, validate/reload your Caddy configuration, and open that HTTPS address.
Keep the backend on loopback. Do not add a host-wide Basic-auth gate: panel and image
producer bearer tokens must reach Deskmate unchanged. Do not expose the plaintext
backend port. Sign-in origin checks require the browser URL to match `PUBLIC_URL`.

Use **Add a panel** in Chrome/Edge on a computer connected to the panel by USB. The
claimed link URL will be `wss://desk.example.org/v1/device/link`. The panel's Wi-Fi
network must reach that hostname and trust its TLS certificate; browser trust in a
local development certificate does not establish panel trust. The Wi-Fi password goes
only over USB. See [the protocol](protocol/v2.md) for firmware compatibility; do not
change firmware merely to verify web setup.

For persistent Linux service operation, create a systemd unit with your actual user,
paths and installed Python. Keep bundle/data ownership with that unprivileged user:

```ini
[Unit]
Description=Deskmate self-host
After=network-online.target
Wants=network-online.target

[Service]
User=you
UMask=0077
ExecStart=/usr/bin/python3 /home/you/deskmate-release/instance.py run /home/you/deskmate-instance
Restart=on-failure
RestartSec=5
TimeoutStopSec=30
NoNewPrivileges=true

[Install]
WantedBy=multi-user.target
```

Save as `/etc/systemd/system/deskmate-self-host.service`, then run
`sudo systemctl daemon-reload` and `sudo systemctl enable --now deskmate-self-host`.
Read setup/sign-in messages with `sudo journalctl -u deskmate-self-host -f`.
Restrict journal access and configure log retention. On macOS, keep the foreground
process running for the local recipe; automatic service installation is not provided.

## Environment reference

Paths below should be absolute. Optional variables should be omitted, not set empty.

| Variable | Meaning / default |
| --- | --- |
| `DESKMATE_ADMIN_TOKEN` | Required CLI/recovery bearer; generated by init. It cannot sign into the browser. Keep secret. |
| `DESKMATE_PUBLIC_URL` | Required canonical origin; HTTPS except localhost/loopback development. Used for sign-in, cookies and claimed panel links. |
| `DESKMATE_SERVER_BIND` | Listener; default `127.0.0.1:8443`. No TLS in this listener. |
| `DESKMATE_CONFIG_DIR` | Entire persistent identity/account/config store; binary default `/var/lib/deskmate/configs`. Helper sets instance-local path. |
| `DESKMATE_FIRMWARE_DIR` | Firmware image directory; binary default `/var/lib/deskmate/firmware`. Helper creates an empty directory. |
| `DESKMATE_FIRMWARE_VERSION` | Required exact advertised firmware version. Helper reads the checkout's `firmware/version.txt`; see firmware caution below. |
| `DESKMATE_WEB_DIR` | Built SPA directory containing `index.html`. Read per request. Unset is API-only, without a browser UI. |
| `DESKMATE_FACES_DIR` | Separate faces directory containing `src/main.ts` and installed dependencies. Unset disables server-rendered faces, not external PNG producers. |
| `DESKMATE_BUN` | Absolute Bun executable; default `/usr/local/bin/bun`. Helper resolves installed Bun. |
| `DESKMATE_SIGNUPS` | `closed` (default) or `open`; seeds the stored setting. Owner changes it in account settings after setup. |
| `DESKMATE_OWNER_EMAIL` | Only for migration from legacy flat config; not required for a fresh instance. |
| `DESKMATE_SMTP_URL`, `DESKMATE_MAIL_FROM` | Both or neither. Without them, sign-in links are logged. Example: `smtps://user:password@smtp.example.org:465` and quoted `Deskmate <mail@example.org>`. URL-encode credential special characters. |
| `DESKMATE_GOOGLE_CLIENT_ID` | Optional operator-owned Google OAuth web client; requires a client secret. Sign-in derives its callback from `DESKMATE_PUBLIC_URL`. |
| `DESKMATE_GOOGLE_CLIENT_SECRET_FILE` | Preferred secret source, owner-only file (0600); wins over inline secret. |
| `DESKMATE_GOOGLE_CLIENT_SECRET` | Inline alternative, kept only in private environment. |
| `DESKMATE_GOOGLE_REDIRECT_URI` | Optional: enables the separate OAuth integration path, usually `https://desk.example.org/v1/integrations/google/callback`. Register this additional URI only when using integrations. Omit for sign-in alone. |
| `DESKMATE_SECRETS_KEY_FILE` | Required for the encrypted integration store when `DESKMATE_GOOGLE_REDIRECT_URI` is set; sign-in alone needs no store/key. File containing a base64-encoded 32-byte key; protect as 0600 outside the configs directory and back it up separately. |
| `DESKMATE_SECRETS_KEY` | Inline base64 alternative; file wins. Never replace the key on an existing encrypted store. |
| `DESKMATE_GOOGLE_AUTH_URI`, `DESKMATE_GOOGLE_TOKEN_URI`, `DESKMATE_GOOGLE_REVOKE_URI` | Normally unset. Defaults are Google's authorize/token/revoke endpoints; all overrides must be HTTPS and outbound token transport remains constrained. Not a generic OAuth-provider switch. |
| `RUST_LOG` | Default `info`. Keep info enabled for setup/log-mail. |

There is no `DESKMATE_DATA_CARDS`, licence, hosted-plan or billing environment variable.
Faces get a cleared environment; setting arbitrary API secrets in the server environment
does not make them available to plugins.

## Your integrations and plugins

SMTP and Google are optional. Use your own configured service/client and enable only
services whose costs you accept; the package provisions none. Generate a master key
with `umask 077; openssl rand -base64 32 > /absolute/private/secrets.key` if enabling
Google integrations, and store the client secret in another 0600 file. Google's sign-in setup is
separate from any future per-account plugin OAuth support.

For email delivery, set both SMTP variables above and restart. The sign-in page reports
whether delivery is by email or server log. Without SMTP, a person who cannot read the
log needs the server owner to retrieve their link; no email is sent. Email delivery
failures are recorded in the server log without revealing account existence in the UI.

For Google sign-in alone, create a Google OAuth **Web application** client, register
`https://desk.example.org/v1/app/auth/google/callback` as an authorized redirect URI
(using your actual public origin), and set `DESKMATE_GOOGLE_CLIENT_ID` plus
`DESKMATE_GOOGLE_CLIENT_SECRET_FILE`. Restart and use **Sign in with Google**. No
integration redirect or encryption key is needed. Configure the consent screen and its
test users/publishing status for your intended audience. Existing email-link accounts
are automatically matched only for Google-managed mailboxes (Gmail or Workspace);
other existing addresses must use an email link. Returning Google users are matched by
their stable Google identity even if their Google email changes.

Built-in weather uses Open-Meteo without a key; Hacker News and RSS need no built-in
credentials. RSS needs a public feed URL. Token uses CoinGecko; its renderer accepts
an optional `api_key` setting (Demo key), but the current editor does not expose that
field. An operator can set it in the account's `data-cards.json` while stopped; it is
plaintext configuration, not the encrypted integration store. Provider restrictions
or rate limits can prevent refresh and are shown as face errors. No provider access
or paid tier is bundled or guaranteed.

Install local plugins under `<bundle>/faces/plugins/<id>/` with `plugin.json` and
`index.js`; see [the plugin contract and worked example](plugins/contract-v1.md).
The bundled `github-stats` is a local example, not a managed directory listing.
The catalog is re-read periodically (allow a minute). Per-account plugin credentials
belong in `<instance>/configs/accounts/<account-id>/plugin-secrets.json`, mode 0600:

```json
{"plugin-id": {"manifest-secret-key": "your-value"}}
```

Use the exact keys declared by that plugin. These are plaintext operator-owned secrets;
the host substitutes them into permitted requests, not into sandbox code. Public
network/SSRF/sandbox guards remain in force. External producers instead obtain a
source-specific bearer and POST PNGs; follow [the picture producer guide](images/producer-guide.md).

## Persistence, updates and recovery

Keep the entire instance directory, its `server.env`, optional key files, and your local
plugins. Stop the service before copying configs so SQLite databases and related files
are consistent. Make a private backup of the whole instance, not just card JSON;
losing identities requires re-provisioning panels. Keep encrypted data and its key in
separate protected backups. Restore to an empty instance, retain permissions, adjust
absolute paths in `server.env`, and start the same software revision first. Never run
`init` over a restored instance.

Build updates from a new clean checkout into a new bundle path. Stop, back up, update
`DESKMATE_WEB_DIR`/`DESKMATE_FACES_DIR` and the service's helper path, restore local
plugins into the new faces directory, then restart. Keep the old bundle and pre-upgrade
data backup for rollback; a binary rollback alone cannot undo a data migration.
For a compatible UI-only update, replace the complete built `web/` directory; for a
faces-only update, install dependencies on the target host then replace `faces/`.
Neither is embedded in Rust. Web files are read per request and faces at render/catalog
use; no Rust rebuild is required. Prefer an atomic directory/symlink switch to a partial
copy while requests are active.

**Firmware is not included in this server bundle.** The empty firmware directory is
sufficient for first-run setup. The advertised version is a pin, including downgrades:
before connecting a real panel, ensure it matches that panel's installed compatible
firmware, or deliberately publish the matching `<version>.bin` there and set the exact
version. An empty directory cannot deliver OTA; a version mismatch would advertise an
unavailable update. Never substitute a random binary or reuse a failed firmware version.
The CLI is built for this host and performs cable maintenance; run it without arguments
for its usage. No hardware or OTA verification is implied by a successful server build.

## Verification record

Observed 2026-09-29 on macOS arm64, Rust 1.98.0, Bun 1.3.8, Python 3.14.7,
headless system Google Chrome. A new local Git clone at `6473074` had no managed
components, node_modules, target directory or private dependencies. The initial build
at `44784ae` downloaded/verified public components and built all artifacts; review then
added the missing face fonts. The clean checkout fast-forwarded to `6473074` and the
corrected bundle build passed, reusing its verified public downloads/build cache.

- Real server: `/v1/app/instance` reported self-hosted, setup required, Google disabled,
  signups closed. Chrome showed “Set up this server,” accepted the logged setup code,
  and showed “Add your panel.” No browser JavaScript errors.
- Graceful stop/restart preserved the owner and session. A separate browser signed in
  successfully using a link from the log. No SMTP, Google or physical panel used.
- Instance/config permissions were 0700/0600. Reinitialization refused without changing
  credentials; permissive environment-file permissions were refused before startup.
  A deliberately changed preview header was refused without overwriting its bytes.
- Bundled `github-stats` rendered a 10,765-byte PNG using one offline fixture response,
  zero external requests, QuickJS, Satori, resvg and the shipped Inter fonts. Visually
  inspected the PNG and the setup screenshot.
- Gates: fmt and Clippy passed; Rust all-targets 829 passed, 0 failed, 1 intentionally
  ignored fixture printer; doctest command passed (0 tests). Web 216 passed, with
  typecheck, format and production build passing. Faces 425 passed, with typecheck,
  lint and format passing. Local documentation links and `git diff --check` passed.

This verifies a clean source checkout on an existing toolchain host, not a freshly
installed operating system. Debian/systemd/Caddy installation, public HTTPS, actual
provider delivery, panel provisioning and OTA were not exercised. Repository access was restored later on 2026-09-29: authenticated `git fetch origin`
and `gh run list` succeeded. Current main is already contained in this branch.
Branch PR/CI verification and merge are tracked in the packaging work record.
