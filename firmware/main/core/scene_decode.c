#include "scene_decode.h"

#include <string.h>

#include "scene_binding.h"
#include "scene_model.h"

/* ------------------------------------------------------------------
 * The scene wire shape.
 *
 * Every map below is a CBOR map with unsigned-integer keys in strictly
 * increasing order, definite length, shortest-form heads. Unknown integer
 * keys are SKIPPED, not rejected, so a later protocol revision can add a
 * field without stranding a deployed device; a key that repeats or goes
 * backwards is rejected, because that is a malformed encoder, not a newer
 * one.
 *
 * scene    {0: revision (uint32), 1: background (uint32), 2: [node, ...]}
 *          all three required, at most SCENE_MAX_NODES nodes.
 *
 * node     {0: kind (scene_node_kind_t), 1: <kind-specific map>}
 *          both required. Key 0 precedes key 1 by the increasing-key rule,
 *          so the kind is always known before its payload is read.
 *
 * RECT     {0: x, 1: y, 2: w, 3: h, 4: radius, 5: fill, 6: opacity}
 *          required 0-3; radius defaults 0, fill 0, opacity 255 (an
 *          omitted opacity means opaque -- 0 would draw nothing).
 * ARC      {0: cx, 1: cy, 2: r, 3: start_deg, 4: end_deg, 5: width,
 *           6: color, 7: rounded (bool), 8: end_binding (text)}
 *          required 0-5; color defaults 0, rounded false, end_binding "".
 * LINE     {0: [x, ...], 1: [y, ...], 2: width, 3: color}
 *          required 0-2; color defaults 0. The two arrays must be the same
 *          length, at most SCENE_MAX_LINE_POINTS; that length is
 *          point_count.
 * TEXT     {0: x, 1: baseline_y, 2: w, 3: align, 4: <font>, 5: color,
 *           6: <value>, 7: ellipsize (bool)}
 *          required 0, 1, 2, 4, 6; align defaults SCENE_ALIGN_LEFT (NOT
 *          zero -- SCENE_ALIGN_LEFT is 1, so a zero-initialised align is
 *          the invalid value scene_model_validate() rejects), color 0,
 *          ellipsize false.
 * IMAGE    {0: x, 1: y, 2: w, 3: h, 4: digest (32 bytes), 5: recolor
 *           (bool), 6: color}
 *          required 0-4; recolor defaults false, color 0.
 * GLYPH    {0: x, 1: baseline_y, 2: size, 3: digest (32 bytes),
 *           4: name (text), 5: color}
 *          required 0-4; color defaults 0.
 * SCALE    {0: x, 1: y, 2: box, 3: total_tick_count, 4: major_tick_every,
 *           5: major_tick_color}
 *          required 0-4; major_tick_color defaults 0. The rest of the dial
 *          is a renderer constant, not a wire field -- see scene_scale_t.
 * LABEL    {0: x, 1: y, 2: <font>, 3: <value>, 4: ink, 5: fill,
 *           6: fill_opacity, 7: radius, 8: pad_hor, 9: pad_ver,
 *           10: letter_space, 11: hide_when_empty (bool)}
 *          required 0-3; colours/style integers default 0,
 *          hide_when_empty defaults false.
 * ROT_RECT {0: x, 1: y, 2: w, 3: h, 4: radius, 5: fill, 6: pivot_x,
 *           7: pivot_y, 8: rotation, 9: rotation_binding (text)}
 *          required 0-3; style/transform integers default 0 and an empty
 *          rotation_binding means the fixed rotation field is used.
 *
 * font     {0: kind, 1: baked (tier), 2: digest (32 bytes), 3: pixel_size}
 *          required 0, plus 2 when the kind is SCENE_FONT_ASSET. The tier
 *          and pixel_size are left to validate_font() in scene_model.c --
 *          an omitted one leaves 0 behind, which that function rejects for
 *          the kind that needs it. The DIGEST is the exception, and the
 *          reason this map has a conditional required key: validate_font()
 *          checks only pixel_size for an asset font (scene_model.c), so an
 *          all-zero digest would sail through and fail at draw time when
 *          the asset lookup missed. An asset font with no digest can never
 *          resolve, so it is refused here instead.
 * value    {0: kind, 1: literal (text), 2: binding (text)}
 *          required 0; literal and binding default to "".
 *
 * A BAKED font carries no digest, and the value map's two strings are
 * optional, which is what keeps a 24-node scene inside the 2034-byte
 * payload: were a digest required unconditionally, a baked-font TEXT node
 * would carry 34 wasted bytes of it, and 24 of those alone would nearly
 * fill the envelope. That is why the digest is required for the asset kind
 * specifically rather than for the map as a whole.
 *
 * ------------------------------------------------------------------
 * Decoding style.
 *
 * This mirrors core/protocol_message.c exactly and deliberately:
 * open_scene_map() validates the whole payload once up front, read_key()
 * enforces strictly increasing keys, skip_value() absorbs unknown ones,
 * and each map checks a required-key bitmask AFTER its loop. The helpers
 * are duplicated rather than shared because that file's are static to it,
 * and because this is the only other translation unit in core/ that may
 * include TinyCBOR.
 *
 * Nothing here allocates. Nothing here declares a scene_t, a scene_node_t,
 * or any other large object: every decoder writes through a pointer into
 * the caller's scene, so the deepest stack frame is a CborValue plus a few
 * scalars. The one non-trivial local is decode_value()'s scene_binding_t
 * (~56 bytes), which exists because scene_binding_parse() needs somewhere
 * to put its result -- the parse is for its verdict, not its output.
 * ------------------------------------------------------------------ */

#define REQUIRED_BIT(key) (UINT32_C(1) << (key))

/* Every TinyCBOR failure is structural: the payload is not a scene. There
 * is no CBOR-specific scene_model_result_t and none is added. */
static scene_model_result_t cbor_result(CborError error)
{
    return error == CborNoError ? SCENE_MODEL_OK : SCENE_MODEL_ERR_ARGUMENT;
}

/* Parses and whole-payload-validates a standalone scene buffer, leaving
 * `root` positioned AT the scene map without entering it -- entering is
 * scene_decode_map()'s job, so that one function is the only place a scene
 * map is read.
 *
 * The flag set is unchanged from before that extraction, CborValidate-
 * CompleteData included: a standalone payload with trailing bytes is not a
 * scene. A scene nested in a larger message inherits the equivalent
 * guarantees from its enclosing payload's own root validation -- see
 * scene_decode.h. */
static scene_model_result_t open_scene_payload(const uint8_t *payload,
                                                size_t payload_length,
                                                CborParser *parser,
                                                CborValue *root)
{
    CborError error =
        cbor_parser_init(payload, payload_length, 0, parser, root);
    if (error != CborNoError) {
        return cbor_result(error);
    }
    error = cbor_value_validate(
        root,
        CborValidateCanonicalFormat | CborValidateMapKeysAreUnique |
            CborValidateUtf8 | CborValidateCompleteData |
            CborValidateNoUndefined | CborValidateNoTags);
    return cbor_result(error);
}

static scene_model_result_t enter_map(CborValue *value, CborValue *fields,
                                       size_t *count)
{
    if (!cbor_value_is_map(value)) {
        return SCENE_MODEL_ERR_ARGUMENT;
    }
    CborError error = cbor_value_get_map_length(value, count);
    if (error != CborNoError) {
        return cbor_result(error);
    }
    return cbor_result(cbor_value_enter_container(value, fields));
}

static scene_model_result_t read_unsigned(CborValue *value, uint64_t *result)
{
    if (!cbor_value_is_unsigned_integer(value)) {
        return SCENE_MODEL_ERR_ARGUMENT;
    }
    CborError error = cbor_value_get_uint64(value, result);
    if (error == CborNoError) {
        error = cbor_value_advance_fixed(value);
    }
    return cbor_result(error);
}

static scene_model_result_t read_signed(CborValue *value, int64_t *result)
{
    if (!cbor_value_is_integer(value)) {
        return SCENE_MODEL_ERR_ARGUMENT;
    }
    CborError error = cbor_value_get_int64_checked(value, result);
    if (error == CborNoError) {
        error = cbor_value_advance_fixed(value);
    }
    return cbor_result(error);
}

static scene_model_result_t read_boolean(CborValue *value, bool *result)
{
    if (!cbor_value_is_boolean(value)) {
        return SCENE_MODEL_ERR_ARGUMENT;
    }
    CborError error = cbor_value_get_boolean(value, result);
    if (error == CborNoError) {
        error = cbor_value_advance_fixed(value);
    }
    return cbor_result(error);
}

/* A wire integer that does not fit the C type holding it is rejected, not
 * truncated: a coordinate near INT32_MAX that silently became a small
 * positive number would pass scene_model_validate() and draw in the wrong
 * place. */
static scene_model_result_t read_int32(CborValue *value, int32_t *result)
{
    int64_t raw = 0;
    scene_model_result_t status = read_signed(value, &raw);
    if (status != SCENE_MODEL_OK) {
        return status;
    }
    if (raw < INT32_MIN || raw > INT32_MAX) {
        return SCENE_MODEL_ERR_GEOMETRY;
    }
    *result = (int32_t)raw;
    return SCENE_MODEL_OK;
}

static scene_model_result_t read_uint32(CborValue *value, uint32_t *result)
{
    uint64_t raw = 0U;
    scene_model_result_t status = read_unsigned(value, &raw);
    if (status != SCENE_MODEL_OK) {
        return status;
    }
    if (raw > UINT32_MAX) {
        return SCENE_MODEL_ERR_GEOMETRY;
    }
    *result = (uint32_t)raw;
    return SCENE_MODEL_OK;
}

static scene_model_result_t read_uint8(CborValue *value, uint8_t *result)
{
    uint64_t raw = 0U;
    scene_model_result_t status = read_unsigned(value, &raw);
    if (status != SCENE_MODEL_OK) {
        return status;
    }
    if (raw > UINT8_MAX) {
        return SCENE_MODEL_ERR_GEOMETRY;
    }
    *result = (uint8_t)raw;
    return SCENE_MODEL_OK;
}

/* The length is taken from the CBOR head and checked BEFORE any copy, so a
 * host-declared string longer than the destination never reaches memcpy.
 * The terminator is written here; nothing downstream may assume the wire
 * supplied one. */
static scene_model_result_t read_text(CborValue *value, char *destination,
                                       size_t destination_capacity,
                                       size_t maximum_length)
{
    if (!cbor_value_is_text_string(value)) {
        return SCENE_MODEL_ERR_ARGUMENT;
    }
    size_t length = 0U;
    CborError error = cbor_value_calculate_string_length(value, &length);
    if (error != CborNoError) {
        return cbor_result(error);
    }
    if (length > maximum_length || destination_capacity <= length) {
        return SCENE_MODEL_ERR_TEXT;
    }
    size_t copied = destination_capacity - 1U;
    CborValue next;
    error = cbor_value_copy_text_string(value, destination, &copied, &next);
    if (error != CborNoError) {
        return cbor_result(error);
    }
    destination[copied] = '\0';
    *value = next;
    return SCENE_MODEL_OK;
}

static scene_model_result_t read_digest(CborValue *value, uint8_t *destination)
{
    if (!cbor_value_is_byte_string(value)) {
        return SCENE_MODEL_ERR_ARGUMENT;
    }
    size_t length = 0U;
    CborError error = cbor_value_calculate_string_length(value, &length);
    if (error != CborNoError) {
        return cbor_result(error);
    }
    if (length != ASSET_DIGEST_BYTES) {
        return SCENE_MODEL_ERR_ARGUMENT;
    }
    size_t copied = ASSET_DIGEST_BYTES;
    CborValue next;
    error = cbor_value_copy_byte_string(value, destination, &copied, &next);
    if (error != CborNoError) {
        return cbor_result(error);
    }
    *value = next;
    return SCENE_MODEL_OK;
}

static scene_model_result_t read_key(CborValue *contents, uint64_t *key,
                                      uint64_t *previous, bool *has_previous)
{
    scene_model_result_t status = read_unsigned(contents, key);
    if (status != SCENE_MODEL_OK) {
        return status;
    }
    if (*has_previous && *key <= *previous) {
        return SCENE_MODEL_ERR_ARGUMENT;
    }
    *previous = *key;
    *has_previous = true;
    return SCENE_MODEL_OK;
}

static scene_model_result_t skip_value(CborValue *value)
{
    return cbor_result(cbor_value_advance(value));
}

/* ------------------------------------------------------------------
 * Node payloads.
 * ------------------------------------------------------------------ */

static scene_model_result_t decode_rect(CborValue *value, scene_rect_t *rect)
{
    CborValue fields;
    size_t count = 0U;
    scene_model_result_t status = enter_map(value, &fields, &count);
    if (status != SCENE_MODEL_OK) {
        return status;
    }

    rect->opacity = UINT8_MAX;

    uint32_t present = 0U;
    uint64_t previous = 0U;
    bool has_previous = false;
    for (size_t i = 0U; i < count; ++i) {
        uint64_t key = 0U;
        status = read_key(&fields, &key, &previous, &has_previous);
        if (status != SCENE_MODEL_OK) {
            return status;
        }
        switch (key) {
        case 0U: status = read_int32(&fields, &rect->x); break;
        case 1U: status = read_int32(&fields, &rect->y); break;
        case 2U: status = read_int32(&fields, &rect->w); break;
        case 3U: status = read_int32(&fields, &rect->h); break;
        case 4U: status = read_int32(&fields, &rect->radius); break;
        case 5U: status = read_uint32(&fields, &rect->fill); break;
        case 6U: status = read_uint8(&fields, &rect->opacity); break;
        default: status = skip_value(&fields); break;
        }
        if (status != SCENE_MODEL_OK) {
            return status;
        }
        if (key <= 6U) {
            present |= REQUIRED_BIT((uint32_t)key);
        }
    }
    if ((present & UINT32_C(0x0f)) != UINT32_C(0x0f)) {
        return SCENE_MODEL_ERR_ARGUMENT;
    }
    return cbor_result(cbor_value_leave_container(value, &fields));
}

static scene_model_result_t decode_arc(CborValue *value, scene_arc_t *arc)
{
    CborValue fields;
    size_t count = 0U;
    scene_model_result_t status = enter_map(value, &fields, &count);
    if (status != SCENE_MODEL_OK) {
        return status;
    }

    uint32_t present = 0U;
    uint64_t previous = 0U;
    bool has_previous = false;
    for (size_t i = 0U; i < count; ++i) {
        uint64_t key = 0U;
        status = read_key(&fields, &key, &previous, &has_previous);
        if (status != SCENE_MODEL_OK) {
            return status;
        }
        switch (key) {
        case 0U: status = read_int32(&fields, &arc->cx); break;
        case 1U: status = read_int32(&fields, &arc->cy); break;
        case 2U: status = read_int32(&fields, &arc->r); break;
        case 3U: status = read_int32(&fields, &arc->start_deg); break;
        case 4U: status = read_int32(&fields, &arc->end_deg); break;
        case 5U: status = read_int32(&fields, &arc->width); break;
        case 6U: status = read_uint32(&fields, &arc->color); break;
        case 7U: status = read_boolean(&fields, &arc->rounded); break;
        case 8U:
            status = read_text(&fields, arc->end_binding,
                               sizeof arc->end_binding, SCENE_MAX_BINDING);
            break;
        default: status = skip_value(&fields); break;
        }
        if (status != SCENE_MODEL_OK) {
            return status;
        }
        if (key <= 8U) {
            present |= REQUIRED_BIT((uint32_t)key);
        }
    }
    if ((present & UINT32_C(0x3f)) != UINT32_C(0x3f)) {
        return SCENE_MODEL_ERR_ARGUMENT;
    }
    /* An arc that names a binding the device cannot evaluate refuses the
     * whole scene here, rather than drawing an arc with no data behind it.
     * An absent end_binding is the empty string, which is not a binding. */
    if (arc->end_binding[0] != '\0') {
        scene_binding_t parsed;
        if (scene_binding_parse(arc->end_binding, &parsed) !=
            SCENE_BINDING_OK) {
            return SCENE_MODEL_ERR_TEXT;
        }
    }
    return cbor_result(cbor_value_leave_container(value, &fields));
}

/* Reads one of a LINE's two coordinate arrays. The array length is bounded
 * against SCENE_MAX_LINE_POINTS before a single element is read, so an
 * untrusted count cannot walk off the fixed arrays in scene_line_t. */
static scene_model_result_t read_point_array(CborValue *value,
                                              int32_t *points,
                                              uint32_t *point_count)
{
    if (!cbor_value_is_array(value)) {
        return SCENE_MODEL_ERR_ARGUMENT;
    }
    size_t count = 0U;
    CborError error = cbor_value_get_array_length(value, &count);
    if (error != CborNoError) {
        return cbor_result(error);
    }
    if (count > SCENE_MAX_LINE_POINTS) {
        return SCENE_MODEL_ERR_GEOMETRY;
    }
    CborValue items;
    error = cbor_value_enter_container(value, &items);
    if (error != CborNoError) {
        return cbor_result(error);
    }
    for (size_t i = 0U; i < count; ++i) {
        scene_model_result_t status = read_int32(&items, &points[i]);
        if (status != SCENE_MODEL_OK) {
            return status;
        }
    }
    *point_count = (uint32_t)count;
    return cbor_result(cbor_value_leave_container(value, &items));
}

static scene_model_result_t decode_line(CborValue *value, scene_line_t *line)
{
    CborValue fields;
    size_t count = 0U;
    scene_model_result_t status = enter_map(value, &fields, &count);
    if (status != SCENE_MODEL_OK) {
        return status;
    }

    uint32_t x_count = 0U;
    uint32_t y_count = 0U;
    uint32_t present = 0U;
    uint64_t previous = 0U;
    bool has_previous = false;
    for (size_t i = 0U; i < count; ++i) {
        uint64_t key = 0U;
        status = read_key(&fields, &key, &previous, &has_previous);
        if (status != SCENE_MODEL_OK) {
            return status;
        }
        switch (key) {
        case 0U: status = read_point_array(&fields, line->xs, &x_count); break;
        case 1U: status = read_point_array(&fields, line->ys, &y_count); break;
        case 2U: status = read_int32(&fields, &line->width); break;
        case 3U: status = read_uint32(&fields, &line->color); break;
        default: status = skip_value(&fields); break;
        }
        if (status != SCENE_MODEL_OK) {
            return status;
        }
        if (key <= 3U) {
            present |= REQUIRED_BIT((uint32_t)key);
        }
    }
    if ((present & UINT32_C(0x07)) != UINT32_C(0x07)) {
        return SCENE_MODEL_ERR_ARGUMENT;
    }
    /* Two arrays of different lengths describe no polyline at all; taking
     * the shorter would silently drop a point the host meant to draw. The
     * lower bound of 2 belongs to scene_model_validate(). */
    if (x_count != y_count) {
        return SCENE_MODEL_ERR_GEOMETRY;
    }
    line->point_count = x_count;
    return cbor_result(cbor_value_leave_container(value, &fields));
}

static scene_model_result_t decode_font(CborValue *value,
                                         scene_font_ref_t *font)
{
    CborValue fields;
    size_t count = 0U;
    scene_model_result_t status = enter_map(value, &fields, &count);
    if (status != SCENE_MODEL_OK) {
        return status;
    }

    uint32_t present = 0U;
    uint64_t previous = 0U;
    bool has_previous = false;
    for (size_t i = 0U; i < count; ++i) {
        uint64_t key = 0U;
        status = read_key(&fields, &key, &previous, &has_previous);
        if (status != SCENE_MODEL_OK) {
            return status;
        }
        switch (key) {
        case 0U: {
            uint32_t raw = 0U;
            status = read_uint32(&fields, &raw);
            if (status == SCENE_MODEL_OK) {
                font->kind = (scene_font_kind_t)raw;
            }
            break;
        }
        case 1U: {
            uint32_t raw = 0U;
            status = read_uint32(&fields, &raw);
            if (status == SCENE_MODEL_OK) {
                font->baked = (scene_font_tier_t)raw;
            }
            break;
        }
        case 2U: status = read_digest(&fields, font->digest); break;
        case 3U: status = read_int32(&fields, &font->pixel_size); break;
        default: status = skip_value(&fields); break;
        }
        if (status != SCENE_MODEL_OK) {
            return status;
        }
        if (key <= 3U) {
            present |= REQUIRED_BIT((uint32_t)key);
        }
    }
    /* The kind is always required. Whether the TIER or the PIXEL SIZE had
     * to come with it depends on that kind, and validate_font() in
     * scene_model.c is where that lives -- it rejects a zero tier and an
     * out-of-range pixel size, so neither is re-checked here.
     *
     * The DIGEST is the one thing validate_font() does not cover: for
     * SCENE_FONT_ASSET it checks pixel_size and nothing else, so a font
     * map of {0: 2, 3: 20} would decode to an asset font with an all-zero
     * digest and be reported OK. That scene cannot draw -- the asset
     * lookup misses -- and this plan's rule is to refuse a scene whole
     * rather than let it fail at draw time. */
    if ((present & UINT32_C(0x01)) != UINT32_C(0x01)) {
        return SCENE_MODEL_ERR_ARGUMENT;
    }
    if (font->kind == SCENE_FONT_ASSET &&
        (present & REQUIRED_BIT(2)) == 0U) {
        return SCENE_MODEL_ERR_ARGUMENT;
    }
    return cbor_result(cbor_value_leave_container(value, &fields));
}

static scene_model_result_t decode_value(CborValue *value,
                                          scene_value_t *result)
{
    CborValue fields;
    size_t count = 0U;
    scene_model_result_t status = enter_map(value, &fields, &count);
    if (status != SCENE_MODEL_OK) {
        return status;
    }

    uint32_t present = 0U;
    uint64_t previous = 0U;
    bool has_previous = false;
    for (size_t i = 0U; i < count; ++i) {
        uint64_t key = 0U;
        status = read_key(&fields, &key, &previous, &has_previous);
        if (status != SCENE_MODEL_OK) {
            return status;
        }
        switch (key) {
        case 0U: {
            uint32_t raw = 0U;
            status = read_uint32(&fields, &raw);
            if (status == SCENE_MODEL_OK) {
                result->kind = (scene_value_kind_t)raw;
            }
            break;
        }
        case 1U:
            status = read_text(&fields, result->literal,
                               sizeof result->literal, SCENE_MAX_TEXT_BYTES);
            break;
        case 2U:
            status = read_text(&fields, result->binding,
                               sizeof result->binding, SCENE_MAX_BINDING);
            break;
        default: status = skip_value(&fields); break;
        }
        if (status != SCENE_MODEL_OK) {
            return status;
        }
        if (key <= 2U) {
            present |= REQUIRED_BIT((uint32_t)key);
        }
    }
    if ((present & UINT32_C(0x01)) != UINT32_C(0x01)) {
        return SCENE_MODEL_ERR_ARGUMENT;
    }
    /* THE point of parsing during decode: a scene carrying a binding this
     * device cannot evaluate is refused whole, right here. Accepting it and
     * discovering the problem at draw time would leave the panel
     * half-rendered. `parsed` is discarded -- the verdict is what matters;
     * ui/scene_view.c parses again when it builds its bound-node table. */
    if (result->kind == SCENE_VALUE_BINDING) {
        scene_binding_t parsed;
        if (scene_binding_parse(result->binding, &parsed) !=
            SCENE_BINDING_OK) {
            return SCENE_MODEL_ERR_TEXT;
        }
    }
    return cbor_result(cbor_value_leave_container(value, &fields));
}

static scene_model_result_t decode_text(CborValue *value, scene_text_t *text)
{
    CborValue fields;
    size_t count = 0U;
    scene_model_result_t status = enter_map(value, &fields, &count);
    if (status != SCENE_MODEL_OK) {
        return status;
    }

    /* SCENE_ALIGN_LEFT is 1, so this default cannot come from zeroing the
     * node: a text node that omits key 3 would otherwise carry align 0 and
     * be rejected by scene_model_validate() on arrival. */
    text->align = SCENE_ALIGN_LEFT;

    uint32_t present = 0U;
    uint64_t previous = 0U;
    bool has_previous = false;
    for (size_t i = 0U; i < count; ++i) {
        uint64_t key = 0U;
        status = read_key(&fields, &key, &previous, &has_previous);
        if (status != SCENE_MODEL_OK) {
            return status;
        }
        switch (key) {
        case 0U: status = read_int32(&fields, &text->x); break;
        case 1U: status = read_int32(&fields, &text->baseline_y); break;
        case 2U: status = read_int32(&fields, &text->w); break;
        case 3U: {
            uint32_t raw = 0U;
            status = read_uint32(&fields, &raw);
            if (status == SCENE_MODEL_OK) {
                text->align = (scene_align_t)raw;
            }
            break;
        }
        case 4U: status = decode_font(&fields, &text->font); break;
        case 5U: status = read_uint32(&fields, &text->color); break;
        case 6U: status = decode_value(&fields, &text->value); break;
        case 7U: status = read_boolean(&fields, &text->ellipsize); break;
        default: status = skip_value(&fields); break;
        }
        if (status != SCENE_MODEL_OK) {
            return status;
        }
        if (key <= 7U) {
            present |= REQUIRED_BIT((uint32_t)key);
        }
    }
    if ((present & UINT32_C(0x57)) != UINT32_C(0x57)) {
        return SCENE_MODEL_ERR_ARGUMENT;
    }
    return cbor_result(cbor_value_leave_container(value, &fields));
}

static scene_model_result_t decode_image(CborValue *value,
                                          scene_image_t *image)
{
    CborValue fields;
    size_t count = 0U;
    scene_model_result_t status = enter_map(value, &fields, &count);
    if (status != SCENE_MODEL_OK) {
        return status;
    }

    uint32_t present = 0U;
    uint64_t previous = 0U;
    bool has_previous = false;
    for (size_t i = 0U; i < count; ++i) {
        uint64_t key = 0U;
        status = read_key(&fields, &key, &previous, &has_previous);
        if (status != SCENE_MODEL_OK) {
            return status;
        }
        switch (key) {
        case 0U: status = read_int32(&fields, &image->x); break;
        case 1U: status = read_int32(&fields, &image->y); break;
        case 2U: status = read_int32(&fields, &image->w); break;
        case 3U: status = read_int32(&fields, &image->h); break;
        case 4U: status = read_digest(&fields, image->digest); break;
        case 5U: status = read_boolean(&fields, &image->recolor); break;
        case 6U: status = read_uint32(&fields, &image->color); break;
        default: status = skip_value(&fields); break;
        }
        if (status != SCENE_MODEL_OK) {
            return status;
        }
        if (key <= 6U) {
            present |= REQUIRED_BIT((uint32_t)key);
        }
    }
    if ((present & UINT32_C(0x1f)) != UINT32_C(0x1f)) {
        return SCENE_MODEL_ERR_ARGUMENT;
    }
    return cbor_result(cbor_value_leave_container(value, &fields));
}

static scene_model_result_t decode_glyph(CborValue *value,
                                          scene_glyph_t *glyph)
{
    CborValue fields;
    size_t count = 0U;
    scene_model_result_t status = enter_map(value, &fields, &count);
    if (status != SCENE_MODEL_OK) {
        return status;
    }

    uint32_t present = 0U;
    uint64_t previous = 0U;
    bool has_previous = false;
    for (size_t i = 0U; i < count; ++i) {
        uint64_t key = 0U;
        status = read_key(&fields, &key, &previous, &has_previous);
        if (status != SCENE_MODEL_OK) {
            return status;
        }
        switch (key) {
        case 0U: status = read_int32(&fields, &glyph->x); break;
        case 1U: status = read_int32(&fields, &glyph->baseline_y); break;
        case 2U: status = read_int32(&fields, &glyph->size); break;
        case 3U: status = read_digest(&fields, glyph->digest); break;
        case 4U:
            status = read_text(&fields, glyph->name, sizeof glyph->name,
                               SCENE_MAX_GLYPH_NAME);
            break;
        case 5U: status = read_uint32(&fields, &glyph->color); break;
        default: status = skip_value(&fields); break;
        }
        if (status != SCENE_MODEL_OK) {
            return status;
        }
        if (key <= 5U) {
            present |= REQUIRED_BIT((uint32_t)key);
        }
    }
    if ((present & UINT32_C(0x1f)) != UINT32_C(0x1f)) {
        return SCENE_MODEL_ERR_ARGUMENT;
    }
    return cbor_result(cbor_value_leave_container(value, &fields));
}

static scene_model_result_t decode_scale(CborValue *value,
                                          scene_scale_t *scale)
{
    CborValue fields;
    size_t count = 0U;
    scene_model_result_t status = enter_map(value, &fields, &count);
    if (status != SCENE_MODEL_OK) {
        return status;
    }

    uint32_t present = 0U;
    uint64_t previous = 0U;
    bool has_previous = false;
    for (size_t i = 0U; i < count; ++i) {
        uint64_t key = 0U;
        status = read_key(&fields, &key, &previous, &has_previous);
        if (status != SCENE_MODEL_OK) {
            return status;
        }
        switch (key) {
        case 0U: status = read_int32(&fields, &scale->x); break;
        case 1U: status = read_int32(&fields, &scale->y); break;
        case 2U: status = read_int32(&fields, &scale->box); break;
        case 3U: status = read_uint32(&fields, &scale->total_tick_count); break;
        case 4U: status = read_uint32(&fields, &scale->major_tick_every); break;
        case 5U: status = read_uint32(&fields, &scale->major_tick_color); break;
        default: status = skip_value(&fields); break;
        }
        if (status != SCENE_MODEL_OK) {
            return status;
        }
        if (key <= 5U) {
            present |= REQUIRED_BIT((uint32_t)key);
        }
    }
    if ((present & UINT32_C(0x1f)) != UINT32_C(0x1f)) {
        return SCENE_MODEL_ERR_ARGUMENT;
    }
    /* The tick bounds -- SCENE_SCALE_MAX_TOTAL_TICKS, and major_tick_every
     * within the total -- belong to scene_model_validate(). */
    return cbor_result(cbor_value_leave_container(value, &fields));
}

static scene_model_result_t decode_label(CborValue *value,
                                         scene_label_t *label)
{
    CborValue fields;
    size_t count = 0U;
    scene_model_result_t status = enter_map(value, &fields, &count);
    if (status != SCENE_MODEL_OK) {
        return status;
    }

    uint32_t present = 0U;
    uint64_t previous = 0U;
    bool has_previous = false;
    for (size_t i = 0U; i < count; ++i) {
        uint64_t key = 0U;
        status = read_key(&fields, &key, &previous, &has_previous);
        if (status != SCENE_MODEL_OK) {
            return status;
        }
        switch (key) {
        case 0U: status = read_int32(&fields, &label->x); break;
        case 1U: status = read_int32(&fields, &label->y); break;
        case 2U: status = decode_font(&fields, &label->font); break;
        case 3U: status = decode_value(&fields, &label->value); break;
        case 4U: status = read_uint32(&fields, &label->ink); break;
        case 5U: status = read_uint32(&fields, &label->fill); break;
        case 6U: status = read_uint8(&fields, &label->fill_opacity); break;
        case 7U: status = read_int32(&fields, &label->radius); break;
        case 8U: status = read_int32(&fields, &label->pad_hor); break;
        case 9U: status = read_int32(&fields, &label->pad_ver); break;
        case 10U: status = read_int32(&fields, &label->letter_space); break;
        case 11U: status = read_boolean(&fields, &label->hide_when_empty); break;
        default: status = skip_value(&fields); break;
        }
        if (status != SCENE_MODEL_OK) {
            return status;
        }
        if (key <= 11U) {
            present |= REQUIRED_BIT((uint32_t)key);
        }
    }
    if ((present & UINT32_C(0x0f)) != UINT32_C(0x0f)) {
        return SCENE_MODEL_ERR_ARGUMENT;
    }
    return cbor_result(cbor_value_leave_container(value, &fields));
}

static scene_model_result_t decode_rot_rect(CborValue *value,
                                            scene_rot_rect_t *rect)
{
    CborValue fields;
    size_t count = 0U;
    scene_model_result_t status = enter_map(value, &fields, &count);
    if (status != SCENE_MODEL_OK) {
        return status;
    }

    uint32_t present = 0U;
    uint64_t previous = 0U;
    bool has_previous = false;
    for (size_t i = 0U; i < count; ++i) {
        uint64_t key = 0U;
        status = read_key(&fields, &key, &previous, &has_previous);
        if (status != SCENE_MODEL_OK) {
            return status;
        }
        switch (key) {
        case 0U: status = read_int32(&fields, &rect->x); break;
        case 1U: status = read_int32(&fields, &rect->y); break;
        case 2U: status = read_int32(&fields, &rect->w); break;
        case 3U: status = read_int32(&fields, &rect->h); break;
        case 4U: status = read_int32(&fields, &rect->radius); break;
        case 5U: status = read_uint32(&fields, &rect->fill); break;
        case 6U: status = read_int32(&fields, &rect->pivot_x); break;
        case 7U: status = read_int32(&fields, &rect->pivot_y); break;
        case 8U: status = read_int32(&fields, &rect->rotation); break;
        case 9U:
            status = read_text(&fields, rect->rotation_binding,
                               sizeof rect->rotation_binding,
                               SCENE_MAX_BINDING);
            break;
        default: status = skip_value(&fields); break;
        }
        if (status != SCENE_MODEL_OK) {
            return status;
        }
        if (key <= 9U) {
            present |= REQUIRED_BIT((uint32_t)key);
        }
    }
    if ((present & UINT32_C(0x0f)) != UINT32_C(0x0f)) {
        return SCENE_MODEL_ERR_ARGUMENT;
    }
    return cbor_result(cbor_value_leave_container(value, &fields));
}

static scene_model_result_t decode_node_payload(CborValue *value,
                                                 scene_node_t *node)
{
    switch (node->kind) {
    case SCENE_NODE_RECT:  return decode_rect(value, &node->value.rect);
    case SCENE_NODE_ARC:   return decode_arc(value, &node->value.arc);
    case SCENE_NODE_LINE:  return decode_line(value, &node->value.line);
    case SCENE_NODE_TEXT:  return decode_text(value, &node->value.text);
    case SCENE_NODE_IMAGE: return decode_image(value, &node->value.image);
    case SCENE_NODE_GLYPH: return decode_glyph(value, &node->value.glyph);
    case SCENE_NODE_SCALE: return decode_scale(value, &node->value.scale);
    case SCENE_NODE_LABEL: return decode_label(value, &node->value.label);
    case SCENE_NODE_ROT_RECT:
        return decode_rot_rect(value, &node->value.rot_rect);
    default:               return SCENE_MODEL_ERR_NODE_KIND;
    }
}

static scene_model_result_t decode_node(CborValue *value, scene_node_t *node)
{
    CborValue fields;
    size_t count = 0U;
    scene_model_result_t status = enter_map(value, &fields, &count);
    if (status != SCENE_MODEL_OK) {
        return status;
    }

    /* Clears whichever union member the kind selects, so every optional
     * key's default is this decoder's, not the caller's zeroing. */
    memset(&node->value, 0, sizeof node->value);

    uint32_t present = 0U;
    uint64_t previous = 0U;
    bool has_previous = false;
    for (size_t i = 0U; i < count; ++i) {
        uint64_t key = 0U;
        status = read_key(&fields, &key, &previous, &has_previous);
        if (status != SCENE_MODEL_OK) {
            return status;
        }
        switch (key) {
        case 0U: {
            uint32_t raw = 0U;
            status = read_uint32(&fields, &raw);
            if (status == SCENE_MODEL_OK) {
                node->kind = (scene_node_kind_t)raw;
            }
            break;
        }
        case 1U:
            /* Keys are strictly increasing, so key 0 has already set the
             * kind by the time the payload arrives. A payload with no kind
             * before it falls to decode_node_payload()'s default and is
             * refused as an unknown kind. */
            status = decode_node_payload(&fields, node);
            break;
        default: status = skip_value(&fields); break;
        }
        if (status != SCENE_MODEL_OK) {
            return status;
        }
        if (key <= 1U) {
            present |= REQUIRED_BIT((uint32_t)key);
        }
    }
    if ((present & UINT32_C(0x03)) != UINT32_C(0x03)) {
        return SCENE_MODEL_ERR_ARGUMENT;
    }
    return cbor_result(cbor_value_leave_container(value, &fields));
}

static scene_model_result_t decode_nodes(CborValue *value, scene_t *out,
                                          uint32_t *node_count)
{
    if (!cbor_value_is_array(value)) {
        return SCENE_MODEL_ERR_ARGUMENT;
    }
    size_t count = 0U;
    CborError error = cbor_value_get_array_length(value, &count);
    if (error != CborNoError) {
        return cbor_result(error);
    }
    /* Bounded before a single node is read: the array length is a host
     * number and out->nodes is fixed at SCENE_MAX_NODES. Rejected, not
     * truncated to the first 24. */
    if (count > SCENE_MAX_NODES) {
        return SCENE_MODEL_ERR_NODE_COUNT;
    }
    CborValue items;
    error = cbor_value_enter_container(value, &items);
    if (error != CborNoError) {
        return cbor_result(error);
    }
    for (size_t i = 0U; i < count; ++i) {
        scene_model_result_t status = decode_node(&items, &out->nodes[i]);
        if (status != SCENE_MODEL_OK) {
            return status;
        }
    }
    *node_count = (uint32_t)count;
    return cbor_result(cbor_value_leave_container(value, &items));
}

/* The scene map itself. Split out of scene_decode() so a scene nested
 * under another message's key -- PushScene's key 2 -- decodes through this
 * exact code rather than a second implementation of it. See
 * scene_decode.h for what a caller must have established before calling,
 * and for the result contract, which is unchanged. */
scene_model_result_t scene_decode_map(CborValue *value, scene_t *out)
{
    /* Cleared BEFORE the argument guard, not after it -- the same ordering
     * scene_decode() needs, and for the same reason. The contract in
     * scene_decode.h promises node_count is 0 on ANY non-OK result without
     * qualification, and a `value == NULL` call with a valid `out` is a
     * non-OK result; a caller whose scene_t came from heap_caps_malloc()
     * rather than calloc() would otherwise read uninitialised heap as a
     * node count. "Did not touch it" is not "left it empty".
     *
     * Cleared on entry rather than trusted from the caller for the same
     * reason it covers the is-it-a-map rejection below: this function owes
     * the guarantee itself. It clears the COUNT only, not the whole ~6 KB
     * struct -- the node array below the count is meaningless either way,
     * and the sole caller has already zeroed its message. */
    if (out != NULL) {
        out->node_count = 0U;
    }
    if (value == NULL || out == NULL) {
        return SCENE_MODEL_ERR_ARGUMENT;
    }

    CborValue contents;
    size_t count = 0U;
    scene_model_result_t status = enter_map(value, &contents, &count);
    if (status != SCENE_MODEL_OK) {
        return status;
    }

    /* Held back until the whole map has decoded, so every failure path
     * below leaves out->node_count at 0 and no partially decoded node is
     * reachable. This is what "refused whole" means concretely. */
    uint32_t node_count = 0U;
    uint32_t present = 0U;
    uint64_t previous = 0U;
    bool has_previous = false;
    for (size_t i = 0U; i < count; ++i) {
        uint64_t key = 0U;
        status = read_key(&contents, &key, &previous, &has_previous);
        if (status != SCENE_MODEL_OK) {
            return status;
        }
        switch (key) {
        case 0U: status = read_uint32(&contents, &out->revision); break;
        case 1U: status = read_uint32(&contents, &out->background); break;
        case 2U: status = decode_nodes(&contents, out, &node_count); break;
        default: status = skip_value(&contents); break;
        }
        if (status != SCENE_MODEL_OK) {
            return status;
        }
        if (key <= 2U) {
            present |= REQUIRED_BIT((uint32_t)key);
        }
    }
    if ((present & UINT32_C(0x07)) != UINT32_C(0x07)) {
        return SCENE_MODEL_ERR_ARGUMENT;
    }

    /* Advances the CALLER's cursor past this map, which is what lets an
     * enclosing decoder go on reading its own sibling keys. A standalone
     * payload has no siblings, so this was not needed before the split;
     * skipping it on a nested scene would leave the parent parked inside
     * the scene. */
    status = cbor_result(cbor_value_leave_container(value, &contents));
    if (status != SCENE_MODEL_OK) {
        return status;
    }

    out->node_count = node_count;

    /* The validation half of this function's contract -- see scene_model.h.
     * Every bound the model owns is checked exactly once, here, and this
     * file does not duplicate any of them. A failure leaves node_count set,
     * so it is cleared before returning. */
    scene_model_result_t validation = scene_model_validate(out);
    if (validation != SCENE_MODEL_OK) {
        out->node_count = 0U;
    }
    return validation;
}

scene_model_result_t scene_decode(const uint8_t *payload, size_t length,
                                  scene_t *out)
{
    /* Cleared BEFORE the argument guard, not after it, for the same reason
     * scene_decode_map() clears node_count on entry -- and kept here as
     * well as there because these three exits return without ever reaching
     * it. */
    if (out != NULL) {
        memset(out, 0, sizeof *out);
    }
    if (payload == NULL || out == NULL || length == 0U) {
        return SCENE_MODEL_ERR_ARGUMENT;
    }

    CborParser parser;
    CborValue root;
    scene_model_result_t status =
        open_scene_payload(payload, length, &parser, &root);
    if (status != SCENE_MODEL_OK) {
        return status;
    }
    return scene_decode_map(&root, out);
}
