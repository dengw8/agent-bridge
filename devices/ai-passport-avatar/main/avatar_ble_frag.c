#include "avatar_ble_frag.h"

#include <string.h>

void avatar_ble_reassembler_reset(avatar_ble_reassembler_t *state)
{
    memset(state, 0, sizeof(*state));
}

bool avatar_ble_reassembler_push(avatar_ble_reassembler_t *state,
                                 const uint8_t *value,
                                 size_t length,
                                 uint32_t now_ms,
                                 uint8_t output[ABP_FRAME_SIZE])
{
    if (state == NULL || value == NULL || output == NULL ||
        length < AVATAR_BLE_FRAGMENT_HEADER) {
        if (state != NULL) avatar_ble_reassembler_reset(state);
        return false;
    }
    const uint8_t transfer_id = value[0];
    const uint8_t index = value[1];
    const uint8_t count = value[2];
    const uint8_t frame_length = value[3];
    const uint8_t fragment_length = value[4];
    if (count == 0 || count > 16 || frame_length != ABP_FRAME_SIZE ||
        fragment_length == 0 || fragment_length > AVATAR_BLE_FRAGMENT_DATA ||
        length != AVATAR_BLE_FRAGMENT_HEADER + fragment_length) {
        avatar_ble_reassembler_reset(state);
        return false;
    }
    if (index == 0) {
        avatar_ble_reassembler_reset(state);
        state->transfer_id = transfer_id;
        state->fragment_count = count;
        state->chunk_capacity = fragment_length;
        state->started_ms = now_ms;
        state->active = true;
    }
    const bool timed_out = state->active &&
                           (uint32_t)(now_ms - state->started_ms) >
                               AVATAR_BLE_REASSEMBLY_TIMEOUT_MS;
    if (timed_out || !state->active || transfer_id != state->transfer_id ||
        count != state->fragment_count || index != state->next_index ||
        (index + 1 < count && fragment_length != state->chunk_capacity)) {
        avatar_ble_reassembler_reset(state);
        return false;
    }
    const size_t offset = (size_t)index * state->chunk_capacity;
    const bool final = index + 1 == count;
    const size_t final_length = ABP_FRAME_SIZE - offset;
    if (offset != state->length || offset + fragment_length > ABP_FRAME_SIZE ||
        (final && fragment_length != final_length)) {
        avatar_ble_reassembler_reset(state);
        return false;
    }
    memcpy(state->bytes + state->length, value + AVATAR_BLE_FRAGMENT_HEADER,
           fragment_length);
    state->length += fragment_length;
    state->next_index++;
    if (!final) return false;
    const bool complete = state->length == ABP_FRAME_SIZE;
    if (complete) memcpy(output, state->bytes, ABP_FRAME_SIZE);
    avatar_ble_reassembler_reset(state);
    return complete;
}

bool avatar_ble_fragment_frame(const uint8_t frame[ABP_FRAME_SIZE],
                               uint8_t transfer_id,
                               avatar_fragment_sink_t sink,
                               void *context)
{
    if (frame == NULL || sink == NULL) return false;
    const uint8_t count =
        (ABP_FRAME_SIZE + AVATAR_BLE_FRAGMENT_DATA - 1) / AVATAR_BLE_FRAGMENT_DATA;
    for (uint8_t index = 0; index < count; ++index) {
        const size_t offset = (size_t)index * AVATAR_BLE_FRAGMENT_DATA;
        const uint8_t data_length =
            (uint8_t)((ABP_FRAME_SIZE - offset) < AVATAR_BLE_FRAGMENT_DATA
                          ? ABP_FRAME_SIZE - offset
                          : AVATAR_BLE_FRAGMENT_DATA);
        uint8_t value[AVATAR_BLE_VALUE_MAX] = {
            transfer_id, index, count, ABP_FRAME_SIZE, data_length,
        };
        memcpy(value + AVATAR_BLE_FRAGMENT_HEADER, frame + offset, data_length);
        if (!sink(value, AVATAR_BLE_FRAGMENT_HEADER + data_length, context)) return false;
    }
    return true;
}
