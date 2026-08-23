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

// Host -> device: render a runtime asset font onto the active screen so a
// following DEV_CAPTURE_REQUEST_TYPE capture can be compared against
// lvgl-sim's `asset-font--72px-digits` golden
// (companion/crates/lvgl-sim/src/cases.rs). Payload is exactly
// ASSET_DIGEST_BYTES (32) raw bytes: the asset store digest of a font
// already committed via AssetBegin/AssetChunk/AssetCommit (this probe does
// not implement transfer). Text ("12:34") and pixel size (72) are pinned
// to match the golden, not carried on the wire -- only the digest varies,
// so this stays useful for other assets without risking a mismatched
// comparison. Any other payload length, or a request before the referenced
// asset is committed, is rejected/no-ops the same way malformed release
// requests are (protocol_task.c's dispatch, not this file).
#define DEV_ASSET_PROBE_REQUEST_TYPE 0x7DU

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

// Handles one 0x7D request end to end: acquires `digest` at 72px through
// font_registry_acquire() (Task 8/9), warms the digits+colon glyphs, and
// draws "12:34" centred on a fresh full-screen black canvas -- the same
// bare screen + centred label lvgl-sim's sim_render_asset_font() builds
// (csrc/sim_shim.c), not template_view.c's card chrome, so the two frames
// are comparable pixel for pixel. Runs under lvgl_port_lock()/
// lvgl_port_unlock() because it mutates LVGL objects from the protocol
// task, not an LVGL callback. On success it HANDS its acquire to the screen
// rather than releasing inline -- an LV_EVENT_DELETE callback releases the
// face when the screen (and the label styled with it) is destroyed, because
// LVGL keeps the bare lv_font_t* and takes no reference of its own. So the
// face stays pinned for exactly as long as something draws with it, which
// is what makes it un-evictable; the failure paths, which never reach a
// live screen, still release inline. One consequence: an AssetRelease
// arriving while a probe render is on the panel is answered with
// PROTOCOL_ERROR_BUSY and deferred -- font_registry_reset() refuses while
// any face is pinned rather than destroying one this probe is still using.
// Load another screen and the release succeeds. `digest` must point to
// exactly ASSET_DIGEST_BYTES readable bytes; the caller (protocol_task.c's
// dispatch) is responsible for validating the request payload length
// before calling this.
void dev_capture_handle_asset_probe(const uint8_t *digest);

#endif // DESKMATE_DEV_DIAG
