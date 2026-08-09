#include "abp.h"

#include <dirent.h>
#include <errno.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#ifndef ABP_REPO_ROOT
#error "ABP_REPO_ROOT must point to the Agent Bridge repository"
#endif

#define CHECK(condition, ...)                         \
    do {                                              \
        if (!(condition)) {                           \
            fprintf(stderr, __VA_ARGS__);             \
            fprintf(stderr, "\n");                    \
            return 1;                                 \
        }                                             \
    } while (0)

static int read_file(const char *path, uint8_t **out_bytes, size_t *out_length) {
    FILE *file = fopen(path, "rb");
    if (file == NULL) return -1;
    if (fseek(file, 0, SEEK_END) != 0) return -1;
    long length = ftell(file);
    if (length < 0 || fseek(file, 0, SEEK_SET) != 0) return -1;
    uint8_t *bytes = malloc((size_t)length + 1);
    if (bytes == NULL) return -1;
    size_t actual = fread(bytes, 1, (size_t)length, file);
    fclose(file);
    if (actual != (size_t)length) {
        free(bytes);
        return -1;
    }
    *out_bytes = bytes;
    *out_length = actual;
    return 0;
}

static int json_string(const char *line, const char *key, char *out, size_t capacity) {
    char marker[64];
    snprintf(marker, sizeof(marker), "\"%s\": \"", key);
    const char *start = strstr(line, marker);
    if (start == NULL) return -1;
    start += strlen(marker);
    const char *end = strchr(start, '"');
    if (end == NULL || (size_t)(end - start) >= capacity) return -1;
    memcpy(out, start, (size_t)(end - start));
    out[end - start] = '\0';
    return 0;
}

static int json_u32(const char *line, const char *key, uint32_t *out) {
    char marker[64];
    snprintf(marker, sizeof(marker), "\"%s\": ", key);
    const char *start = strstr(line, marker);
    if (start == NULL) return -1;
    start += strlen(marker);
    errno = 0;
    char *end = NULL;
    unsigned long value = strtoul(start, &end, 10);
    if (errno != 0 || end == start || value > UINT32_MAX) return -1;
    *out = (uint32_t)value;
    return 0;
}

static int hex_bytes(const char *hex, uint8_t *out, size_t capacity, size_t *out_length) {
    size_t length = strlen(hex);
    if ((length & 1u) != 0 || length / 2 > capacity) return -1;
    for (size_t index = 0; index < length / 2; ++index) {
        unsigned int value = 0;
        if (sscanf(hex + index * 2, "%2x", &value) != 1) return -1;
        out[index] = (uint8_t)value;
    }
    *out_length = length / 2;
    return 0;
}

static int test_golden_packets(void) {
    const char *manifest_path = ABP_REPO_ROOT "/protocol/golden/manifest.json";
    FILE *manifest = fopen(manifest_path, "r");
    CHECK(manifest != NULL, "cannot open %s", manifest_path);
    char line[1024];
    size_t count = 0;
    while (fgets(line, sizeof(line), manifest) != NULL) {
        if (strstr(line, "\"name\"") == NULL) continue;
        char name[128];
        char payload_hex[2 * ABP_PAYLOAD_CAPACITY + 1];
        uint32_t type_id = 0;
        uint32_t sequence = 0;
        uint32_t sender_uptime_ms = 0;
        CHECK(json_string(line, "name", name, sizeof(name)) == 0, "invalid manifest name line");
        CHECK(json_string(line, "payloadHex", payload_hex, sizeof(payload_hex)) == 0,
              "%s: invalid payloadHex", name);
        CHECK(json_u32(line, "typeId", &type_id) == 0 && type_id <= UINT8_MAX,
              "%s: invalid typeId", name);
        CHECK(json_u32(line, "sequence", &sequence) == 0, "%s: invalid sequence", name);
        CHECK(json_u32(line, "senderUptimeMs", &sender_uptime_ms) == 0,
              "%s: invalid senderUptimeMs", name);

        char path[1024];
        snprintf(path, sizeof(path), ABP_REPO_ROOT "/protocol/golden/%s.bin", name);
        uint8_t *bytes = NULL;
        size_t length = 0;
        CHECK(read_file(path, &bytes, &length) == 0, "%s: cannot read Golden Packet", name);
        CHECK(length == ABP_FRAME_SIZE, "%s: wrong Golden Packet length", name);
        abp_frame_t frame;
        CHECK(abp_decode(bytes, length, &frame) == ABP_OK, "%s: C decoder rejected legal packet", name);
        CHECK(frame.message_type == (uint8_t)type_id && frame.sequence == sequence &&
                  frame.sender_uptime_ms == sender_uptime_ms,
              "%s: decoded manifest fields differ", name);
        uint8_t expected_payload[ABP_PAYLOAD_CAPACITY];
        size_t expected_length = 0;
        CHECK(hex_bytes(payload_hex, expected_payload, sizeof(expected_payload), &expected_length) == 0,
              "%s: payloadHex is invalid", name);
        CHECK(frame.payload_length == expected_length &&
                  memcmp(frame.payload, expected_payload, expected_length) == 0,
              "%s: decoded payload differs", name);
        uint8_t encoded[ABP_FRAME_SIZE];
        CHECK(abp_encode(&frame, encoded) == ABP_OK && memcmp(encoded, bytes, ABP_FRAME_SIZE) == 0,
              "%s: C round trip differs", name);
        free(bytes);
        ++count;
    }
    fclose(manifest);
    CHECK(count == 22, "expected 22 legal packets, got %zu", count);
    return 0;
}

static int test_invalid_corpus(void) {
    const char *directory_path = ABP_REPO_ROOT "/protocol/fuzz/corpus";
    DIR *directory = opendir(directory_path);
    CHECK(directory != NULL, "cannot open %s", directory_path);
    size_t count = 0;
    struct dirent *entry;
    while ((entry = readdir(directory)) != NULL) {
        size_t name_length = strlen(entry->d_name);
        if (name_length < 4 || strcmp(entry->d_name + name_length - 4, ".bin") != 0) continue;
        char path[1024];
        snprintf(path, sizeof(path), "%s/%s", directory_path, entry->d_name);
        uint8_t *bytes = NULL;
        size_t length = 0;
        CHECK(read_file(path, &bytes, &length) == 0, "%s: cannot read invalid packet", entry->d_name);
        abp_frame_t frame;
        memset(&frame, 0x5a, sizeof(frame));
        abp_frame_t unchanged = frame;
        CHECK(abp_decode(bytes, length, &frame) != ABP_OK, "%s: invalid packet was accepted", entry->d_name);
        CHECK(memcmp(&frame, &unchanged, sizeof(frame)) == 0,
              "%s: rejection mutated caller-visible state", entry->d_name);
        free(bytes);
        ++count;
    }
    closedir(directory);
    CHECK(count == 95, "expected 95 invalid packets, got %zu", count);
    return 0;
}

static bool expected_input_combination(uint8_t kind, uint8_t id, uint8_t phase, int16_t value) {
    bool valid_id = (kind == 0 && id <= 5) || (kind == 1 && id <= 6) ||
                    (kind == 2 && id <= 3) || ((kind == 3 || kind == 4) && id == 0);
    bool valid_phase = ((kind == 0 || kind == 1 || kind == 3) && phase <= 1) ||
                       (kind == 2 && (phase == 2 || phase == 3)) || (kind == 4 && phase == 4);
    bool valid_value = phase == 4 ? value != 0 && value >= -32 && value <= 32 : value == 0;
    return valid_id && valid_phase && valid_value;
}

static int test_control_phase_boundaries(void) {
    const int16_t values[] = {-33, -32, -1, 0, 1, 32, 33};
    for (uint8_t kind = 0; kind <= 5; ++kind) {
        for (uint8_t id = 0; id <= 7; ++id) {
            for (uint8_t phase = 0; phase <= 5; ++phase) {
                for (size_t value_index = 0; value_index < sizeof(values) / sizeof(values[0]); ++value_index) {
                    uint8_t bytes[ABP_FRAME_SIZE] = {0};
                    bytes[0] = 0xab;
                    bytes[1] = 1;
                    bytes[3] = 3;
                    bytes[5] = 9;
                    bytes[6] = 1;
                    bytes[14] = 1;
                    bytes[18] = kind;
                    bytes[19] = id;
                    bytes[20] = phase;
                    uint16_t wire_value = (uint16_t)values[value_index];
                    bytes[21] = (uint8_t)wire_value;
                    bytes[22] = (uint8_t)(wire_value >> 8u);
                    bool expected = expected_input_combination(kind, id, phase, values[value_index]);
                    CHECK((abp_decode(bytes, sizeof(bytes), NULL) == ABP_OK) == expected,
                          "control boundary mismatch kind=%u id=%u phase=%u value=%d",
                          kind, id, phase, values[value_index]);
                }
            }
        }
    }
    return 0;
}

static int test_random_frames(void) {
    uint32_t state = 0x5eedcafeu;
    for (size_t iteration = 0; iteration < 20000; ++iteration) {
        uint8_t bytes[ABP_FRAME_SIZE];
        for (size_t index = 0; index < sizeof(bytes); ++index) {
            state = state * 1664525u + 1013904223u;
            bytes[index] = (uint8_t)(state >> 24u);
        }
        abp_frame_t frame;
        if (abp_decode(bytes, sizeof(bytes), &frame) == ABP_OK) {
            uint8_t encoded[ABP_FRAME_SIZE];
            CHECK(abp_encode(&frame, encoded) == ABP_OK && memcmp(bytes, encoded, sizeof(bytes)) == 0,
                  "accepted random frame was not canonical");
        }
    }
    return 0;
}

int main(void) {
    CHECK(test_golden_packets() == 0, "Golden Packet test failed");
    CHECK(test_invalid_corpus() == 0, "invalid corpus test failed");
    CHECK(test_control_phase_boundaries() == 0, "Control/Phase boundary test failed");
    CHECK(test_random_frames() == 0, "random frame test failed");
    puts("ABP native tests passed: 22 legal packets, 95 invalid packets, exhaustive Control/Phase boundaries");
    return 0;
}
