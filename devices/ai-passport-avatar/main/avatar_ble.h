#pragma once

#include "avatar_model.h"
#include "avatar_session.h"
#include "freertos/FreeRTOS.h"
#include "freertos/semphr.h"

void avatar_ble_init(avatar_model_t *model,
                     avatar_session_t *session,
                     SemaphoreHandle_t state_lock);
bool avatar_ble_notify_frame(const uint8_t frame[ABP_FRAME_SIZE]);
