#pragma once

#include <stdbool.h>

#include "esp_err.h"

/** Start the single protocol parser/dispatcher task on the application CPU. */
esp_err_t protocol_task_start(void);

/** True once the long-lived protocol task has been created successfully. */
bool protocol_task_is_running(void);

/** Thread-safe OTA deferral snapshot maintained by the protocol owner task. */
bool protocol_task_ota_blocked(void);
