# Reference-only C template oracle

This code does **not ship on any device**. Only `lvgl-sim` compiles it, so the
scene parity suite can compare host-built scenes against the former hand-written
C template implementations byte for byte.

Treat these sources as a historical parity oracle, not as production firmware.
Fixes to a shipping face belong in its scene builder or the shared scene
renderer. Change this directory only when intentionally changing the oracle and
the parity contract it defines.
