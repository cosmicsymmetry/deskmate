#pragma once

/* Hardware-independent mapping from queued UI work to the renderer action
 * the LVGL runtime must take. Keep this under core/ so the standalone-clock
 * fallback rule is covered by the plain-C host suite. */
typedef enum {
    UI_COMMAND_SHOW_STANDALONE = 0,
    UI_COMMAND_SHOW_CARD_FALLBACK,
    UI_COMMAND_LINK_STATE,
    UI_COMMAND_TIME_OFFSET,
} ui_command_type_t;

typedef enum {
    UI_COMMAND_ACTION_NONE = 0,
    UI_COMMAND_ACTION_SHOW_STANDALONE_CLOCK,
    UI_COMMAND_ACTION_APPLY_LINK_STATE,
    UI_COMMAND_ACTION_APPLY_TIME_OFFSET,
} ui_command_action_t;

ui_command_action_t ui_command_action(ui_command_type_t type);
