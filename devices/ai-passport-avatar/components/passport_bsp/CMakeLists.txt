idf_component_register(
    SRCS "src/passport_display.c" "src/passport_buttons.c"
    INCLUDE_DIRS "include"
    REQUIRES driver esp_lcd esp_lvgl_port button esp_adc
)
