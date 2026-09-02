#include <assert.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

#include "abp.h"
#include "h0_session.h"
#include "h0_visual.h"

static void host_frame(uint8_t type, uint32_t sequence, const uint8_t *payload,
                       uint8_t payload_length, uint8_t out[ABP_FRAME_SIZE]) {
    abp_frame_t frame = {
        .message_type = type,
        .sequence = sequence,
        .payload_length = payload_length,
    };
    if (payload_length > 0) memcpy(frame.payload, payload, payload_length);
    assert(abp_encode(&frame, out) == ABP_OK);
}

static h0_session_t ready_session(void) {
    const uint8_t device_id[16] = {1};
    h0_session_t session;
    h0_session_init(&session, 42, device_id);
    h0_session_mount(&session);
    uint8_t encoded[ABP_FRAME_SIZE];
    assert(h0_session_make_hello(&session, 10, encoded));
    abp_frame_t hello;
    assert(abp_decode(encoded, sizeof(encoded), &hello) == ABP_OK);
    assert(hello.message_type == ABP_DEVICE_HELLO);
    assert(hello.sequence == 1);
    assert(hello.payload[20] == H0_HARDWARE_REVISION);

    uint8_t request[ABP_FRAME_SIZE];
    uint8_t response[ABP_FRAME_SIZE];
    uint8_t visual[20];
    host_frame(ABP_GET_INPUT_SNAPSHOT, 1, NULL, 0, request);
    assert(h0_session_accept_host(&session, request, 20, response, visual) ==
           H0_HOST_SNAPSHOT_REQUESTED);
    abp_frame_t snapshot;
    assert(abp_decode(response, sizeof(response), &snapshot) == ABP_OK);
    assert(snapshot.message_type == ABP_INPUT_SNAPSHOT);
    assert(snapshot.sequence == 2);
    assert(session.ready);
    return session;
}

static void test_handshake_and_ag00(void) {
    h0_session_t session = ready_session();
    uint8_t encoded[ABP_FRAME_SIZE];
    assert(h0_session_make_ag00(&session, true, 30, encoded));
    abp_frame_t event;
    assert(abp_decode(encoded, sizeof(encoded), &event) == ABP_OK);
    assert(event.message_type == ABP_INPUT_EVENT);
    assert(event.payload[4] == 0 && event.payload[5] == 0 && event.payload[6] == 0);
    assert(!h0_session_make_ag00(&session, true, 31, encoded));
    assert(h0_session_make_ag00(&session, false, 32, encoded));
    assert(abp_decode(encoded, sizeof(encoded), &event) == ABP_OK);
    assert(event.payload[6] == 1);
}

static void test_host_sequence_is_fail_closed(void) {
    h0_session_t session = ready_session();
    uint8_t payload[20] = {0};
    payload[13] = 1;
    payload[15] = 255;
    payload[16] = 2;
    payload[17] = 25;
    uint8_t frame[ABP_FRAME_SIZE];
    uint8_t response[ABP_FRAME_SIZE];
    uint8_t visual[20];
    host_frame(ABP_VISUAL_STATE, 3, payload, sizeof(payload), frame);
    assert(h0_session_accept_host(&session, frame, 40, response, visual) == H0_HOST_REJECTED);
    assert(session.next_host_sequence == 2);
    host_frame(ABP_VISUAL_STATE, 2, payload, sizeof(payload), frame);
    assert(h0_session_accept_host(&session, frame, 41, response, visual) == H0_HOST_VISUAL_STATE);
    assert(memcmp(payload, visual, sizeof(payload)) == 0);
}

static void test_visual_states_and_brightness_cap(void) {
    uint8_t payload[20] = {0};
    payload[4] = 2;
    payload[10] = 1;
    payload[12] = 1;
    payload[13] = 1;
    payload[15] = 255;
    payload[16] = 2;
    payload[17] = 100;
    h0_rgb_t pixels[H0_LED_COUNT];
    h0_visual_render(payload, pixels);
    assert(pixels[0].red > 0 || pixels[0].blue > 0);
    assert(pixels[7].green > 0);
    assert(pixels[0].red <= 64 && pixels[0].green <= 64 && pixels[0].blue <= 64);
    payload[4] = 5;
    h0_visual_render(payload, pixels);
    assert(pixels[0].red > 0 && pixels[0].green == 0 && pixels[0].blue == 0);
    assert(pixels[7].red > 0 && pixels[7].green == 0 && pixels[7].blue == 0);
}

int main(void) {
    test_handshake_and_ag00();
    test_host_sequence_is_fail_closed();
    test_visual_states_and_brightness_cap();
    puts("H0 native session/visual tests passed");
    return 0;
}
