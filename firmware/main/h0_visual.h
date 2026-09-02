#ifndef AGENT_BRIDGE_H0_VISUAL_H
#define AGENT_BRIDGE_H0_VISUAL_H

#include <stdint.h>

#define H0_LED_COUNT 8u
#define H0_MAX_BRIGHTNESS_PERCENT 25u

typedef struct {
    uint8_t red;
    uint8_t green;
    uint8_t blue;
} h0_rgb_t;

void h0_visual_connection(h0_rgb_t pixels[H0_LED_COUNT]);
void h0_visual_render(const uint8_t payload[20], h0_rgb_t pixels[H0_LED_COUNT]);

#endif
