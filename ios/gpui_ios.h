// Copied from gpui-mobile's example. Must match the Rust exports.

#ifndef GPUI_IOS_H
#define GPUI_IOS_H

#include <stdint.h>
#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

/// Returns NULL if initialization fails.
void* gpui_ios_initialize(void);

void gpui_ios_did_finish_launching(void* app_ptr);

void gpui_ios_will_enter_foreground(void* app_ptr);

void gpui_ios_did_become_active(void* app_ptr);

void gpui_ios_will_resign_active(void* app_ptr);

void gpui_ios_did_enter_background(void* app_ptr);

void gpui_ios_will_terminate(void* app_ptr);

void gpui_ios_handle_touch(void* window_ptr, void* touch_ptr, void* event_ptr);

/// Draws only if something changed. Returns whether gpui wants another frame; a host with a
/// frame waker may pause its display link on false. A NULL window_ptr returns false.
bool gpui_ios_request_frame(void* window_ptr);

/// gpui calls waker(context) on the main thread when it wants frames again. Pass NULL to
/// clear; context must stay valid until then.
void gpui_ios_set_frame_waker(void* window_ptr, void (*waker)(void* context), void* context);

/// The most recently created window, or NULL if there is none.
void* gpui_ios_get_window(void);

/// The root UIViewController of the window's UIWindow.
void* gpui_ios_view_controller(void* window_ptr);

/// Defined in flint's src/lib.rs, not in gpui-mobile. Call it before gpui_ios_run_demo().
void gpui_ios_register_app(void);

/// Runs the app set by gpui_ios_register_app(), or an empty window if there is none.
void gpui_ios_run_demo(void);

void gpui_ios_show_keyboard(void* window_ptr);

void gpui_ios_hide_keyboard(void* window_ptr);

/// text_ptr is an NSString.
void gpui_ios_handle_text_input(void* window_ptr, void* text_ptr);

/// key_code is a UIKeyboardHIDUsage, modifiers are UIKeyModifierFlags.
void gpui_ios_handle_key_event(void* window_ptr, uint32_t key_code, uint32_t modifiers, _Bool is_key_down);

/// url_ptr is an NSString.
void gpui_ios_handle_open_url(void* url_ptr);

#ifdef __cplusplus
}
#endif

#endif /* GPUI_IOS_H */