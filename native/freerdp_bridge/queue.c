/* Per-session command queue — pure C, pthread only (no FreeRDP/WinPR).
 *
 * Single producer (AppKit main thread) / single consumer (RDP worker). Kept as
 * a contiguous array so coalescing the tail move and evicting the oldest move
 * are simple, and so the widget is unit-testable in isolation (see the Rust
 * FFI test at `src-tauri/src/rdp/queue_ffi_test.rs`).
 */

#include "queue.h"

#include <pthread.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define JR_CMDQ_CAP 4096

struct jr_cmd_queue
{
	pthread_mutex_t lock;
	jr_cmd items[JR_CMDQ_CAP];
	size_t count;
	int destroyed;
	int dropped_warned; /* rate-limit the overflow diagnostic */
};

jr_cmd_queue* jr_cmdq_create(void)
{
	jr_cmd_queue* q = (jr_cmd_queue*)calloc(1, sizeof(jr_cmd_queue));
	if (!q)
		return NULL;
	if (pthread_mutex_init(&q->lock, NULL) != 0)
	{
		free(q);
		return NULL;
	}
	return q;
}

void jr_cmdq_destroy(jr_cmd_queue* q)
{
	if (!q)
		return;
	pthread_mutex_lock(&q->lock);
	q->destroyed = 1;
	for (size_t i = 0; i < q->count; i++)
		free(q->items[i].owned_utf8);
	q->count = 0;
	pthread_mutex_unlock(&q->lock);
	pthread_mutex_destroy(&q->lock);
	free(q);
}

/* Append under lock; on overflow evicts the OLDEST pending MOUSE_MOVE so a
 * button/key pairing is never broken by a dropped boundary event. */
static void push_internal(jr_cmd_queue* q, jr_cmd* c)
{
	if (q->destroyed)
	{
		free(c->owned_utf8);
		return;
	}
	if (q->count < JR_CMDQ_CAP)
	{
		q->items[q->count++] = *c;
		return;
	}

	/* Full: find the oldest MOUSE_MOVE from the front. */
	size_t evict = (size_t)-1;
	for (size_t i = 0; i < q->count; i++)
	{
		if (q->items[i].kind == JR_CMD_MOUSE_MOVE)
		{
			evict = i;
			break;
		}
	}
	if (evict == (size_t)-1)
	{
		/* Nothing safe to evict — drop the new command rather than unbalance a
		 * button/key sequence. */
		if (!q->dropped_warned && c->kind != JR_CMD_MOUSE_MOVE)
		{
			fprintf(stderr,
			        "[jr-input] command queue overflow (%d pending); dropping kind=%d\n",
			        JR_CMDQ_CAP, (int)c->kind);
			q->dropped_warned = 1;
		}
		free(c->owned_utf8);
		return;
	}
	memmove(&q->items[evict], &q->items[evict + 1], (q->count - evict - 1) * sizeof(jr_cmd));
	q->count--;
	q->items[q->count++] = *c;
}

void jr_cmdq_enqueue_move(jr_cmd_queue* q, int x, int y)
{
	jr_cmd c = { .kind = JR_CMD_MOUSE_MOVE, .a = x, .b = y };
	if (!q)
		return;
	pthread_mutex_lock(&q->lock);
	if (q->destroyed)
	{
		pthread_mutex_unlock(&q->lock);
		return;
	}
	/* Coalesce consecutive moves to the LATEST position. Never coalesces across
	 * a button/key transition because the tail kind must be MOUSE_MOVE. */
	if (q->count > 0 && q->items[q->count - 1].kind == JR_CMD_MOUSE_MOVE)
	{
		q->items[q->count - 1].a = x;
		q->items[q->count - 1].b = y;
		pthread_mutex_unlock(&q->lock);
		return;
	}
	if (q->count >= JR_CMDQ_CAP)
	{
		/* Full and tail isn't a move: dropping this MOVE is always safe (the
		 * mouse position is refreshed on the next event anyway). */
		pthread_mutex_unlock(&q->lock);
		return;
	}
	q->items[q->count++] = c;
	pthread_mutex_unlock(&q->lock);
}

void jr_cmdq_enqueue_button(jr_cmd_queue* q, int button, int down, int x, int y)
{
	jr_cmd c = { .kind = JR_CMD_MOUSE_BUTTON, .a = button, .b = down, .c = x, .d = y };
	if (!q)
		return;
	pthread_mutex_lock(&q->lock);
	push_internal(q, &c);
	pthread_mutex_unlock(&q->lock);
}

void jr_cmdq_enqueue_wheel(jr_cmd_queue* q, int delta, int negative, int hdelta, int hnegative,
                           int x, int y)
{
	jr_cmd c = { .kind = JR_CMD_MOUSE_WHEEL,
		     .a = delta,
		     .b = negative,
		     .c = hdelta,
		     .d = hnegative,
		     .e = x,
		     .f = y };
	if (!q)
		return;
	pthread_mutex_lock(&q->lock);
	push_internal(q, &c);
	pthread_mutex_unlock(&q->lock);
}

void jr_cmdq_enqueue_scancode(jr_cmd_queue* q, int down, int repeat, int scancode, int extended)
{
	jr_cmd c = { .kind = JR_CMD_KEY_SCANCODE, .a = down, .b = repeat, .c = scancode, .d = extended };
	if (!q)
		return;
	pthread_mutex_lock(&q->lock);
	push_internal(q, &c);
	pthread_mutex_unlock(&q->lock);
}

static void enqueue_text(jr_cmd_queue* q, jr_cmd_kind kind, const char* utf8)
{
	jr_cmd c = { .kind = kind };
	if (!q)
		return;
	if (utf8)
		c.owned_utf8 = strdup(utf8);
	else
		c.owned_utf8 = strdup("");
	pthread_mutex_lock(&q->lock);
	push_internal(q, &c);
	pthread_mutex_unlock(&q->lock);
}

void jr_cmdq_enqueue_unicode(jr_cmd_queue* q, const char* utf8)
{
	enqueue_text(q, JR_CMD_UNICODE_TEXT, utf8);
}

void jr_cmdq_enqueue_clipboard(jr_cmd_queue* q, const char* utf8)
{
	enqueue_text(q, JR_CMD_LOCAL_CLIPBOARD_TEXT, utf8);
}

void jr_cmdq_enqueue_resize(jr_cmd_queue* q, int w, int h)
{
	jr_cmd c = { .kind = JR_CMD_RESIZE, .a = w, .b = h };
	if (!q)
		return;
	pthread_mutex_lock(&q->lock);
	push_internal(q, &c);
	pthread_mutex_unlock(&q->lock);
}

void jr_cmdq_enqueue_reset_modifiers(jr_cmd_queue* q)
{
	jr_cmd c = { .kind = JR_CMD_RESET_MODIFIERS };
	if (!q)
		return;
	pthread_mutex_lock(&q->lock);
	push_internal(q, &c);
	pthread_mutex_unlock(&q->lock);
}

size_t jr_cmdq_drain(jr_cmd_queue* q, jr_cmdq_drain_fn cb, void* user)
{
	size_t drained = 0;
	if (!q)
		return 0;

	for (;;)
	{
		jr_cmd c;
		c.owned_utf8 = NULL;

		pthread_mutex_lock(&q->lock);
		if (q->count == 0)
		{
			pthread_mutex_unlock(&q->lock);
			break;
		}
		c = q->items[0];
		memmove(&q->items[0], &q->items[1], (q->count - 1) * sizeof(jr_cmd));
		q->count--;
		q->items[q->count].owned_utf8 = NULL; /* clear the vacated tail slot */
		pthread_mutex_unlock(&q->lock);

		if (cb)
			cb(&c, user);
		free(c.owned_utf8);
		drained++;
	}
	return drained;
}