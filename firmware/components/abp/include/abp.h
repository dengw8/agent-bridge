#ifndef AGENT_BRIDGE_ABP_H
#define AGENT_BRIDGE_ABP_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define ABP_FRAME_SIZE 63u
#define ABP_PAYLOAD_CAPACITY 48u
#define ABP_PROTOCOL_MAJOR 1u
#define ABP_PROTOCOL_MINOR 0u

typedef enum {
    ABP_OK = 0,
    ABP_ERROR_FRAME_LENGTH,
    ABP_ERROR_MAGIC,
    ABP_ERROR_VERSION,
    ABP_ERROR_MESSAGE_TYPE,
    ABP_ERROR_FLAGS,
    ABP_ERROR_PAYLOAD_LENGTH,
    ABP_ERROR_SEQUENCE,
    ABP_ERROR_PADDING,
    ABP_ERROR_RESERVED,
    ABP_ERROR_PAYLOAD,
} abp_result_t;

typedef enum {
    ABP_DIRECTION_DEVICE_TO_HOST = 0,
    ABP_DIRECTION_HOST_TO_DEVICE = 1,
} abp_direction_t;

typedef struct {
    uint8_t message_type;
    uint32_t sequence;
    uint32_t sender_uptime_ms;
    uint8_t payload_length;
    uint8_t payload[ABP_PAYLOAD_CAPACITY];
} abp_frame_t;

abp_result_t abp_decode(const uint8_t *bytes, size_t length, abp_frame_t *out_frame);
abp_result_t abp_encode(const abp_frame_t *frame, uint8_t out_bytes[ABP_FRAME_SIZE]);
abp_result_t abp_message_direction(uint8_t message_type, abp_direction_t *out_direction);

#ifdef __cplusplus
}
#endif

#endif
