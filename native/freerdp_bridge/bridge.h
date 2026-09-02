#ifndef JR_BRIDGE_H
#define JR_BRIDGE_H

#include <stdint.h>

#ifdef __cplusplus
extern "C"
{
#endif

/* ------------------------------------------------------------------ */
/* Embedded FreeRDP bridge — stable C interface consumed by Rust.      */
/* The FreeRDP/WinPR headers and types are confined to the .c/.m side; */
/* Rust only sees opaque handles and plain C types.                    */
/* ------------------------------------------------------------------ */

typedef struct jr_session jr_session_t;

/* Session lifecycle callbacks (called from the RDP worker thread). */
typedef struct
{
	void* user;
	void (*on_connected)(void* user);
	void (*on_disconnected)(void* user);
	void (*on_frame_updated)(void* user, int32_t x, int32_t y, int32_t w, int32_t h);
	void (*on_desktop_resized)(void* user, int32_t width, int32_t height);
	void (*on_log)(void* user, const char* message);
} jr_session_callbacks_t;

/* Certificate metadata surfaced for a TOFU trust decision. */
typedef struct
{
	const char* host;
	const char* common_name;
	const char* subject;
	const char* issuer;
	const char* fingerprint;
} jr_cert_info_t;

/* Certificate decision callbacks (Rust decides; NEVER auto-accept-changed).
 * verify_certificate: unknown cert → 1 accept+store, 2 session-only, 0 reject.
 * verify_changed_certificate: changed cert → 1/2/0 (default reject). */
typedef struct
{
	void* user;
	int (*verify_certificate)(void* user, const jr_cert_info_t* info);
	int (*verify_changed_certificate)(void* user, const jr_cert_info_t* new_info,
	                                  const jr_cert_info_t* old_info);
} jr_cert_callbacks_t;

typedef struct
{
	const char* host;
	uint16_t port;
	const char* username;
	const char* password;
	int width;
	int height;
	int color_depth; /* 32 → BGRA32 */
} jr_connect_params_t;

/* Session */
jr_session_t* jr_session_create(const jr_connect_params_t* params,
                                const jr_session_callbacks_t* cb,
                                const jr_cert_callbacks_t* cert);
void jr_session_destroy(jr_session_t* s);
int jr_session_connect(jr_session_t* s);    /* blocking; runs the event loop */
int jr_session_disconnect(jr_session_t* s); /* thread-safe stop signal */
int jr_session_get_size(jr_session_t* s, int* width, int* height);
int jr_session_get_framebuffer(jr_session_t* s, const uint8_t** buffer, int* width,
                               int* height, int* stride);
/* Input forwarding. Semantic wrappers over freerdp_input_send_* (the bridge
 * translates to the RDP pointer/keyboard flag constants); safe to call from the AppKit main
 * thread while the session worker thread runs the event loop.
 *   mouse_move:  buttons = bitmask 1=left 2=right 4=middle (held while moving)
 *   mouse_button:button 1=left 2=right 3=middle, down 1/0
 *   mouse_wheel: delta/hdelta in [0,511], negative 1/0 per axis
 *   key:         down 1/0, repeat 1/0 (auto-repeat), scancode = XT make code,
 *                extended 1/0 (E0-prefixed keys: arrows, rctrl, ralt, ...)   */
int jr_session_send_mouse_move(jr_session_t* s, int x, int y, int buttons);
int jr_session_send_mouse_button(jr_session_t* s, int button, int down, int x, int y);
int jr_session_send_mouse_wheel(jr_session_t* s, int delta, int negative, int hdelta,
                                int hnegative, int x, int y);
int jr_session_send_key_scancode(jr_session_t* s, int down, int repeat, int scancode,
                                 int extended);
/* Reset: release every modifier key (LCtrl/RCtrl/LShift/RShift/LAlt/RAlt/
 * LMeta/RMeta). Heals a "stuck Super" on the remote side (KI-023): macOS can
 * swallow the key-up of Command (system shortcuts like Cmd+Tab/Cmd+Q), and
 * xrdp keeps the stale modifier state in its persistent X session across
 * reconnects. Releasing a key that isn't held is a no-op remotely, so this is
 * safe to call on every connect / focus change / input attach. */
int jr_session_reset_keyboard_modifiers(jr_session_t* s);
int jr_session_set_size(jr_session_t* s, int width, int height);
/* Clipboard (text only). set_text offers local text to the remote; the
 * remote->local direction is pushed to the Mac pasteboard automatically. */
int jr_session_set_clipboard_text(jr_session_t* s, const char* utf8);
void jr_clipboard_sync_start(void* session); /* poll Mac pasteboard -> remote */
void jr_clipboard_sync_stop(void);
const char* jr_last_error(jr_session_t* s);

/* Native desktop view (implemented in macos_view.m). */
void* jr_view_create(void);
void jr_view_destroy(void* view);
void jr_view_set_frame(void* view, double x, double y, double width, double height);
void jr_view_add_to_window(void* view, void* ns_window);
/* Like jr_view_add_to_window, but leaves `top_inset` points free at the top
 * (multi-device tab bar, V0.4). Autoresizing keeps the inset on resize. */
void jr_view_add_to_window_inset(void* view, void* ns_window, double top_inset);
void jr_view_remove_from_window(void* view);
void jr_view_set_fill(void* view, uint8_t r, uint8_t g, uint8_t b);
/* Attach a live session for input forwarding (NULL to detach; synchronous). */
void jr_view_attach_input(void* view, void* session);
/* Content-view size of an NSWindow in points (synchronous; 0 on failure). */
void jr_window_content_size(void* ns_window, double* w, double* h);
void jr_view_present_buffer(void* view, const uint8_t* buffer, int width, int height,
                            int stride, int dirty_x, int dirty_y, int dirty_w, int dirty_h);

/* Diagnostics */
const char* jr_freerdp_version(void);

#ifdef __cplusplus
}
#endif

#endif /* JR_BRIDGE_H */