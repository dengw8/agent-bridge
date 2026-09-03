#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "abp.h"

#define AVATAR_BLE_VALUE_MAX 20u
#define AVATAR_BLE_FRAGMENT_HEADER 5u
#define AVATAR_BLE_FRAGMENT_DATA (AVATAR_BLE_VALUE_MAX - AVATAR_BLE_FRAGMENT_HEADER)
#define AVATAR_BLE_REASSEMBLY_TIMEOUT_MS 100u

typedef struct {
    uint8_t transfer_id;
    uint8_t fragment_count;
    uint8_t next_index;
    uint8_t chunk_capacity;
    uint8_t length;
    uint32_t started_ms;
    bool active;
    uint8_t bytes[ABP_FRAME_SIZE];
} avatar_ble_reassembler_t;

typedef bool (*avatar_fragment_sink_t)(const uint8_t *value, size_t length, void *context);

void avatar_ble_reassembler_reset(avatar_ble_reassembler_t *state);
bool avatar_ble_reassembler_push(avatar_ble_reassembler_t *state,
                                 const uint8_t *value,
                                 size_t length,
                                 uint32_t now_ms,
                                 uint8_t output[ABP_FRAME_SIZE]);
bool avatar_ble_fragment_frame(const uint8_t frame[ABP_FRAME_SIZE],
                               uint8_t transfer_id,
                               avatar_fragment_sink_t sink,
                               void *context);
