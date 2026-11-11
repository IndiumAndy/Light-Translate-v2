#pragma once
#include <stdbool.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef void (*lt_input_event_cb)(const char *json_utf8);

void lt_macos_start_input_monitor(lt_input_event_cb callback);
void lt_macos_update_trigger_config(int32_t modifier_kind, bool double_tap_selection);

void lt_macos_get_pointer(double *out_x, double *out_y);
void lt_macos_get_monitor_bounds(double x, double y, double *out_x, double *out_y, double *out_w, double *out_h);

char *lt_macos_app_id_at_point(double x, double y);
char *lt_macos_focused_app_id(void);

bool lt_macos_check_accessibility(bool prompt);
bool lt_macos_check_screen_recording(void);
bool lt_macos_request_screen_recording(void);

bool lt_macos_write_clipboard(const char *text_utf8);

char *lt_macos_extract_selection_json(double x, double y, bool allow_clipboard_fallback);
char *lt_macos_extract_hover_ax_json(double x, double y);
char *lt_macos_ocr_at_point_json(double x, double y, bool sentence_mode);
char *lt_macos_ocr_region_json(double x, double y, double w, double h);

void lt_macos_configure_popup_window(void *ns_window_ptr);
void lt_macos_present_popup_window(void *ns_window_ptr, double x, double y, double w, double h, bool interactive);
void lt_macos_set_popup_interactive(void *ns_window_ptr, bool interactive);
void lt_macos_hide_popup_window(void *ns_window_ptr);

void lt_macos_free_string(char *ptr);

#ifdef __cplusplus
}
#endif
