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

static void test_a_rect_with_a_canvas_bounded_clip_validates(void)
{
    scene_t scene = minimal_scene();
    scene.nodes[0].value.rect.has_clip = true;
    scene.nodes[0].value.rect.clip.x = 64;
    scene.nodes[0].value.rect.clip.y = 64;
    scene.nodes[0].value.rect.clip.w = 120;
    scene.nodes[0].value.rect.clip.h = 120;
    assert(scene_model_validate(&scene) == SCENE_MODEL_OK);
}

static void test_a_rect_clip_past_the_canvas_is_rejected(void)
{
    scene_t scene = minimal_scene();
    scene.nodes[0].value.rect.has_clip = true;
    scene.nodes[0].value.rect.clip.x = SCENE_CANVAS_WIDTH;
    scene.nodes[0].value.rect.clip.y = 64;
    scene.nodes[0].value.rect.clip.w = 120;
    scene.nodes[0].value.rect.clip.h = 120;
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

static void test_an_arc_with_a_running_color_validates(void)
{
    scene_t scene = minimal_scene();
    scene.nodes[0].kind = SCENE_NODE_ARC;
    scene.nodes[0].value.arc.cx = 100;
    scene.nodes[0].value.arc.cy = 100;
    scene.nodes[0].value.arc.r = 50;
    scene.nodes[0].value.arc.has_running_color = true;
    scene.nodes[0].value.arc.running_color = UINT32_C(0x00FF9F0A);
    assert(scene_model_validate(&scene) == SCENE_MODEL_OK);
}

static void test_text_with_a_running_color_validates(void)
{
    scene_t scene = minimal_scene();
    scene.nodes[0].kind = SCENE_NODE_TEXT;
    scene.nodes[0].value.text.w = 100;
    scene.nodes[0].value.text.align = SCENE_ALIGN_LEFT;
    scene.nodes[0].value.text.font.kind = SCENE_FONT_BAKED;
    scene.nodes[0].value.text.font.baked = SCENE_FONT_BODY;
    scene.nodes[0].value.text.has_running_color = true;
    scene.nodes[0].value.text.running_color = UINT32_C(0x00FF9F0A);
    assert(scene_model_validate(&scene) == SCENE_MODEL_OK);
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

static scene_t scale_scene(void)
{
    scene_t scene = minimal_scene();
    scene.nodes[0].kind = SCENE_NODE_SCALE;
    scene.nodes[0].value.scale.x = 10;
    scene.nodes[0].value.scale.y = 10;
    scene.nodes[0].value.scale.box = 50;
    scene.nodes[0].value.scale.total_tick_count = 13U;
    scene.nodes[0].value.scale.major_tick_every = 3U;
    return scene;
}

static void test_a_scale_within_bounds_validates(void)
{
    scene_t scene = scale_scene();
    assert(scene_model_validate(&scene) == SCENE_MODEL_OK);
}

static void test_a_scale_extending_past_canvas_width_is_rejected(void)
{
    scene_t scene = scale_scene();
    scene.nodes[0].value.scale.x = SCENE_CANVAS_WIDTH;
    assert(scene_model_validate(&scene) == SCENE_MODEL_ERR_GEOMETRY);
}

static void test_a_scale_extending_past_canvas_height_is_rejected(void)
{
    scene_t scene = scale_scene();
    scene.nodes[0].value.scale.y = SCENE_CANVAS_HEIGHT;
    assert(scene_model_validate(&scene) == SCENE_MODEL_ERR_GEOMETRY);
}

static void test_a_scale_with_too_many_ticks_is_rejected(void)
{
    scene_t scene = scale_scene();
    scene.nodes[0].value.scale.total_tick_count =
        SCENE_SCALE_MAX_TOTAL_TICKS + 1U;
    assert(scene_model_validate(&scene) == SCENE_MODEL_ERR_GEOMETRY);
}

static void test_a_scale_with_too_few_ticks_is_rejected(void)
{
    scene_t scene = scale_scene();
    scene.nodes[0].value.scale.total_tick_count = 1U;
    assert(scene_model_validate(&scene) == SCENE_MODEL_ERR_GEOMETRY);
}

static void test_a_scale_with_major_tick_every_zero_is_rejected(void)
{
    scene_t scene = scale_scene();
    scene.nodes[0].value.scale.major_tick_every = 0U;
    assert(scene_model_validate(&scene) == SCENE_MODEL_ERR_GEOMETRY);
}

static void test_a_scale_with_major_tick_every_past_total_is_rejected(void)
{
    scene_t scene = scale_scene();
    scene.nodes[0].value.scale.major_tick_every =
        scene.nodes[0].value.scale.total_tick_count + 1U;
    assert(scene_model_validate(&scene) == SCENE_MODEL_ERR_GEOMETRY);
}

static void test_a_scale_geometry_field_at_int32_max_does_not_wrap(void)
{
    /* Same wrap hazard as test_a_huge_coordinate_does_not_wrap_the_bounds_check,
     * proven again for SCALE's own geometry check rather than assumed from
     * rect's. */
    scene_t scene = scale_scene();
    scene.nodes[0].value.scale.x = INT32_MAX;
    scene.nodes[0].value.scale.box = 10;
    assert(scene_model_validate(&scene) == SCENE_MODEL_ERR_GEOMETRY);
}

static scene_t label_scene(void)
{
    scene_t scene = minimal_scene();
    scene.nodes[0].kind = SCENE_NODE_LABEL;
    scene.nodes[0].value.label.x = 16;
    scene.nodes[0].value.label.y = 16;
    scene.nodes[0].value.label.horizontal_anchor = SCENE_LABEL_ANCHOR_LEFT;
    scene.nodes[0].value.label.font.kind = SCENE_FONT_BAKED;
    scene.nodes[0].value.label.font.baked = SCENE_FONT_CAPTION;
    scene.nodes[0].value.label.value.kind = SCENE_VALUE_LITERAL;
    strcpy(scene.nodes[0].value.label.value.literal, "WEATHER");
    scene.nodes[0].value.label.fill_opacity = UINT8_MAX;
    scene.nodes[0].value.label.radius = 12;
    scene.nodes[0].value.label.pad_hor = 16;
    scene.nodes[0].value.label.pad_ver = 3;
    scene.nodes[0].value.label.letter_space = 1;
    scene.nodes[0].value.label.hide_when_empty = true;
    return scene;
}

static void test_a_label_within_bounds_validates(void)
{
    scene_t scene = label_scene();
    assert(scene_model_validate(&scene) == SCENE_MODEL_OK);
}

static void test_center_and_right_label_anchors_validate(void)
{
    scene_t scene = label_scene();

    scene.nodes[0].value.label.horizontal_anchor =
        SCENE_LABEL_ANCHOR_CENTER;
    assert(scene_model_validate(&scene) == SCENE_MODEL_OK);

    scene.nodes[0].value.label.horizontal_anchor = SCENE_LABEL_ANCHOR_RIGHT;
    scene.nodes[0].value.label.x = SCENE_CANVAS_WIDTH;
    assert(scene_model_validate(&scene) == SCENE_MODEL_OK);
}

static void test_a_right_label_anchor_past_the_canvas_edge_is_rejected(void)
{
    scene_t scene = label_scene();
    scene.nodes[0].value.label.horizontal_anchor = SCENE_LABEL_ANCHOR_RIGHT;
    scene.nodes[0].value.label.x = SCENE_CANVAS_WIDTH + 1;
    assert(scene_model_validate(&scene) == SCENE_MODEL_ERR_GEOMETRY);
}

static void test_an_unknown_label_anchor_is_rejected(void)
{
    scene_t scene = label_scene();
    scene.nodes[0].value.label.horizontal_anchor =
        (scene_label_anchor_t)3;
    assert(scene_model_validate(&scene) == SCENE_MODEL_ERR_GEOMETRY);
}

static void test_a_label_anchor_past_canvas_width_is_rejected(void)
{
    scene_t scene = label_scene();
    scene.nodes[0].value.label.x = SCENE_CANVAS_WIDTH;
    assert(scene_model_validate(&scene) == SCENE_MODEL_ERR_GEOMETRY);
}

static void test_a_label_anchor_past_canvas_height_is_rejected(void)
{
    scene_t scene = label_scene();
    scene.nodes[0].value.label.y = SCENE_CANVAS_HEIGHT;
    assert(scene_model_validate(&scene) == SCENE_MODEL_ERR_GEOMETRY);
}

static void test_a_label_with_an_unknown_font_is_rejected(void)
{
    scene_t scene = label_scene();
    scene.nodes[0].value.label.font.baked = (scene_font_tier_t)99;
    assert(scene_model_validate(&scene) == SCENE_MODEL_ERR_FONT);
}

static void test_a_label_with_an_unterminated_value_is_rejected(void)
{
    scene_t scene = label_scene();
    memset(scene.nodes[0].value.label.value.literal, 'x',
           sizeof scene.nodes[0].value.label.value.literal);
    assert(scene_model_validate(&scene) == SCENE_MODEL_ERR_TEXT);
}

static scene_t rotated_rect_scene(void)
{
    scene_t scene = minimal_scene();
    scene.nodes[0].kind = SCENE_NODE_ROT_RECT;
    scene.nodes[0].value.rot_rect.x = 221;
    scene.nodes[0].value.rot_rect.y = 80;
    scene.nodes[0].value.rot_rect.w = 6;
    scene.nodes[0].value.rot_rect.h = 104;
    scene.nodes[0].value.rot_rect.radius = 3;
    scene.nodes[0].value.rot_rect.pivot_x = 3;
    scene.nodes[0].value.rot_rect.pivot_y = 104;
    scene.nodes[0].value.rot_rect.rotation = 900;
    return scene;
}

static void test_a_rotated_rect_within_bounds_validates(void)
{
    scene_t scene = rotated_rect_scene();
    assert(scene_model_validate(&scene) == SCENE_MODEL_OK);
}

static void test_a_rotated_rect_past_canvas_bounds_is_rejected(void)
{
    scene_t scene = rotated_rect_scene();
    scene.nodes[0].value.rot_rect.x = SCENE_CANVAS_WIDTH;
    assert(scene_model_validate(&scene) == SCENE_MODEL_ERR_GEOMETRY);
}

static void test_a_rotated_rect_rotation_past_one_turn_is_rejected(void)
{
    scene_t scene = rotated_rect_scene();
    scene.nodes[0].value.rot_rect.rotation = 3601;
    assert(scene_model_validate(&scene) == SCENE_MODEL_ERR_GEOMETRY);
}

static void test_a_rotated_rect_negative_one_turn_validates(void)
{
    scene_t scene = rotated_rect_scene();
    scene.nodes[0].value.rot_rect.rotation = -3600;
    assert(scene_model_validate(&scene) == SCENE_MODEL_OK);
}

static void test_a_rotated_rect_external_pivot_for_an_analog_tick_validates(void)
{
    scene_t scene = rotated_rect_scene();
    scene.nodes[0].value.rot_rect.h = 8;
    scene.nodes[0].value.rot_rect.pivot_y = 160;
    assert(scene_model_validate(&scene) == SCENE_MODEL_OK);
}

static void test_a_rotated_rect_with_a_canvas_bounded_clip_validates(void)
{
    scene_t scene = rotated_rect_scene();
    scene.nodes[0].value.rot_rect.has_clip = true;
    scene.nodes[0].value.rot_rect.clip.x = 64;
    scene.nodes[0].value.rot_rect.clip.y = 24;
    scene.nodes[0].value.rot_rect.clip.w = 320;
    scene.nodes[0].value.rot_rect.clip.h = 320;
    assert(scene_model_validate(&scene) == SCENE_MODEL_OK);
}

static void test_a_rotated_rect_clip_past_the_canvas_is_rejected(void)
{
    scene_t scene = rotated_rect_scene();
    scene.nodes[0].value.rot_rect.has_clip = true;
    scene.nodes[0].value.rot_rect.clip.x = SCENE_CANVAS_WIDTH;
    scene.nodes[0].value.rot_rect.clip.y = 24;
    scene.nodes[0].value.rot_rect.clip.w = 320;
    scene.nodes[0].value.rot_rect.clip.h = 320;
    assert(scene_model_validate(&scene) == SCENE_MODEL_ERR_GEOMETRY);
}

static void test_a_rotated_rect_pivot_past_canvas_width_is_rejected(void)
{
    scene_t scene = rotated_rect_scene();
    scene.nodes[0].value.rot_rect.pivot_x = SCENE_CANVAS_WIDTH + 1;
    assert(scene_model_validate(&scene) == SCENE_MODEL_ERR_GEOMETRY);
}

static void test_a_rotated_rect_pivot_past_canvas_height_is_rejected(void)
{
    scene_t scene = rotated_rect_scene();
    scene.nodes[0].value.rot_rect.pivot_y = SCENE_CANVAS_HEIGHT + 1;
    assert(scene_model_validate(&scene) == SCENE_MODEL_ERR_GEOMETRY);
}

static void test_a_rotated_rect_with_unterminated_binding_is_rejected(void)
{
    scene_t scene = rotated_rect_scene();
    memset(scene.nodes[0].value.rot_rect.rotation_binding, 'x',
           sizeof scene.nodes[0].value.rot_rect.rotation_binding);
    assert(scene_model_validate(&scene) == SCENE_MODEL_ERR_TEXT);
}

static void test_arc_span_of_a_full_turn_is_360(void)
{
    assert(scene_model_arc_span(270, 630) == 360);
}

static void test_arc_span_of_a_partial_sweep(void)
{
    assert(scene_model_arc_span(270, 450) == 180);
}

static void test_arc_span_of_a_negative_raw_difference(void)
{
    assert(scene_model_arc_span(270, 90) == 180);
}

static void test_arc_span_of_equal_endpoints_is_zero(void)
{
    assert(scene_model_arc_span(270, 270) == 0);
}

static void test_arc_span_does_not_wrap_at_int32_extremes(void)
{
    /* An int32 subtraction of these two wraps to a raw difference of 1
     * (INT32_MIN - INT32_MAX mod 2^32 == 1), which would yield a 1-degree
     * span. Widening to int64 before subtracting gives the correct 105. */
    assert(scene_model_arc_span(INT32_MAX, INT32_MIN) == 105);
}

static void test_arc_origin_wraps_a_full_turn(void)
{
    assert(scene_model_arc_origin(630) == 270);
}

static void test_arc_origin_normalises_a_negative_angle(void)
{
    assert(scene_model_arc_origin(-90) == 270);
}

int main(void)
{
    test_a_minimal_scene_validates();
    test_too_many_nodes_is_rejected();
    test_an_unknown_node_kind_is_rejected();
    test_a_node_outside_the_canvas_is_rejected();
    test_a_rect_with_a_canvas_bounded_clip_validates();
    test_a_rect_clip_past_the_canvas_is_rejected();
    test_an_unterminated_text_value_is_rejected();
    test_a_line_with_too_many_points_is_rejected();
    test_an_unknown_baked_font_tier_is_rejected();
    test_an_unknown_text_align_is_rejected();
    test_an_arc_within_bounds_validates();
    test_an_arc_extending_left_of_canvas_is_rejected();
    test_an_arc_extending_right_of_canvas_is_rejected();
    test_an_arc_with_unterminated_end_binding_is_rejected();
    test_an_arc_with_a_running_color_validates();
    test_text_with_a_running_color_validates();
    test_an_image_within_bounds_validates();
    test_an_image_extending_past_canvas_width_is_rejected();
    test_a_glyph_within_bounds_validates();
    test_a_glyph_extending_past_canvas_width_is_rejected();
    test_a_glyph_taller_than_the_canvas_is_rejected();
    test_a_glyph_with_an_unterminated_name_is_rejected();
    test_a_line_with_populated_points_validates();
    test_a_line_with_a_non_first_point_outside_canvas_is_rejected();
    test_a_huge_coordinate_does_not_wrap_the_bounds_check();
    test_a_scale_within_bounds_validates();
    test_a_scale_extending_past_canvas_width_is_rejected();
    test_a_scale_extending_past_canvas_height_is_rejected();
    test_a_scale_with_too_many_ticks_is_rejected();
    test_a_scale_with_too_few_ticks_is_rejected();
    test_a_scale_with_major_tick_every_zero_is_rejected();
    test_a_scale_with_major_tick_every_past_total_is_rejected();
    test_a_scale_geometry_field_at_int32_max_does_not_wrap();
    test_a_label_within_bounds_validates();
    test_center_and_right_label_anchors_validate();
    test_a_right_label_anchor_past_the_canvas_edge_is_rejected();
    test_an_unknown_label_anchor_is_rejected();
    test_a_label_anchor_past_canvas_width_is_rejected();
    test_a_label_anchor_past_canvas_height_is_rejected();
    test_a_label_with_an_unknown_font_is_rejected();
    test_a_label_with_an_unterminated_value_is_rejected();
    test_a_rotated_rect_within_bounds_validates();
    test_a_rotated_rect_past_canvas_bounds_is_rejected();
    test_a_rotated_rect_rotation_past_one_turn_is_rejected();
    test_a_rotated_rect_negative_one_turn_validates();
    test_a_rotated_rect_external_pivot_for_an_analog_tick_validates();
    test_a_rotated_rect_with_a_canvas_bounded_clip_validates();
    test_a_rotated_rect_clip_past_the_canvas_is_rejected();
    test_a_rotated_rect_pivot_past_canvas_width_is_rejected();
    test_a_rotated_rect_pivot_past_canvas_height_is_rejected();
    test_a_rotated_rect_with_unterminated_binding_is_rejected();
    test_arc_span_of_a_full_turn_is_360();
    test_arc_span_of_a_partial_sweep();
    test_arc_span_of_a_negative_raw_difference();
    test_arc_span_of_equal_endpoints_is_zero();
    test_arc_span_does_not_wrap_at_int32_extremes();
    test_arc_origin_wraps_a_full_turn();
    test_arc_origin_normalises_a_negative_angle();
    return 0;
}
