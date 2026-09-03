#include <string.h>

#include "avatar_ble.h"
#include "avatar_model.h"
#include "avatar_session.h"
#include "avatar_ui.h"
#include "esp_mac.h"
#include "esp_random.h"
#include "esp_timer.h"
#include "nvs_flash.h"
#include "passport_bsp.h"

static avatar_model_t s_model;
static avatar_session_t s_session;
static SemaphoreHandle_t s_state_lock;

static uint32_t uptime_ms(void)
{
    return (uint32_t)(esp_timer_get_time() / 1000);
}

static void on_button(passport_button_t button,
                      passport_button_event_t event,
                      void *context)
{
    (void)context;
    if (xSemaphoreTake(s_state_lock, pdMS_TO_TICKS(20)) != pdTRUE) return;
    uint8_t frame[ABP_FRAME_SIZE];
    bool send = false;
    if (event == PASSPORT_BUTTON_CLICKED && button != PASSPORT_BUTTON_OK) {
        avatar_model_move(&s_model, button == PASSPORT_BUTTON_DOWN ? 1 : -1);
        avatar_session_select(&s_session, s_model.selected_slot);
    } else if (button == PASSPORT_BUTTON_OK && event == PASSPORT_BUTTON_PRESSED) {
        send = avatar_session_make_selected(&s_session, true, uptime_ms(), frame);
    } else if (button == PASSPORT_BUTTON_OK && event == PASSPORT_BUTTON_RELEASED) {
        send = avatar_session_make_selected(&s_session, false, uptime_ms(), frame);
    }
    xSemaphoreGive(s_state_lock);
    if (send) avatar_ble_notify_frame(frame);
}

void app_main(void)
{
    ESP_ERROR_CHECK(nvs_flash_init());
    ESP_ERROR_CHECK(passport_display_init());
    ESP_ERROR_CHECK(passport_lvgl_init() == NULL ? ESP_FAIL : ESP_OK);

    avatar_model_init(&s_model);
    uint8_t device_id[16] = {'A', 'G', 'E', 'N', 'T', 'P', 'A', 'S', 'S', 'P'};
    ESP_ERROR_CHECK(esp_read_mac(device_id + 10, ESP_MAC_BT));
    uint32_t boot_id = esp_random();
    if (boot_id == 0) boot_id = 1;
    avatar_session_init(&s_session, boot_id, device_id);
    s_state_lock = xSemaphoreCreateMutex();
    ESP_ERROR_CHECK(s_state_lock == NULL ? ESP_ERR_NO_MEM : ESP_OK);

    if (passport_lvgl_lock(-1)) {
        avatar_ui_init(&s_model, s_state_lock);
        passport_lvgl_unlock();
    }
    ESP_ERROR_CHECK(passport_buttons_init(on_button, NULL));
    avatar_ble_init(&s_model, &s_session, s_state_lock);
}
