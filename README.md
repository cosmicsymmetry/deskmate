# deskmate

Deskmate is a monitor-clip ESP32-S3 AMOLED display, a Rust companion workspace, and a
Tauri/React settings app. The current product description is [PRODUCT.md](PRODUCT.md),
and the repository’s durable implementation contract is [CLAUDE.md](CLAUDE.md).

## Current state

The live milestone status is in the
[roadmap](docs/superpowers/plans/2026-08-03-deskmate-roadmap.md). V1 is closed pending
declaration/tag authorization; V2’s network-owned device path is implemented, with two
observations still owed. Scene-native rendering is delivered in software. Manifest-based
plugins and the server-side SVG raster fallback have been retired; server-side cards are
now pictures pushed by external producers. The remaining work and hardware gates live in
these active plans:

- [V2 networked device](docs/superpowers/plans/2026-08-18-deskmate-v2-networked-device.md)
- [Picture cards](docs/superpowers/plans/2026-09-10-deskmate-picture-cards.md)

The current application config is frozen at
[schema v9](docs/config/v9.md), while the additive wire contract remains
[protocol v1](docs/protocol/v1.md). Clock and pomodoro render as host-built scenes;
picture frames use durable device assets. Hardware observations and unresolved board gates
are recorded in [board notes](docs/hardware/board-notes.md).

## Companion app

With the locked dependencies already installed:

```sh
cd companion/apps/deskmate
PATH="$HOME/.cargo/bin:$PATH" bun run tauri dev
```

The app authors the card library and playlists, previews built-in cards, configures
mounting and ownership, and keeps its runtime alive in the tray when the window closes.
In networked tier the deployed server owns the device; provisioning remains a USB cable
operation.

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

## Legacy CLI inspection

`deskmate-cli` retains the M1 `status`, `time-sync`, and `push-data` commands and the M2
config/demo commands as a manual inspection path. The M2 commands are legacy tooling for
template-era firmware: scene-native firmware cannot draw a card from them. See the
[M1 protocol/CLI plan](docs/superpowers/plans/2026-08-04-deskmate-m1-protocol-link-cli.md)
and [M2 walkthrough](docs/superpowers/plans/2026-08-04-deskmate-m2-template-first-widgets.md)
for the commands and their historical acceptance flow.

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
