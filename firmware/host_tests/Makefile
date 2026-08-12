CC ?= cc
CFLAGS = -Wall -Wextra -Werror -std=c11 -DCBOR_PARSER_MAX_RECURSIONS=8
CBOR_DIR = ../managed_components/espressif__cbor/tinycbor/src
CBOR_SRCS = $(CBOR_DIR)/cborencoder.c \
	$(CBOR_DIR)/cborencoder_close_container_checked.c \
	$(CBOR_DIR)/cborparser.c \
	$(CBOR_DIR)/cborparser_float.c \
	$(CBOR_DIR)/cborvalidation.c
PROTOCOL_SRCS = ../main/core/protocol_frame.c ../main/core/protocol_message.c \
	$(CBOR_SRCS)

test: test_timefmt test_protocol test_link_state test_template_fields \
	test_widget_model test_ui_command_queue test_navigation \
	test_interrupt_state test_device_event_queue test_weather_icon \
	test_clock_source
	./test_timefmt
	./test_protocol
	./test_link_state
	./test_template_fields
	./test_widget_model
	./test_ui_command_queue
	./test_navigation
	./test_interrupt_state
	./test_device_event_queue
	./test_weather_icon
	./test_clock_source

test_timefmt: test_timefmt.c ../main/core/timefmt.c
	$(CC) $(CFLAGS) -o $@ $^

test_protocol: test_protocol.c $(PROTOCOL_SRCS)
	$(CC) $(CFLAGS) -I$(CBOR_DIR) -o $@ $^

test_link_state: test_link_state.c ../main/core/link_state.c
	$(CC) $(CFLAGS) -o $@ $^

test_template_fields: test_template_fields.c ../main/core/template_fields.c \
	$(PROTOCOL_SRCS)
	$(CC) $(CFLAGS) -I$(CBOR_DIR) -o $@ $^

test_widget_model: test_widget_model.c ../main/core/widget_model.c \
	../main/core/template_fields.c ../main/core/link_state.c \
	$(PROTOCOL_SRCS)
	$(CC) $(CFLAGS) -I$(CBOR_DIR) -o $@ $^

test_ui_command_queue: test_ui_command_queue.c \
	../main/ui/ui_command_queue.c ../main/core/template_fields.c
	$(CC) $(CFLAGS) -I../main -o $@ $^

test_navigation: test_navigation.c ../main/core/navigation.c
	$(CC) $(CFLAGS) -o $@ $^

test_interrupt_state: test_interrupt_state.c \
	../main/core/interrupt_state.c
	$(CC) $(CFLAGS) -o $@ $^

test_device_event_queue: test_device_event_queue.c \
	../main/core/device_event_queue.c
	$(CC) $(CFLAGS) -o $@ $^

test_weather_icon: test_weather_icon.c \
	../main/ui/templates/weather_icon.c
	$(CC) $(CFLAGS) -DWEATHER_ICON_HOST_TEST -o $@ $^

test_clock_source: test_clock_source.c ../main/core/clock_source.c
	$(CC) $(CFLAGS) -o $@ $^

sanitize: clean
	$(MAKE) CFLAGS="$(CFLAGS) -O1 -g -fsanitize=address,undefined" test

clean:
	rm -f test_timefmt test_protocol test_link_state test_template_fields \
		test_widget_model test_ui_command_queue
	rm -f test_navigation
	rm -f test_interrupt_state
	rm -f test_device_event_queue
	rm -f test_weather_icon
	rm -f test_clock_source
	rm -rf test_timefmt.dSYM test_protocol.dSYM test_link_state.dSYM \
		test_template_fields.dSYM test_widget_model.dSYM
	rm -rf test_ui_command_queue.dSYM
	rm -rf test_navigation.dSYM
	rm -rf test_interrupt_state.dSYM
	rm -rf test_device_event_queue.dSYM
	rm -rf test_weather_icon.dSYM
	rm -rf test_clock_source.dSYM
.PHONY: test sanitize clean
