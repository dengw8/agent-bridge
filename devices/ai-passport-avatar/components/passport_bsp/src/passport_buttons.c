#include "passport_bsp.h"
#include "passport_pins.h"

#include <stdint.h>

#include "button_adc.h"
#include "esp_adc/adc_oneshot.h"
#include "esp_check.h"
#include "iot_button.h"

static const uint16_t s_ranges[PASSPORT_BUTTON_COUNT][2] = PASSPORT_BUTTON_MV_RANGES;
static button_handle_t s_buttons[PASSPORT_BUTTON_COUNT];
static adc_oneshot_unit_handle_t s_adc;
static passport_button_callback_t s_callback;
static void *s_context;

static void dispatch(void *argument, void *user_data, passport_button_event_t event)
{
    (void)argument;
    if (s_callback != NULL) {
        s_callback((passport_button_t)(intptr_t)user_data, event, s_context);
    }
}

static void pressed(void *argument, void *user_data)
{
    dispatch(argument, user_data, PASSPORT_BUTTON_PRESSED);
}

static void released(void *argument, void *user_data)
{
    dispatch(argument, user_data, PASSPORT_BUTTON_RELEASED);
}

static void clicked(void *argument, void *user_data)
{
    dispatch(argument, user_data, PASSPORT_BUTTON_CLICKED);
}

esp_err_t passport_buttons_init(passport_button_callback_t callback, void *context)
{
    s_callback = callback;
    s_context = context;
    const adc_oneshot_unit_init_cfg_t unit = {.unit_id = PASSPORT_BUTTON_ADC_UNIT};
    ESP_RETURN_ON_ERROR(adc_oneshot_new_unit(&unit, &s_adc), "passport_buttons", "ADC init");

    for (int index = 0; index < PASSPORT_BUTTON_COUNT; ++index) {
        const button_adc_config_t adc = {
            .adc_handle = &s_adc,
            .unit_id = PASSPORT_BUTTON_ADC_UNIT,
            .adc_channel = PASSPORT_BUTTON_ADC_CHANNEL,
            .button_index = index,
            .min = s_ranges[index][0],
            .max = s_ranges[index][1],
        };
        const button_config_t config = {0};
        ESP_RETURN_ON_ERROR(iot_button_new_adc_device(&config, &adc, &s_buttons[index]),
                            "passport_buttons", "button init");
        void *button_index = (void *)(intptr_t)index;
        iot_button_register_cb(s_buttons[index], BUTTON_PRESS_DOWN, NULL, pressed, button_index);
        iot_button_register_cb(s_buttons[index], BUTTON_PRESS_UP, NULL, released, button_index);
        iot_button_register_cb(s_buttons[index], BUTTON_SINGLE_CLICK, NULL, clicked, button_index);
    }
    return ESP_OK;
}
