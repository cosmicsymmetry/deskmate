#pragma once

#include <stdbool.h>
#include <stdint.h>

#include "core/protocol_message.h"
#include "core/scene_binding.h"
#include "core/scene_model.h"
#include "lvgl.h"
#include "ui/font_registry.h"

/* The scene interpreter: the device-side half of the declarative display
 * list. It is the scene model's replacement for ui/template_view.c, and
 * deliberately wears the same show/destroy/active/screen surface so the rest
 * of the UI drives it the same way.
 *
 * This header and its implementation carry NO ESP-IDF include, so the whole
 * renderer compiles into companion/crates/lvgl-sim. That is not a tidiness
 * preference: the stage-2 parity gate compares a device framebuffer against a
 * simulator framebuffer produced by this exact C, and it is only meaningful
 * while both sides run the same translation unit.
 *
 * All functions run in LVGL context; callers outside it use ui_runtime. */

/* Wires the digest -> mapped-bytes lookup that `image` nodes need. The
 * typedefs are font_registry.h's, for the same reason they exist there:
 * firmware passes an asset_flash-backed resolver, lvgl-sim passes a
 * RAM-backed one, and this file stays free of the platform either way.
 *
 * Until this is called, a scene containing an `image` node is refused (see
 * scene_view_show). Text, glyph, rect, arc and line nodes do not need it. */
void scene_view_set_asset_resolver(asset_resolver_fn resolver,
                                   asset_release_fn release);

/* Builds `scene` into a fresh screen and loads it, replacing whatever was
 * displayed. Returns false without touching the live screen when the scene
 * cannot be rendered exactly as specified -- an invalid model, an
 * unallocatable object, or an asset (font, icon font, image) that could not
 * be acquired. A refusal leaves the previous screen up: this renderer never
 * blanks the panel and never substitutes a different face for one it could
 * not get, because a silently-substituted face is both wrong on the glass and
 * very hard to diagnose.
 *
 * `context` is used to evaluate binding-valued nodes for the first paint. It
 * may be NULL, in which case those nodes start empty and take their first
 * value from scene_view_refresh_bindings(). */
bool scene_view_show(const scene_t *scene,
                     const scene_binding_context_t *context);

/* Re-evaluates every binding-valued node against an authoritative `context`
 * and updates those objects in place. PushData comes through here and never
 * rebuilds the screen; ordinary ticks use the preserving path below. */
void scene_view_refresh_bindings(const scene_binding_context_t *context);

/* Refreshes wall-clock/field bindings while preserving the scene's local
 * timer snapshot. The protocol task uses this for its 250 ms tick; PushData
 * uses scene_view_refresh_bindings() above and replaces the timer snapshot. */
void scene_view_tick_bindings(const scene_binding_context_t *context);

/* Applies optimistic timer feedback from an LVGL tap callback and repaints
 * through the same bound-node refresh path. */
void scene_view_apply_local_action(protocol_event_action_t action);

/* Destroys the scene's screen, which is also what drops its hold on every
 * asset font it acquired. Returns false -- without destroying anything -- if
 * the scene is the currently loaded screen; load another screen first. */
bool scene_view_destroy(void);

bool scene_view_active(void);
lv_obj_t *scene_view_screen(void);
