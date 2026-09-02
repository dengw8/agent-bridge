#ifndef AGENT_BRIDGE_H0_SESSION_H
#define AGENT_BRIDGE_H0_SESSION_H

#include <stdbool.h>
#include <stdint.h>

#include "abp.h"

#define H0_INPUT_REPORT_ID 0x01u
#define H0_OUTPUT_REPORT_ID 0x02u
#define H0_HARDWARE_REVISION 1u
#define H0_FIRMWARE_MAJOR 0u
#define H0_FIRMWARE_MINOR 2u

typedef enum {
    H0_HOST_REJECTED = 0,
    H0_HOST_SNAPSHOT_REQUESTED,
    H0_HOST_VISUAL_STATE,
} h0_host_result_t;

typedef struct {
    uint32_t boot_id;
    uint8_t device_id[16];
    uint32_t next_device_sequence;
    uint32_t next_host_sequence;
    bool mounted;
    bool ready;
    bool ag00_pressed;
} h0_session_t;

void h0_session_init(h0_session_t *session, uint32_t boot_id, const uint8_t device_id[16]);
void h0_session_mount(h0_session_t *session);
void h0_session_unmount(h0_session_t *session);
bool h0_session_make_hello(h0_session_t *session, uint32_t uptime_ms,
                           uint8_t out_frame[ABP_FRAME_SIZE]);
h0_host_result_t h0_session_accept_host(h0_session_t *session,
                                        const uint8_t frame[ABP_FRAME_SIZE],
                                        uint32_t uptime_ms,
                                        uint8_t out_response[ABP_FRAME_SIZE],
                                        uint8_t out_visual_payload[20]);
bool h0_session_make_ag00(h0_session_t *session, bool pressed, uint32_t uptime_ms,
                          uint8_t out_frame[ABP_FRAME_SIZE]);

#endif
