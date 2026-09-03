#include <assert.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

#include "abp.h"
#include "avatar_ble_frag.h"
#include "avatar_display_packet.h"
#include "avatar_model.h"
#include "avatar_session.h"

typedef struct {
    uint8_t values[5][AVATAR_BLE_VALUE_MAX];
    size_t lengths[5];
    size_t count;
} fragment_capture_t;

static bool capture(const uint8_t *value, size_t length, void *context)
{
    fragment_capture_t *capture = context;
    assert(capture->count < 5);
    memcpy(capture->values[capture->count], value, length);
    capture->lengths[capture->count] = length;
    capture->count++;
    return true;
}

static void make_host_frame(uint8_t type,
                            uint32_t sequence,
                            const uint8_t *payload,
                            uint8_t payload_length,
                            uint8_t output[ABP_FRAME_SIZE])
{
    abp_frame_t frame = {
        .message_type = type,
        .sequence = sequence,
        .sender_uptime_ms = 10,
        .payload_length = payload_length,
    };
    if (payload_length != 0) memcpy(frame.payload, payload, payload_length);
    assert(abp_encode(&frame, output) == ABP_OK);
}

static void test_fragment_round_trip_and_timeout(void)
{
    uint8_t host[ABP_FRAME_SIZE];
    make_host_frame(ABP_GET_INPUT_SNAPSHOT, 1, NULL, 0, host);
    fragment_capture_t capture_state = {0};
    assert(avatar_ble_fragment_frame(host, 7, capture, &capture_state));
    assert(capture_state.count == 5);

    avatar_ble_reassembler_t reassembler = {0};
    uint8_t output[ABP_FRAME_SIZE];
    for (size_t index = 0; index < capture_state.count; ++index) {
        const bool complete = avatar_ble_reassembler_push(
            &reassembler, capture_state.values[index], capture_state.lengths[index],
            10, output);
        assert(complete == (index + 1 == capture_state.count));
    }
    assert(memcmp(output, host, sizeof(host)) == 0);

    assert(!avatar_ble_reassembler_push(&reassembler, capture_state.values[0],
                                        capture_state.lengths[0], 20, output));
    assert(!avatar_ble_reassembler_push(&reassembler, capture_state.values[1],
                                        capture_state.lengths[1],
                                        20 + AVATAR_BLE_REASSEMBLY_TIMEOUT_MS + 1,
                                        output));
}

static void test_session_and_model(void)
{
    const uint8_t device_id[16] = {1, 2, 3, 4, 5, 6, 7, 8,
                                   9, 10, 11, 12, 13, 14, 15, 16};
    avatar_session_t session;
    avatar_session_init(&session, 0x12345678, device_id);
    avatar_session_connect(&session);
    uint8_t frame[ABP_FRAME_SIZE];
    memset(frame, 0xa5, sizeof(frame));
    assert(avatar_session_schedule_hello(&session));
    assert(!avatar_session_schedule_hello(&session));
    assert(session.next_device_sequence == 1);
    assert(session.hello_pending);
    assert(frame[0] == 0xa5);
    assert(avatar_session_take_scheduled_hello(&session, 1, frame));
    assert(!session.hello_pending);
    assert(session.next_device_sequence == 2);
    abp_frame_t decoded;
    assert(abp_decode(frame, sizeof(frame), &decoded) == ABP_OK);
    assert(decoded.message_type == ABP_DEVICE_HELLO);

    uint8_t host[ABP_FRAME_SIZE];
    make_host_frame(ABP_GET_INPUT_SNAPSHOT, 1, NULL, 0, host);
    uint8_t response[ABP_FRAME_SIZE];
    uint8_t visual[20];
    assert(avatar_session_accept_host(&session, host, 2, response, visual) ==
           AVATAR_HOST_SNAPSHOT_REQUESTED);
    assert(abp_decode(response, sizeof(response), &decoded) == ABP_OK);
    assert(decoded.message_type == ABP_INPUT_SNAPSHOT);

    uint8_t visual_payload[20] = {0};
    visual_payload[0] = 1;
    visual_payload[4] = AVATAR_AGENT_IDLE;
    visual_payload[5] = AVATAR_AGENT_THINKING;
    visual_payload[12] = 1;
    visual_payload[13] = 1;
    visual_payload[14] = 1;
    visual_payload[15] = 255;
    visual_payload[17] = 65;
    make_host_frame(ABP_VISUAL_STATE, 2, visual_payload, sizeof(visual_payload), host);
    assert(avatar_session_accept_host(&session, host, 3, response, visual) ==
           AVATAR_HOST_VISUAL_STATE);

    avatar_model_t model;
    avatar_model_init(&model);
    model.ble_connected = true;
    model.session_ready = true;
    assert(avatar_model_apply_visual(&model, visual, 3));
    avatar_model_move(&model, 1);
    assert(model.selected_slot == 1);
    avatar_session_select(&session, model.selected_slot);
    assert(avatar_session_make_selected(&session, true, 4, frame));
    assert(abp_decode(frame, sizeof(frame), &decoded) == ABP_OK);
    assert(decoded.message_type == ABP_INPUT_EVENT);
    assert(decoded.payload[5] == 1);
    assert(avatar_model_online(&model, 5003));
    assert(!avatar_model_online(&model, 5004));
}

static void test_display_title_packet(void)
{
    uint8_t packet[ABP_FRAME_SIZE] = {0};
    static const char expected[] = "深入了解 ai-passport";
    packet[0] = AVATAR_DISPLAY_PACKET_MAGIC;
    packet[1] = AVATAR_DISPLAY_PACKET_VERSION;
    packet[2] = AVATAR_DISPLAY_PACKET_AGENT_TITLE;
    packet[3] = 2;
    packet[4] = sizeof(expected) - 1u;
    memcpy(packet + 5, expected, sizeof(expected) - 1u);

    avatar_display_title_t title;
    assert(avatar_display_packet_is_candidate(packet));
    assert(avatar_display_packet_decode_title(packet, &title));
    assert(title.slot == 2);
    assert(strcmp(title.title, expected) == 0);

    avatar_model_t model;
    avatar_model_init(&model);
    assert(avatar_model_apply_title(&model, &title));
    assert(strcmp(model.agent_titles[2], expected) == 0);

    packet[4]--;
    assert(!avatar_display_packet_decode_title(packet, &title));
    packet[4]++;
    packet[62] = 1;
    assert(!avatar_display_packet_decode_title(packet, &title));
}

int main(void)
{
    test_fragment_round_trip_and_timeout();
    test_session_and_model();
    test_display_title_packet();
    puts("AI Passport avatar native tests passed");
    return 0;
}
