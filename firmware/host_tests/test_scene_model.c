#include <assert.h>
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

int main(void)
{
    test_a_minimal_scene_validates();
    test_too_many_nodes_is_rejected();
    test_an_unknown_node_kind_is_rejected();
    test_a_node_outside_the_canvas_is_rejected();
    test_an_unterminated_text_value_is_rejected();
    test_a_line_with_too_many_points_is_rejected();
    test_an_unknown_baked_font_tier_is_rejected();
    return 0;
}
