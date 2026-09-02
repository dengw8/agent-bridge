#include "h0_visual.h"

#include <stdbool.h>
#include <string.h>

static h0_rgb_t agent_color(uint8_t state, bool selected) {
    switch (state) {
        case 1: return selected ? (h0_rgb_t){0, 160, 255} : (h0_rgb_t){0, 32, 48};
        case 2: return (h0_rgb_t){128, 0, 255};
        case 3: return (h0_rgb_t){0, 220, 40};
        case 4: return (h0_rgb_t){255, 90, 0};
        case 5: return (h0_rgb_t){255, 0, 0};
        default: return (h0_rgb_t){0, 0, 0};
    }
}

static h0_rgb_t scale(h0_rgb_t color, uint8_t percent) {
    return (h0_rgb_t){
        .red = (uint8_t)(((uint16_t)color.red * percent) / 100u),
        .green = (uint8_t)(((uint16_t)color.green * percent) / 100u),
        .blue = (uint8_t)(((uint16_t)color.blue * percent) / 100u),
    };
}

void h0_visual_connection(h0_rgb_t pixels[H0_LED_COUNT]) {
    memset(pixels, 0, sizeof(h0_rgb_t) * H0_LED_COUNT);
    pixels[7] = (h0_rgb_t){0, 0, 16};
}

void h0_visual_render(const uint8_t payload[20], h0_rgb_t pixels[H0_LED_COUNT]) {
    uint8_t brightness = payload[17] < H0_MAX_BRIGHTNESS_PERCENT
                             ? payload[17]
                             : H0_MAX_BRIGHTNESS_PERCENT;
    bool error = payload[12] == 3;
    for (uint8_t index = 0; index < 6; ++index) {
        error = error || payload[4 + index] == 5;
        pixels[index] = scale(agent_color(payload[4 + index],
                                          (payload[10] & (1u << index)) != 0),
                              brightness);
    }
    pixels[6] = (h0_rgb_t){0, 0, 0};
    pixels[7] = scale(error ? (h0_rgb_t){255, 0, 0}
                            : payload[13] == 1 ? (h0_rgb_t){0, 180, 50}
                                               : (h0_rgb_t){0, 0, 160},
                      brightness);
}
