#include <assert.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

#include "core/scene_binding.h"
#include "core/scene_model.h"

/* ------------------------------------------------------------------
 * A hand-built CBOR emitter.
 *
 * These fixtures are written byte by byte rather than through TinyCBOR's
 * encoder on purpose: several of the cases below are payloads the encoder
 * would refuse to produce (a duplicate map key, a truncated UTF-8 sequence,
 * trailing bytes past the scene, twelve levels of nesting). A decoder facing
 * an untrusted host has to survive exactly those, so the test has to be able
 * to write them.
 *
 * Every helper emits canonical, definite-length CBOR -- shortest-form heads,
 * keys in strictly increasing order -- because that is what scene_decode()
 * accepts. A test that wants a non-canonical payload writes it explicitly.
 * ------------------------------------------------------------------ */

typedef struct {
    uint8_t bytes[2048];
    size_t length;
} builder_t;

static void put_raw(builder_t *b, uint8_t value)
{
    assert(b->length < sizeof b->bytes);
    b->bytes[b->length++] = value;
}

static void put_head(builder_t *b, uint8_t major, uint64_t value)
{
    uint8_t tag = (uint8_t)(major << 5);

    if (value < 24U) {
        put_raw(b, (uint8_t)(tag | (uint8_t)value));
    } else if (value <= UINT8_MAX) {
        put_raw(b, (uint8_t)(tag | 24U));
        put_raw(b, (uint8_t)value);
    } else if (value <= UINT16_MAX) {
        put_raw(b, (uint8_t)(tag | 25U));
        put_raw(b, (uint8_t)(value >> 8));
        put_raw(b, (uint8_t)value);
    } else if (value <= UINT32_MAX) {
        put_raw(b, (uint8_t)(tag | 26U));
        for (int shift = 24; shift >= 0; shift -= 8) {
            put_raw(b, (uint8_t)(value >> shift));
        }
    } else {
        put_raw(b, (uint8_t)(tag | 27U));
        for (int shift = 56; shift >= 0; shift -= 8) {
            put_raw(b, (uint8_t)(value >> shift));
        }
    }
}

static void put_uint(builder_t *b, uint64_t value)
{
    put_head(b, 0U, value);
}

static void put_int(builder_t *b, int64_t value)
{
    if (value < 0) {
        put_head(b, 1U, (uint64_t)(-(value + 1)));
    } else {
        put_head(b, 0U, (uint64_t)value);
    }
}

static void put_bytes(builder_t *b, const uint8_t *data, size_t length)
{
    put_head(b, 2U, length);
    for (size_t i = 0U; i < length; i++) {
        put_raw(b, data[i]);
    }
}

static void put_text_raw(builder_t *b, const char *text, size_t length)
{
    put_head(b, 3U, length);
    for (size_t i = 0U; i < length; i++) {
        put_raw(b, (uint8_t)text[i]);
    }
}

static void put_text(builder_t *b, const char *text)
{
    put_text_raw(b, text, strlen(text));
}

static void put_array(builder_t *b, size_t count)
{
    put_head(b, 4U, count);
}

static void put_map(builder_t *b, size_t count)
{
    put_head(b, 5U, count);
}

static void put_bool(builder_t *b, bool value)
{
    put_raw(b, value ? 0xF5U : 0xF4U);
}

/* ------------------------------------------------------------------
 * Scene fixture helpers. The key numbering they emit is the wire contract
 * documented at the top of core/scene_decode.c.
 * ------------------------------------------------------------------ */

static const uint8_t k_digest[ASSET_DIGEST_BYTES] = {
    0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08,
    0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f, 0x10,
    0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18,
    0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e, 0x1f, 0x20,
};

static void begin_scene(builder_t *b, uint64_t revision, uint64_t background,
                        size_t node_count)
{
    b->length = 0U;
    put_map(b, 3U);
    put_uint(b, 0U);
    put_uint(b, revision);
    put_uint(b, 1U);
    put_uint(b, background);
    put_uint(b, 2U);
    put_array(b, node_count);
}

static void put_rect_node(builder_t *b, int64_t x, int64_t y, int64_t w,
                          int64_t h)
{
    put_map(b, 2U);
    put_uint(b, 0U);
    put_uint(b, SCENE_NODE_RECT);
    put_uint(b, 1U);
    put_map(b, 4U);
    put_uint(b, 0U);
    put_int(b, x);
    put_uint(b, 1U);
    put_int(b, y);
    put_uint(b, 2U);
    put_int(b, w);
    put_uint(b, 3U);
    put_int(b, h);
}

static void put_baked_font(builder_t *b, uint64_t tier)
{
    put_map(b, 2U);
    put_uint(b, 0U);
    put_uint(b, SCENE_FONT_BAKED);
    put_uint(b, 1U);
    put_uint(b, tier);
}

static void put_literal_value(builder_t *b, const char *literal)
{
    put_map(b, 2U);
    put_uint(b, 0U);
    put_uint(b, SCENE_VALUE_LITERAL);
    put_uint(b, 1U);
    put_text(b, literal);
}

static void put_literal_value_raw(builder_t *b, const char *literal,
                                  size_t length)
{
    put_map(b, 2U);
    put_uint(b, 0U);
    put_uint(b, SCENE_VALUE_LITERAL);
    put_uint(b, 1U);
    put_text_raw(b, literal, length);
}

static void put_binding_value(builder_t *b, const char *binding)
{
    put_map(b, 2U);
    put_uint(b, 0U);
    put_uint(b, SCENE_VALUE_BINDING);
    put_uint(b, 2U);
    put_text(b, binding);
}

/* A TEXT node whose three required geometry keys are on-canvas and whose
 * font is a legal baked tier -- so anything a test asserts about is the
 * thing the test changed. The caller supplies the value map. */
static void begin_text_node(builder_t *b, size_t payload_keys)
{
    put_map(b, 2U);
    put_uint(b, 0U);
    put_uint(b, SCENE_NODE_TEXT);
    put_uint(b, 1U);
    put_map(b, payload_keys);
    put_uint(b, 0U);
    put_int(b, 10);
    put_uint(b, 1U);
    put_int(b, 200);
    put_uint(b, 2U);
    put_int(b, 400);
    put_uint(b, 4U);
    put_baked_font(b, SCENE_FONT_BODY);
    put_uint(b, 6U);
}

static scene_t *new_scene(void)
{
    /* Heap, never a local: sizeof(scene_t) is ~6 KB. A stack overflow here
     * would be confusing rather than instructive, and the firmware places
     * this structure deliberately for the same reason. */
    scene_t *scene = malloc(sizeof *scene);
    assert(scene != NULL);
    memset(scene, 0xAA, sizeof *scene);
    return scene;
}

/* ------------------------------------------------------------------ */

static void test_a_valid_text_scene_roundtrips(void)
{
    builder_t b;
    scene_t *scene = new_scene();

    begin_scene(&b, 7U, UINT32_C(0x00101010), 1U);
    put_map(&b, 2U);
    put_uint(&b, 0U);
    put_uint(&b, SCENE_NODE_TEXT);
    put_uint(&b, 1U);
    put_map(&b, 8U);
    put_uint(&b, 0U);
    put_int(&b, 10);
    put_uint(&b, 1U);
    put_int(&b, 200);
    put_uint(&b, 2U);
    put_int(&b, 400);
    put_uint(&b, 3U);
    put_uint(&b, SCENE_ALIGN_CENTER);
    put_uint(&b, 4U);
    put_baked_font(&b, SCENE_FONT_HERO);
    put_uint(&b, 5U);
    put_uint(&b, UINT32_C(0x00FFFFFF));
    put_uint(&b, 6U);
    put_literal_value(&b, "12:34");
    put_uint(&b, 7U);
    put_bool(&b, true);

    assert(scene_decode(b.bytes, b.length, scene) == SCENE_MODEL_OK);
    assert(scene->revision == 7U);
    assert(scene->background == UINT32_C(0x00101010));
    assert(scene->node_count == 1U);

    const scene_text_t *text = &scene->nodes[0].value.text;
    assert(scene->nodes[0].kind == SCENE_NODE_TEXT);
    assert(text->x == 10);
    assert(text->baseline_y == 200);
    assert(text->w == 400);
    assert(text->align == SCENE_ALIGN_CENTER);
    assert(text->font.kind == SCENE_FONT_BAKED);
    assert(text->font.baked == SCENE_FONT_HERO);
    assert(text->color == UINT32_C(0x00FFFFFF));
    assert(text->value.kind == SCENE_VALUE_LITERAL);
    assert(strcmp(text->value.literal, "12:34") == 0);
    assert(text->ellipsize);

    free(scene);
}

/* Every node kind decodes, including SCALE -- the seventh kind, added
 * because digital_clock.c's dial is an lv_scale widget. A decoder that
 * handled six would pass every other test in this file. */
static void test_every_node_kind_decodes(void)
{
    builder_t b;
    scene_t *scene = new_scene();

    begin_scene(&b, 1U, 0U, 7U);

    put_rect_node(&b, 4, 8, 100, 50);

    put_map(&b, 2U); /* ARC */
    put_uint(&b, 0U);
    put_uint(&b, SCENE_NODE_ARC);
    put_uint(&b, 1U);
    put_map(&b, 9U);
    put_uint(&b, 0U);
    put_int(&b, 200);
    put_uint(&b, 1U);
    put_int(&b, 180);
    put_uint(&b, 2U);
    put_int(&b, 120);
    put_uint(&b, 3U);
    put_int(&b, 270);
    put_uint(&b, 4U);
    put_int(&b, 630);
    put_uint(&b, 5U);
    put_int(&b, 12);
    put_uint(&b, 6U);
    put_uint(&b, UINT32_C(0x00FF8800));
    put_uint(&b, 7U);
    put_bool(&b, true);
    put_uint(&b, 8U);
    put_text(&b, "timer.pct");

    put_map(&b, 2U); /* LINE */
    put_uint(&b, 0U);
    put_uint(&b, SCENE_NODE_LINE);
    put_uint(&b, 1U);
    put_map(&b, 4U);
    put_uint(&b, 0U);
    put_array(&b, 3U);
    put_int(&b, 0);
    put_int(&b, 10);
    put_int(&b, 20);
    put_uint(&b, 1U);
    put_array(&b, 3U);
    put_int(&b, 1);
    put_int(&b, 11);
    put_int(&b, 21);
    put_uint(&b, 2U);
    put_int(&b, 3);
    put_uint(&b, 3U);
    put_uint(&b, UINT32_C(0x00123456));

    begin_text_node(&b, 5U); /* TEXT */
    put_literal_value(&b, "hello");

    put_map(&b, 2U); /* IMAGE */
    put_uint(&b, 0U);
    put_uint(&b, SCENE_NODE_IMAGE);
    put_uint(&b, 1U);
    put_map(&b, 7U);
    put_uint(&b, 0U);
    put_int(&b, 16);
    put_uint(&b, 1U);
    put_int(&b, 24);
    put_uint(&b, 2U);
    put_int(&b, 64);
    put_uint(&b, 3U);
    put_int(&b, 64);
    put_uint(&b, 4U);
    put_bytes(&b, k_digest, sizeof k_digest);
    put_uint(&b, 5U);
    put_bool(&b, true);
    put_uint(&b, 6U);
    put_uint(&b, UINT32_C(0x00ABCDEF));

    put_map(&b, 2U); /* GLYPH */
    put_uint(&b, 0U);
    put_uint(&b, SCENE_NODE_GLYPH);
    put_uint(&b, 1U);
    put_map(&b, 6U);
    put_uint(&b, 0U);
    put_int(&b, 30);
    put_uint(&b, 1U);
    put_int(&b, 60);
    put_uint(&b, 2U);
    put_int(&b, 40);
    put_uint(&b, 3U);
    put_bytes(&b, k_digest, sizeof k_digest);
    put_uint(&b, 4U);
    put_text(&b, "cloud-rain");
    put_uint(&b, 5U);
    put_uint(&b, UINT32_C(0x00224466));

    put_map(&b, 2U); /* SCALE */
    put_uint(&b, 0U);
    put_uint(&b, SCENE_NODE_SCALE);
    put_uint(&b, 1U);
    put_map(&b, 6U);
    put_uint(&b, 0U);
    put_int(&b, 40);
    put_uint(&b, 1U);
    put_int(&b, 8);
    put_uint(&b, 2U);
    put_int(&b, 300);
    put_uint(&b, 3U);
    put_uint(&b, 60U);
    put_uint(&b, 4U);
    put_uint(&b, 5U);
    put_uint(&b, 5U);
    put_uint(&b, UINT32_C(0x0000FF00));

    assert(scene_decode(b.bytes, b.length, scene) == SCENE_MODEL_OK);
    assert(scene->node_count == 7U);
    assert(scene->nodes[0].kind == SCENE_NODE_RECT);
    assert(scene->nodes[1].kind == SCENE_NODE_ARC);
    assert(scene->nodes[2].kind == SCENE_NODE_LINE);
    assert(scene->nodes[3].kind == SCENE_NODE_TEXT);
    assert(scene->nodes[4].kind == SCENE_NODE_IMAGE);
    assert(scene->nodes[5].kind == SCENE_NODE_GLYPH);
    assert(scene->nodes[6].kind == SCENE_NODE_SCALE);

    assert(scene->nodes[1].value.arc.start_deg == 270);
    assert(scene->nodes[1].value.arc.end_deg == 630);
    assert(scene->nodes[1].value.arc.rounded);
    assert(strcmp(scene->nodes[1].value.arc.end_binding, "timer.pct") == 0);

    assert(scene->nodes[2].value.line.point_count == 3U);
    assert(scene->nodes[2].value.line.xs[2] == 20);
    assert(scene->nodes[2].value.line.ys[2] == 21);

    assert(memcmp(scene->nodes[4].value.image.digest, k_digest,
                  sizeof k_digest) == 0);
    assert(scene->nodes[4].value.image.recolor);

    assert(strcmp(scene->nodes[5].value.glyph.name, "cloud-rain") == 0);

    assert(scene->nodes[6].value.scale.box == 300);
    assert(scene->nodes[6].value.scale.total_tick_count == 60U);
    assert(scene->nodes[6].value.scale.major_tick_every == 5U);
    assert(scene->nodes[6].value.scale.major_tick_color == UINT32_C(0x0000FF00));

    free(scene);
}

static void test_more_nodes_than_the_cap_is_rejected(void)
{
    builder_t b;
    scene_t *scene = new_scene();

    begin_scene(&b, 1U, 0U, SCENE_MAX_NODES + 1U);
    for (size_t i = 0U; i <= SCENE_MAX_NODES; i++) {
        put_rect_node(&b, 0, 0, 10, 10);
    }

    assert(scene_decode(b.bytes, b.length, scene) ==
           SCENE_MODEL_ERR_NODE_COUNT);
    /* Refused whole: not clipped to the first 24. */
    assert(scene->node_count == 0U);

    free(scene);
}

static void test_a_node_with_an_unknown_kind_is_rejected(void)
{
    builder_t b;
    scene_t *scene = new_scene();

    begin_scene(&b, 1U, 0U, 1U);
    put_map(&b, 2U);
    put_uint(&b, 0U);
    put_uint(&b, 99U);
    put_uint(&b, 1U);
    put_map(&b, 0U);

    assert(scene_decode(b.bytes, b.length, scene) ==
           SCENE_MODEL_ERR_NODE_KIND);

    free(scene);
}

static void test_an_unknown_integer_key_is_skipped_not_rejected(void)
{
    builder_t b;
    scene_t *scene = new_scene();

    /* Unknown keys at both levels -- scene map key 9, node payload key 20 --
     * placed after the known ones so the map stays canonically sorted. A
     * later protocol revision adding a field must not break this decoder. */
    b.length = 0U;
    put_map(&b, 4U);
    put_uint(&b, 0U);
    put_uint(&b, 3U);
    put_uint(&b, 1U);
    put_uint(&b, 0U);
    put_uint(&b, 2U);
    put_array(&b, 1U);
    put_map(&b, 2U);
    put_uint(&b, 0U);
    put_uint(&b, SCENE_NODE_RECT);
    put_uint(&b, 1U);
    put_map(&b, 5U);
    put_uint(&b, 0U);
    put_int(&b, 1);
    put_uint(&b, 1U);
    put_int(&b, 2);
    put_uint(&b, 2U);
    put_int(&b, 30);
    put_uint(&b, 3U);
    put_int(&b, 40);
    put_uint(&b, 20U);
    put_text(&b, "a field from the future");
    put_uint(&b, 9U);
    put_array(&b, 2U);
    put_uint(&b, 1U);
    put_uint(&b, 2U);

    assert(scene_decode(b.bytes, b.length, scene) == SCENE_MODEL_OK);
    assert(scene->revision == 3U);
    assert(scene->node_count == 1U);
    assert(scene->nodes[0].value.rect.w == 30);
    assert(scene->nodes[0].value.rect.h == 40);

    free(scene);
}

static void test_a_duplicate_key_is_rejected(void)
{
    builder_t b;
    scene_t *scene = new_scene();

    b.length = 0U;
    put_map(&b, 3U);
    put_uint(&b, 0U);
    put_uint(&b, 1U);
    put_uint(&b, 0U); /* key 0 again */
    put_uint(&b, 2U);
    put_uint(&b, 2U);
    put_array(&b, 0U);

    assert(scene_decode(b.bytes, b.length, scene) == SCENE_MODEL_ERR_ARGUMENT);

    free(scene);
}

static void test_an_out_of_order_key_is_rejected(void)
{
    builder_t b;
    scene_t *scene = new_scene();

    b.length = 0U;
    put_map(&b, 3U);
    put_uint(&b, 1U);
    put_uint(&b, 0U);
    put_uint(&b, 0U);
    put_uint(&b, 1U);
    put_uint(&b, 2U);
    put_array(&b, 0U);

    assert(scene_decode(b.bytes, b.length, scene) == SCENE_MODEL_ERR_ARGUMENT);

    free(scene);
}

static void test_an_unterminated_utf8_text_is_rejected(void)
{
    builder_t b;
    scene_t *scene = new_scene();

    /* 0xC3 opens a two-byte sequence whose continuation byte never arrives.
     * The array is length-prefixed, so nothing here is a C string -- the
     * decoder must never reach for a terminator that was not sent. */
    begin_scene(&b, 1U, 0U, 1U);
    begin_text_node(&b, 5U);
    put_literal_value_raw(&b, "\xC3", 1U);

    assert(scene_decode(b.bytes, b.length, scene) == SCENE_MODEL_ERR_ARGUMENT);

    free(scene);
}

static void test_a_text_longer_than_the_cap_is_rejected(void)
{
    builder_t b;
    scene_t *scene = new_scene();
    char literal[SCENE_MAX_TEXT_BYTES + 2U];

    memset(literal, 'a', SCENE_MAX_TEXT_BYTES + 1U);
    literal[SCENE_MAX_TEXT_BYTES + 1U] = '\0';

    begin_scene(&b, 1U, 0U, 1U);
    begin_text_node(&b, 5U);
    put_literal_value(&b, literal);

    assert(scene_decode(b.bytes, b.length, scene) == SCENE_MODEL_ERR_TEXT);

    free(scene);
}

static void test_a_text_exactly_at_the_cap_is_accepted(void)
{
    builder_t b;
    scene_t *scene = new_scene();
    char literal[SCENE_MAX_TEXT_BYTES + 1U];

    memset(literal, 'a', SCENE_MAX_TEXT_BYTES);
    literal[SCENE_MAX_TEXT_BYTES] = '\0';

    begin_scene(&b, 1U, 0U, 1U);
    begin_text_node(&b, 5U);
    put_literal_value(&b, literal);

    assert(scene_decode(b.bytes, b.length, scene) == SCENE_MODEL_OK);
    assert(strlen(scene->nodes[0].value.text.value.literal) ==
           SCENE_MAX_TEXT_BYTES);

    free(scene);
}

/* The one that matters most. A scene carrying a binding the device cannot
 * evaluate is refused WHOLE, at decode time -- not accepted and then failed
 * at draw time, which would leave the panel half-rendered. */
static void test_an_unknown_binding_is_rejected_by_the_decoder(void)
{
    builder_t b;
    scene_t *scene = new_scene();

    begin_scene(&b, 1U, 0U, 2U);
    put_rect_node(&b, 0, 0, 10, 10);
    begin_text_node(&b, 5U);
    put_binding_value(&b, "weather.temperature");

    assert(scene_decode(b.bytes, b.length, scene) == SCENE_MODEL_ERR_TEXT);
    /* The earlier, perfectly good RECT node did not survive either. */
    assert(scene->node_count == 0U);

    free(scene);
}

static void test_a_known_binding_is_accepted_by_the_decoder(void)
{
    builder_t b;
    scene_t *scene = new_scene();

    begin_scene(&b, 1U, 0U, 1U);
    begin_text_node(&b, 5U);
    put_binding_value(&b, "time:HH:mm");

    assert(scene_decode(b.bytes, b.length, scene) == SCENE_MODEL_OK);
    assert(scene->nodes[0].value.text.value.kind == SCENE_VALUE_BINDING);
    assert(strcmp(scene->nodes[0].value.text.value.binding, "time:HH:mm") == 0);

    free(scene);
}

static void test_an_unknown_arc_end_binding_is_rejected_by_the_decoder(void)
{
    builder_t b;
    scene_t *scene = new_scene();

    begin_scene(&b, 1U, 0U, 1U);
    put_map(&b, 2U);
    put_uint(&b, 0U);
    put_uint(&b, SCENE_NODE_ARC);
    put_uint(&b, 1U);
    put_map(&b, 7U);
    put_uint(&b, 0U);
    put_int(&b, 200);
    put_uint(&b, 1U);
    put_int(&b, 180);
    put_uint(&b, 2U);
    put_int(&b, 120);
    put_uint(&b, 3U);
    put_int(&b, 270);
    put_uint(&b, 4U);
    put_int(&b, 630);
    put_uint(&b, 5U);
    put_int(&b, 12);
    put_uint(&b, 8U);
    put_text(&b, "stock.price");

    assert(scene_decode(b.bytes, b.length, scene) == SCENE_MODEL_ERR_TEXT);

    free(scene);
}

static void test_trailing_bytes_after_the_scene_are_rejected(void)
{
    builder_t b;
    scene_t *scene = new_scene();

    begin_scene(&b, 1U, 0U, 1U);
    put_rect_node(&b, 0, 0, 10, 10);
    put_raw(&b, 0x00U); /* one byte past the end of the scene */

    assert(scene_decode(b.bytes, b.length, scene) == SCENE_MODEL_ERR_ARGUMENT);

    free(scene);
}

static void test_recursion_is_bounded(void)
{
    builder_t b;
    scene_t *scene = new_scene();

    /* Twelve nested arrays hung off an unknown key, against the build's
     * CBOR_PARSER_MAX_RECURSIONS of 8. skip_value() would otherwise walk
     * them recursively on an 8 KiB task stack. */
    b.length = 0U;
    put_map(&b, 4U);
    put_uint(&b, 0U);
    put_uint(&b, 1U);
    put_uint(&b, 1U);
    put_uint(&b, 0U);
    put_uint(&b, 2U);
    put_array(&b, 0U);
    put_uint(&b, 9U);
    for (int depth = 0; depth < 12; depth++) {
        put_array(&b, 1U);
    }
    put_uint(&b, 0U);

    assert(scene_decode(b.bytes, b.length, scene) == SCENE_MODEL_ERR_ARGUMENT);

    free(scene);
}

static void test_a_missing_required_key_is_rejected(void)
{
    builder_t b;
    scene_t *scene = new_scene();

    /* The scene map without its node array. */
    b.length = 0U;
    put_map(&b, 2U);
    put_uint(&b, 0U);
    put_uint(&b, 1U);
    put_uint(&b, 1U);
    put_uint(&b, 0U);

    assert(scene_decode(b.bytes, b.length, scene) == SCENE_MODEL_ERR_ARGUMENT);

    free(scene);
}

/* A node map carrying a kind but no payload. Nothing downstream catches
 * this: a RECT with a zeroed union is a 0x0 rect at the origin, which
 * scene_model_validate() accepts as perfectly on-canvas. Only the node
 * map's required-key mask sees it. */
static void test_a_node_without_a_payload_is_rejected(void)
{
    builder_t b;
    scene_t *scene = new_scene();

    begin_scene(&b, 1U, 0U, 1U);
    put_map(&b, 1U);
    put_uint(&b, 0U);
    put_uint(&b, SCENE_NODE_RECT);

    assert(scene_decode(b.bytes, b.length, scene) == SCENE_MODEL_ERR_ARGUMENT);

    free(scene);
}

static void test_a_node_missing_a_required_geometry_key_is_rejected(void)
{
    builder_t b;
    scene_t *scene = new_scene();

    begin_scene(&b, 1U, 0U, 1U);
    put_map(&b, 2U);
    put_uint(&b, 0U);
    put_uint(&b, SCENE_NODE_RECT);
    put_uint(&b, 1U);
    put_map(&b, 3U); /* x, y, w -- no h */
    put_uint(&b, 0U);
    put_int(&b, 1);
    put_uint(&b, 1U);
    put_int(&b, 2);
    put_uint(&b, 2U);
    put_int(&b, 30);

    assert(scene_decode(b.bytes, b.length, scene) == SCENE_MODEL_ERR_ARGUMENT);

    free(scene);
}

/* SCENE_ALIGN_LEFT is 1, so a zero-initialised align is not a legal value
 * and scene_model_validate() rejects it. An omitted align key must be
 * defaulted explicitly. */
static void test_an_omitted_align_defaults_to_left(void)
{
    builder_t b;
    scene_t *scene = new_scene();

    begin_scene(&b, 1U, 0U, 1U);
    begin_text_node(&b, 5U);
    put_literal_value(&b, "left");

    assert(scene_decode(b.bytes, b.length, scene) == SCENE_MODEL_OK);
    assert(scene->nodes[0].value.text.align == SCENE_ALIGN_LEFT);

    free(scene);
}

/* The truncation has to land on a LEGAL coordinate for this test to mean
 * anything. 0x1'0000'000A narrowed to int32 is 10 -- an ordinary on-canvas
 * x that scene_model_validate() would wave straight through. A test using
 * INT32_MAX + 1 proves nothing, because that narrows to INT32_MIN and the
 * model rejects the negative for its own reasons. */
static void test_a_coordinate_outside_int32_is_rejected(void)
{
    builder_t b;
    scene_t *scene = new_scene();

    begin_scene(&b, 1U, 0U, 1U);
    put_map(&b, 2U);
    put_uint(&b, 0U);
    put_uint(&b, SCENE_NODE_RECT);
    put_uint(&b, 1U);
    put_map(&b, 4U);
    put_uint(&b, 0U);
    put_int(&b, INT64_C(0x10000000A));
    put_uint(&b, 1U);
    put_int(&b, 0);
    put_uint(&b, 2U);
    put_int(&b, 10);
    put_uint(&b, 3U);
    put_int(&b, 10);

    assert(scene_decode(b.bytes, b.length, scene) ==
           SCENE_MODEL_ERR_GEOMETRY);

    free(scene);
}

/* Same reasoning for the unsigned readers. A revision of 0x1'0000'0007
 * narrows to 7, and nothing downstream constrains a revision at all, so
 * only the decoder can catch this. */
static void test_an_unsigned_outside_uint32_is_rejected(void)
{
    builder_t b;
    scene_t *scene = new_scene();

    b.length = 0U;
    put_map(&b, 3U);
    put_uint(&b, 0U);
    put_uint(&b, UINT64_C(0x100000007));
    put_uint(&b, 1U);
    put_uint(&b, 0U);
    put_uint(&b, 2U);
    put_array(&b, 0U);

    assert(scene_decode(b.bytes, b.length, scene) ==
           SCENE_MODEL_ERR_GEOMETRY);

    free(scene);
}

/* And for the one uint8 field. An opacity of 256 narrows to 0 -- fully
 * transparent, which the model does not constrain, so a rect the host asked
 * to draw opaque would silently vanish. */
static void test_an_opacity_outside_uint8_is_rejected(void)
{
    builder_t b;
    scene_t *scene = new_scene();

    begin_scene(&b, 1U, 0U, 1U);
    put_map(&b, 2U);
    put_uint(&b, 0U);
    put_uint(&b, SCENE_NODE_RECT);
    put_uint(&b, 1U);
    put_map(&b, 5U);
    put_uint(&b, 0U);
    put_int(&b, 0);
    put_uint(&b, 1U);
    put_int(&b, 0);
    put_uint(&b, 2U);
    put_int(&b, 10);
    put_uint(&b, 3U);
    put_int(&b, 10);
    put_uint(&b, 6U);
    put_uint(&b, 256U);

    assert(scene_decode(b.bytes, b.length, scene) ==
           SCENE_MODEL_ERR_GEOMETRY);

    free(scene);
}

static void test_more_line_points_than_the_cap_is_rejected(void)
{
    builder_t b;
    scene_t *scene = new_scene();

    begin_scene(&b, 1U, 0U, 1U);
    put_map(&b, 2U);
    put_uint(&b, 0U);
    put_uint(&b, SCENE_NODE_LINE);
    put_uint(&b, 1U);
    put_map(&b, 3U);
    put_uint(&b, 0U);
    put_array(&b, SCENE_MAX_LINE_POINTS + 1U);
    for (size_t i = 0U; i <= SCENE_MAX_LINE_POINTS; i++) {
        put_int(&b, 1);
    }
    put_uint(&b, 1U);
    put_array(&b, SCENE_MAX_LINE_POINTS + 1U);
    for (size_t i = 0U; i <= SCENE_MAX_LINE_POINTS; i++) {
        put_int(&b, 1);
    }
    put_uint(&b, 2U);
    put_int(&b, 2);

    assert(scene_decode(b.bytes, b.length, scene) ==
           SCENE_MODEL_ERR_GEOMETRY);

    free(scene);
}

/* SCENE_MAX_LINE_POINTS + 1 is not enough to prove the decoder bounds this
 * itself: nine points still fit inside scene_node_t, and
 * scene_model_validate() rejects the count afterwards anyway, so the two
 * are indistinguishable. 1800 points is 7200 bytes against a 6144-byte
 * scene_t, which leaves the allocation entirely -- the only version of this
 * test the decoder's own bound can be the reason for. */
static void test_a_line_point_array_far_past_the_cap_is_rejected(void)
{
    builder_t b;
    scene_t *scene = new_scene();
    const size_t huge = 1800U;

    begin_scene(&b, 1U, 0U, 1U);
    put_map(&b, 2U);
    put_uint(&b, 0U);
    put_uint(&b, SCENE_NODE_LINE);
    put_uint(&b, 1U);
    put_map(&b, 3U);
    put_uint(&b, 0U);
    put_array(&b, huge);
    for (size_t i = 0U; i < huge; i++) {
        put_int(&b, 1);
    }
    put_uint(&b, 1U);
    put_array(&b, 0U);
    put_uint(&b, 2U);
    put_int(&b, 2);

    assert(scene_decode(b.bytes, b.length, scene) ==
           SCENE_MODEL_ERR_GEOMETRY);

    free(scene);
}

static void test_mismatched_line_point_arrays_are_rejected(void)
{
    builder_t b;
    scene_t *scene = new_scene();

    begin_scene(&b, 1U, 0U, 1U);
    put_map(&b, 2U);
    put_uint(&b, 0U);
    put_uint(&b, SCENE_NODE_LINE);
    put_uint(&b, 1U);
    put_map(&b, 3U);
    put_uint(&b, 0U);
    put_array(&b, 3U);
    put_int(&b, 1);
    put_int(&b, 2);
    put_int(&b, 3);
    put_uint(&b, 1U);
    put_array(&b, 2U);
    put_int(&b, 1);
    put_int(&b, 2);
    put_uint(&b, 2U);
    put_int(&b, 2);

    assert(scene_decode(b.bytes, b.length, scene) ==
           SCENE_MODEL_ERR_GEOMETRY);

    free(scene);
}

/* A TEXT node with no value map is not a text node. Without this the map
 * would decode to value.kind 0 and an empty literal, which the model
 * accepts -- a blank string drawn where the host meant words. */
static void test_a_text_node_without_a_value_is_rejected(void)
{
    builder_t b;
    scene_t *scene = new_scene();

    begin_scene(&b, 1U, 0U, 1U);
    put_map(&b, 2U);
    put_uint(&b, 0U);
    put_uint(&b, SCENE_NODE_TEXT);
    put_uint(&b, 1U);
    put_map(&b, 4U);
    put_uint(&b, 0U);
    put_int(&b, 10);
    put_uint(&b, 1U);
    put_int(&b, 200);
    put_uint(&b, 2U);
    put_int(&b, 400);
    put_uint(&b, 4U);
    put_baked_font(&b, SCENE_FONT_BODY);

    assert(scene_decode(b.bytes, b.length, scene) == SCENE_MODEL_ERR_ARGUMENT);

    free(scene);
}

static void test_a_text_node_without_a_font_is_rejected(void)
{
    builder_t b;
    scene_t *scene = new_scene();

    begin_scene(&b, 1U, 0U, 1U);
    put_map(&b, 2U);
    put_uint(&b, 0U);
    put_uint(&b, SCENE_NODE_TEXT);
    put_uint(&b, 1U);
    put_map(&b, 4U);
    put_uint(&b, 0U);
    put_int(&b, 10);
    put_uint(&b, 1U);
    put_int(&b, 200);
    put_uint(&b, 2U);
    put_int(&b, 400);
    put_uint(&b, 6U);
    put_literal_value(&b, "hi");

    assert(scene_decode(b.bytes, b.length, scene) == SCENE_MODEL_ERR_ARGUMENT);

    free(scene);
}

/* A font map with no kind. Only the decoder's required mask sees this;
 * validate_font() would reject kind 0 too, but with a different code, and
 * the point is that the wire contract says the key is mandatory. */
static void test_a_font_without_a_kind_is_rejected(void)
{
    builder_t b;
    scene_t *scene = new_scene();

    begin_scene(&b, 1U, 0U, 1U);
    put_map(&b, 2U);
    put_uint(&b, 0U);
    put_uint(&b, SCENE_NODE_TEXT);
    put_uint(&b, 1U);
    put_map(&b, 5U);
    put_uint(&b, 0U);
    put_int(&b, 10);
    put_uint(&b, 1U);
    put_int(&b, 200);
    put_uint(&b, 2U);
    put_int(&b, 400);
    put_uint(&b, 4U);
    put_map(&b, 1U);
    put_uint(&b, 1U);
    put_uint(&b, SCENE_FONT_BODY);
    put_uint(&b, 6U);
    put_literal_value(&b, "hi");

    assert(scene_decode(b.bytes, b.length, scene) == SCENE_MODEL_ERR_ARGUMENT);

    free(scene);
}

/* An asset font that names no digest can never resolve to a face. It is
 * the one font field scene_model_validate() does not cover -- validate_font()
 * checks only pixel_size for SCENE_FONT_ASSET -- so without the decoder's
 * conditional required key this returns SCENE_MODEL_OK and fails at draw
 * time instead. */
static void test_an_asset_font_without_a_digest_is_rejected(void)
{
    builder_t b;
    scene_t *scene = new_scene();

    begin_scene(&b, 1U, 0U, 1U);
    put_map(&b, 2U);
    put_uint(&b, 0U);
    put_uint(&b, SCENE_NODE_TEXT);
    put_uint(&b, 1U);
    put_map(&b, 5U);
    put_uint(&b, 0U);
    put_int(&b, 10);
    put_uint(&b, 1U);
    put_int(&b, 200);
    put_uint(&b, 2U);
    put_int(&b, 400);
    put_uint(&b, 4U);
    put_map(&b, 2U);
    put_uint(&b, 0U);
    put_uint(&b, SCENE_FONT_ASSET);
    put_uint(&b, 3U);
    put_int(&b, 20);
    put_uint(&b, 6U);
    put_literal_value(&b, "hi");

    assert(scene_decode(b.bytes, b.length, scene) == SCENE_MODEL_ERR_ARGUMENT);

    free(scene);
}

/* The same font map WITH its digest is accepted, so the case above is
 * rejected for the missing digest and not for the asset kind itself. */
static void test_an_asset_font_with_a_digest_is_accepted(void)
{
    builder_t b;
    scene_t *scene = new_scene();

    begin_scene(&b, 1U, 0U, 1U);
    put_map(&b, 2U);
    put_uint(&b, 0U);
    put_uint(&b, SCENE_NODE_TEXT);
    put_uint(&b, 1U);
    put_map(&b, 5U);
    put_uint(&b, 0U);
    put_int(&b, 10);
    put_uint(&b, 1U);
    put_int(&b, 200);
    put_uint(&b, 2U);
    put_int(&b, 400);
    put_uint(&b, 4U);
    put_map(&b, 3U);
    put_uint(&b, 0U);
    put_uint(&b, SCENE_FONT_ASSET);
    put_uint(&b, 2U);
    put_bytes(&b, k_digest, sizeof k_digest);
    put_uint(&b, 3U);
    put_int(&b, 20);
    put_uint(&b, 6U);
    put_literal_value(&b, "hi");

    assert(scene_decode(b.bytes, b.length, scene) == SCENE_MODEL_OK);
    assert(scene->nodes[0].value.text.font.kind == SCENE_FONT_ASSET);
    assert(memcmp(scene->nodes[0].value.text.font.digest, k_digest,
                  sizeof k_digest) == 0);

    free(scene);
}

/* A GLYPH with no name draws nothing. The model cannot catch it -- an
 * omitted name is the empty string, which is NUL-terminated and therefore
 * valid as far as scene_model_validate() is concerned. */
static void test_a_glyph_without_a_name_is_rejected(void)
{
    builder_t b;
    scene_t *scene = new_scene();

    begin_scene(&b, 1U, 0U, 1U);
    put_map(&b, 2U);
    put_uint(&b, 0U);
    put_uint(&b, SCENE_NODE_GLYPH);
    put_uint(&b, 1U);
    put_map(&b, 4U);
    put_uint(&b, 0U);
    put_int(&b, 30);
    put_uint(&b, 1U);
    put_int(&b, 60);
    put_uint(&b, 2U);
    put_int(&b, 40);
    put_uint(&b, 3U);
    put_bytes(&b, k_digest, sizeof k_digest);

    assert(scene_decode(b.bytes, b.length, scene) == SCENE_MODEL_ERR_ARGUMENT);

    free(scene);
}

static void test_a_digest_of_the_wrong_length_is_rejected(void)
{
    builder_t b;
    scene_t *scene = new_scene();
    uint8_t short_digest[ASSET_DIGEST_BYTES - 1U];

    memset(short_digest, 0x5A, sizeof short_digest);

    begin_scene(&b, 1U, 0U, 1U);
    put_map(&b, 2U);
    put_uint(&b, 0U);
    put_uint(&b, SCENE_NODE_IMAGE);
    put_uint(&b, 1U);
    put_map(&b, 5U);
    put_uint(&b, 0U);
    put_int(&b, 0);
    put_uint(&b, 1U);
    put_int(&b, 0);
    put_uint(&b, 2U);
    put_int(&b, 32);
    put_uint(&b, 3U);
    put_int(&b, 32);
    put_uint(&b, 4U);
    put_bytes(&b, short_digest, sizeof short_digest);

    assert(scene_decode(b.bytes, b.length, scene) == SCENE_MODEL_ERR_ARGUMENT);

    free(scene);
}

/* A decoded scene has already been through scene_model_validate() -- that is
 * the contract scene_decode()'s header comment states, and callers rely on
 * it. An off-canvas node is well-formed CBOR, so only the model can catch it. */
static void test_a_decoded_scene_has_been_validated(void)
{
    builder_t b;
    scene_t *scene = new_scene();

    begin_scene(&b, 1U, 0U, 1U);
    put_rect_node(&b, SCENE_CANVAS_WIDTH, 0, 10, 10);

    assert(scene_decode(b.bytes, b.length, scene) ==
           SCENE_MODEL_ERR_GEOMETRY);
    /* A model rejection clears node_count too, so the contract's "on any
     * non-OK result node_count is 0" holds for the validation half as well
     * as for the decoding half. */
    assert(scene->node_count == 0U);

    free(scene);
}

static void test_a_non_map_payload_is_rejected(void)
{
    builder_t b;
    scene_t *scene = new_scene();

    b.length = 0U;
    put_array(&b, 0U);

    assert(scene_decode(b.bytes, b.length, scene) == SCENE_MODEL_ERR_ARGUMENT);

    free(scene);
}

/* The three argument-guard exits are held to the same contract as every
 * other exit: on ANY non-OK result node_count is 0. They are the exits most
 * likely to be reached by a caller whose scene_t came from
 * heap_caps_malloc() rather than calloc(), so "the decoder did not touch
 * it" and "the decoder left it empty" are not the same promise at all.
 * new_scene() poisons the allocation with 0xAA precisely so this test can
 * tell those two apart. */
static void test_null_arguments_are_rejected(void)
{
    scene_t *scene = new_scene();
    static const uint8_t empty_map[] = {0xA0U};

    assert(scene_decode(NULL, 1U, scene) == SCENE_MODEL_ERR_ARGUMENT);
    assert(scene->node_count == 0U);

    assert(scene_decode(empty_map, sizeof empty_map, NULL) ==
           SCENE_MODEL_ERR_ARGUMENT);

    memset(scene, 0xAA, sizeof *scene);
    assert(scene_decode(empty_map, 0U, scene) == SCENE_MODEL_ERR_ARGUMENT);
    assert(scene->node_count == 0U);

    free(scene);
}

int main(void)
{
    test_a_valid_text_scene_roundtrips();
    test_every_node_kind_decodes();
    test_more_nodes_than_the_cap_is_rejected();
    test_a_node_with_an_unknown_kind_is_rejected();
    test_an_unknown_integer_key_is_skipped_not_rejected();
    test_a_duplicate_key_is_rejected();
    test_an_out_of_order_key_is_rejected();
    test_an_unterminated_utf8_text_is_rejected();
    test_a_text_longer_than_the_cap_is_rejected();
    test_a_text_exactly_at_the_cap_is_accepted();
    test_an_unknown_binding_is_rejected_by_the_decoder();
    test_a_known_binding_is_accepted_by_the_decoder();
    test_an_unknown_arc_end_binding_is_rejected_by_the_decoder();
    test_trailing_bytes_after_the_scene_are_rejected();
    test_recursion_is_bounded();
    test_a_missing_required_key_is_rejected();
    test_a_node_without_a_payload_is_rejected();
    test_a_node_missing_a_required_geometry_key_is_rejected();
    test_an_omitted_align_defaults_to_left();
    test_a_coordinate_outside_int32_is_rejected();
    test_an_unsigned_outside_uint32_is_rejected();
    test_an_opacity_outside_uint8_is_rejected();
    test_more_line_points_than_the_cap_is_rejected();
    test_a_line_point_array_far_past_the_cap_is_rejected();
    test_mismatched_line_point_arrays_are_rejected();
    test_a_text_node_without_a_value_is_rejected();
    test_a_text_node_without_a_font_is_rejected();
    test_a_font_without_a_kind_is_rejected();
    test_an_asset_font_without_a_digest_is_rejected();
    test_an_asset_font_with_a_digest_is_accepted();
    test_a_glyph_without_a_name_is_rejected();
    test_a_digest_of_the_wrong_length_is_rejected();
    test_a_decoded_scene_has_been_validated();
    test_a_non_map_payload_is_rejected();
    test_null_arguments_are_rejected();
    return 0;
}
