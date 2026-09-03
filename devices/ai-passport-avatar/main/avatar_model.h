#pragma once

#include <stdbool.h>
#include <stdint.h>

#include "avatar_display_packet.h"

typedef enum {
    AVATAR_AGENT_EMPTY = 0,
    AVATAR_AGENT_IDLE = 1,
    AVATAR_AGENT_THINKING = 2,
    AVATAR_AGENT_COMPLETE = 3,
    AVATAR_AGENT_INPUT = 4,
    AVATAR_AGENT_ERROR = 5,
} avatar_agent_status_t;

typedef struct {
    uint32_t revision;
    avatar_agent_status_t agents[6];
    char agent_titles[6][AVATAR_DISPLAY_TITLE_MAX_BYTES + 1u];
    uint8_t host_selected_mask;
    uint8_t selected_slot;
    uint8_t bridge_state;
    uint8_t connection_state;
    uint32_t last_visual_ms;
    bool ble_connected;
    bool session_ready;
} avatar_model_t;

void avatar_model_init(avatar_model_t *model);
bool avatar_model_apply_visual(avatar_model_t *model,
                               const uint8_t payload[20],
                               uint32_t now_ms);
bool avatar_model_apply_title(avatar_model_t *model,
                              const avatar_display_title_t *title);
void avatar_model_move(avatar_model_t *model, int direction);
bool avatar_model_online(const avatar_model_t *model, uint32_t now_ms);
const char *avatar_model_status_name(avatar_agent_status_t status);
