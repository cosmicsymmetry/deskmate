idf_component_register(
    SRCS "main.c" "board/board_i2c.c" "board/display.c" "board/touch.c"
         "core/timefmt.c" "ui/clock_screen.c"
    INCLUDE_DIRS "." "board" "core" "ui"
)
