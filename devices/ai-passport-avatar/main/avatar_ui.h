#pragma once

#include "avatar_model.h"
#include "freertos/FreeRTOS.h"
#include "freertos/semphr.h"

void avatar_ui_init(avatar_model_t *model, SemaphoreHandle_t state_lock);
