#include "avatar_ui.h"

#include <stdio.h>
#include <string.h>

#include "esp_timer.h"
#include "lvgl.h"
#include "passport_bsp.h"

#define COLOR_BACKGROUND 0x08111F
#define COLOR_PANEL 0x12243A
#define COLOR_TEXT 0xEAF7FF
#define COLOR_MUTED 0x6E879B
#define COLOR_CYAN 0x38DDF8
#define COLOR_GREEN 0x52E39A
#define COLOR_YELLOW 0xFFD166
#define COLOR_RED 0xFF5D73
#define COLOR_PURPLE 0x9B8AFB

static avatar_model_t *s_model;
static SemaphoreHandle_t s_state_lock;
static lv_obj_t *s_connection;
static lv_obj_t *s_face;
static lv_obj_t *s_session_title;
static lv_obj_t *s_agent_panels[6];
static lv_obj_t *s_agent_labels[6];
static lv_obj_t *s_state;
static avatar_model_t s_rendered_model;
static bool s_has_rendered;
static bool s_rendered_online;

static uint32_t status_color(avatar_agent_status_t status)
{
    switch (status) {
        case AVATAR_AGENT_THINKING: return COLOR_PURPLE;
        case AVATAR_AGENT_COMPLETE: return COLOR_GREEN;
        case AVATAR_AGENT_INPUT: return COLOR_YELLOW;
        case AVATAR_AGENT_ERROR: return COLOR_RED;
        case AVATAR_AGENT_IDLE: return COLOR_CYAN;
        default: return COLOR_MUTED;
    }
}

LV_FONT_DECLARE(lv_font_agent_title_16)

static bool visual_content_matches(const avatar_model_t *model, bool online)
{
    return s_has_rendered && online == s_rendered_online &&
           memcmp(model->agents, s_rendered_model.agents, sizeof(model->agents)) == 0 &&
           memcmp(model->agent_titles, s_rendered_model.agent_titles,
                  sizeof(model->agent_titles)) == 0 &&
           model->host_selected_mask == s_rendered_model.host_selected_mask &&
           model->selected_slot == s_rendered_model.selected_slot &&
           model->ble_connected == s_rendered_model.ble_connected &&
           model->session_ready == s_rendered_model.session_ready;
}

static void render(lv_timer_t *timer)
{
    (void)timer;
    avatar_model_t model;
    if (xSemaphoreTake(s_state_lock, 0) != pdTRUE) return;
    model = *s_model;
    xSemaphoreGive(s_state_lock);
    const uint32_t now_ms = (uint32_t)(esp_timer_get_time() / 1000);
    const bool online = avatar_model_online(&model, now_ms);
    if (visual_content_matches(&model, online)) return;
    s_rendered_model = model;
    s_rendered_online = online;
    s_has_rendered = true;
    const avatar_agent_status_t selected = model.agents[model.selected_slot];

    lv_label_set_text(s_connection,
                      online ? "LIVE" : (model.ble_connected ? "SYNCING" : "FIND MAC"));
    lv_obj_set_style_text_color(
        s_connection, lv_color_hex(online ? COLOR_GREEN : COLOR_YELLOW), 0);
    lv_obj_set_style_bg_color(s_face, lv_color_hex(status_color(selected)), 0);
    const char *session_title = model.agent_titles[model.selected_slot];
    if (session_title[0] == '\0') {
        session_title = selected == AVATAR_AGENT_EMPTY ? "NO SESSION" : "UNTITLED SESSION";
    }
    lv_label_set_text(s_session_title, session_title);
    lv_label_set_text_fmt(s_state, "AGENT %u / %s",
                          (unsigned)model.selected_slot + 1,
                          online ? avatar_model_status_name(selected) : "OFFLINE");

    for (size_t index = 0; index < 6; ++index) {
        const bool active = index == model.selected_slot;
        lv_obj_set_style_bg_color(s_agent_panels[index],
                                  lv_color_hex(status_color(model.agents[index])), 0);
        lv_obj_set_style_border_color(s_agent_panels[index],
                                      lv_color_hex(active ? COLOR_TEXT : COLOR_BACKGROUND), 0);
        lv_obj_set_style_border_width(s_agent_panels[index], active ? 3 : 1, 0);
        lv_label_set_text_fmt(s_agent_labels[index],
                              (model.host_selected_mask & (1u << index)) ? "A%u*" : "A%u",
                              (unsigned)index + 1);
    }

}

void avatar_ui_init(avatar_model_t *model, SemaphoreHandle_t state_lock)
{
    s_model = model;
    s_state_lock = state_lock;
    lv_obj_t *screen = lv_obj_create(NULL);
    lv_obj_remove_flag(screen, LV_OBJ_FLAG_SCROLLABLE);
    lv_obj_set_style_bg_color(screen, lv_color_hex(COLOR_BACKGROUND), 0);
    lv_obj_set_style_border_width(screen, 0, 0);
    lv_obj_set_style_pad_all(screen, 0, 0);

    lv_obj_t *title = lv_label_create(screen);
    lv_label_set_text(title, "AGENT / TWIN");
    lv_obj_set_style_text_font(title, &lv_font_montserrat_20, 0);
    lv_obj_set_style_text_color(title, lv_color_hex(COLOR_TEXT), 0);
    lv_obj_set_pos(title, 14, 12);

    s_connection = lv_label_create(screen);
    lv_obj_set_style_text_font(s_connection, &lv_font_montserrat_14, 0);
    lv_obj_align(s_connection, LV_ALIGN_TOP_RIGHT, -12, 16);

    s_face = lv_obj_create(screen);
    lv_obj_remove_flag(s_face, LV_OBJ_FLAG_SCROLLABLE);
    lv_obj_set_pos(s_face, 14, 52);
    lv_obj_set_size(s_face, 212, 108);
    lv_obj_set_style_radius(s_face, 22, 0);
    lv_obj_set_style_border_width(s_face, 4, 0);
    lv_obj_set_style_border_color(s_face, lv_color_hex(COLOR_TEXT), 0);
    lv_obj_set_style_shadow_width(s_face, 18, 0);
    lv_obj_set_style_shadow_color(s_face, lv_color_hex(COLOR_CYAN), 0);

    s_session_title = lv_label_create(s_face);
    lv_label_set_long_mode(s_session_title, LV_LABEL_LONG_DOT);
    lv_obj_set_size(s_session_title, 180, 58);
    lv_obj_set_style_text_font(s_session_title, &lv_font_agent_title_16, 0);
    lv_obj_set_style_text_color(s_session_title, lv_color_hex(COLOR_BACKGROUND), 0);
    lv_obj_set_style_text_align(s_session_title, LV_TEXT_ALIGN_CENTER, 0);
    lv_obj_center(s_session_title);

    s_state = lv_label_create(screen);
    lv_obj_set_width(s_state, 220);
    lv_obj_set_style_text_align(s_state, LV_TEXT_ALIGN_CENTER, 0);
    lv_obj_set_style_text_color(s_state, lv_color_hex(COLOR_TEXT), 0);
    lv_obj_set_pos(s_state, 10, 174);

    for (size_t index = 0; index < 6; ++index) {
        const int column = index % 3;
        const int row = index / 3;
        s_agent_panels[index] = lv_obj_create(screen);
        lv_obj_remove_flag(s_agent_panels[index], LV_OBJ_FLAG_SCROLLABLE);
        lv_obj_set_pos(s_agent_panels[index], 13 + column * 76, 205 + row * 42);
        lv_obj_set_size(s_agent_panels[index], 62, 32);
        lv_obj_set_style_radius(s_agent_panels[index], 8, 0);
        lv_obj_set_style_pad_all(s_agent_panels[index], 0, 0);
        s_agent_labels[index] = lv_label_create(s_agent_panels[index]);
        lv_obj_set_style_text_color(s_agent_labels[index], lv_color_hex(COLOR_BACKGROUND), 0);
        lv_obj_center(s_agent_labels[index]);
    }

    lv_obj_t *hint = lv_label_create(screen);
    lv_label_set_text(hint, "UP/DOWN choose    OK enter");
    lv_obj_set_style_text_color(hint, lv_color_hex(COLOR_MUTED), 0);
    lv_obj_align(hint, LV_ALIGN_BOTTOM_MID, 0, -10);
    lv_screen_load(screen);
    lv_timer_create(render, 120, NULL);
    render(NULL);
}
