#include "ui_command_policy.h"

ui_command_action_t ui_command_action(ui_command_type_t type)
{
    switch (type) {
    case UI_COMMAND_SHOW_CARD_FALLBACK:
        return UI_COMMAND_ACTION_SHOW_STANDALONE_CLOCK;
    case UI_COMMAND_LINK_STATE:
        return UI_COMMAND_ACTION_APPLY_LINK_STATE;
    case UI_COMMAND_TIME_OFFSET:
        return UI_COMMAND_ACTION_APPLY_TIME_OFFSET;
    default:
        return UI_COMMAND_ACTION_NONE;
    }
}
