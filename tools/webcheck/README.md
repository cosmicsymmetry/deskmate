# webcheck

Two checks for the web companion that the test suite cannot make. Both need Python
Playwright (`pip install playwright pillow`) and system Chrome; neither is a repo
dependency, and neither runs in CI.

## `pixelab.py` — is this build pixel-identical to that one?

The DOM suite runs in happy-dom, which never applies a stylesheet. Every CSS mutation
survives it: deleting a selected-state rule, swapping an error colour and breaking a
transform origin all left 68 component tests green. For a CSS or markup *refactor*, the
honest check is that nothing on screen moved.

```sh
(cd <checkout-A>/companion/apps/deskmate && VITE_DESKMATE_MOCK=1 bunx vite --port 5301 --strictPort) &
(cd <checkout-B>/companion/apps/deskmate && VITE_DESKMATE_MOCK=1 bunx vite --port 5302 --strictPort) &
python3 tools/webcheck/pixelab.py http://127.0.0.1:5301 http://127.0.0.1:5302 /tmp/ab
```

It walks 12 scenarios x light/dark x desktop/mobile x every UI state (236 states) and
reports byte-identical / different / text-different. **Run A against A first.** A
noise floor that is not zero means the instrument is broken, not the build. Two things
make that floor zero, and both cost an afternoon to find:

- **A and B must be captured in the same browser launch.** Anti-aliasing of rounded
  corners is not bit-stable across Chrome processes, so hashes from separate runs differ
  by a few levels on a few dozen corner pixels with nothing changed.
- **Time must be driven, not merely set.** `page.clock.install()` sets the epoch and lets
  it run, and the mock backend ticks its pomodoro on a real `setInterval`. The harness
  installs the clock, `pause_at`s a fixed instant, and advances by fixed steps.

Differences within 16 levels on at most 400 px with identical text are reported as AA
noise rather than differences; they still occasionally appear within one launch.

It is blind to everything the mock harness is blind to — anything that needs the server.

## `smoke.sh` — does the real page work against the real server?

```sh
tools/webcheck/smoke.sh <checkout> <label> [port]
```

Builds the server and `dist/` from the checkout, starts the binary on localhost with a
throwaway config dir and loopback public URL. Headless Chrome completes first-run setup
using the code from the server log and checks “Add your panel.” The admin API then mints
a device for that owner, without USB provisioning. Chrome reloads into the grid, adds a
pomodoro, checks the save response, decodes the 448×368 preview PNG from the real LVGL
path, and verifies the saved tile count and session after reload. It then sends SIGTERM
**with the page and its SSE stream still open** and times the drain. Startup timeouts,
failed browser assertions and unsuccessful shutdowns return nonzero; `result.json`,
`server.log` and `smoke.err` remain under `$TMPDIR/deskmate-smoke/<label>/` (or `/tmp`).

Run it against two checkouts to get a before/after. On 2026-09-19 that is what showed,
against `main`: no `/v1/faces` request and no event stream after signing in, and a server
still alive 15 s after SIGTERM because one open tab's event stream held Axum's drain.
