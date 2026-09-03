#pragma once

#include <stdbool.h>
#include <stdint.h>

#include "esp_err.h"

struct _lv_display_t;

typedef enum {
    PASSPORT_BUTTON_UP = 0,
    PASSPORT_BUTTON_DOWN = 1,
    PASSPORT_BUTTON_OK = 2,
} passport_button_t;

typedef enum {
    PASSPORT_BUTTON_PRESSED = 0,
    PASSPORT_BUTTON_RELEASED = 1,
    PASSPORT_BUTTON_CLICKED = 2,
} passport_button_event_t;

typedef void (*passport_button_callback_t)(passport_button_t button,
                                           passport_button_event_t event,
                                           void *context);

esp_err_t passport_display_init(void);
struct _lv_display_t *passport_lvgl_init(void);
bool passport_lvgl_lock(int timeout_ms);
void passport_lvgl_unlock(void);
void passport_backlight_set(uint8_t percent);

esp_err_t passport_buttons_init(passport_button_callback_t callback, void *context);
