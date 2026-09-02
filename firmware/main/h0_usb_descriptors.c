#include "h0_usb_descriptors.h"

#include "class/hid/hid_device.h"
#include "h0_report_descriptor.h"
#include "tusb.h"
#include "tinyusb_default_config.h"

#define H0_USB_VID 0xcafe
#define H0_USB_PID 0xab01
#define H0_USB_PRODUCT "Agent Bridge H0 Vendor HID Dev"
#define H0_USB_TOTAL_LENGTH (TUD_CONFIG_DESC_LEN + TUD_HID_DESC_LEN)

_Static_assert(sizeof(H0_USB_PRODUCT) - 1 <= 31,
               "USB product string exceeds esp_tinyusb descriptor limit");

static const tusb_desc_device_t H0_DEVICE_DESCRIPTOR = {
    .bLength = sizeof(tusb_desc_device_t),
    .bDescriptorType = TUSB_DESC_DEVICE,
    .bcdUSB = 0x0200,
    .bDeviceClass = 0,
    .bDeviceSubClass = 0,
    .bDeviceProtocol = 0,
    .bMaxPacketSize0 = CFG_TUD_ENDPOINT0_SIZE,
    .idVendor = H0_USB_VID,
    .idProduct = H0_USB_PID,
    .bcdDevice = 0x0002,
    .iManufacturer = 1,
    .iProduct = 2,
    .iSerialNumber = 0,
    .bNumConfigurations = 1,
};

static const char *H0_STRING_DESCRIPTORS[] = {
    (const char[]){0x09, 0x04},
    "Agent Bridge Development",
    H0_USB_PRODUCT,
    "Agent Bridge ABP v1",
};

static const uint8_t H0_CONFIGURATION_DESCRIPTOR[] = {
    TUD_CONFIG_DESCRIPTOR(1, 1, 0, H0_USB_TOTAL_LENGTH, 0, 500),
    TUD_HID_DESCRIPTOR(0, 3, false, sizeof(H0_REPORT_DESCRIPTOR), 0x81, 64, 1),
};

tinyusb_config_t h0_tinyusb_config(void) {
    tinyusb_config_t config = TINYUSB_DEFAULT_CONFIG();
    config.descriptor.device = &H0_DEVICE_DESCRIPTOR;
    config.descriptor.full_speed_config = H0_CONFIGURATION_DESCRIPTOR;
    config.descriptor.string = H0_STRING_DESCRIPTORS;
    config.descriptor.string_count =
        sizeof(H0_STRING_DESCRIPTORS) / sizeof(H0_STRING_DESCRIPTORS[0]);
    return config;
}

uint8_t const *tud_hid_descriptor_report_cb(uint8_t instance) {
    (void)instance;
    return H0_REPORT_DESCRIPTOR;
}

uint16_t tud_hid_get_report_cb(uint8_t instance, uint8_t report_id,
                               hid_report_type_t report_type, uint8_t *buffer,
                               uint16_t requested_length) {
    (void)instance;
    (void)report_id;
    (void)report_type;
    (void)buffer;
    (void)requested_length;
    return 0;
}
