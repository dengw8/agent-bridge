#include "abp.h"

#include <stdbool.h>
#include <string.h>

static uint16_t read_u16(const uint8_t *bytes) {
    return (uint16_t)bytes[0] | ((uint16_t)bytes[1] << 8u);
}

static uint32_t read_u32(const uint8_t *bytes) {
    return (uint32_t)bytes[0] | ((uint32_t)bytes[1] << 8u) |
           ((uint32_t)bytes[2] << 16u) | ((uint32_t)bytes[3] << 24u);
}

static void write_u32(uint8_t *bytes, uint32_t value) {
    bytes[0] = (uint8_t)value;
    bytes[1] = (uint8_t)(value >> 8u);
    bytes[2] = (uint8_t)(value >> 16u);
    bytes[3] = (uint8_t)(value >> 24u);
}

static bool all_zero(const uint8_t *bytes, size_t length) {
    uint8_t combined = 0;
    for (size_t index = 0; index < length; ++index) combined |= bytes[index];
    return combined == 0;
}

static bool valid_percent(uint8_t value) {
    return value <= 100u;
}

static bool valid_percent_or_unknown(uint8_t value) {
    return valid_percent(value) || value == 255u;
}

static abp_result_t exact_length(const abp_frame_t *frame, uint8_t expected) {
    return frame->payload_length == expected ? ABP_OK : ABP_ERROR_PAYLOAD_LENGTH;
}

static abp_result_t validate_input_event(const abp_frame_t *frame) {
    const uint8_t *payload = frame->payload;
    if (exact_length(frame, 9) != ABP_OK) return ABP_ERROR_PAYLOAD_LENGTH;
    if (read_u32(payload) == 0) return ABP_ERROR_PAYLOAD;
    const uint8_t kind = payload[4];
    const uint8_t control_id = payload[5];
    const uint8_t phase = payload[6];
    int32_t value = read_u16(payload + 7);
    if (value >= 0x8000) value -= 0x10000;
    bool valid_id = false;
    bool valid_phase = false;
    switch (kind) {
        case 0:
            valid_id = control_id <= 5;
            valid_phase = phase <= 1;
            break;
        case 1:
            valid_id = control_id <= 6;
            valid_phase = phase <= 1;
            break;
        case 2:
            valid_id = control_id <= 3;
            valid_phase = phase == 2 || phase == 3;
            break;
        case 3:
            valid_id = control_id == 0;
            valid_phase = phase <= 1;
            break;
        case 4:
            valid_id = control_id == 0;
            valid_phase = phase == 4;
            break;
        default:
            return ABP_ERROR_PAYLOAD;
    }
    if (!valid_id || !valid_phase) return ABP_ERROR_PAYLOAD;
    if (phase == 4) {
        if (value == 0 || value < -32 || value > 32) return ABP_ERROR_PAYLOAD;
    } else if (value != 0) {
        return ABP_ERROR_PAYLOAD;
    }
    return ABP_OK;
}

static abp_result_t validate_visual_state(const abp_frame_t *frame) {
    const uint8_t *payload = frame->payload;
    if (exact_length(frame, 20) != ABP_OK) return ABP_ERROR_PAYLOAD_LENGTH;
    for (size_t index = 4; index < 10; ++index) {
        if (payload[index] > 5) return ABP_ERROR_PAYLOAD;
    }
    if ((payload[10] & (uint8_t)~0x3fu) != 0 || payload[11] > 6 || payload[12] > 3 ||
        payload[13] > 4 || payload[14] > 3 || !valid_percent_or_unknown(payload[15]) ||
        (payload[16] & (uint8_t)~0x07u) != 0 || !valid_percent(payload[17]) ||
        read_u16(payload + 18) > 3600) {
        return ABP_ERROR_PAYLOAD;
    }
    return ABP_OK;
}

static abp_result_t validate_payload(const abp_frame_t *frame) {
    const uint8_t *payload = frame->payload;
    switch (frame->message_type) {
        case ABP_DEVICE_HELLO:
            if (exact_length(frame, 24) != ABP_OK) return ABP_ERROR_PAYLOAD_LENGTH;
            return read_u32(payload) != 0 && !all_zero(payload + 4, 16) ? ABP_OK : ABP_ERROR_PAYLOAD;
        case ABP_AUTH_RESPONSE:
            if (exact_length(frame, 36) != ABP_OK) return ABP_ERROR_PAYLOAD_LENGTH;
            return read_u32(payload) != 0 && !all_zero(payload + 4, 32) ? ABP_OK : ABP_ERROR_PAYLOAD;
        case ABP_INPUT_EVENT:
            return validate_input_event(frame);
        case ABP_INPUT_SNAPSHOT:
            if (exact_length(frame, 8) != ABP_OK) return ABP_ERROR_PAYLOAD_LENGTH;
            if (read_u32(payload) == 0 || (payload[4] & (uint8_t)~0x3fu) != 0 ||
                (payload[5] & (uint8_t)~0x7fu) != 0 || (payload[6] & (uint8_t)~0x0fu) != 0 ||
                (payload[6] != 0 && (payload[6] & (uint8_t)(payload[6] - 1u)) != 0) || payload[7] > 1) {
                return ABP_ERROR_PAYLOAD;
            }
            return ABP_OK;
        case ABP_DEVICE_STATUS:
            if (exact_length(frame, 10) != ABP_OK) return ABP_ERROR_PAYLOAD_LENGTH;
            return read_u32(payload) != 0 && payload[4] <= 3 && valid_percent_or_unknown(payload[5]) &&
                           (payload[6] & (uint8_t)~0x07u) == 0
                       ? ABP_OK
                       : ABP_ERROR_PAYLOAD;
        case ABP_PONG:
            if (exact_length(frame, 8) != ABP_OK) return ABP_ERROR_PAYLOAD_LENGTH;
            return read_u32(payload) != 0 ? ABP_OK : ABP_ERROR_PAYLOAD;
        case ABP_ACK:
            if (exact_length(frame, 9) != ABP_OK) return ABP_ERROR_PAYLOAD_LENGTH;
            return read_u32(payload) != 0 && read_u32(payload + 4) != 0 && payload[8] <= 2
                       ? ABP_OK
                       : ABP_ERROR_PAYLOAD;
        case ABP_PROTOCOL_ERROR:
            if (exact_length(frame, 10) != ABP_OK) return ABP_ERROR_PAYLOAD_LENGTH;
            return read_u32(payload) != 0 && payload[8] >= 1 && payload[8] <= 9
                       ? ABP_OK
                       : ABP_ERROR_PAYLOAD;
        case ABP_UPDATE_STATUS:
            if (exact_length(frame, 8) != ABP_OK) return ABP_ERROR_PAYLOAD_LENGTH;
            return read_u32(payload) != 0 && payload[4] <= 5 && valid_percent(payload[5]) && payload[7] == 0
                       ? ABP_OK
                       : ABP_ERROR_PAYLOAD;
        case ABP_HOST_HELLO:
            if (exact_length(frame, 20) != ABP_OK) return ABP_ERROR_PAYLOAD_LENGTH;
            return !all_zero(payload, 16) && (read_u32(payload + 16) & ~1u) == 0
                       ? ABP_OK
                       : ABP_ERROR_PAYLOAD;
        case ABP_AUTH_CHALLENGE:
        case ABP_CLAIM_COMMIT:
        case ABP_UPDATE_COMMIT:
            if (exact_length(frame, 32) != ABP_OK) return ABP_ERROR_PAYLOAD_LENGTH;
            return !all_zero(payload, 32) ? ABP_OK : ABP_ERROR_PAYLOAD;
        case ABP_CLAIM_BEGIN:
            if (exact_length(frame, 48) != ABP_OK) return ABP_ERROR_PAYLOAD_LENGTH;
            return !all_zero(payload, 15) && (payload[15] == 2 || payload[15] == 3) &&
                           !all_zero(payload + 16, 32)
                       ? ABP_OK
                       : ABP_ERROR_PAYLOAD;
        case ABP_GET_INPUT_SNAPSHOT:
        case ABP_GET_DEVICE_STATUS:
            return exact_length(frame, 0);
        case ABP_VISUAL_STATE:
            return validate_visual_state(frame);
        case ABP_SET_DEVICE_CONFIG:
            if (exact_length(frame, 6) != ABP_OK) return ABP_ERROR_PAYLOAD_LENGTH;
            return valid_percent(payload[0]) && read_u16(payload + 1) <= 3600 && payload[3] <= 3 &&
                           payload[4] <= 1 && payload[5] == 0
                       ? ABP_OK
                       : ABP_ERROR_PAYLOAD;
        case ABP_PING:
            return exact_length(frame, 4);
        case ABP_HOST_GOING_AWAY:
            if (exact_length(frame, 1) != ABP_OK) return ABP_ERROR_PAYLOAD_LENGTH;
            return payload[0] <= 3 ? ABP_OK : ABP_ERROR_PAYLOAD;
        case ABP_UPDATE_BEGIN:
            if (exact_length(frame, 44) != ABP_OK) return ABP_ERROR_PAYLOAD_LENGTH;
            return read_u32(payload) != 0 && (payload[9] & (uint8_t)~1u) == 0 &&
                           !all_zero(payload + 10, 32) && read_u16(payload + 42) >= 1 &&
                           read_u16(payload + 42) <= 43
                       ? ABP_OK
                       : ABP_ERROR_PAYLOAD;
        case ABP_UPDATE_CHUNK: {
            if (frame->payload_length < 5 || frame->payload_length > 48) return ABP_ERROR_PAYLOAD_LENGTH;
            const uint8_t data_length = payload[4];
            return data_length >= 1 && data_length <= 43 && frame->payload_length == (uint8_t)(5 + data_length)
                       ? ABP_OK
                       : ABP_ERROR_PAYLOAD;
        }
        default:
            return ABP_ERROR_MESSAGE_TYPE;
    }
}

abp_result_t abp_message_direction(uint8_t message_type, abp_direction_t *out_direction) {
    bool known = (message_type >= ABP_DEVICE_HELLO && message_type <= ABP_UPDATE_STATUS) ||
                 (message_type >= ABP_HOST_HELLO && message_type <= ABP_UPDATE_COMMIT);
    if (!known) return ABP_ERROR_MESSAGE_TYPE;
    if (out_direction != NULL) {
        *out_direction = message_type < ABP_HOST_HELLO ? ABP_DIRECTION_DEVICE_TO_HOST
                                                       : ABP_DIRECTION_HOST_TO_DEVICE;
    }
    return ABP_OK;
}

abp_result_t abp_decode(const uint8_t *bytes, size_t length, abp_frame_t *out_frame) {
    if (bytes == NULL || length != ABP_FRAME_SIZE) return ABP_ERROR_FRAME_LENGTH;
    if (bytes[0] != 0xab) return ABP_ERROR_MAGIC;
    if (bytes[1] != ABP_PROTOCOL_MAJOR || bytes[2] > ABP_PROTOCOL_MINOR) return ABP_ERROR_VERSION;
    if (abp_message_direction(bytes[3], NULL) != ABP_OK) return ABP_ERROR_MESSAGE_TYPE;
    if (bytes[4] != 0) return ABP_ERROR_FLAGS;
    if (bytes[5] > ABP_PAYLOAD_CAPACITY) return ABP_ERROR_PAYLOAD_LENGTH;
    if (read_u32(bytes + 6) == 0) return ABP_ERROR_SEQUENCE;
    for (size_t index = 14u + bytes[5]; index < 62u; ++index) {
        if (bytes[index] != 0) return ABP_ERROR_PADDING;
    }
    if (bytes[62] != 0) return ABP_ERROR_RESERVED;

    abp_frame_t candidate;
    memset(&candidate, 0, sizeof(candidate));
    candidate.message_type = bytes[3];
    candidate.payload_length = bytes[5];
    candidate.sequence = read_u32(bytes + 6);
    candidate.sender_uptime_ms = read_u32(bytes + 10);
    memcpy(candidate.payload, bytes + 14, candidate.payload_length);
    abp_result_t result = validate_payload(&candidate);
    if (result != ABP_OK) return result;
    if (out_frame != NULL) *out_frame = candidate;
    return ABP_OK;
}

abp_result_t abp_encode(const abp_frame_t *frame, uint8_t out_bytes[ABP_FRAME_SIZE]) {
    if (frame == NULL || out_bytes == NULL) return ABP_ERROR_FRAME_LENGTH;
    if (abp_message_direction(frame->message_type, NULL) != ABP_OK) return ABP_ERROR_MESSAGE_TYPE;
    if (frame->sequence == 0) return ABP_ERROR_SEQUENCE;
    if (frame->payload_length > ABP_PAYLOAD_CAPACITY) return ABP_ERROR_PAYLOAD_LENGTH;
    abp_result_t result = validate_payload(frame);
    if (result != ABP_OK) return result;

    uint8_t candidate[ABP_FRAME_SIZE] = {0};
    candidate[0] = 0xab;
    candidate[1] = ABP_PROTOCOL_MAJOR;
    candidate[2] = ABP_PROTOCOL_MINOR;
    candidate[3] = frame->message_type;
    candidate[5] = frame->payload_length;
    write_u32(candidate + 6, frame->sequence);
    write_u32(candidate + 10, frame->sender_uptime_ms);
    memcpy(candidate + 14, frame->payload, frame->payload_length);
    memcpy(out_bytes, candidate, ABP_FRAME_SIZE);
    return ABP_OK;
}
