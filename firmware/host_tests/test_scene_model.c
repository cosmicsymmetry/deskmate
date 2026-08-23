#include <assert.h>
#include <stdint.h>
#include <string.h>

#include "core/scene_model.h"

static scene_t minimal_scene(void)
{
    scene_t scene;
    memset(&scene, 0, sizeof scene);
    scene.node_count = 1U;
    scene.nodes[0].kind = SCENE_NODE_RECT;
    scene.nodes[0].value.rect.w = 100;
    scene.nodes[0].value.rect.h = 50;
    return scene;
}

static void test_a_minimal_scene_validates(void)
{
    scene_t scene = minimal_scene();
    assert(scene_model_validate(&scene) == SCENE_MODEL_OK);
}

static void test_too_many_nodes_is_rejected(void)
{
    scene_t scene = minimal_scene();
    scene.node_count = SCENE_MAX_NODES + 1U;
    assert(scene_model_validate(&scene) == SCENE_MODEL_ERR_NODE_COUNT);
}

static void test_an_unknown_node_kind_is_rejected(void)
{
    scene_t scene = minimal_scene();
    scene.nodes[0].kind = (scene_node_kind_t)99;
    assert(scene_model_validate(&scene) == SCENE_MODEL_ERR_NODE_KIND);
}

static void test_a_node_outside_the_canvas_is_rejected(void)
{
    /* The canvas is the only coordinate space that exists. A node placed
     * outside it is not clipped silently -- a host that computes a layout
     * off-canvas has a bug, and hiding it makes that bug invisible. */
    scene_t scene = minimal_scene();
    scene.nodes[0].value.rect.x = SCENE_CANVAS_WIDTH;
    assert(scene_model_validate(&scene) == SCENE_MODEL_ERR_GEOMETRY);
}

static void test_an_unterminated_text_value_is_rejected(void)
{
    scene_t scene = minimal_scene();
    scene.nodes[0].kind = SCENE_NODE_TEXT;
    scene.nodes[0].value.text.font.kind = SCENE_FONT_BAKED;
    scene.nodes[0].value.text.font.baked = SCENE_FONT_BODY;
    memset(scene.nodes[0].value.text.value.literal, 'a',
           SCENE_MAX_TEXT_BYTES + 1U);
    assert(scene_model_validate(&scene) == SCENE_MODEL_ERR_TEXT);
}

static void test_a_line_with_too_many_points_is_rejected(void)
{
    scene_t scene = minimal_scene();
    scene.nodes[0].kind = SCENE_NODE_LINE;
    scene.nodes[0].value.line.point_count = SCENE_MAX_LINE_POINTS + 1U;
    assert(scene_model_validate(&scene) == SCENE_MODEL_ERR_GEOMETRY);
}

static void test_an_unknown_baked_font_tier_is_rejected(void)
{
    scene_t scene = minimal_scene();
    scene.nodes[0].kind = SCENE_NODE_TEXT;
    scene.nodes[0].value.text.font.kind = SCENE_FONT_BAKED;
    scene.nodes[0].value.text.font.baked = (scene_font_tier_t)42;
    assert(scene_model_validate(&scene) == SCENE_MODEL_ERR_FONT);
}

static void test_an_unknown_text_align_is_rejected(void)
{
    scene_t scene = minimal_scene();
    scene.nodes[0].kind = SCENE_NODE_TEXT;
    scene.nodes[0].value.text.font.kind = SCENE_FONT_BAKED;
    scene.nodes[0].value.text.font.baked = SCENE_FONT_BODY;
    scene.nodes[0].value.text.align = (scene_align_t)0;
    assert(scene_model_validate(&scene) == SCENE_MODEL_ERR_GEOMETRY);
}

static void test_an_arc_within_bounds_validates(void)
{
    scene_t scene = minimal_scene();
    scene.nodes[0].kind = SCENE_NODE_ARC;
    scene.nodes[0].value.arc.cx = 100;
    scene.nodes[0].value.arc.cy = 100;
    scene.nodes[0].value.arc.r = 50;
    assert(scene_model_validate(&scene) == SCENE_MODEL_OK);
}

static void test_an_arc_extending_left_of_canvas_is_rejected(void)
{
    /* cx - r < 0: the centre is closer to the left edge than the radius. */
    scene_t scene = minimal_scene();
    scene.nodes[0].kind = SCENE_NODE_ARC;
    scene.nodes[0].value.arc.cx = 10;
    scene.nodes[0].value.arc.cy = 100;
    scene.nodes[0].value.arc.r = 50;
    assert(scene_model_validate(&scene) == SCENE_MODEL_ERR_GEOMETRY);
}

static void test_an_arc_extending_right_of_canvas_is_rejected(void)
{
    /* cx + r > SCENE_CANVAS_WIDTH: the centre is closer to the right edge
     * than the radius. */
    scene_t scene = minimal_scene();
    scene.nodes[0].kind = SCENE_NODE_ARC;
    scene.nodes[0].value.arc.cx = 440;
    scene.nodes[0].value.arc.cy = 100;
    scene.nodes[0].value.arc.r = 50;
    assert(scene_model_validate(&scene) == SCENE_MODEL_ERR_GEOMETRY);
}

static void test_an_arc_with_unterminated_end_binding_is_rejected(void)
{
    scene_t scene = minimal_scene();
    scene.nodes[0].kind = SCENE_NODE_ARC;
    scene.nodes[0].value.arc.cx = 100;
    scene.nodes[0].value.arc.cy = 100;
    scene.nodes[0].value.arc.r = 50;
    memset(scene.nodes[0].value.arc.end_binding, 'a',
           sizeof scene.nodes[0].value.arc.end_binding);
    assert(scene_model_validate(&scene) == SCENE_MODEL_ERR_TEXT);
}

static void test_an_image_within_bounds_validates(void)
{
    scene_t scene = minimal_scene();
    scene.nodes[0].kind = SCENE_NODE_IMAGE;
    scene.nodes[0].value.image.x = 0;
    scene.nodes[0].value.image.y = 0;
    scene.nodes[0].value.image.w = 100;
    scene.nodes[0].value.image.h = 50;
    assert(scene_model_validate(&scene) == SCENE_MODEL_OK);
}

static void test_an_image_extending_past_canvas_width_is_rejected(void)
{
    scene_t scene = minimal_scene();
    scene.nodes[0].kind = SCENE_NODE_IMAGE;
    scene.nodes[0].value.image.x = 400;
    scene.nodes[0].value.image.y = 0;
    scene.nodes[0].value.image.w = 100;
    scene.nodes[0].value.image.h = 50;
    assert(scene_model_validate(&scene) == SCENE_MODEL_ERR_GEOMETRY);
}

static void test_a_glyph_within_bounds_validates(void)
{
    scene_t scene = minimal_scene();
    scene.nodes[0].kind = SCENE_NODE_GLYPH;
    scene.nodes[0].value.glyph.x = 10;
    scene.nodes[0].value.glyph.baseline_y = 100;
    scene.nodes[0].value.glyph.size = 50;
    assert(scene_model_validate(&scene) == SCENE_MODEL_OK);
}

static void test_a_glyph_extending_past_canvas_width_is_rejected(void)
{
    scene_t scene = minimal_scene();
    scene.nodes[0].kind = SCENE_NODE_GLYPH;
    scene.nodes[0].value.glyph.x = 400;
    scene.nodes[0].value.glyph.baseline_y = 100;
    scene.nodes[0].value.glyph.size = 50;
    assert(scene_model_validate(&scene) == SCENE_MODEL_ERR_GEOMETRY);
}

static void test_a_glyph_taller_than_the_canvas_is_rejected(void)
{
    scene_t scene = minimal_scene();
    scene.nodes[0].kind = SCENE_NODE_GLYPH;
    scene.nodes[0].value.glyph.x = 0;
    scene.nodes[0].value.glyph.baseline_y = 0;
    scene.nodes[0].value.glyph.size = 400;
    assert(scene_model_validate(&scene) == SCENE_MODEL_ERR_GEOMETRY);
}

static void test_a_glyph_with_an_unterminated_name_is_rejected(void)
{
    scene_t scene = minimal_scene();
    scene.nodes[0].kind = SCENE_NODE_GLYPH;
    scene.nodes[0].value.glyph.x = 10;
    scene.nodes[0].value.glyph.baseline_y = 100;
    scene.nodes[0].value.glyph.size = 50;
    memset(scene.nodes[0].value.glyph.name, 'a',
           sizeof scene.nodes[0].value.glyph.name);
    assert(scene_model_validate(&scene) == SCENE_MODEL_ERR_TEXT);
}

static void test_a_line_with_populated_points_validates(void)
{
    scene_t scene = minimal_scene();
    scene.nodes[0].kind = SCENE_NODE_LINE;
    scene.nodes[0].value.line.point_count = 3U;
    scene.nodes[0].value.line.xs[0] = 10;
    scene.nodes[0].value.line.ys[0] = 10;
    scene.nodes[0].value.line.xs[1] = 20;
    scene.nodes[0].value.line.ys[1] = 20;
    scene.nodes[0].value.line.xs[2] = 30;
    scene.nodes[0].value.line.ys[2] = 30;
    assert(scene_model_validate(&scene) == SCENE_MODEL_OK);
}

static void test_a_line_with_a_non_first_point_outside_canvas_is_rejected(void)
{
    /* The per-point loop is the thing under test: put the offending point
     * at index 1, not index 0, so a loop that only checked the first point
     * would wrongly accept this scene. */
    scene_t scene = minimal_scene();
    scene.nodes[0].kind = SCENE_NODE_LINE;
    scene.nodes[0].value.line.point_count = 3U;
    scene.nodes[0].value.line.xs[0] = 10;
    scene.nodes[0].value.line.ys[0] = 10;
    scene.nodes[0].value.line.xs[1] = SCENE_CANVAS_WIDTH + 1;
    scene.nodes[0].value.line.ys[1] = 20;
    scene.nodes[0].value.line.xs[2] = 30;
    scene.nodes[0].value.line.ys[2] = 30;
    assert(scene_model_validate(&scene) == SCENE_MODEL_ERR_GEOMETRY);
}

static void test_a_huge_coordinate_does_not_wrap_the_bounds_check(void)
{
    /* If the check were written as `x + w > LIMIT` instead of
     * `x > LIMIT - w`, INT32_MAX + w would overflow and wrap to a small
     * (even negative) value, which could pass the comparison and wrongly
     * accept an off-canvas node. */
    scene_t scene = minimal_scene();
    scene.nodes[0].value.rect.x = INT32_MAX;
    scene.nodes[0].value.rect.w = 10;
    assert(scene_model_validate(&scene) == SCENE_MODEL_ERR_GEOMETRY);
}

int main(void)
{
    test_a_minimal_scene_validates();
    test_too_many_nodes_is_rejected();
    test_an_unknown_node_kind_is_rejected();
    test_a_node_outside_the_canvas_is_rejected();
    test_an_unterminated_text_value_is_rejected();
    test_a_line_with_too_many_points_is_rejected();
    test_an_unknown_baked_font_tier_is_rejected();
    test_an_unknown_text_align_is_rejected();
    test_an_arc_within_bounds_validates();
    test_an_arc_extending_left_of_canvas_is_rejected();
    test_an_arc_extending_right_of_canvas_is_rejected();
    test_an_arc_with_unterminated_end_binding_is_rejected();
    test_an_image_within_bounds_validates();
    test_an_image_extending_past_canvas_width_is_rejected();
    test_a_glyph_within_bounds_validates();
    test_a_glyph_extending_past_canvas_width_is_rejected();
    test_a_glyph_taller_than_the_canvas_is_rejected();
    test_a_glyph_with_an_unterminated_name_is_rejected();
    test_a_line_with_populated_points_validates();
    test_a_line_with_a_non_first_point_outside_canvas_is_rejected();
    test_a_huge_coordinate_does_not_wrap_the_bounds_check();
    return 0;
}
