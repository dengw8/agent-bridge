idf_component_register(
    SRCS "main.c"
         "avatar_ble.c"
         "avatar_ble_frag.c"
         "avatar_display_packet.c"
         "lv_font_agent_title_16.c"
         "avatar_model.c"
         "avatar_session.c"
         "avatar_ui.c"
    INCLUDE_DIRS "."
    REQUIRES abp passport_bsp bt esp_hw_support esp_timer nvs_flash
)
