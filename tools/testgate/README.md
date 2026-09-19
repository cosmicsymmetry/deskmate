# testgate

Two mechanical gates for a change that is supposed to touch ONLY tests -- a consolidation,
a helper extraction, a table-drive. For that kind of change "the suite still passes" says
nothing, because the change IS the suite. Neither script is a repo dependency or a CI step.

```sh
python3 tools/testgate/prodcheck.py <worktree>          # production code byte-identical?
cargo install cargo-llvm-cov && rustup component add llvm-tools-preview
(cd <base>/companion    && cargo llvm-cov -p server --all-targets --lcov --output-path /tmp/before.lcov)
(cd <changed>/companion && cargo llvm-cov -p server --all-targets --lcov --output-path /tmp/after.lcov)
python3 tools/testgate/covdiff.py /tmp/before.lcov /tmp/after.lcov <changed-worktree>   # exit 1 if any line lost
```

`prodcheck.py` allows a pure test file to change freely, and a production `.rs` file to
change only at or below its top-level `#[cfg(test)] mod`. `covdiff.py` reports every
production line the tests executed before and no longer execute. Coverage here is
deterministic run to run (measured: zero noise), so the threshold is zero lines.

Line coverage cannot see a weakened ASSERTION. It did catch one by accident -- a table that
dropped `is_timeout()` stopped executing that function -- but the real check for that is a
reviewer breaking production behaviour and watching whether the row still fails and names
itself. Use both.

## Three ways this gate lied before it told the truth (2026-09-19)

- **Do not share one instrumented target dir across worktrees.** `cargo llvm-cov` picks up
  test binaries by crate name, so objects built from ANOTHER checkout of the same workspace
  are folded into the report. With paths normalized, that can MASK a real loss. Measure
  every candidate in one fixed worktree: apply its patch, run, revert.
- **An inline `#[cfg(test)]` module is test code living in a production file.** Count only
  lines above the marker -- and the marker may be followed by a comment or an attribute
  before `mod` (`device/src/session.rs` is).
- **Never snapshot a worktree while a reviewer is running mutation probes in it.** The
  snapshot will contain the reviewer's sabotage. A false alarm from that was passed to an
  implementer as a real failure, and it reverted its whole batch. A gate that is wrong is
  worse than no gate, because people obey it.
