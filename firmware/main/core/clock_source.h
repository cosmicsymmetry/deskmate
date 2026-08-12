#pragma once

#include <stdint.h>

/**
 * The single wall-clock read for hardware-independent code. Defaults to the
 * system clock; tests and the host preview harness may pin a moment. Not
 * thread-safe by design: overrides are set only from single-threaded test or
 * harness contexts, never on device.
 */
int64_t clock_source_now(void);
void clock_source_set_override(int64_t unix_seconds);
void clock_source_clear_override(void);
