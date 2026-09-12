CC ?= cc
CFLAGS = -Wall -Wextra -Werror -std=c11 -DCBOR_PARSER_MAX_RECURSIONS=8
CBOR_DIR = ../managed_components/espressif__cbor/tinycbor/src
CBOR_SRCS = $(CBOR_DIR)/cborencoder.c \
	$(CBOR_DIR)/cborencoder_close_container_checked.c \
	$(CBOR_DIR)/cborparser.c \
	$(CBOR_DIR)/cborparser_float.c \
	$(CBOR_DIR)/cborvalidation.c
# core/protocol_message.c decodes PushScene's nested scene map through
# scene_decode_map(), so every binary that links it now links the scene
# module too -- and scene_binding.c, which the scene decoder calls to refuse
# a binding the device could not evaluate, and timefmt.c/asset_store.c
# beneath those.
SCENE_SRCS = ../main/core/scene_decode.c ../main/core/scene_model.c \
	../main/core/scene_binding.c ../main/core/timefmt.c \
	../main/core/asset_store.c
PROTOCOL_SRCS = ../main/core/protocol_frame.c ../main/core/protocol_message.c \
	$(SCENE_SRCS) $(CBOR_SRCS)

test: test_timefmt test_protocol test_link_state \
	test_widget_model test_ui_command_queue test_navigation \
	test_interrupt_state test_device_event_queue \
	test_clock_source test_net_config test_net_ring test_reconnect_backoff \
	test_ota_policy test_asset_store test_asset_transfer test_scene_model \
	test_rle565 test_volatile_asset_store test_scene_binding test_scene_decode test_font_registry
	./test_timefmt
	./test_protocol
	./test_link_state
	./test_widget_model
	./test_ui_command_queue
	./test_navigation
	./test_interrupt_state
	./test_device_event_queue
	./test_clock_source
	./test_net_config
	./test_net_ring
	./test_reconnect_backoff
	./test_ota_policy
	./test_asset_store
	./test_asset_transfer
	./test_rle565
	./test_volatile_asset_store
	./test_scene_model
	./test_scene_binding
	./test_scene_decode
	./test_font_registry

test_timefmt: test_timefmt.c ../main/core/timefmt.c
	$(CC) $(CFLAGS) -o $@ $^

test_protocol: test_protocol.c $(PROTOCOL_SRCS)
	$(CC) $(CFLAGS) -I$(CBOR_DIR) -I../main -o $@ $^

test_link_state: test_link_state.c ../main/core/link_state.c
	$(CC) $(CFLAGS) -I../main -o $@ $^

test_widget_model: test_widget_model.c ../main/core/widget_model.c \
	../main/core/link_state.c
	$(CC) $(CFLAGS) -I../main -o $@ $^

test_ui_command_queue: test_ui_command_queue.c ../main/ui/ui_command_queue.c \
	../main/core/ui_command_policy.c
	$(CC) $(CFLAGS) -I../main -o $@ $^

test_navigation: test_navigation.c ../main/core/navigation.c
	$(CC) $(CFLAGS) -o $@ $^

test_interrupt_state: test_interrupt_state.c \
	../main/core/interrupt_state.c
	$(CC) $(CFLAGS) -I../main -o $@ $^

test_device_event_queue: test_device_event_queue.c \
	../main/core/device_event_queue.c
	$(CC) $(CFLAGS) -I../main -o $@ $^

test_clock_source: test_clock_source.c ../main/core/clock_source.c
	$(CC) $(CFLAGS) -o $@ $^

test_net_config: test_net_config.c ../main/core/net_config.c $(PROTOCOL_SRCS)
	$(CC) $(CFLAGS) -I$(CBOR_DIR) -I../main -o $@ $^

test_net_ring: test_net_ring.c ../main/core/net_ring.c
	$(CC) $(CFLAGS) -o $@ $^

test_reconnect_backoff: test_reconnect_backoff.c \
	../main/core/reconnect_backoff.c
	$(CC) $(CFLAGS) -o $@ $^

test_ota_policy: test_ota_policy.c ../main/core/ota_policy.c
	$(CC) $(CFLAGS) -I../main -o $@ $^

test_asset_store: test_asset_store.c ../main/core/asset_store.c
	$(CC) $(CFLAGS) -I../main -o $@ $^

test_asset_transfer: test_asset_transfer.c ../main/core/asset_transfer.c \
	../main/core/asset_store.c
	$(CC) $(CFLAGS) -I../main -o $@ $^

test_rle565: test_rle565.c ../main/core/rle565.c
	$(CC) $(CFLAGS) -I../main -o $@ $^

test_volatile_asset_store: test_volatile_asset_store.c \
	../main/core/volatile_asset_store.c ../main/core/asset_transfer.c \
	../main/core/asset_store.c ../main/core/rle565.c
	$(CC) $(CFLAGS) -I../main -o $@ $^

test_scene_model: test_scene_model.c ../main/core/scene_model.c \
	../main/core/asset_store.c
	$(CC) $(CFLAGS) -I../main -o $@ $^

test_scene_binding: test_scene_binding.c ../main/core/scene_binding.c \
	../main/core/timefmt.c
	$(CC) $(CFLAGS) -I../main -o $@ $^

test_scene_decode: test_scene_decode.c ../main/core/scene_decode.c \
	../main/core/scene_model.c ../main/core/scene_binding.c \
	../main/core/timefmt.c ../main/core/asset_store.c $(CBOR_SRCS)
	$(CC) $(CFLAGS) -I$(CBOR_DIR) -I../main -o $@ $^

# -Istubs supplies host_tests/stubs/lvgl.h, the four-symbol stand-in that
# lets font_registry.c's reference-counting/eviction policy be tested without
# building LVGL. See that header for why.
test_font_registry: test_font_registry.c ../main/ui/font_registry.c
	$(CC) $(CFLAGS) -I../main -Istubs -o $@ $^

sanitize: clean
	$(MAKE) CFLAGS="$(CFLAGS) -O1 -g -fsanitize=address,undefined" test

clean:
	rm -f test_timefmt test_protocol test_link_state \
		test_widget_model test_ui_command_queue
	rm -f test_navigation
	rm -f test_interrupt_state
	rm -f test_device_event_queue
	rm -f test_clock_source
	rm -f test_net_config
	rm -f test_net_ring
	rm -f test_reconnect_backoff
	rm -f test_ota_policy
	rm -f test_asset_store
	rm -f test_asset_transfer
	rm -f test_rle565
	rm -f test_volatile_asset_store
	rm -f test_scene_model
	rm -f test_scene_binding
	rm -f test_scene_decode
	rm -f test_font_registry
	rm -rf test_timefmt.dSYM test_protocol.dSYM test_link_state.dSYM \
		test_widget_model.dSYM
	rm -rf test_ui_command_queue.dSYM
	rm -rf test_navigation.dSYM
	rm -rf test_interrupt_state.dSYM
	rm -rf test_device_event_queue.dSYM
	rm -rf test_clock_source.dSYM
	rm -rf test_net_config.dSYM
	rm -rf test_net_ring.dSYM
	rm -rf test_reconnect_backoff.dSYM
	rm -rf test_ota_policy.dSYM
	rm -rf test_asset_store.dSYM
	rm -rf test_asset_transfer.dSYM
	rm -rf test_rle565.dSYM
	rm -rf test_volatile_asset_store.dSYM
	rm -rf test_scene_model.dSYM
	rm -rf test_scene_binding.dSYM
	rm -rf test_scene_decode.dSYM
	rm -rf test_font_registry.dSYM
.PHONY: test sanitize clean
