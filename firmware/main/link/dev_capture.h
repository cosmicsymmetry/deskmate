#pragma once

// Dev-only framebuffer capture (V1 reset design spec
// docs/superpowers/specs/2026-08-11-deskmate-v1-reset-design.md §3.2.2,
// §3.2.3). Everything this header declares exists only when
// DESKMATE_DEV_DIAG is defined on the CMake command line
// (`idf.py -DDESKMATE_DEV_DIAG=1 build`, wired in firmware/main/CMakeLists.txt);
// a plain build never sees these symbols, the 0x7E/0x7F message ids, or the
// dev_capture.c log tag. The message ids deliberately sit outside
// protocol_message_type_t's frozen 1-12 release range (protocol_message.h)
// and are absent from docs/protocol/v1.md -- the release wire protocol does
// not move for this feature.
#ifdef DESKMATE_DEV_DIAG

#include <stdint.h>

// Host -> device: request a framebuffer capture. Carries no payload; any
// non-empty payload is rejected the same way malformed release requests
// are (protocol_task.c's dispatch, not this file).
#define DEV_CAPTURE_REQUEST_TYPE 0x7EU

// Device -> host: one chunk of a capture response. Payload layout:
// {offset: u32 LE, total: u32 LE, crc32: u32 LE} followed by up to
// DEV_CAPTURE_CHUNK_DATA_SIZE bytes of raw RGB565 pixel data. `total` is
// the full capture length in bytes; `offset` is this chunk's byte offset
// into it; `crc32` (protocol_crc32c) covers only this chunk's data bytes.
// Every chunk for one request echoes that request's request_id, exactly
// like the release ack/response pattern, so host tooling can correlate
// chunks without a session concept of its own.
#define DEV_CAPTURE_CHUNK_TYPE 0x7FU

// Handles one 0x7E request end to end: snapshots the currently active LVGL
// screen with lv_snapshot_take() into a PSRAM buffer and streams it back as
// a sequence of 0x7F chunk frames over the existing USB link. Runs on the
// protocol task with the same untrusted-input posture as every other
// dispatch handler -- it never reboots, and a snapshot or write failure
// simply ends the response early (the host times out waiting for the
// missing chunk rather than getting a wrong one).
void dev_capture_handle_request(uint32_t request_id);

#endif // DESKMATE_DEV_DIAG
