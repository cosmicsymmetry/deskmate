idf_component_register(
    SRCS "main.c" "board/board_i2c.c" "board/display.c" "board/touch.c"
         "core/clock_source.c" "core/device_event_queue.c" "core/interrupt_state.c"
         "core/link_state.c" "core/navigation.c"
         "core/net_config.c"
         "core/protocol_frame.c"
         "core/protocol_message.c"
         "core/template_fields.c" "core/timefmt.c" "core/widget_model.c"
         "link/dev_capture.c" "link/net_store.c" "link/protocol_task.c" "link/usb_link.c" "ui/carousel.c"
         "ui/clock_screen.c"
         "ui/template_view.c" "ui/ui_command_queue.c"
         "ui/ui_runtime.c"
         "ui/templates/analog_clock.c" "ui/templates/big_number_label.c"
         "ui/templates/digital_clock.c"
         "ui/templates/icon_badge_text.c"
         "ui/templates/progress_ring.c"
         "ui/templates/row_list.c" "ui/templates/template_style.c"
         "ui/templates/weather_icon.c"
         "ui/fonts/deskmate_font_18.c" "ui/fonts/deskmate_font_28.c"
         "ui/fonts/deskmate_font_56.c" "ui/fonts/deskmate_font_96.c"
    INCLUDE_DIRS "." "board" "core" "link" "ui"
    REQUIRES cbor esp_app_format esp_timer nvs_flash
)

# TinyCBOR defaults to 1024 recursive containers, which is inappropriate for
# an 8 KiB embedded task stack and untrusted host payloads. M1 schemas need at
# most two levels; eight preserves evolution room while bounding stack use.
idf_component_get_property(cbor_lib espressif__cbor COMPONENT_LIB)
target_compile_definitions(${cbor_lib} PRIVATE CBOR_PARSER_MAX_RECURSIONS=8)

# Make the single checked-in firmware/lv_conf.h (CONFIG_LV_CONF_SKIP=n, see
# sdkconfig.defaults) visible to every translation unit that includes
# lvgl.h -- not just this component. lv_conf_internal.h resolves lv_conf.h
# via a plain quote-search ("#include "lv_conf.h""); LVGL's own component
# only puts firmware/managed_components/ (not firmware/) on its include
# path, and a target_compile_definitions()/target_include_directories() set
# here on this component's own library would only apply to *this*
# component's sources, not LVGL's (PUBLIC compile options flow to a
# target's dependents, not backwards to its dependencies). Appending to the
# global COMPILE_OPTIONS build property instead reaches every component,
# including the vendored lvgl and esp_lvgl_port components, because each
# component's directory-scoped compile options resolve this property via a
# generator expression at CMake generate time -- after every component,
# including this one, has registered.
idf_build_set_property(COMPILE_OPTIONS "-I${CMAKE_CURRENT_LIST_DIR}/.." APPEND)

# Dev-only framebuffer capture (Task 10, spec §3.2.3). link/dev_capture.c is
# always compiled, but its entire body is `#ifdef DESKMATE_DEV_DIAG`-guarded,
# so it is an empty translation unit without the flag. `idf.py -C firmware
# -DDESKMATE_DEV_DIAG=1 build` sets a CMake cache variable at configure time;
# map it to the matching C macro here so a plain build never defines it, and
# the release binary contains neither the handler, the 0x7E/0x7F message
# ids, nor dev_capture.c's log tag.
if(DESKMATE_DEV_DIAG)
    target_compile_definitions(${COMPONENT_LIB} PRIVATE DESKMATE_DEV_DIAG=1)
endif()
