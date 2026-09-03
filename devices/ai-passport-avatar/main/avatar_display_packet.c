#include "avatar_display_packet.h"

#include <string.h>

static bool valid_utf8(const uint8_t *bytes, size_t length)
{
    size_t index = 0;
    while (index < length) {
        const uint8_t first = bytes[index++];
        if (first <= 0x7fu) {
            if (first < 0x20u || first == 0x7fu) return false;
            continue;
        }

        uint32_t codepoint;
        size_t continuation_count;
        if (first >= 0xc2u && first <= 0xdfu) {
            codepoint = first & 0x1fu;
            continuation_count = 1;
        } else if (first >= 0xe0u && first <= 0xefu) {
            codepoint = first & 0x0fu;
            continuation_count = 2;
        } else if (first >= 0xf0u && first <= 0xf4u) {
            codepoint = first & 0x07u;
            continuation_count = 3;
        } else {
            return false;
        }
        if (index + continuation_count > length) return false;
        for (size_t offset = 0; offset < continuation_count; ++offset) {
            const uint8_t next = bytes[index++];
            if ((next & 0xc0u) != 0x80u) return false;
            codepoint = (codepoint << 6u) | (next & 0x3fu);
        }
        if ((continuation_count == 2 && codepoint < 0x800u) ||
            (continuation_count == 3 && codepoint < 0x10000u) ||
            (codepoint >= 0xd800u && codepoint <= 0xdfffu) ||
            codepoint > 0x10ffffu) {
            return false;
        }
    }
    return true;
}

bool avatar_display_packet_is_candidate(const uint8_t packet[ABP_FRAME_SIZE])
{
    return packet != NULL && packet[0] == AVATAR_DISPLAY_PACKET_MAGIC;
}

bool avatar_display_packet_decode_title(const uint8_t packet[ABP_FRAME_SIZE],
                                        avatar_display_title_t *title)
{
    if (packet == NULL || title == NULL ||
        packet[0] != AVATAR_DISPLAY_PACKET_MAGIC ||
        packet[1] != AVATAR_DISPLAY_PACKET_VERSION ||
        packet[2] != AVATAR_DISPLAY_PACKET_AGENT_TITLE || packet[3] >= 6u ||
        packet[4] > AVATAR_DISPLAY_TITLE_MAX_BYTES) {
        return false;
    }
    const size_t length = packet[4];
    if (!valid_utf8(packet + 5, length)) return false;
    for (size_t index = 5u + length; index < ABP_FRAME_SIZE; ++index) {
        if (packet[index] != 0) return false;
    }
    title->slot = packet[3];
    title->length = (uint8_t)length;
    memcpy(title->title, packet + 5, length);
    title->title[length] = '\0';
    return true;
}
