idf_component_register(
    SRCS
        "main.c"
        "h0_session.c"
        "h0_visual.c"
        "h0_usb_descriptors.c"
    INCLUDE_DIRS "." "../generated"
    REQUIRES abp
    PRIV_REQUIRES esp_driver_gpio esp_timer mbedtls
)
