#ifndef JR_QUEUE_H
#define JR_QUEUE_H

#include <stddef.h>

#ifdef __cplusplus
extern "C"
{
#endif

/*
 * Per-session command queue (single producer / single consumer).
 *
 * The AppKit main thread ENQUEUES mouse/keyboard/IME/clipboard commands and
 * returns immediately; the RDP worker thread drains them and — and only there
 * — calls the FreeRDP APIs (single-owner rule). The queue itself is pure C
 * (pthread only, no FreeRDP/WinPR) so it is testable standalone.
 *
 * Ordering invariants (KI-018 / regression guide §2.1):
 *   - DOWN → MOVE → MOVE → UP must survive queueing and coalescing.
 *   - Consecutive MOUSE_MOVE commands coalesce to the LATEST move.
 *   - Coalescing NEVER crosses a button/key transition.
 *   - On overflow, the OLDEST pending MOUSE_MOVE is evicted (a move is always
 *     safe to drop) instead of dropping a button/key and breaking a pairing.
 *
 * The wake HANDLE lives OUTSIDE this queue (bridge.c owns a WinPR event and
 * SetEvent's it after enqueue) so this file stays FreeRDP-free.
 */

typedef enum
{
	JR_CMD_MOUSE_MOVE = 0,
	JR_CMD_MOUSE_BUTTON,      /* a=button(1..3) b=down c=x d=y   */
	JR_CMD_MOUSE_WHEEL,       /* a=delta b=negative c=hdelta d=hnegative e=x f=y */
	JR_CMD_KEY_SCANCODE,      /* a=down b=repeat c=scancode d=extended */
	JR_CMD_UNICODE_TEXT,      /* owned_utf8                       */
	JR_CMD_LOCAL_CLIPBOARD_TEXT, /* owned_utf8                    */
	JR_CMD_RESIZE,            /* a=width b=height                 */
	JR_CMD_RESET_MODIFIERS    /* no args                          */
} jr_cmd_kind;

typedef struct
{
	jr_cmd_kind kind;
	int a, b, c, d;
	int e, f;        /* JR_CMD_MOUSE_WHEEL x/y (unused otherwise) */
	char* owned_utf8; /* owned by the queue; freed after the drain callback */
} jr_cmd;

typedef struct jr_cmd_queue jr_cmd_queue;

/* Heap-backed queue (opaque). NULL only on allocation failure. */
jr_cmd_queue* jr_cmdq_create(void);
/* Frees every pending owned_utf8, marks destroyed, releases the lock. */
void jr_cmdq_destroy(jr_cmd_queue* q);

/* Enqueue entry points (main thread). Text arguments are copied (strdup). */
void jr_cmdq_enqueue_move(jr_cmd_queue* q, int x, int y);
void jr_cmdq_enqueue_button(jr_cmd_queue* q, int button, int down, int x, int y);
void jr_cmdq_enqueue_wheel(jr_cmd_queue* q, int delta, int negative, int hdelta, int hnegative,
                           int x, int y);
void jr_cmdq_enqueue_scancode(jr_cmd_queue* q, int down, int repeat, int scancode, int extended);
void jr_cmdq_enqueue_unicode(jr_cmd_queue* q, const char* utf8);
void jr_cmdq_enqueue_clipboard(jr_cmd_queue* q, const char* utf8);
void jr_cmdq_enqueue_resize(jr_cmd_queue* q, int w, int h);
void jr_cmdq_enqueue_reset_modifiers(jr_cmd_queue* q);

/* Consumer callback. `cmd` is valid only for the duration of the call; its
 * `owned_utf8` is freed by the queue immediately after the callback returns
 * (copy it with strdup if the worker must retain it). */
typedef void (*jr_cmdq_drain_fn)(jr_cmd* cmd, void* user);

/* Drain all pending commands in FIFO order. Returns the count drained. The
 * callback runs OUTSIDE the queue lock so slow dispatches never block the
 * producer. `cb` may be NULL to just discard + free. */
size_t jr_cmdq_drain(jr_cmd_queue* q, jr_cmdq_drain_fn cb, void* user);

#ifdef __cplusplus
}
#endif

#endif /* JR_QUEUE_H */