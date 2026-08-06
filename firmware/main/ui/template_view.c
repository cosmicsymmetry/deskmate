#include "template_view.h"

#include <string.h>

#include "lvgl.h"
#include "templates/template_internal.h"

typedef struct {
    lv_obj_t *screen;
    template_widget_view_t widget;
    protocol_template_kind_t template_kind;
    protocol_size_class_t size_class;
    bool active;
} template_view_state_t;

static template_view_state_t s_view;
static lv_timer_t *s_tick_timer;
static int16_t s_utc_offset_minutes;
static uint32_t s_generation;
static uint32_t s_patch_count;

static void tick_cb(lv_timer_t *timer)
{
    (void)timer;
    if (!s_view.active || s_view.screen == NULL) {
        return;
    }
    if (s_view.template_kind == PROTOCOL_TEMPLATE_DIGITAL_CLOCK) {
        digital_clock_tick(&s_view.widget, s_utc_offset_minutes);
    } else if (s_view.template_kind == PROTOCOL_TEMPLATE_ANALOG_CLOCK) {
        analog_clock_tick(&s_view.widget, s_utc_offset_minutes);
    } else if (s_view.template_kind == PROTOCOL_TEMPLATE_PROGRESS_RING) {
        progress_ring_tick(&s_view.widget);
    }
}

static void clear_live_view(void)
{
    memset(&s_view, 0, sizeof(s_view));
    if (s_tick_timer != NULL) {
        lv_timer_pause(s_tick_timer);
    }
}

static void screen_deleted_cb(lv_event_t *event)
{
    if (lv_event_get_target_obj(event) == s_view.screen) {
        clear_live_view();
    }
}

static bool create_widget(template_view_state_t *view, lv_obj_t *parent)
{
    /* Size classes remain accepted for protocol-v1 compatibility, but every
     * M3 widget now receives the clean full canvas. */
    const protocol_size_class_t rendered_size = PROTOCOL_SIZE_FULL;
    if (view->template_kind == PROTOCOL_TEMPLATE_DIGITAL_CLOCK) {
        return digital_clock_create(&view->widget, parent,
                                    rendered_size);
    }
    if (view->template_kind == PROTOCOL_TEMPLATE_ANALOG_CLOCK) {
        return analog_clock_create(&view->widget, parent, rendered_size);
    }
    if (view->template_kind == PROTOCOL_TEMPLATE_PROGRESS_RING) {
        return progress_ring_create(&view->widget, parent,
                                    rendered_size);
    }
    if (view->template_kind == PROTOCOL_TEMPLATE_ROW_LIST) {
        return row_list_create(&view->widget, parent, rendered_size);
    }
    return false;
}

static void update_data_state(template_widget_view_t *widget,
                              bool stale,
                              const char *error)
{
    if (widget == NULL || widget->state_label == NULL) {
        return;
    }
    if (error != NULL && error[0] != '\0') {
        lv_label_set_text(widget->state_label, error);
        lv_obj_set_style_text_color(widget->state_label,
                                    lv_color_hex(0xff6b6b), 0);
    } else if (stale) {
        lv_label_set_text(widget->state_label, "Stale");
        lv_obj_set_style_text_color(widget->state_label,
                                    lv_color_hex(0xf2c94c), 0);
    } else {
        lv_label_set_text(widget->state_label, "");
    }
}

static void update_data_state_from_fields(
    template_widget_view_t *widget,
    const template_field_state_t *fields)
{
    const template_field_value_t *error = template_fields_get(fields,
                                                               "error");
    const template_field_value_t *stale = template_fields_get(fields,
                                                               "stale");
    update_data_state(widget,
                      stale != NULL && stale->value.boolean,
                      error != NULL ? error->value.text : NULL);
}

static void patch_widget(template_view_state_t *view,
                         const template_field_state_t *fields,
                         uint16_t dirty_mask)
{
    if (view->template_kind == PROTOCOL_TEMPLATE_DIGITAL_CLOCK) {
        digital_clock_patch(&view->widget, fields, dirty_mask);
        digital_clock_tick(&view->widget, s_utc_offset_minutes);
    } else if (view->template_kind == PROTOCOL_TEMPLATE_ANALOG_CLOCK) {
        analog_clock_patch(&view->widget, fields, dirty_mask);
        analog_clock_tick(&view->widget, s_utc_offset_minutes);
    } else if (view->template_kind == PROTOCOL_TEMPLATE_PROGRESS_RING) {
        progress_ring_patch(&view->widget, fields, dirty_mask);
    } else if (view->template_kind == PROTOCOL_TEMPLATE_ROW_LIST) {
        row_list_patch(&view->widget, fields, dirty_mask);
    }
    update_data_state_from_fields(&view->widget, fields);
}

bool template_view_show(protocol_template_kind_t template_kind,
                        protocol_size_class_t size_class,
                        const template_field_state_t *fields)
{
    if (fields == NULL || fields->template_kind != template_kind ||
        (size_class != PROTOCOL_SIZE_FULL &&
         size_class != PROTOCOL_SIZE_STANDARD)) {
        return false;
    }

    /* Build against local pointer state so a failed allocation leaves the
     * currently displayed view and its object graph untouched. */
    template_view_state_t candidate = {
        .template_kind = template_kind,
        .size_class = size_class,
    };
    candidate.screen = lv_obj_create(NULL);
    if (candidate.screen == NULL) {
        return false;
    }
    lv_obj_remove_flag(candidate.screen, LV_OBJ_FLAG_SCROLLABLE);
    lv_obj_add_flag(candidate.screen, LV_OBJ_FLAG_CLICKABLE);
    lv_obj_set_style_bg_color(candidate.screen, lv_color_hex(0x101020), 0);
    lv_obj_set_style_bg_opa(candidate.screen, LV_OPA_COVER, 0);

    lv_obj_t *content = lv_obj_create(candidate.screen);
    if (content == NULL) {
        lv_obj_delete(candidate.screen);
        return false;
    }
    lv_obj_remove_style_all(content);
    /* Full-canvas layout containers must not win LVGL hit testing. The
     * carousel screen owns press/release classification for both swipes and
     * widget taps. */
    lv_obj_remove_flag(content,
                       LV_OBJ_FLAG_CLICKABLE | LV_OBJ_FLAG_SCROLLABLE);
    lv_obj_set_width(content, LV_PCT(100));
    lv_obj_set_height(content, LV_PCT(100));
    lv_obj_align(content, LV_ALIGN_CENTER, 0, 0);

    if (!create_widget(&candidate, content)) {
        lv_obj_delete(candidate.screen);
        return false;
    }
    patch_widget(&candidate, fields, UINT16_MAX);

    lv_obj_add_event_cb(candidate.screen, screen_deleted_cb,
                        LV_EVENT_DELETE, NULL);
    candidate.active = true;
    s_view = candidate;
    ++s_generation;
    if (s_tick_timer == NULL) {
        s_tick_timer = lv_timer_create(tick_cb, 250U, NULL);
    } else {
        lv_timer_resume(s_tick_timer);
    }
    lv_screen_load_anim(candidate.screen, LV_SCREEN_LOAD_ANIM_NONE,
                        0U, 0U, true);
    return true;
}

bool template_view_patch(protocol_template_kind_t template_kind,
                         const template_field_state_t *fields,
                         uint16_t dirty_mask)
{
    if (s_view.screen == NULL || fields == NULL ||
        template_kind != s_view.template_kind ||
        fields->template_kind != s_view.template_kind) {
        return false;
    }
    patch_widget(&s_view, fields, dirty_mask);
    ++s_patch_count;
    return true;
}

void template_view_set_data_state(bool stale, const char *error)
{
    update_data_state(&s_view.widget, stale, error);
}

void template_view_set_utc_offset_minutes(int16_t offset_minutes)
{
    s_utc_offset_minutes = offset_minutes;
    tick_cb(NULL);
}

void template_view_apply_local_action(protocol_event_action_t action)
{
    if (s_view.template_kind == PROTOCOL_TEMPLATE_PROGRESS_RING) {
        progress_ring_local_action(&s_view.widget, action);
    }
}

bool template_view_activate(void)
{
    if (s_view.screen == NULL) {
        return false;
    }
    s_view.active = true;
    if (s_tick_timer != NULL) {
        lv_timer_resume(s_tick_timer);
    }
    if (lv_screen_active() != s_view.screen) {
        lv_screen_load_anim(s_view.screen, LV_SCREEN_LOAD_ANIM_NONE,
                            0U, 0U, true);
    }
    tick_cb(NULL);
    return true;
}

void template_view_deactivate(void)
{
    s_view.active = false;
    if (s_tick_timer != NULL) {
        lv_timer_pause(s_tick_timer);
    }
}

bool template_view_destroy(void)
{
    if (s_view.screen == NULL) {
        return true;
    }
    if (lv_screen_active() == s_view.screen) {
        return false;
    }
    lv_obj_delete(s_view.screen);
    return true;
}

bool template_view_active(void)
{
    return s_view.active && s_view.screen != NULL &&
           lv_screen_active() == s_view.screen;
}

lv_obj_t *template_view_screen(void)
{
    return s_view.screen;
}

uint32_t template_view_generation(void)
{
    return s_generation;
}

uint32_t template_view_patch_count(void)
{
    return s_patch_count;
}
