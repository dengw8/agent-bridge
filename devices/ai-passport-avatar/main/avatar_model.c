#include "avatar_model.h"

#include <string.h>

static uint32_t read_u32(const uint8_t *bytes)
{
    return (uint32_t)bytes[0] | ((uint32_t)bytes[1] << 8u) |
           ((uint32_t)bytes[2] << 16u) | ((uint32_t)bytes[3] << 24u);
}

void avatar_model_init(avatar_model_t *model)
{
    memset(model, 0, sizeof(*model));
}

bool avatar_model_apply_visual(avatar_model_t *model,
                               const uint8_t payload[20],
                               uint32_t now_ms)
{
    if (model == NULL || payload == NULL) return false;
    for (size_t index = 0; index < 6; ++index) {
        if (payload[4 + index] > AVATAR_AGENT_ERROR) return false;
    }
    const uint32_t revision = read_u32(payload);
    if (model->last_visual_ms != 0 && revision == model->revision) return false;
    model->revision = revision;
    for (size_t index = 0; index < 6; ++index) {
        model->agents[index] = (avatar_agent_status_t)payload[4 + index];
    }
    model->host_selected_mask = payload[10];
    model->bridge_state = payload[12];
    model->connection_state = payload[13];
    model->last_visual_ms = now_ms;
    return true;
}

bool avatar_model_apply_title(avatar_model_t *model,
                              const avatar_display_title_t *title)
{
    if (model == NULL || title == NULL || title->slot >= 6u ||
        title->length > AVATAR_DISPLAY_TITLE_MAX_BYTES) {
        return false;
    }
    memcpy(model->agent_titles[title->slot], title->title, title->length + 1u);
    return true;
}

void avatar_model_move(avatar_model_t *model, int direction)
{
    if (model == NULL || direction == 0) return;
    const int step = direction > 0 ? 1 : -1;
    uint8_t candidate = model->selected_slot;
    for (size_t attempts = 0; attempts < 6; ++attempts) {
        candidate = (uint8_t)((candidate + step + 6) % 6);
        if (model->agents[candidate] != AVATAR_AGENT_EMPTY) {
            model->selected_slot = candidate;
            return;
        }
    }
    model->selected_slot = (uint8_t)((model->selected_slot + step + 6) % 6);
}

bool avatar_model_online(const avatar_model_t *model, uint32_t now_ms)
{
    return model != NULL && model->ble_connected && model->session_ready &&
           model->last_visual_ms != 0 &&
           (uint32_t)(now_ms - model->last_visual_ms) <= 5000u;
}

const char *avatar_model_status_name(avatar_agent_status_t status)
{
    switch (status) {
        case AVATAR_AGENT_IDLE: return "READY";
        case AVATAR_AGENT_THINKING: return "THINKING";
        case AVATAR_AGENT_COMPLETE: return "DONE";
        case AVATAR_AGENT_INPUT: return "NEEDS YOU";
        case AVATAR_AGENT_ERROR: return "ERROR";
        case AVATAR_AGENT_EMPTY:
        default: return "EMPTY";
    }
}
