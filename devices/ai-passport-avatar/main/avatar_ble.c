#include "avatar_ble.h"

#include <string.h>

#include "avatar_ble_frag.h"
#include "avatar_display_packet.h"
#include "esp_err.h"
#include "esp_log.h"
#include "esp_timer.h"
#include "freertos/task.h"
#include "host/ble_gap.h"
#include "host/ble_gatt.h"
#include "host/ble_hs.h"
#include "host/util/util.h"
#include "nimble/nimble_port.h"
#include "nimble/nimble_port_freertos.h"
#include "os/os_mbuf.h"
#include "services/gap/ble_svc_gap.h"
#include "services/gatt/ble_svc_gatt.h"

#define AVATAR_DEVICE_NAME "AgentPassport"
#define AVATAR_HELLO_DELAY_MS 100
#define AVATAR_HELLO_RETRY_DELAY_MS 250
#define AVATAR_HELLO_MAX_ATTEMPTS 20
#define AVATAR_HELLO_TASK_STACK 2048
#define AVATAR_HELLO_TASK_PRIORITY 5

static const char *TAG = "avatar_ble";
static const ble_uuid128_t s_service_uuid = BLE_UUID128_INIT(
    0x53, 0x6f, 0x29, 0xa7, 0x6c, 0x0a, 0xce, 0x9b,
    0x22, 0x4d, 0x6c, 0x5f, 0x10, 0x7a, 0x6f, 0x6c);
static const ble_uuid128_t s_host_to_device_uuid = BLE_UUID128_INIT(
    0x53, 0x6f, 0x29, 0xa7, 0x6c, 0x0a, 0xce, 0x9b,
    0x22, 0x4d, 0x6c, 0x5f, 0x11, 0x7a, 0x6f, 0x6c);
static const ble_uuid128_t s_device_to_host_uuid = BLE_UUID128_INIT(
    0x53, 0x6f, 0x29, 0xa7, 0x6c, 0x0a, 0xce, 0x9b,
    0x22, 0x4d, 0x6c, 0x5f, 0x12, 0x7a, 0x6f, 0x6c);
static const ble_uuid128_t s_device_info_uuid = BLE_UUID128_INIT(
    0x53, 0x6f, 0x29, 0xa7, 0x6c, 0x0a, 0xce, 0x9b,
    0x22, 0x4d, 0x6c, 0x5f, 0x13, 0x7a, 0x6f, 0x6c);

static avatar_model_t *s_model;
static avatar_session_t *s_session;
static SemaphoreHandle_t s_state_lock;
static SemaphoreHandle_t s_notify_lock;
static avatar_ble_reassembler_t s_reassembler;
static uint16_t s_connection = BLE_HS_CONN_HANDLE_NONE;
static uint16_t s_notify_handle;
static uint8_t s_address_type;
static uint8_t s_transfer_id;
static uint32_t s_connection_generation;

static uint32_t uptime_ms(void)
{
    return (uint32_t)(esp_timer_get_time() / 1000);
}

static bool notify_fragment(const uint8_t *value, size_t length, void *context)
{
    (void)context;
    if (s_connection == BLE_HS_CONN_HANDLE_NONE) return false;
    struct os_mbuf *buffer = ble_hs_mbuf_from_flat(value, length);
    if (buffer == NULL) return false;
    return ble_gatts_notify_custom(s_connection, s_notify_handle, buffer) == 0;
}

bool avatar_ble_notify_frame(const uint8_t frame[ABP_FRAME_SIZE])
{
    if (frame == NULL || s_notify_lock == NULL ||
        xSemaphoreTake(s_notify_lock, pdMS_TO_TICKS(100)) != pdTRUE) {
        return false;
    }
    s_transfer_id++;
    const bool sent = avatar_ble_fragment_frame(frame, s_transfer_id, notify_fragment, NULL);
    xSemaphoreGive(s_notify_lock);
    return sent;
}

static void hello_task(void *context)
{
    const uint32_t generation = (uint32_t)(uintptr_t)context;
    vTaskDelay(pdMS_TO_TICKS(AVATAR_HELLO_DELAY_MS));

    uint8_t hello[ABP_FRAME_SIZE];
    bool prepared = false;
    if (xSemaphoreTake(s_state_lock, pdMS_TO_TICKS(20)) == pdTRUE) {
        if (generation == s_connection_generation &&
            s_connection != BLE_HS_CONN_HANDLE_NONE) {
            prepared = avatar_session_take_scheduled_hello(
                s_session, uptime_ms(), hello);
        }
        xSemaphoreGive(s_state_lock);
    }

    if (prepared) {
        for (uint8_t attempt = 1; attempt <= AVATAR_HELLO_MAX_ATTEMPTS; ++attempt) {
            bool awaiting_snapshot = false;
            if (xSemaphoreTake(s_state_lock, pdMS_TO_TICKS(20)) == pdTRUE) {
                awaiting_snapshot = generation == s_connection_generation &&
                                    s_connection != BLE_HS_CONN_HANDLE_NONE &&
                                    !s_session->ready &&
                                    s_session->next_host_sequence == 1;
                xSemaphoreGive(s_state_lock);
            }
            if (!awaiting_snapshot) break;
            if (!avatar_ble_notify_frame(hello)) {
                ESP_LOGW(TAG, "Hello send attempt %u was rejected", (unsigned)attempt);
            }
            vTaskDelay(pdMS_TO_TICKS(AVATAR_HELLO_RETRY_DELAY_MS));
        }

        bool acknowledged = false;
        if (xSemaphoreTake(s_state_lock, pdMS_TO_TICKS(20)) == pdTRUE) {
            acknowledged = generation == s_connection_generation && s_session->ready;
            xSemaphoreGive(s_state_lock);
        }
        if (!acknowledged && generation == s_connection_generation &&
            s_connection != BLE_HS_CONN_HANDLE_NONE) {
            ESP_LOGW(TAG, "Hello was not acknowledged after %u attempts",
                     (unsigned)AVATAR_HELLO_MAX_ATTEMPTS);
        }
    }
    vTaskDelete(NULL);
}

static int access_characteristic(uint16_t connection_handle,
                                 uint16_t attribute_handle,
                                 struct ble_gatt_access_ctxt *context,
                                 void *argument)
{
    (void)connection_handle;
    (void)attribute_handle;
    (void)argument;
    if (ble_uuid_cmp(context->chr->uuid, &s_device_info_uuid.u) == 0 &&
        context->op == BLE_GATT_ACCESS_OP_READ_CHR) {
        static const char info[] = "agent-passport-avatar/0.1;abp/1.0";
        return os_mbuf_append(context->om, info, sizeof(info) - 1) == 0
                   ? 0
                   : BLE_ATT_ERR_INSUFFICIENT_RES;
    }
    if (ble_uuid_cmp(context->chr->uuid, &s_host_to_device_uuid.u) != 0 ||
        context->op != BLE_GATT_ACCESS_OP_WRITE_CHR) {
        return BLE_ATT_ERR_UNLIKELY;
    }

    uint8_t value[AVATAR_BLE_VALUE_MAX];
    uint16_t length = 0;
    if (ble_hs_mbuf_to_flat(context->om, value, sizeof(value), &length) != 0) {
        return BLE_ATT_ERR_INVALID_ATTR_VALUE_LEN;
    }
    uint8_t frame[ABP_FRAME_SIZE];
    if (!avatar_ble_reassembler_push(&s_reassembler, value, length, uptime_ms(), frame)) {
        return 0;
    }
    if (xSemaphoreTake(s_state_lock, pdMS_TO_TICKS(20)) != pdTRUE) {
        return BLE_ATT_ERR_UNLIKELY;
    }
    avatar_display_title_t title;
    if (avatar_display_packet_is_candidate(frame)) {
        const bool accepted = s_session->ready &&
                              avatar_display_packet_decode_title(frame, &title) &&
                              avatar_model_apply_title(s_model, &title);
        xSemaphoreGive(s_state_lock);
        return accepted ? 0 : BLE_ATT_ERR_UNLIKELY;
    }
    uint8_t response[ABP_FRAME_SIZE];
    uint8_t visual[20];
    const avatar_host_result_t result = avatar_session_accept_host(
        s_session, frame, uptime_ms(), response, visual);
    if (result == AVATAR_HOST_VISUAL_STATE) {
        avatar_model_apply_visual(s_model, visual, uptime_ms());
        s_model->session_ready = true;
    } else if (result == AVATAR_HOST_SNAPSHOT_REQUESTED) {
        s_model->session_ready = true;
    } else if (result == AVATAR_HOST_DISCONNECTED) {
        s_model->session_ready = false;
    }
    xSemaphoreGive(s_state_lock);
    if (result == AVATAR_HOST_SNAPSHOT_REQUESTED) {
        avatar_ble_notify_frame(response);
    }
    return result == AVATAR_HOST_REJECTED ? BLE_ATT_ERR_UNLIKELY : 0;
}

static const struct ble_gatt_svc_def s_services[] = {
    {
        .type = BLE_GATT_SVC_TYPE_PRIMARY,
        .uuid = &s_service_uuid.u,
        .characteristics = (struct ble_gatt_chr_def[]) {
            {
                .uuid = &s_host_to_device_uuid.u,
                .access_cb = access_characteristic,
                .flags = BLE_GATT_CHR_F_WRITE | BLE_GATT_CHR_F_WRITE_NO_RSP |
                         BLE_GATT_CHR_F_WRITE_ENC,
            },
            {
                .uuid = &s_device_to_host_uuid.u,
                .access_cb = access_characteristic,
                .val_handle = &s_notify_handle,
                .flags = BLE_GATT_CHR_F_NOTIFY,
            },
            {
                .uuid = &s_device_info_uuid.u,
                .access_cb = access_characteristic,
                .flags = BLE_GATT_CHR_F_READ,
            },
            {0},
        },
    },
    {0},
};

static int advertise(void);

static int gap_event(struct ble_gap_event *event, void *context)
{
    (void)context;
    switch (event->type) {
        case BLE_GAP_EVENT_CONNECT:
            if (event->connect.status == 0) {
                s_connection = event->connect.conn_handle;
                s_connection_generation++;
                if (s_connection_generation == 0) s_connection_generation = 1;
                avatar_ble_reassembler_reset(&s_reassembler);
                if (xSemaphoreTake(s_state_lock, pdMS_TO_TICKS(20)) == pdTRUE) {
                    avatar_session_connect(s_session);
                    s_model->ble_connected = true;
                    s_model->session_ready = false;
                    xSemaphoreGive(s_state_lock);
                }
                ble_gap_security_initiate(s_connection);
            } else {
                advertise();
            }
            return 0;
        case BLE_GAP_EVENT_DISCONNECT:
            s_connection = BLE_HS_CONN_HANDLE_NONE;
            avatar_ble_reassembler_reset(&s_reassembler);
            if (xSemaphoreTake(s_state_lock, pdMS_TO_TICKS(20)) == pdTRUE) {
                avatar_session_disconnect(s_session);
                s_model->ble_connected = false;
                s_model->session_ready = false;
                xSemaphoreGive(s_state_lock);
            }
            advertise();
            return 0;
        case BLE_GAP_EVENT_SUBSCRIBE:
            if (event->subscribe.attr_handle == s_notify_handle &&
                event->subscribe.cur_notify) {
                bool scheduled = false;
                uint32_t generation = 0;
                if (xSemaphoreTake(s_state_lock, pdMS_TO_TICKS(20)) == pdTRUE) {
                    scheduled = avatar_session_schedule_hello(s_session);
                    generation = s_connection_generation;
                    xSemaphoreGive(s_state_lock);
                }
                if (scheduled &&
                    xTaskCreate(hello_task, "avatar_hello", AVATAR_HELLO_TASK_STACK,
                                (void *)(uintptr_t)generation,
                                AVATAR_HELLO_TASK_PRIORITY, NULL) != pdPASS) {
                    ESP_LOGE(TAG, "Unable to schedule Hello");
                }
            }
            return 0;
        case BLE_GAP_EVENT_ADV_COMPLETE:
            advertise();
            return 0;
        default:
            return 0;
    }
}

static int advertise(void)
{
    struct ble_hs_adv_fields fields = {0};
    fields.flags = BLE_HS_ADV_F_DISC_GEN | BLE_HS_ADV_F_BREDR_UNSUP;
    fields.uuids128 = (ble_uuid128_t *)&s_service_uuid;
    fields.num_uuids128 = 1;
    fields.uuids128_is_complete = 1;
    int result = ble_gap_adv_set_fields(&fields);
    if (result != 0) return result;
    struct ble_hs_adv_fields response = {0};
    response.name = (const uint8_t *)AVATAR_DEVICE_NAME;
    response.name_len = strlen(AVATAR_DEVICE_NAME);
    response.name_is_complete = 1;
    result = ble_gap_adv_rsp_set_fields(&response);
    if (result != 0) return result;
    struct ble_gap_adv_params parameters = {0};
    parameters.conn_mode = BLE_GAP_CONN_MODE_UND;
    parameters.disc_mode = BLE_GAP_DISC_MODE_GEN;
    return ble_gap_adv_start(s_address_type, NULL, BLE_HS_FOREVER,
                             &parameters, gap_event, NULL);
}

static void on_reset(int reason)
{
    ESP_LOGE(TAG, "NimBLE reset: %d", reason);
}

static void on_sync(void)
{
    int result = ble_hs_util_ensure_addr(0);
    if (result == 0) result = ble_hs_id_infer_auto(0, &s_address_type);
    if (result == 0) result = advertise();
    if (result != 0) ESP_LOGE(TAG, "BLE sync failed: %d", result);
}

static void host_task(void *context)
{
    (void)context;
    nimble_port_run();
    nimble_port_freertos_deinit();
}

void avatar_ble_init(avatar_model_t *model,
                     avatar_session_t *session,
                     SemaphoreHandle_t state_lock)
{
    s_model = model;
    s_session = session;
    s_state_lock = state_lock;
    s_notify_lock = xSemaphoreCreateMutex();
    ESP_ERROR_CHECK(nimble_port_init());
    ble_svc_gap_init();
    ble_svc_gatt_init();
    ESP_ERROR_CHECK(ble_svc_gap_device_name_set(AVATAR_DEVICE_NAME));
    ESP_ERROR_CHECK(ble_gatts_count_cfg(s_services));
    ESP_ERROR_CHECK(ble_gatts_add_svcs(s_services));
    ble_hs_cfg.reset_cb = on_reset;
    ble_hs_cfg.sync_cb = on_sync;
    ble_hs_cfg.sm_io_cap = BLE_HS_IO_NO_INPUT_OUTPUT;
    ble_hs_cfg.sm_bonding = 1;
    ble_hs_cfg.sm_mitm = 0;
    ble_hs_cfg.sm_sc = 1;
    nimble_port_freertos_init(host_task);
}
