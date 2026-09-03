#include "passport_bsp.h"
#include "passport_pins.h"

#include "driver/ledc.h"
#include "driver/spi_master.h"
#include "esp_check.h"
#include "esp_lcd_panel_io.h"
#include "esp_lcd_panel_ops.h"
#include "esp_lcd_panel_vendor.h"
#include "esp_lvgl_port.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"

static esp_lcd_panel_handle_t s_panel;
static esp_lcd_panel_io_handle_t s_io;
static lv_display_t *s_display;

typedef struct {
    uint8_t command;
    uint8_t data[16];
    uint8_t length;
    uint16_t delay_ms;
} panel_command_t;

static const panel_command_t s_panel_commands[] = {
    {0xB2, {0x05, 0x05, 0x00, 0x33, 0x33}, 5, 0},
    {0xB7, {0x35}, 1, 0},
    {0xBB, {0x21}, 1, 0},
    {0xC0, {0x2C}, 1, 0},
    {0xC2, {0x01}, 1, 0},
    {0xC3, {0x0B}, 1, 0},
    {0xC4, {0x20}, 1, 0},
    {0xC6, {0x0F}, 1, 0},
    {0xD0, {0xA7, 0xA1}, 2, 0},
    {0xD0, {0xA4, 0xA1}, 2, 0},
    {0xD6, {0xA1}, 1, 0},
    {0xE0, {0xD0, 0x04, 0x08, 0x0A, 0x09, 0x05, 0x2D, 0x43,
            0x49, 0x09, 0x16, 0x15, 0x26, 0x2B}, 14, 0},
    {0xE1, {0xD0, 0x03, 0x09, 0x0A, 0x0A, 0x06, 0x2E, 0x44,
            0x40, 0x3A, 0x15, 0x15, 0x26, 0x2A}, 14, 10},
};

void passport_backlight_set(uint8_t percent)
{
    if (percent > 100) {
        percent = 100;
    }
    const uint32_t maximum = (1u << LEDC_TIMER_10_BIT) - 1u;
    ledc_set_duty(LEDC_LOW_SPEED_MODE, LEDC_CHANNEL_0, maximum * percent / 100u);
    ledc_update_duty(LEDC_LOW_SPEED_MODE, LEDC_CHANNEL_0);
}

esp_err_t passport_display_init(void)
{
    const spi_bus_config_t bus = {
        .mosi_io_num = PASSPORT_LCD_MOSI,
        .sclk_io_num = PASSPORT_LCD_CLOCK,
        .miso_io_num = -1,
        .quadwp_io_num = -1,
        .quadhd_io_num = -1,
        .max_transfer_sz = PASSPORT_LCD_WIDTH * 40 * 2,
    };
    ESP_RETURN_ON_ERROR(spi_bus_initialize(PASSPORT_LCD_HOST, &bus, SPI_DMA_CH_AUTO),
                        "passport_display", "SPI init");
    const esp_lcd_panel_io_spi_config_t io = {
        .cs_gpio_num = PASSPORT_LCD_CS,
        .dc_gpio_num = PASSPORT_LCD_DC,
        .pclk_hz = PASSPORT_LCD_CLOCK_HZ,
        .spi_mode = 0,
        .lcd_cmd_bits = 8,
        .lcd_param_bits = 8,
        .trans_queue_depth = 10,
    };
    ESP_RETURN_ON_ERROR(
        esp_lcd_new_panel_io_spi((esp_lcd_spi_bus_handle_t)PASSPORT_LCD_HOST, &io, &s_io),
        "passport_display", "panel IO");
    const esp_lcd_panel_dev_config_t panel = {
        .reset_gpio_num = PASSPORT_LCD_RESET,
        .rgb_ele_order = LCD_RGB_ELEMENT_ORDER_RGB,
        .bits_per_pixel = 16,
    };
    ESP_RETURN_ON_ERROR(esp_lcd_new_panel_st7789(s_io, &panel, &s_panel),
                        "passport_display", "panel init");
    ESP_RETURN_ON_ERROR(esp_lcd_panel_reset(s_panel), "passport_display", "panel reset");
    ESP_RETURN_ON_ERROR(esp_lcd_panel_init(s_panel), "passport_display", "panel init");
    for (size_t index = 0; index < sizeof(s_panel_commands) / sizeof(s_panel_commands[0]); ++index) {
        const panel_command_t *command = &s_panel_commands[index];
        ESP_RETURN_ON_ERROR(esp_lcd_panel_io_tx_param(
                                s_io, command->command, command->data, command->length),
                            "passport_display", "panel command");
        if (command->delay_ms != 0) {
            vTaskDelay(pdMS_TO_TICKS(command->delay_ms));
        }
    }
    ESP_RETURN_ON_ERROR(esp_lcd_panel_invert_color(s_panel, true),
                        "passport_display", "panel invert");
    ESP_RETURN_ON_ERROR(esp_lcd_panel_disp_on_off(s_panel, true),
                        "passport_display", "panel on");

    const ledc_timer_config_t timer = {
        .speed_mode = LEDC_LOW_SPEED_MODE,
        .timer_num = LEDC_TIMER_0,
        .duty_resolution = LEDC_TIMER_10_BIT,
        .freq_hz = 5000,
        .clk_cfg = LEDC_AUTO_CLK,
    };
    ESP_RETURN_ON_ERROR(ledc_timer_config(&timer), "passport_display", "backlight timer");
    const ledc_channel_config_t channel = {
        .gpio_num = PASSPORT_LCD_BACKLIGHT,
        .speed_mode = LEDC_LOW_SPEED_MODE,
        .channel = LEDC_CHANNEL_0,
        .timer_sel = LEDC_TIMER_0,
        .duty = 0,
        .hpoint = 0,
    };
    ESP_RETURN_ON_ERROR(ledc_channel_config(&channel), "passport_display", "backlight");
    passport_backlight_set(65);
    return ESP_OK;
}

lv_display_t *passport_lvgl_init(void)
{
    const lvgl_port_cfg_t port = ESP_LVGL_PORT_INIT_CONFIG();
    if (lvgl_port_init(&port) != ESP_OK) {
        return NULL;
    }
    const lvgl_port_display_cfg_t display = {
        .panel_handle = s_panel,
        .io_handle = s_io,
        .buffer_size = PASSPORT_LCD_WIDTH * 20,
        .double_buffer = false,
        .hres = PASSPORT_LCD_WIDTH,
        .vres = PASSPORT_LCD_HEIGHT,
        .rotation = {.swap_xy = false, .mirror_x = false, .mirror_y = false},
        .flags = {.buff_dma = true, .swap_bytes = true},
    };
    s_display = lvgl_port_add_disp(&display);
    return s_display;
}

bool passport_lvgl_lock(int timeout_ms)
{
    return lvgl_port_lock(timeout_ms);
}

void passport_lvgl_unlock(void)
{
    lvgl_port_unlock();
}
