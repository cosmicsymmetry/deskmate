#include "ui/scene_view.h"

#include <string.h>
#include <time.h>

#include "lvgl.h"
#include "ui/fonts/deskmate_fonts.h"

/* ------------------------------------------------------------------
 * What this file relies on, and therefore does not re-check.
 *
 * scene_model_validate() (core/scene_model.c) is called once at the top of
 * scene_view_show(). What it guarantees, for every node, is: geometry inside
 * the 448x368 canvas with no int32 wrap, non-negative extents,
 * NUL-terminated text/binding/glyph-name/arc-end-binding arrays, a legal
 * text alignment on TEXT nodes, a legal font reference on TEXT nodes,
 * 2..SCENE_MAX_LINE_POINTS line points, and -- for SCALE --
 * 2..SCENE_SCALE_MAX_TOTAL_TICKS total ticks with major_tick_every in
 * [1, total_tick_count], LABEL anchors/style extents are bounded with legal
 * fonts and terminated values, and ROT_RECT geometry/pivot/rotation/binding
 * are legal. None of that is checked again here. Redundant
 * defensive validation has concealed four defects on this plan already; the
 * model is the one place those bounds live.
 *
 * This list is load-bearing -- the whole safety argument of this file is that
 * it is exact -- so what it deliberately does NOT cover is spelled out too:
 *
 *  - A GLYPH's font is not validated. validate_font() runs only for
 *    SCENE_NODE_TEXT, at its call site in scene_model_validate()'s TEXT
 *    case; a glyph carries a bare digest and a `size` checked only as
 *    0 <= size <= canvas, in glyph_within_canvas(), so a size of 0 reaches
 *    font_registry_acquire(). That is safe without a
 *    check here because the registry rejects pixel_size <= 0 and returns
 *    NULL, and NULL refuses the scene -- which is the same outcome any other
 *    unavailable face gets. It is safe by the registry's contract, not by
 *    the model's.
 *  - An arc's start/end degrees and stroke width, and a line's stroke width,
 *    are unbounded by design (plan ruling 7 assigned them to the renderer).
 *    They are normalised/clamped here, at their point of use, and nowhere
 *    else.
 *  - scene_value_t.kind is not constrained and cannot be. Anything that is
 *    not SCENE_VALUE_BINDING is read as a literal, which is always safe
 *    because the literal array is NUL-terminated and bounded.
 *  - A rect's radius is not constrained; a negative one is read as 0.
 * ------------------------------------------------------------------ */

/* A binding-valued node, remembered so scene_view_refresh_bindings() can
 * update it without rebuilding the screen. This is the whole of the
 * node -> object identity mechanism: only nodes that can change are
 * recorded, in scene order, each holding its own already-parsed binding.
 * Nodes that cannot change are drawn once and forgotten. */
typedef struct {
    lv_obj_t *object;
    scene_binding_t binding;
    /* SCENE_NODE_ARC only: the sweep the binding scales, captured at build
     * time because the node it came from is not retained. The origin needs no
     * entry here -- it lives on the object as its rotation. */
    int32_t arc_span_deg;
    uint8_t kind; /* scene_node_kind_t */
    uint8_t rotation_binding; /* scene_rotation_binding_t, ROT_RECT only */
    bool hide_when_empty;     /* LABEL only */
} scene_bound_node_t;

/* Per-scene state. Allocated from the LVGL heap and owned by the screen --
 * NOT a file-scope table. A fixed SCENE_MAX_NODES table would be ~1.7 KB of
 * DIRAM .bss, and on this board a ~105-byte .bss shift once broke OTA
 * downloads outright with every test green. The only file-scope objects this
 * module adds are the three pointers below. */
typedef struct {
    lv_obj_t *screen;
    uint32_t bound_count;
    scene_bound_node_t bound[]; /* exactly bound_count entries */
} scene_view_state_t;

/* An `image` node's LVGL descriptor plus the asset mapping it reads through.
 * lv_image stores the bare descriptor pointer and the descriptor stores a
 * bare pointer into mapped flash, so both must outlive the object; they are
 * freed and released together on the object's LV_EVENT_DELETE. */
typedef struct {
    lv_image_dsc_t dsc;
    const void *mapping;
} scene_image_asset_t;

static scene_view_state_t *s_state;
static asset_resolver_fn s_asset_resolver;
static asset_release_fn s_asset_release;

/* Longest string a binding may render, matched to the literal capacity so a
 * bound node can say exactly as much as a literal one. */
#define SCENE_VIEW_TEXT_CAPACITY (SCENE_MAX_TEXT_BYTES + 1U)

/* ---------------------------------------------------------------- fonts */

/* The four baked tiers resolve to the identical lv_font_t objects the C
 * templates use -- ui/templates/template_internal.h's DESKMATE_FONT_CAPTION /
 * _BODY / _DISPLAY / _HERO are (&deskmate_font_18) / (&deskmate_font_28) /
 * (&deskmate_font_56) / (&deskmate_font_96). That pointer identity is what
 * makes the parity gate winnable, so if those macros are ever repointed this
 * function must move with them. The macros themselves are not used here
 * because template_internal.h drags in the whole template vocabulary, and
 * this file's dependency surface is load-bearing (see the header). */
static const lv_font_t *baked_font(scene_font_tier_t tier)
{
    switch (tier) {
    case SCENE_FONT_CAPTION:
        return &deskmate_font_18;
    case SCENE_FONT_BODY:
        return &deskmate_font_28;
    case SCENE_FONT_DISPLAY:
        return &deskmate_font_56;
    case SCENE_FONT_HERO:
        return &deskmate_font_96;
    default:
        break;
    }
    return NULL;
}

/* Drops one object's hold on a registry face, at the only moment
 * font_registry.h's ownership contract permits it: after the object styled
 * with that face is gone. LVGL stores the bare lv_font_t* and takes no
 * reference of its own, so releasing any earlier would leave the face
 * unpinned while a live label still draws with it -- and the very next
 * acquire of a different face could then evict it. Mirrors
 * link/dev_capture.c's release_probe_font(). */
static void release_font_cb(lv_event_t *event)
{
    font_registry_release((lv_font_t *)lv_event_get_user_data(event));
}

/* Resolves a font reference. `*out_acquired` is non-NULL only for the
 * registry path, and its caller then owns exactly one acquire. Baked tiers
 * are compiled-in objects: they are never acquired and never released.
 *
 * Returns NULL when a registry face could not be acquired -- all
 * FONT_REGISTRY_MAX_OPEN_FACES slots full and pinned, a missing or non-font
 * digest, a rasterizer failure. Every caller turns that into a refusal of the
 * whole scene. */
static const lv_font_t *resolve_font(const scene_font_ref_t *ref,
                                     lv_font_t **out_acquired)
{
    *out_acquired = NULL;
    if (ref->kind == SCENE_FONT_ASSET) {
        lv_font_t *font = font_registry_acquire(ref->digest, ref->pixel_size);
        *out_acquired = font;
        return font;
    }
    return baked_font(ref->baked);
}

/* ------------------------------------------------------------- geometry */

/* digital_clock.c:44's baseline_offset() is the distance from a label box's
 * TOP edge down to the baseline of the type set in it:
 *
 *     baseline_offset(f) = lv_font_get_line_height(f) - f->base_line
 *
 * because base_line is measured UP from the bottom of the line box, so
 * line_height - base_line counts down from the top instead.
 *
 * A scene text node carries the baseline, not the box top (see
 * scene_model.h's field comment), so placing it is the exact inverse:
 *
 *     top = baseline_y - baseline_offset(f)
 *         = baseline_y - (lv_font_get_line_height(f) - f->base_line)
 *
 * Substituting back gives top + baseline_offset(f) == baseline_y, i.e. the
 * baseline of the type in the placed box lands on baseline_y, which is the
 * whole contract. This is the highest-risk line in the file: an error here is
 * invisible to every host test and only surfaces as a failed pixel diff. */
static int32_t baseline_offset(const lv_font_t *font)
{
    return lv_font_get_line_height(font) - font->base_line;
}

static int32_t baseline_box_top(int32_t baseline_y, const lv_font_t *font)
{
    return baseline_y - baseline_offset(font);
}

/* The templates set alignment inside a fixed-width label box with
 * lv_obj_set_style_text_align() -- see template_style.c's
 * deskmate_label_box(), which the faces call with LV_TEXT_ALIGN_LEFT and
 * LV_TEXT_ALIGN_CENTER. The scene enum maps straight onto that, and NOT onto
 * lv_obj_align(), which would move the box rather than the type in it and so
 * would break the baseline contract above. */
static lv_text_align_t align_to_lv(scene_align_t align)
{
    switch (align) {
    case SCENE_ALIGN_CENTER:
        return LV_TEXT_ALIGN_CENTER;
    case SCENE_ALIGN_RIGHT:
        return LV_TEXT_ALIGN_RIGHT;
    case SCENE_ALIGN_LEFT:
    default:
        break;
    }
    return LV_TEXT_ALIGN_LEFT;
}

/* An arc is placed the way progress_ring.c places one: the ORIGIN travels on
 * the object's rotation and the sweep is set as angles 0..span.
 *
 * That arrangement is not a stylistic echo, it is the only one that can draw
 * a full turn. lv_arc_set_start_angle()/set_end_angle() each fold their
 * argument with a single `if (x > 360) x -= 360` (lv_arc.c:167, :189), so
 * feeding them absolute angles loses exactly one bit of information: a whole
 * circle beginning anywhere but three o'clock -- say start_deg=270,
 * end_deg=630 -- folds to set_angles(270, 270), and lv_draw_arc draws
 * nothing when start == end. progress_ring.c:190-193 avoids that by calling
 * lv_arc_set_rotation(270) and then lv_arc_set_bg_angles(0, 360): the
 * rotation carries the twelve-o'clock origin so the fold can never truncate
 * the turn. scene_arc_t has no rotation field, so the origin is derived from
 * start_deg here and the same arrangement is reproduced.
 *
 * The origin/sweep math itself (scene_model_arc_origin()/
 * scene_model_arc_span()) lives in core/scene_model.c: it is pure integer
 * arithmetic with zero LVGL dependency, so it is host-tested directly there
 * rather than only indirectly through this file's `lv_*` calls. */

/* Stroke clamps. The model leaves both unbounded on purpose (plan ruling 7:
 * "renderer concerns"), so they are bounded here. An arc stroke wider than
 * its radius would reach past the centre; a line stroke wider than the canvas
 * is meaningless. Both inputs are already known non-negative. */
static int32_t clamp_arc_width(int32_t width, int32_t radius)
{
    return width > radius ? radius : width;
}

static int32_t clamp_line_width(int32_t width)
{
    return width > SCENE_CANVAS_HEIGHT ? SCENE_CANVAS_HEIGHT : width;
}

/* -------------------------------------------------------------- values */

static bool value_is_bound(const scene_value_t *value)
{
    return value->kind == SCENE_VALUE_BINDING;
}

/* Evaluates one binding into `out`. A placeholder ("--", "--:--") is a
 * SCENE_BINDING_OK result and renders as itself. A genuine failure -- an
 * unparseable format, a field value longer than a literal may be -- renders
 * NOTHING for that node rather than failing the whole scene: one broken
 * binding should cost its own node, not the other 23. */
static void evaluate_binding(const scene_binding_t *binding,
                             const scene_binding_context_t *context,
                             char *out, size_t capacity)
{
    if (context == NULL ||
        scene_binding_evaluate(binding, context, out, capacity) !=
            SCENE_BINDING_OK) {
        out[0] = '\0';
    }
}

/* Reads an evaluated `timer.pct` as a percentage. Anything that is not a run
 * of digits -- notably the "--" placeholder an inactive timer produces -- is
 * read as 0, which collapses the arc to its start angle. */
static int32_t parse_percent(const char *text)
{
    int32_t value = 0;
    if (text[0] == '\0') {
        return 0;
    }
    for (const char *cursor = text; *cursor != '\0'; ++cursor) {
        if (*cursor < '0' || *cursor > '9') {
            return 0;
        }
        value = value * 10 + (*cursor - '0');
        if (value >= 100) {
            return 100;
        }
    }
    return value;
}

/* Scales the arc's sweep to the bound percentage. The indicator's start
 * angle stays at 0 and the object's rotation carries the origin (see
 * scene_model_arc_origin()/scene_model_arc_span()), so only the end angle
 * moves, and it can never exceed the 360 the setter's fold would truncate:
 * span <= 360 and percent <= 100. */
static void apply_arc_binding(const scene_bound_node_t *bound,
                              const char *evaluated)
{
    int32_t percent = parse_percent(evaluated);
    lv_arc_set_end_angle(
        bound->object,
        (lv_value_precise_t)(bound->arc_span_deg * percent / 100));
}

/* Exactly deskmate_chip_set_text()'s visible-state transition. In
 * particular, empty text hides a chip instead of leaving its padding and
 * background behind as a coloured blob. */
static void apply_label_text(lv_obj_t *label, const char *text,
                             bool hide_when_empty)
{
    if (hide_when_empty && (text == NULL || text[0] == '\0')) {
        lv_obj_add_flag(label, LV_OBJ_FLAG_HIDDEN);
        lv_label_set_text(label, "");
        return;
    }
    lv_label_set_text(label, text != NULL ? text : "");
    lv_obj_remove_flag(label, LV_OBJ_FLAG_HIDDEN);
}

/* The same integer angle formulas analog_clock_tick() hands to
 * set_hand_angle(), with the final *10 kept because a scene rotation is
 * already in LVGL's tenths-of-a-degree unit. */
static int32_t rotation_for_time(scene_rotation_binding_t binding,
                                 const scene_binding_context_t *context,
                                 int32_t fallback)
{
    if (binding == SCENE_ROTATION_BINDING_NONE || context == NULL) {
        return fallback;
    }
    int64_t local_seconds =
        context->unix_seconds +
        (int64_t)context->utc_offset_minutes * INT64_C(60);
    time_t local_time = (time_t)local_seconds;
    struct tm now;
    if (gmtime_r(&local_time, &now) == NULL) {
        return fallback;
    }
    switch (binding) {
    case SCENE_ROTATION_BINDING_HOUR:
        return (((now.tm_hour % 12) * 30) + (now.tm_min / 2)) * 10;
    case SCENE_ROTATION_BINDING_MINUTE:
        return now.tm_min * 60;
    case SCENE_ROTATION_BINDING_SECOND:
        return now.tm_sec * 60;
    case SCENE_ROTATION_BINDING_NONE:
    default:
        break;
    }
    return fallback;
}

/* ------------------------------------------------------------- builders */

/* A scene display list is flat, but LVGL clips a template child to its
 * parent. RECT and ROT_RECT use this same styleless wrapper whenever that
 * parent relationship is part of the pixels being reproduced. Removing all
 * styles makes the wrapper's content box exactly the declared clip box. */
static lv_obj_t *build_clip_wrapper(lv_obj_t *parent,
                                    const scene_clip_rect_t *clip)
{
    lv_obj_t *wrapper = lv_obj_create(parent);
    if (wrapper == NULL) {
        return NULL;
    }
    lv_obj_remove_style_all(wrapper);
    lv_obj_remove_flag(wrapper,
                       LV_OBJ_FLAG_CLICKABLE | LV_OBJ_FLAG_SCROLLABLE |
                           LV_OBJ_FLAG_OVERFLOW_VISIBLE);
    lv_obj_set_size(wrapper, clip->w, clip->h);
    lv_obj_set_pos(wrapper, clip->x, clip->y);
    return wrapper;
}

/* `rect` -- the same object template_style.c's deskmate_module() builds: a
 * styleless lv_obj that neither scrolls nor takes clicks, carrying only a
 * fill and a corner radius. remove_style_all() is what strips the theme's
 * border, padding and scrollbars. */
static lv_obj_t *build_rect(lv_obj_t *parent, const scene_rect_t *rect)
{
    lv_obj_t *wrapper = NULL;
    lv_obj_t *object_parent = parent;
    int32_t object_x = rect->x;
    int32_t object_y = rect->y;

    if (rect->has_clip) {
        wrapper = build_clip_wrapper(parent, &rect->clip);
        if (wrapper == NULL) {
            return NULL;
        }
        object_parent = wrapper;
        object_x = rect->x - rect->clip.x;
        object_y = rect->y - rect->clip.y;
    }

    lv_obj_t *object = lv_obj_create(object_parent);
    if (object == NULL) {
        if (wrapper != NULL) {
            lv_obj_delete(wrapper);
        }
        return NULL;
    }
    lv_obj_remove_style_all(object);
    lv_obj_remove_flag(object, LV_OBJ_FLAG_CLICKABLE | LV_OBJ_FLAG_SCROLLABLE);
    lv_obj_set_size(object, rect->w, rect->h);
    lv_obj_set_pos(object, object_x, object_y);
    lv_obj_set_style_bg_color(object, lv_color_hex(rect->fill), 0);
    lv_obj_set_style_bg_opa(object, (lv_opa_t)rect->opacity, 0);
    lv_obj_set_style_radius(object, rect->radius < 0 ? 0 : rect->radius, 0);
    return object;
}

/* `arc` -- one stroke, drawn the way progress_ring.c draws its meaningful
 * arc: a square lv_arc of side 2r positioned so LVGL's own centre lands on
 * (cx, cy), knob removed, not clickable. The node carries a single colour, so
 * it is a single stroke: it is drawn on LV_PART_INDICATOR and the background
 * track on LV_PART_MAIN is made transparent. A progress ring in scene form is
 * therefore two arc nodes -- a dim full-circle track and a bound indicator --
 * which is exactly what progress_ring.c draws.
 *
 * Angles use LVGL's convention -- 0 degrees is 3 o'clock, increasing
 * clockwise -- and are absolute on the canvas: `start_deg` is the origin and
 * the sweep runs clockwise to `end_deg`. The object's rotation carries that
 * origin and the indicator is set to 0..span, which is what lets a full turn
 * survive LVGL's single-subtraction fold; see scene_model_arc_origin() and
 * scene_model_arc_span(). */
static lv_obj_t *build_arc(lv_obj_t *parent, const scene_arc_t *arc)
{
    lv_obj_t *object = lv_arc_create(parent);
    if (object == NULL) {
        return NULL;
    }
    lv_obj_set_size(object, arc->r * 2, arc->r * 2);
    lv_obj_set_pos(object, arc->cx - arc->r, arc->cy - arc->r);
    lv_obj_remove_style(object, NULL, LV_PART_KNOB);
    lv_obj_remove_flag(object, LV_OBJ_FLAG_CLICKABLE);
    lv_obj_set_style_arc_opa(object, LV_OPA_TRANSP, LV_PART_MAIN);
    lv_obj_set_style_arc_color(object, lv_color_hex(arc->color),
                               LV_PART_INDICATOR);
    lv_obj_set_style_arc_opa(object, (lv_opa_t)arc->opacity,
                             LV_PART_INDICATOR);
    lv_obj_set_style_arc_width(object, clamp_arc_width(arc->width, arc->r),
                               LV_PART_INDICATOR);
    lv_obj_set_style_arc_rounded(object, arc->rounded, LV_PART_INDICATOR);
    lv_arc_set_rotation(object, scene_model_arc_origin(arc->start_deg));
    lv_arc_set_angles(object, (lv_value_precise_t)0,
                      (lv_value_precise_t)scene_model_arc_span(arc->start_deg,
                                                               arc->end_deg));
    return object;
}

/* Frees a line's point array once the line that reads it is gone. lv_line
 * stores the array by pointer and never copies it. */
static void free_points_cb(lv_event_t *event)
{
    lv_free(lv_event_get_user_data(event));
}

/* `line` -- styled like the clock hands in digital_clock.c: a coloured,
 * rounded stroke. Every lv_line in the firmware is rounded, so scene lines
 * are too. Points are absolute canvas coordinates and the object sits at the
 * canvas origin, so lv_line's LV_SIZE_CONTENT self-size (max x, max y from
 * its origin) always encloses them. */
static lv_obj_t *build_line(lv_obj_t *parent, const scene_line_t *line)
{
    lv_point_precise_t *points =
        lv_malloc(sizeof(lv_point_precise_t) * line->point_count);
    if (points == NULL) {
        return NULL;
    }
    for (uint32_t i = 0U; i < line->point_count; i++) {
        points[i].x = (lv_value_precise_t)line->xs[i];
        points[i].y = (lv_value_precise_t)line->ys[i];
    }

    lv_obj_t *object = lv_line_create(parent);
    if (object == NULL) {
        lv_free(points);
        return NULL;
    }
    lv_obj_add_event_cb(object, free_points_cb, LV_EVENT_DELETE, points);
    lv_obj_set_style_line_color(object, lv_color_hex(line->color), 0);
    lv_obj_set_style_line_width(object, clamp_line_width(line->width), 0);
    lv_obj_set_style_line_rounded(object, true, 0);
    lv_line_set_points(object, points, line->point_count);
    lv_obj_set_pos(object, 0, 0);
    return object;
}

/* `text` -- an lv_label in a fixed-width, one-line box, positioned by
 * baseline. The box is what makes alignment mean anything: template_style.c's
 * deskmate_label_box() pins the height to exactly one line of the tier for
 * the same reason, because with LV_SIZE_CONTENT height LVGL breaks an
 * over-long run onto further lines instead of honouring the mode.
 *
 * Long mode: DOTS when the node asks to ellipsize (deskmate_label_box's
 * choice), otherwise CLIP. CLIP is not merely "no dots" -- it sets LVGL's
 * expand flag, so the text stays on one line and is clipped to the box rather
 * than wrapping a too-wide word onto an invisible second line. */
static lv_obj_t *build_text(lv_obj_t *parent, const scene_text_t *text,
                            const scene_binding_context_t *context,
                            scene_binding_t *out_binding, bool *out_is_bound)
{
    lv_font_t *acquired = NULL;
    const lv_font_t *font = resolve_font(&text->font, &acquired);
    if (font == NULL) {
        return NULL;
    }

    lv_obj_t *label = lv_label_create(parent);
    if (label == NULL) {
        if (acquired != NULL) {
            font_registry_release(acquired);
        }
        return NULL;
    }
    /* Registered before anything else can fail, so every later exit path --
     * including deleting the whole candidate screen -- releases the face. */
    if (acquired != NULL) {
        lv_obj_add_event_cb(label, release_font_cb, LV_EVENT_DELETE, acquired);
    }

    lv_obj_set_style_text_font(label, font, 0);
    lv_obj_set_style_text_color(label, lv_color_hex(text->color), 0);
    lv_obj_set_style_text_align(label, align_to_lv(text->align), 0);
    lv_label_set_long_mode(label, text->ellipsize ? LV_LABEL_LONG_MODE_DOTS
                                                  : LV_LABEL_LONG_MODE_CLIP);
    lv_obj_set_width(label, text->w);
    lv_obj_set_height(label, lv_font_get_line_height(font));

    char buffer[SCENE_VIEW_TEXT_CAPACITY];
    const char *initial = text->value.literal;
    *out_is_bound = false;
    if (value_is_bound(&text->value)) {
        /* A binding that will not parse renders nothing and is not recorded,
         * so no later refresh can resurrect it. */
        initial = "";
        if (scene_binding_parse(text->value.binding, out_binding) ==
            SCENE_BINDING_OK) {
            *out_is_bound = true;
            evaluate_binding(out_binding, context, buffer, sizeof buffer);
            initial = buffer;
        }
    }
    lv_label_set_text(label, initial);
    lv_obj_set_pos(label, text->x, baseline_box_top(text->baseline_y, font));
    return label;
}

/* `label` -- deskmate_chip()'s content-sized lv_label, call for call and in
 * the same order. The node supplies the values but does not change the
 * construction: LVGL still owns glyph advances, 4.4 kerning and
 * letter_space, so its content width is the chip width by construction.
 * A transparent fill is the eyebrow form; no separate kind or host-side
 * size calculation is involved. */
static lv_obj_t *build_label(lv_obj_t *parent, const scene_label_t *node,
                             const scene_binding_context_t *context,
                             scene_binding_t *out_binding,
                             bool *out_is_bound)
{
    lv_font_t *acquired = NULL;
    const lv_font_t *font = resolve_font(&node->font, &acquired);
    if (font == NULL) {
        return NULL;
    }

    lv_obj_t *label = lv_label_create(parent);
    if (label == NULL) {
        if (acquired != NULL) {
            font_registry_release(acquired);
        }
        return NULL;
    }
    if (acquired != NULL) {
        lv_obj_add_event_cb(label, release_font_cb, LV_EVENT_DELETE, acquired);
    }

    /* Keep this sequence identical to template_style.c:deskmate_chip(). */
    lv_obj_set_style_text_font(label, font, 0);
    lv_obj_set_style_text_color(label, lv_color_hex(node->ink), 0);
    lv_obj_set_style_bg_color(label, lv_color_hex(node->fill), 0);
    lv_obj_set_style_bg_opa(label, (lv_opa_t)node->fill_opacity, 0);
    lv_obj_set_style_radius(label, node->radius, 0);
    lv_obj_set_style_pad_hor(label, node->pad_hor, 0);
    lv_obj_set_style_pad_ver(label, node->pad_ver, 0);
    lv_obj_set_style_text_letter_space(label, node->letter_space, 0);
    lv_label_set_text(label, "");

    char buffer[SCENE_VIEW_TEXT_CAPACITY];
    const char *initial = node->value.literal;
    *out_is_bound = false;
    if (value_is_bound(&node->value)) {
        initial = "";
        if (scene_binding_parse(node->value.binding, out_binding) ==
            SCENE_BINDING_OK) {
            *out_is_bound = true;
            evaluate_binding(out_binding, context, buffer, sizeof buffer);
            initial = buffer;
        }
    }
    apply_label_text(label, initial, node->hide_when_empty);
    /* The align style stays attached to the object, so LVGL recalculates the
     * position whenever a bound value changes the content-sized width. Its
     * own layout path performs parent_width / 2 - object_width / 2 for MID
     * and parent_width - object_width for RIGHT. The renderer never measures
     * or subtracts the label width, including for odd widths. */
    switch (node->horizontal_anchor) {
    case SCENE_LABEL_ANCHOR_LEFT:
        lv_obj_align(label, LV_ALIGN_TOP_LEFT, node->x, node->y);
        break;
    case SCENE_LABEL_ANCHOR_CENTER:
        lv_obj_align(label, LV_ALIGN_TOP_MID,
                     node->x - SCENE_CANVAS_WIDTH / 2, node->y);
        break;
    case SCENE_LABEL_ANCHOR_RIGHT:
        lv_obj_align(label, LV_ALIGN_TOP_RIGHT,
                     node->x - SCENE_CANVAS_WIDTH, node->y);
        break;
    default:
        /* scene_model_validate() rejects this before rendering. */
        lv_obj_delete(label);
        return NULL;
    }
    return label;
}

/* `rot_rect` -- make_hand() followed by set_hand_angle(), with position
 * supplied directly by the scene instead of the template's align call. The
 * object-transform path is essential: lv_line anti-aliases through a
 * different renderer and cannot reproduce a hand byte-for-byte. */
static lv_obj_t *build_rot_rect(lv_obj_t *parent,
                                const scene_rot_rect_t *rect,
                                const scene_binding_context_t *context,
                                scene_rotation_binding_t *out_binding,
                                bool *out_is_bound)
{
    lv_obj_t *wrapper = NULL;
    lv_obj_t *object_parent = parent;
    int32_t object_x = rect->x;
    int32_t object_y = rect->y;

    if (rect->has_clip) {
        wrapper = build_clip_wrapper(parent, &rect->clip);
        if (wrapper == NULL) {
            return NULL;
        }
        object_parent = wrapper;
        object_x = rect->x - rect->clip.x;
        object_y = rect->y - rect->clip.y;
    }

    lv_obj_t *object = lv_obj_create(object_parent);
    if (object == NULL) {
        if (wrapper != NULL) {
            lv_obj_delete(wrapper);
        }
        return NULL;
    }

    /* Keep this sequence identical to analog_clock.c:make_hand(). */
    lv_obj_remove_style_all(object);
    lv_obj_remove_flag(object,
                       LV_OBJ_FLAG_CLICKABLE | LV_OBJ_FLAG_SCROLLABLE);
    lv_obj_set_size(object, rect->w, rect->h);
    lv_obj_set_style_radius(object, rect->radius, 0);
    lv_obj_set_style_bg_color(object, lv_color_hex(rect->fill), 0);
    lv_obj_set_style_bg_opa(object, LV_OPA_COVER, 0);
    lv_obj_set_style_transform_pivot_x(object, rect->pivot_x, 0);
    lv_obj_set_style_transform_pivot_y(object, rect->pivot_y, 0);

    lv_obj_set_pos(object, object_x, object_y);
    *out_is_bound = scene_model_parse_rotation_binding(
        rect->rotation_binding, out_binding) &&
        *out_binding != SCENE_ROTATION_BINDING_NONE;
    lv_obj_set_style_transform_rotation(
        object, rotation_for_time(*out_binding, context, rect->rotation), 0);
    return object;
}

/* Releases an image node's asset mapping and descriptor together, once the
 * lv_image reading through both is gone. */
static void release_image_cb(lv_event_t *event)
{
    scene_image_asset_t *asset =
        (scene_image_asset_t *)lv_event_get_user_data(event);
    if (asset == NULL) {
        return;
    }
    if (s_asset_release != NULL && asset->mapping != NULL) {
        s_asset_release(asset->mapping);
    }
    lv_free(asset);
}

/* `image` -- an lv_image over the mapped asset bytes. The asset is an LVGL
 * binary image (converted server-side, spec section 1: "the device needs no
 * PNG decoder"), whose file layout is an lv_image_header_t followed by the
 * pixel data. That is not itself an lv_image_dsc_t, so one is built pointing
 * into the mapping rather than copying the pixels: the mapping is flash, and
 * copying it would cost PSRAM for nothing. */
static lv_obj_t *build_image(lv_obj_t *parent, const scene_image_t *image)
{
    const void *bytes = NULL;
    uint32_t length = 0U;
    uint8_t kind = 0U;

    if (s_asset_resolver == NULL) {
        return NULL;
    }
    if (!s_asset_resolver(image->digest, &bytes, &length, &kind)) {
        return NULL;
    }
    if (bytes == NULL || kind != (uint8_t)ASSET_KIND_IMAGE ||
        length <= (uint32_t)sizeof(lv_image_header_t)) {
        if (s_asset_release != NULL && bytes != NULL) {
            s_asset_release(bytes);
        }
        return NULL;
    }

    scene_image_asset_t *asset = lv_malloc(sizeof *asset);
    if (asset == NULL) {
        if (s_asset_release != NULL) {
            s_asset_release(bytes);
        }
        return NULL;
    }
    lv_memzero(asset, sizeof *asset);
    memcpy(&asset->dsc.header, bytes, sizeof(lv_image_header_t));
    asset->dsc.data = (const uint8_t *)bytes + sizeof(lv_image_header_t);
    asset->dsc.data_size = length - (uint32_t)sizeof(lv_image_header_t);
    asset->mapping = bytes;

    lv_obj_t *object = NULL;
    if (asset->dsc.header.magic == LV_IMAGE_HEADER_MAGIC) {
        object = lv_image_create(parent);
    }
    if (object == NULL) {
        if (s_asset_release != NULL) {
            s_asset_release(asset->mapping);
        }
        lv_free(asset);
        return NULL;
    }
    lv_obj_add_event_cb(object, release_image_cb, LV_EVENT_DELETE, asset);
    lv_image_set_src(object, &asset->dsc);
    /* Sized from the node rather than from the asset header, so a blob that
     * does not match what the server said it laid out is clipped to the
     * declared box instead of silently overflowing its neighbours. */
    lv_obj_set_size(object, image->w, image->h);
    lv_obj_set_pos(object, image->x, image->y);
    if (image->recolor) {
        lv_obj_set_style_image_recolor(object, lv_color_hex(image->color), 0);
        lv_obj_set_style_image_recolor_opa(object, LV_OPA_COVER, 0);
    }
    return object;
}

/* `glyph` -- an lv_label carrying one codepoint in an icon font, positioned
 * by baseline exactly as `text` is. Icon fonts are always assets, so this
 * always goes through the registry.
 *
 * NAME RESOLUTION: there is no device-side icon-name -> codepoint map, and
 * none is specified anywhere in this plan. The server holds the icon font and
 * its map and validates that a plugin's `icon: "cloud-rain"` resolves (spec
 * section 1), so resolution belongs there, next to every other layout
 * decision. This accepts the resolved forms it can emit: "U+XXXX" hex, or the
 * glyph's own UTF-8 bytes. An unresolved name falls through as literal text,
 * which renders as placeholder boxes -- visible and diagnosable, rather than
 * an invisible missing icon. */
static uint32_t glyph_codepoint(const char *name)
{
    uint32_t codepoint = 0U;
    size_t digits = 0U;

    if ((name[0] != 'U' && name[0] != 'u') || name[1] != '+') {
        return 0U;
    }
    for (const char *cursor = name + 2; *cursor != '\0'; ++cursor) {
        uint32_t nibble;
        if (*cursor >= '0' && *cursor <= '9') {
            nibble = (uint32_t)(*cursor - '0');
        } else if (*cursor >= 'a' && *cursor <= 'f') {
            nibble = (uint32_t)(*cursor - 'a') + 10U;
        } else if (*cursor >= 'A' && *cursor <= 'F') {
            nibble = (uint32_t)(*cursor - 'A') + 10U;
        } else {
            return 0U;
        }
        if (digits >= 6U) {
            return 0U;
        }
        codepoint = (codepoint << 4) | nibble;
        ++digits;
    }
    if (digits == 0U || codepoint == 0U || codepoint > 0x10FFFFU) {
        return 0U;
    }
    return codepoint;
}

/* Writes `name`'s glyph as UTF-8 into `out` (at least 5 bytes plus room for
 * the name itself). */
static void glyph_text(const char *name, char *out, size_t capacity)
{
    uint32_t codepoint = glyph_codepoint(name);
    size_t length = 0U;

    if (codepoint == 0U) {
        size_t name_length = strlen(name);
        if (name_length >= capacity) {
            name_length = capacity - 1U;
        }
        memcpy(out, name, name_length);
        out[name_length] = '\0';
        return;
    }
    if (codepoint < 0x80U) {
        out[length++] = (char)codepoint;
    } else if (codepoint < 0x800U) {
        out[length++] = (char)(0xC0U | (codepoint >> 6));
        out[length++] = (char)(0x80U | (codepoint & 0x3FU));
    } else if (codepoint < 0x10000U) {
        out[length++] = (char)(0xE0U | (codepoint >> 12));
        out[length++] = (char)(0x80U | ((codepoint >> 6) & 0x3FU));
        out[length++] = (char)(0x80U | (codepoint & 0x3FU));
    } else {
        out[length++] = (char)(0xF0U | (codepoint >> 18));
        out[length++] = (char)(0x80U | ((codepoint >> 12) & 0x3FU));
        out[length++] = (char)(0x80U | ((codepoint >> 6) & 0x3FU));
        out[length++] = (char)(0x80U | (codepoint & 0x3FU));
    }
    out[length] = '\0';
}

static lv_obj_t *build_glyph(lv_obj_t *parent, const scene_glyph_t *glyph)
{
    lv_font_t *font = font_registry_acquire(glyph->digest, glyph->size);
    if (font == NULL) {
        return NULL;
    }
    lv_obj_t *label = lv_label_create(parent);
    if (label == NULL) {
        font_registry_release(font);
        return NULL;
    }
    lv_obj_add_event_cb(label, release_font_cb, LV_EVENT_DELETE, font);
    lv_obj_set_style_text_font(label, font, 0);
    lv_obj_set_style_text_color(label, lv_color_hex(glyph->color), 0);

    char text[SCENE_MAX_GLYPH_NAME + 1U];
    glyph_text(glyph->name, text, sizeof text);
    lv_label_set_text(label, text);
    lv_obj_set_pos(label, glyph->x, baseline_box_top(glyph->baseline_y, font));
    return label;
}

/* The dial's fixed geometry and styling -- everything digital_clock.c never
 * varies. See scene_scale_t's field comment in scene_model.h for why these
 * are renderer constants rather than wire fields. */
#define SCENE_SCALE_VALUE_RANGE_MAX 720 /* digital_clock.c's DIAL_RANGE */
#define SCENE_SCALE_ANGLE_RANGE     360
#define SCENE_SCALE_ROTATION        270 /* twelve o'clock */
#define SCENE_SCALE_MINOR_TICK_WIDTH  2
#define SCENE_SCALE_MINOR_TICK_LENGTH 6
#define SCENE_SCALE_MAJOR_TICK_WIDTH  3
#define SCENE_SCALE_MAJOR_TICK_LENGTH 11
/* DESKMATE_COLOR_TERTIARY (template_internal.h) -- the minor tick colour
 * never varies by widget instance, unlike the major tick's palette.hue, so
 * it stays a constant here rather than becoming a wire field. */
#define SCENE_SCALE_MINOR_TICK_COLOR 0x5c5c66U

/* `scale` -- an lv_scale styled and driven exactly as digital_clock.c's dial
 * is (digital_clock.c:103-131): same calls, same order, same fixed
 * constants for everything that file never varies. Reproducing the call
 * sequence, not merely its visual effect, is the whole point of this node --
 * see scene_scale_t's field comment for why. */
static lv_obj_t *build_scale(lv_obj_t *parent, const scene_scale_t *scale)
{
    lv_obj_t *object = lv_scale_create(parent);
    if (object == NULL) {
        return NULL;
    }
    lv_obj_set_size(object, scale->box, scale->box);
    lv_obj_set_pos(object, scale->x, scale->y);
    lv_scale_set_mode(object, LV_SCALE_MODE_ROUND_INNER);
    lv_scale_set_label_show(object, false);
    lv_scale_set_total_tick_count(object, scale->total_tick_count);
    lv_scale_set_major_tick_every(object, scale->major_tick_every);
    lv_scale_set_range(object, 0, SCENE_SCALE_VALUE_RANGE_MAX);
    lv_scale_set_angle_range(object, SCENE_SCALE_ANGLE_RANGE);
    lv_scale_set_rotation(object, SCENE_SCALE_ROTATION);
    lv_obj_set_style_arc_width(object, 0, LV_PART_MAIN);
    /* minor ticks */
    lv_obj_set_style_line_color(object, lv_color_hex(SCENE_SCALE_MINOR_TICK_COLOR),
                                LV_PART_ITEMS);
    lv_obj_set_style_line_width(object, SCENE_SCALE_MINOR_TICK_WIDTH,
                                LV_PART_ITEMS);
    lv_obj_set_style_length(object, SCENE_SCALE_MINOR_TICK_LENGTH,
                            LV_PART_ITEMS);
    /* major ticks */
    lv_obj_set_style_line_color(object, lv_color_hex(scale->major_tick_color),
                                LV_PART_INDICATOR);
    lv_obj_set_style_line_width(object, SCENE_SCALE_MAJOR_TICK_WIDTH,
                                LV_PART_INDICATOR);
    lv_obj_set_style_length(object, SCENE_SCALE_MAJOR_TICK_LENGTH,
                            LV_PART_INDICATOR);
    lv_obj_set_style_line_opa(object, LV_OPA_70, LV_PART_INDICATOR);
    return object;
}

/* --------------------------------------------------------- the lifecycle */

/* Frees the per-scene state when its screen goes away. LVGL sends
 * LV_EVENT_DELETE to an object before recursing into its children, so this
 * runs before the labels release their faces -- which is fine, because a
 * face's release is bound to its own label and does not read this state. */
static void screen_deleted_cb(lv_event_t *event)
{
    scene_view_state_t *state =
        (scene_view_state_t *)lv_event_get_user_data(event);
    if (state == NULL) {
        return;
    }
    if (s_state == state) {
        s_state = NULL;
    }
    lv_free(state);
}

static uint32_t count_bound_nodes(const scene_t *scene)
{
    uint32_t count = 0U;
    for (uint32_t i = 0U; i < scene->node_count; i++) {
        const scene_node_t *node = &scene->nodes[i];
        if (node->kind == SCENE_NODE_TEXT &&
            value_is_bound(&node->value.text.value)) {
            ++count;
        } else if (node->kind == SCENE_NODE_LABEL &&
                   value_is_bound(&node->value.label.value)) {
            ++count;
        } else if (node->kind == SCENE_NODE_ARC &&
                   node->value.arc.end_binding[0] != '\0') {
            ++count;
        } else if (node->kind == SCENE_NODE_ROT_RECT &&
                   node->value.rot_rect.rotation_binding[0] != '\0') {
            ++count;
        }
    }
    return count;
}

/* Builds every node into `parent`, recording the binding-valued ones.
 * Returns false on the first node that cannot be rendered as specified; the
 * caller then destroys the whole candidate screen, which is what releases any
 * faces already acquired. */
static bool build_nodes(lv_obj_t *parent, const scene_t *scene,
                        const scene_binding_context_t *context,
                        scene_view_state_t *state)
{
    char buffer[SCENE_VIEW_TEXT_CAPACITY];

    for (uint32_t i = 0U; i < scene->node_count; i++) {
        const scene_node_t *node = &scene->nodes[i];
        lv_obj_t *object = NULL;

        switch (node->kind) {
        case SCENE_NODE_RECT:
            object = build_rect(parent, &node->value.rect);
            break;

        case SCENE_NODE_ARC: {
            const scene_arc_t *arc = &node->value.arc;
            scene_binding_t binding;

            object = build_arc(parent, arc);
            if (object == NULL || arc->end_binding[0] == '\0') {
                break;
            }
            /* A binding that will not parse leaves the arc at the static
             * sweep the scene declared, and is not recorded -- so no later
             * refresh can move it. */
            if (scene_binding_parse(arc->end_binding, &binding) !=
                SCENE_BINDING_OK) {
                break;
            }
            scene_bound_node_t *bound = &state->bound[state->bound_count++];
            bound->object = object;
            bound->kind = (uint8_t)SCENE_NODE_ARC;
            bound->binding = binding;
            bound->arc_span_deg =
                scene_model_arc_span(arc->start_deg, arc->end_deg);
            evaluate_binding(&bound->binding, context, buffer, sizeof buffer);
            apply_arc_binding(bound, buffer);
            break;
        }

        case SCENE_NODE_LINE:
            object = build_line(parent, &node->value.line);
            break;

        case SCENE_NODE_TEXT: {
            scene_binding_t binding;
            bool is_bound = false;

            object = build_text(parent, &node->value.text, context, &binding,
                                &is_bound);
            if (object != NULL && is_bound) {
                scene_bound_node_t *bound =
                    &state->bound[state->bound_count++];
                bound->object = object;
                bound->kind = (uint8_t)SCENE_NODE_TEXT;
                bound->binding = binding;
            }
            break;
        }

        case SCENE_NODE_IMAGE:
            object = build_image(parent, &node->value.image);
            break;

        case SCENE_NODE_GLYPH:
            object = build_glyph(parent, &node->value.glyph);
            break;

        case SCENE_NODE_SCALE:
            object = build_scale(parent, &node->value.scale);
            break;

        case SCENE_NODE_LABEL: {
            scene_binding_t binding;
            bool is_bound = false;

            object = build_label(parent, &node->value.label, context,
                                 &binding, &is_bound);
            if (object != NULL && is_bound) {
                scene_bound_node_t *bound =
                    &state->bound[state->bound_count++];
                bound->object = object;
                bound->kind = (uint8_t)SCENE_NODE_LABEL;
                bound->binding = binding;
                bound->hide_when_empty = node->value.label.hide_when_empty;
            }
            break;
        }

        case SCENE_NODE_ROT_RECT: {
            scene_rotation_binding_t binding = SCENE_ROTATION_BINDING_NONE;
            bool is_bound = false;

            object = build_rot_rect(parent, &node->value.rot_rect, context,
                                    &binding, &is_bound);
            if (object != NULL && is_bound) {
                scene_bound_node_t *bound =
                    &state->bound[state->bound_count++];
                bound->object = object;
                bound->kind = (uint8_t)SCENE_NODE_ROT_RECT;
                bound->rotation_binding = (uint8_t)binding;
            }
            break;
        }

        default:
            break;
        }

        if (object == NULL) {
            return false;
        }
    }
    return true;
}

void scene_view_set_asset_resolver(asset_resolver_fn resolver,
                                   asset_release_fn release)
{
    s_asset_resolver = resolver;
    s_asset_release = release;
}

bool scene_view_show(const scene_t *scene,
                     const scene_binding_context_t *context)
{
    if (scene == NULL || scene_model_validate(scene) != SCENE_MODEL_OK) {
        return false;
    }

    size_t bytes = sizeof(scene_view_state_t) +
                   (size_t)count_bound_nodes(scene) *
                       sizeof(scene_bound_node_t);
    scene_view_state_t *state = lv_malloc(bytes);
    if (state == NULL) {
        return false;
    }
    lv_memzero(state, bytes);

    /* Built against local state so a refusal leaves the currently displayed
     * screen and its object graph untouched -- template_view_show()'s
     * candidate pattern, and the reason a scene that cannot be rendered never
     * blanks the panel. */
    lv_obj_t *screen = lv_obj_create(NULL);
    if (screen == NULL) {
        lv_free(state);
        return false;
    }
    lv_obj_remove_flag(screen, LV_OBJ_FLAG_SCROLLABLE);
    lv_obj_add_flag(screen, LV_OBJ_FLAG_CLICKABLE);
    lv_obj_set_style_bg_color(screen, lv_color_hex(scene->background), 0);
    lv_obj_set_style_bg_opa(screen, LV_OPA_COVER, 0);

    /* The same styleless full-canvas container template_view.c puts under its
     * screen. It exists so node coordinates are absolute canvas coordinates
     * with no theme padding between them and the screen edge -- which is what
     * lets a scene land on the same pixels as the C face it reproduces. It
     * must not win hit testing; the carousel screen owns taps. */
    lv_obj_t *content = lv_obj_create(screen);
    if (content == NULL) {
        lv_obj_delete(screen);
        lv_free(state);
        return false;
    }
    lv_obj_remove_style_all(content);
    lv_obj_remove_flag(content,
                       LV_OBJ_FLAG_CLICKABLE | LV_OBJ_FLAG_SCROLLABLE);
    lv_obj_set_width(content, LV_PCT(100));
    lv_obj_set_height(content, LV_PCT(100));
    lv_obj_align(content, LV_ALIGN_CENTER, 0, 0);

    if (!build_nodes(content, scene, context, state)) {
        /* Deleting the screen sends LV_EVENT_DELETE down the whole tree,
         * which is what releases every face acquired so far. */
        lv_obj_delete(screen);
        lv_free(state);
        return false;
    }

    state->screen = screen;
    lv_obj_add_event_cb(screen, screen_deleted_cb, LV_EVENT_DELETE, state);
    /* Published before the load, because loading with auto_del deletes the
     * outgoing screen and its callback must see that it is no longer the
     * live state. */
    s_state = state;
    lv_screen_load_anim(screen, LV_SCREEN_LOAD_ANIM_NONE, 0U, 0U, true);
    return true;
}

void scene_view_refresh_bindings(const scene_binding_context_t *context)
{
    char buffer[SCENE_VIEW_TEXT_CAPACITY];

    if (s_state == NULL || context == NULL) {
        return;
    }
    for (uint32_t i = 0U; i < s_state->bound_count; i++) {
        const scene_bound_node_t *bound = &s_state->bound[i];
        if (bound->kind == (uint8_t)SCENE_NODE_ROT_RECT) {
            lv_obj_set_style_transform_rotation(
                bound->object,
                rotation_for_time(
                    (scene_rotation_binding_t)bound->rotation_binding,
                    context, 0),
                0);
            continue;
        }
        evaluate_binding(&bound->binding, context, buffer, sizeof buffer);
        if (bound->kind == (uint8_t)SCENE_NODE_ARC) {
            apply_arc_binding(bound, buffer);
        } else if (bound->kind == (uint8_t)SCENE_NODE_LABEL) {
            apply_label_text(bound->object, buffer, bound->hide_when_empty);
        } else {
            lv_label_set_text(bound->object, buffer);
        }
    }
}

bool scene_view_destroy(void)
{
    if (s_state == NULL) {
        return true;
    }
    if (lv_screen_active() == s_state->screen) {
        return false;
    }
    lv_obj_delete(s_state->screen);
    return true;
}

bool scene_view_active(void)
{
    return s_state != NULL && lv_screen_active() == s_state->screen;
}

lv_obj_t *scene_view_screen(void)
{
    return s_state != NULL ? s_state->screen : NULL;
}
