#pragma once

#include <stdbool.h>
#include <stdint.h>

#include "lvgl.h"

#define STATUS_STRIP_HEIGHT 32

typedef struct {
    lv_obj_t *root;
    lv_obj_t *time_label;
    lv_obj_t *connection_dot;
    lv_obj_t *interrupt_dots[2];
} status_strip_t;

bool status_strip_create(status_strip_t *strip, lv_obj_t *parent);
void status_strip_set_online(status_strip_t *strip, bool online);
void status_strip_set_interrupts(status_strip_t *strip,
                                 bool active,
                                 bool pending);
void status_strip_tick(status_strip_t *strip, int16_t utc_offset_minutes);
void status_strip_reset(status_strip_t *strip);
