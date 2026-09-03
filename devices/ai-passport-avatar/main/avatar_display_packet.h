#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "abp.h"

#define AVATAR_DISPLAY_PACKET_MAGIC 0xadu
#define AVATAR_DISPLAY_PACKET_VERSION 1u
#define AVATAR_DISPLAY_PACKET_AGENT_TITLE 1u
#define AVATAR_DISPLAY_TITLE_MAX_BYTES 57u

typedef struct {
    uint8_t slot;
    uint8_t length;
    char title[AVATAR_DISPLAY_TITLE_MAX_BYTES + 1u];
} avatar_display_title_t;

bool avatar_display_packet_is_candidate(const uint8_t packet[ABP_FRAME_SIZE]);
bool avatar_display_packet_decode_title(const uint8_t packet[ABP_FRAME_SIZE],
                                        avatar_display_title_t *title);
