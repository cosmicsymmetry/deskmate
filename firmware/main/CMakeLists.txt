idf_component_register(
    SRCS "main.c" "board/board_i2c.c" "board/display.c" "board/touch.c"
         "core/device_event_queue.c" "core/interrupt_state.c"
         "core/link_state.c" "core/navigation.c"
         "core/protocol_frame.c"
         "core/protocol_message.c"
         "core/template_fields.c" "core/timefmt.c" "core/widget_model.c"
         "link/protocol_task.c" "link/usb_link.c" "ui/carousel.c"
         "ui/clock_screen.c"
         "ui/template_view.c" "ui/ui_command_queue.c"
         "ui/ui_runtime.c"
         "ui/templates/digital_clock.c" "ui/templates/progress_ring.c"
         "ui/templates/row_list.c"
    INCLUDE_DIRS "." "board" "core" "link" "ui"
    REQUIRES cbor esp_app_format esp_timer
)

# TinyCBOR defaults to 1024 recursive containers, which is inappropriate for
# an 8 KiB embedded task stack and untrusted host payloads. M1 schemas need at
# most two levels; eight preserves evolution room while bounding stack use.
idf_component_get_property(cbor_lib espressif__cbor COMPONENT_LIB)
target_compile_definitions(${cbor_lib} PRIVATE CBOR_PARSER_MAX_RECURSIONS=8)
