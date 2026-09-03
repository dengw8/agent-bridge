#include "avatar_session.h"

#include <string.h>

static void write_u16(uint8_t *bytes, uint16_t value)
{
    bytes[0] = (uint8_t)value;
    bytes[1] = (uint8_t)(value >> 8u);
}

static void write_u32(uint8_t *bytes, uint32_t value)
{
    bytes[0] = (uint8_t)value;
    bytes[1] = (uint8_t)(value >> 8u);
    bytes[2] = (uint8_t)(value >> 16u);
    bytes[3] = (uint8_t)(value >> 24u);
}

static bool encode_next(avatar_session_t *session,
                        uint8_t type,
                        const uint8_t *payload,
                        uint8_t payload_length,
                        uint32_t uptime_ms,
                        uint8_t output[ABP_FRAME_SIZE])
{
    if (session->next_device_sequence == 0 || output == NULL) return false;
    abp_frame_t frame = {
        .message_type = type,
        .sequence = session->next_device_sequence,
        .sender_uptime_ms = uptime_ms,
        .payload_length = payload_length,
    };
    if (payload_length != 0) memcpy(frame.payload, payload, payload_length);
    if (abp_encode(&frame, output) != ABP_OK) return false;
    session->next_device_sequence++;
    return session->next_device_sequence != 0;
}

void avatar_session_init(avatar_session_t *session,
                         uint32_t boot_id,
                         const uint8_t device_id[16])
{
    memset(session, 0, sizeof(*session));
    session->boot_id = boot_id;
    memcpy(session->device_id, device_id, sizeof(session->device_id));
}

void avatar_session_connect(avatar_session_t *session)
{
    session->connected = true;
    session->ready = false;
    session->ok_pressed = false;
    session->hello_pending = false;
    session->next_device_sequence = 1;
    session->next_host_sequence = 1;
}

void avatar_session_disconnect(avatar_session_t *session)
{
    session->connected = false;
    session->ready = false;
    session->ok_pressed = false;
    session->hello_pending = false;
}

void avatar_session_select(avatar_session_t *session, uint8_t slot)
{
    if (slot < 6 && !session->ok_pressed) session->selected_slot = slot;
}

bool avatar_session_schedule_hello(avatar_session_t *session)
{
    if (!session->connected || session->next_device_sequence != 1 ||
        session->hello_pending) {
        return false;
    }
    session->hello_pending = true;
    return true;
}

bool avatar_session_take_scheduled_hello(avatar_session_t *session,
                                         uint32_t uptime_ms,
                                         uint8_t output[ABP_FRAME_SIZE])
{
    if (!session->connected || session->next_device_sequence != 1 ||
        !session->hello_pending) {
        return false;
    }
    uint8_t payload[24] = {0};
    write_u32(payload, session->boot_id);
    memcpy(payload + 4, session->device_id, sizeof(session->device_id));
    write_u16(payload + 20, AVATAR_HARDWARE_REVISION);
    payload[22] = AVATAR_FIRMWARE_MAJOR;
    payload[23] = AVATAR_FIRMWARE_MINOR;
    if (!encode_next(session, ABP_DEVICE_HELLO, payload, sizeof(payload),
                     uptime_ms, output)) {
        return false;
    }
    session->hello_pending = false;
    return true;
}

avatar_host_result_t avatar_session_accept_host(avatar_session_t *session,
                                                const uint8_t input[ABP_FRAME_SIZE],
                                                uint32_t uptime_ms,
                                                uint8_t response[ABP_FRAME_SIZE],
                                                uint8_t visual_payload[20])
{
    if (!session->connected || input == NULL) return AVATAR_HOST_REJECTED;
    abp_frame_t frame;
    abp_direction_t direction;
    if (abp_decode(input, ABP_FRAME_SIZE, &frame) != ABP_OK ||
        abp_message_direction(frame.message_type, &direction) != ABP_OK ||
        direction != ABP_DIRECTION_HOST_TO_DEVICE ||
        frame.sequence != session->next_host_sequence) {
        return AVATAR_HOST_REJECTED;
    }
    if (frame.message_type == ABP_GET_INPUT_SNAPSHOT && response != NULL) {
        uint8_t payload[8] = {0};
        write_u32(payload, session->boot_id);
        payload[4] = session->ok_pressed ? (uint8_t)(1u << session->selected_slot) : 0;
        if (!encode_next(session, ABP_INPUT_SNAPSHOT, payload, sizeof(payload),
                         uptime_ms, response)) {
            return AVATAR_HOST_REJECTED;
        }
        session->next_host_sequence++;
        session->ready = true;
        return AVATAR_HOST_SNAPSHOT_REQUESTED;
    }
    if (frame.message_type == ABP_VISUAL_STATE && session->ready && visual_payload != NULL) {
        memcpy(visual_payload, frame.payload, 20);
        session->next_host_sequence++;
        return AVATAR_HOST_VISUAL_STATE;
    }
    if (frame.message_type == ABP_HOST_GOING_AWAY) {
        session->next_host_sequence++;
        session->ready = false;
        return AVATAR_HOST_DISCONNECTED;
    }
    return AVATAR_HOST_REJECTED;
}

bool avatar_session_make_selected(avatar_session_t *session,
                                  bool pressed,
                                  uint32_t uptime_ms,
                                  uint8_t output[ABP_FRAME_SIZE])
{
    if (!session->ready || session->ok_pressed == pressed) return false;
    uint8_t payload[9] = {0};
    write_u32(payload, session->boot_id);
    payload[4] = 0;
    payload[5] = session->selected_slot;
    payload[6] = pressed ? 0 : 1;
    if (!encode_next(session, ABP_INPUT_EVENT, payload, sizeof(payload), uptime_ms, output)) {
        return false;
    }
    session->ok_pressed = pressed;
    return true;
}
