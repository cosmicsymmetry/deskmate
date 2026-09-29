# deskmate

Deskmate is a monitor-clip ESP32-S3 AMOLED display with a Rust server and a React web
companion. The current product description is [PRODUCT.md](PRODUCT.md), and the
repository’s durable implementation contract is [CLAUDE.md](CLAUDE.md).

## Current state

The companion is a browser application served by `companion/crates/server`; its React SPA
lives in `companion/apps/deskmate`. The current application config is frozen at
[schema v10](docs/config/v10.md), and the wire contract is frozen at
[protocol v2](docs/protocol/v2.md). Clock and pomodoro render as host-built scenes;
picture frames use durable device assets. What comes next is the
[roadmap](docs/roadmap.md); what came before is the [project history](docs/history.md). Hardware observations and unresolved board gates are
recorded in [board notes](docs/hardware/board-notes.md).

## Self-hosting

[Build and run your own server](docs/self-host.md), including first-run setup, keys,
HTTPS and backups. No owner-specific tunnel or private crate is required.

## Web companion

For UI work without a server or device, run the browser harness:

```sh
cd companion/apps/deskmate
VITE_DESKMATE_MOCK=1 bun run dev
```

Production builds are static assets served by the Rust server. `DESKMATE_WEB_DIR` must be
an absolute path to `apps/deskmate/dist`; the complete server environment and deployment
procedure are in the [server runbook](companion/crates/server/deploy/README.md).

The web companion authors the card loop, previews built-in cards, and configures the
network-owned device. Provisioning remains a USB cable operation.

Frontend checks run from `companion/apps/deskmate`:

```sh
bun run format:check
bun run lint
bun run check
bun test
bun run build
```

## Firmware

ESP-IDF 5.x is required. The repository is developed against the version installed at
`~/esp/esp-idf`:

```sh
. "$HOME/esp/esp-idf/export.sh"
idf.py -C firmware set-target esp32s3
idf.py -C firmware build
idf.py -C firmware -p /dev/cu.usbmodem* flash monitor
```

## CLI inspection

`deskmate-cli` is a manual inspection path over the cable: `status`, `time-sync`,
`push-data`, `provision` and `factory-reset`. See the
[M1 protocol/CLI plan](docs/superpowers/plans/2026-08-04-deskmate-m1-protocol-link-cli.md)
for what each one sends.

The M2 config/demo commands were removed on 2026-09-11. They pushed `ApplyConfig`
and `PushData` with templates, and since stage 3a the device draws only host-pushed
scenes, so none of them could put a face on the panel.

## Verification

Run the narrowest relevant checks while iterating, then the full applicable set before
handoff:

```sh
make -C firmware/host_tests clean test
make -C firmware/host_tests sanitize
. "$HOME/esp/esp-idf/export.sh"
idf.py -C firmware build
```

`sanitize` is not optional for firmware work: two of `core/scene_decode.c`'s bounds
guard out-of-bounds *writes* that `scene_model_validate()` then reports with the same
error code the test asserts, so the plain suite passes against a decoder with both
deleted. ASan is their only proof, and CI now runs it too.

For companion work, run formatting, linting, and workspace tests from `companion/`:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
cargo test --workspace --doc
```

Both test invocations are required: `--all-targets` adds example and integration
targets but removes doctests, so neither invocation alone covers the workspace.
Keep them as separate lines so a failure names the missing coverage directly; do
not simplify them back to one command. The workspace currently has no bench targets.

Hardware-facing changes also require the on-device checks named in the active plan and
an entry in `docs/hardware/board-notes.md` with the observed result.
