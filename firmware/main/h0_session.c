#include "h0_session.h"

#include <string.h>

static void write_u16(uint8_t *bytes, uint16_t value) {
    bytes[0] = (uint8_t)value;
    bytes[1] = (uint8_t)(value >> 8u);
}

static void write_u32(uint8_t *bytes, uint32_t value) {
    bytes[0] = (uint8_t)value;
    bytes[1] = (uint8_t)(value >> 8u);
    bytes[2] = (uint8_t)(value >> 16u);
    bytes[3] = (uint8_t)(value >> 24u);
}

static bool encode_next(h0_session_t *session, uint8_t message_type,
                        const uint8_t *payload, uint8_t payload_length,
                        uint32_t uptime_ms, uint8_t out_frame[ABP_FRAME_SIZE]) {
    if (session->next_device_sequence == 0 || out_frame == NULL) return false;
    abp_frame_t frame = {
        .message_type = message_type,
        .sequence = session->next_device_sequence,
        .sender_uptime_ms = uptime_ms,
        .payload_length = payload_length,
    };
    if (payload_length > 0) memcpy(frame.payload, payload, payload_length);
    if (abp_encode(&frame, out_frame) != ABP_OK) return false;
    session->next_device_sequence++;
    return session->next_device_sequence != 0;
}

void h0_session_init(h0_session_t *session, uint32_t boot_id, const uint8_t device_id[16]) {
    memset(session, 0, sizeof(*session));
    session->boot_id = boot_id;
    memcpy(session->device_id, device_id, sizeof(session->device_id));
}

void h0_session_mount(h0_session_t *session) {
    session->mounted = true;
    session->ready = false;
    session->next_device_sequence = 1;
    session->next_host_sequence = 1;
}

void h0_session_unmount(h0_session_t *session) {
    session->mounted = false;
    session->ready = false;
    session->ag00_pressed = false;
}

bool h0_session_make_hello(h0_session_t *session, uint32_t uptime_ms,
                           uint8_t out_frame[ABP_FRAME_SIZE]) {
    if (!session->mounted || session->next_device_sequence != 1) return false;
    uint8_t payload[24] = {0};
    write_u32(payload, session->boot_id);
    memcpy(payload + 4, session->device_id, sizeof(session->device_id));
    write_u16(payload + 20, H0_HARDWARE_REVISION);
    payload[22] = H0_FIRMWARE_MAJOR;
    payload[23] = H0_FIRMWARE_MINOR;
    return encode_next(session, ABP_DEVICE_HELLO, payload, sizeof(payload), uptime_ms, out_frame);
}

h0_host_result_t h0_session_accept_host(h0_session_t *session,
                                        const uint8_t frame_bytes[ABP_FRAME_SIZE],
                                        uint32_t uptime_ms,
                                        uint8_t out_response[ABP_FRAME_SIZE],
                                        uint8_t out_visual_payload[20]) {
    if (!session->mounted || frame_bytes == NULL) return H0_HOST_REJECTED;
    abp_frame_t frame;
    abp_direction_t direction;
    if (abp_decode(frame_bytes, ABP_FRAME_SIZE, &frame) != ABP_OK ||
        abp_message_direction(frame.message_type, &direction) != ABP_OK ||
        direction != ABP_DIRECTION_HOST_TO_DEVICE ||
        frame.sequence != session->next_host_sequence) {
        return H0_HOST_REJECTED;
    }

    if (frame.message_type == ABP_GET_INPUT_SNAPSHOT && out_response != NULL) {
        uint8_t payload[8] = {0};
        write_u32(payload, session->boot_id);
        payload[4] = session->ag00_pressed ? 0x01u : 0u;
        if (!encode_next(session, ABP_INPUT_SNAPSHOT, payload, sizeof(payload), uptime_ms,
                         out_response)) {
            return H0_HOST_REJECTED;
        }
        session->next_host_sequence++;
        session->ready = true;
        return H0_HOST_SNAPSHOT_REQUESTED;
    }

    if (frame.message_type == ABP_VISUAL_STATE && session->ready && out_visual_payload != NULL) {
        memcpy(out_visual_payload, frame.payload, 20);
        session->next_host_sequence++;
        return H0_HOST_VISUAL_STATE;
    }
    return H0_HOST_REJECTED;
}

bool h0_session_make_ag00(h0_session_t *session, bool pressed, uint32_t uptime_ms,
                          uint8_t out_frame[ABP_FRAME_SIZE]) {
    if (!session->ready || session->ag00_pressed == pressed) return false;
    uint8_t payload[9] = {0};
    write_u32(payload, session->boot_id);
    payload[4] = 0;  // Agent control.
    payload[5] = 0;  // AG00.
    payload[6] = pressed ? 0 : 1;
    if (!encode_next(session, ABP_INPUT_EVENT, payload, sizeof(payload), uptime_ms, out_frame)) {
        return false;
    }
    session->ag00_pressed = pressed;
    return true;
}
