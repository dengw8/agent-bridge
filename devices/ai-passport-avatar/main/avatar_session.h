#pragma once

#include <stdbool.h>
#include <stdint.h>

#include "abp.h"

#define AVATAR_HARDWARE_REVISION 1u
#define AVATAR_FIRMWARE_MAJOR 0u
#define AVATAR_FIRMWARE_MINOR 1u

typedef enum {
    AVATAR_HOST_REJECTED = 0,
    AVATAR_HOST_SNAPSHOT_REQUESTED,
    AVATAR_HOST_VISUAL_STATE,
    AVATAR_HOST_DISCONNECTED,
} avatar_host_result_t;

typedef struct {
    uint32_t boot_id;
    uint8_t device_id[16];
    uint32_t next_device_sequence;
    uint32_t next_host_sequence;
    uint8_t selected_slot;
    bool connected;
    bool ready;
    bool ok_pressed;
    bool hello_pending;
} avatar_session_t;

void avatar_session_init(avatar_session_t *session,
                         uint32_t boot_id,
                         const uint8_t device_id[16]);
void avatar_session_connect(avatar_session_t *session);
void avatar_session_disconnect(avatar_session_t *session);
void avatar_session_select(avatar_session_t *session, uint8_t slot);
bool avatar_session_schedule_hello(avatar_session_t *session);
bool avatar_session_take_scheduled_hello(avatar_session_t *session,
                                         uint32_t uptime_ms,
                                         uint8_t output[ABP_FRAME_SIZE]);
avatar_host_result_t avatar_session_accept_host(avatar_session_t *session,
                                                const uint8_t input[ABP_FRAME_SIZE],
                                                uint32_t uptime_ms,
                                                uint8_t response[ABP_FRAME_SIZE],
                                                uint8_t visual_payload[20]);
bool avatar_session_make_selected(avatar_session_t *session,
                                  bool pressed,
                                  uint32_t uptime_ms,
                                  uint8_t output[ABP_FRAME_SIZE]);
