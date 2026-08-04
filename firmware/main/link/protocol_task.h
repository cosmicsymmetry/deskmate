#pragma once

#include "esp_err.h"

/** Start the single protocol parser/dispatcher task on the application CPU. */
esp_err_t protocol_task_start(void);
