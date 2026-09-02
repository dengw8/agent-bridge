#include <stdbool.h>
#include <stdint.h>
#include <string.h>

#include "class/hid/hid_device.h"
#include "driver/gpio.h"
#include "esp_log.h"
#include "esp_mac.h"
#include "esp_random.h"
#include "esp_timer.h"
#include "freertos/FreeRTOS.h"
#include "freertos/queue.h"
#include "freertos/task.h"
#include "h0_session.h"
#include "h0_usb_descriptors.h"
#include "h0_visual.h"
#include "led_strip.h"
#include "mbedtls/sha256.h"
#include "tinyusb.h"

#define H0_AG00_GPIO GPIO_NUM_0
#define H0_RGB_GPIO GPIO_NUM_18
#define H0_REPORT_QUEUE_DEPTH 8u
#define H0_DEBOUNCE_SAMPLES 3u

static const char *TAG = "agent_bridge_h0";
static QueueHandle_t host_reports;

typedef struct {
    uint8_t bytes[ABP_FRAME_SIZE];
} host_report_t;

static uint32_t uptime_ms(void) {
    return (uint32_t)(esp_timer_get_time() / 1000u);
}

static uint32_t fresh_boot_id(void) {
    uint32_t boot_id = 0;
    while (boot_id == 0) boot_id = esp_random();
    return boot_id;
}

static void stable_device_id(uint8_t out[16]) {
    static const uint8_t label[] = "agent-bridge-h0-device-id-v1";
    uint8_t mac[6];
    uint8_t digest[32];
    ESP_ERROR_CHECK(esp_efuse_mac_get_default(mac));
    mbedtls_sha256_context context;
    mbedtls_sha256_init(&context);
    ESP_ERROR_CHECK(mbedtls_sha256_starts(&context, 0));
    ESP_ERROR_CHECK(mbedtls_sha256_update(&context, label, sizeof(label) - 1));
    ESP_ERROR_CHECK(mbedtls_sha256_update(&context, mac, sizeof(mac)));
    ESP_ERROR_CHECK(mbedtls_sha256_finish(&context, digest));
    mbedtls_sha256_free(&context);
    memcpy(out, digest, 16);
    memset(mac, 0, sizeof(mac));
    memset(digest, 0, sizeof(digest));
}

static led_strip_handle_t rgb_init(void) {
    const led_strip_config_t strip = {
        .strip_gpio_num = H0_RGB_GPIO,
        .max_leds = H0_LED_COUNT,
        .led_model = LED_MODEL_WS2812,
        .color_component_format = LED_STRIP_COLOR_COMPONENT_FMT_GRB,
        .flags.invert_out = false,
    };
    const led_strip_rmt_config_t rmt = {
        .clk_src = RMT_CLK_SRC_DEFAULT,
        .resolution_hz = 10 * 1000 * 1000,
        .mem_block_symbols = 0,
        .flags.with_dma = false,
    };
    led_strip_handle_t handle = NULL;
    ESP_ERROR_CHECK(led_strip_new_rmt_device(&strip, &rmt, &handle));
    ESP_ERROR_CHECK(led_strip_clear(handle));
    return handle;
}

static void rgb_show(led_strip_handle_t strip, const h0_rgb_t pixels[H0_LED_COUNT]) {
    for (uint8_t index = 0; index < H0_LED_COUNT; ++index) {
        ESP_ERROR_CHECK(led_strip_set_pixel(strip, index, pixels[index].red,
                                            pixels[index].green, pixels[index].blue));
    }
    ESP_ERROR_CHECK(led_strip_refresh(strip));
}

static bool send_frame(const uint8_t frame[ABP_FRAME_SIZE]) {
    return tud_hid_ready() && tud_hid_report(H0_INPUT_REPORT_ID, frame, ABP_FRAME_SIZE);
}

void tud_hid_set_report_cb(uint8_t instance, uint8_t report_id,
                           hid_report_type_t report_type, const uint8_t *buffer,
                           uint16_t size) {
    (void)instance;
    if (host_reports == NULL || report_type != HID_REPORT_TYPE_OUTPUT || buffer == NULL) return;
    host_report_t report;
    if (report_id == H0_OUTPUT_REPORT_ID && size == ABP_FRAME_SIZE) {
        memcpy(report.bytes, buffer, ABP_FRAME_SIZE);
    } else if (report_id == 0 && size == ABP_FRAME_SIZE + 1 &&
               buffer[0] == H0_OUTPUT_REPORT_ID) {
        memcpy(report.bytes, buffer + 1, ABP_FRAME_SIZE);
    } else {
        return;
    }
    (void)xQueueSend(host_reports, &report, 0);
}

void app_main(void) {
    const gpio_config_t button = {
        .pin_bit_mask = BIT64(H0_AG00_GPIO),
        .mode = GPIO_MODE_INPUT,
        .pull_up_en = true,
        .pull_down_en = false,
        .intr_type = GPIO_INTR_DISABLE,
    };
    ESP_ERROR_CHECK(gpio_config(&button));
    led_strip_handle_t rgb = rgb_init();
    host_reports = xQueueCreate(H0_REPORT_QUEUE_DEPTH, sizeof(host_report_t));
    ESP_ERROR_CHECK(host_reports == NULL ? ESP_ERR_NO_MEM : ESP_OK);

    uint8_t device_id[16];
    stable_device_id(device_id);
    h0_session_t session;
    h0_session_init(&session, fresh_boot_id(), device_id);
    memset(device_id, 0, sizeof(device_id));

    ESP_LOGI(TAG, "installing development Vendor HID only (VID=cafe PID=ab01)");
    tinyusb_config_t usb_config = h0_tinyusb_config();
    ESP_ERROR_CHECK(tinyusb_driver_install(&usb_config));

    bool was_mounted = false;
    bool hello_valid = false;
    bool hello_sent = false;
    uint32_t last_hello_ms = 0;
    uint8_t hello[ABP_FRAME_SIZE];
    bool raw_pressed = false;
    uint8_t stable_samples = 0;
    h0_rgb_t pixels[H0_LED_COUNT] = {0};

    while (true) {
        const bool mounted = tud_mounted();
        if (mounted && !was_mounted) {
            h0_session_mount(&session);
            h0_visual_connection(pixels);
            rgb_show(rgb, pixels);
            hello_valid = h0_session_make_hello(&session, uptime_ms(), hello);
            hello_sent = false;
            last_hello_ms = uptime_ms() - 1000u;
            ESP_LOGI(TAG, "USB mounted; awaiting ABP snapshot handshake");
        } else if (!mounted && was_mounted) {
            h0_session_unmount(&session);
            ESP_ERROR_CHECK(led_strip_clear(rgb));
            hello_valid = false;
            hello_sent = false;
            xQueueReset(host_reports);
            ESP_LOGI(TAG, "USB unmounted; input disabled");
        }
        was_mounted = mounted;

        const uint32_t now = uptime_ms();
        if (hello_valid && !session.ready && tud_hid_ready() &&
            (!hello_sent || now - last_hello_ms >= 1000u)) {
            hello_sent = send_frame(hello);
            last_hello_ms = now;
        }

        if (mounted && hello_sent && tud_hid_ready()) {
            host_report_t host;
            if (xQueueReceive(host_reports, &host, 0) == pdTRUE) {
                uint8_t response[ABP_FRAME_SIZE];
                uint8_t visual[20];
                const h0_host_result_t result = h0_session_accept_host(
                    &session, host.bytes, uptime_ms(), response, visual);
                if (result == H0_HOST_SNAPSHOT_REQUESTED) {
                    if (send_frame(response)) ESP_LOGI(TAG, "ABP session ready");
                } else if (result == H0_HOST_VISUAL_STATE) {
                    h0_visual_render(visual, pixels);
                    rgb_show(rgb, pixels);
                } else {
                    ESP_LOGW(TAG, "rejected host report without state change");
                }
            }
        }

        const bool sampled_pressed = gpio_get_level(H0_AG00_GPIO) == 0;
        if (sampled_pressed == raw_pressed) {
            if (stable_samples < H0_DEBOUNCE_SAMPLES) stable_samples++;
        } else {
            raw_pressed = sampled_pressed;
            stable_samples = 1;
        }
        if (stable_samples == H0_DEBOUNCE_SAMPLES && session.ag00_pressed != raw_pressed) {
            if (!session.ready) {
                session.ag00_pressed = raw_pressed;
            } else if (tud_hid_ready()) {
                const bool previous = session.ag00_pressed;
                uint8_t event[ABP_FRAME_SIZE];
                if (!h0_session_make_ag00(&session, raw_pressed, uptime_ms(), event) ||
                    !send_frame(event)) {
                    session.ag00_pressed = previous;
                }
            }
        }
        vTaskDelay(pdMS_TO_TICKS(10));
    }
}
