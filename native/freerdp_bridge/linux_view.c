/* Native desktop surface — Linux GTK3.
 *
 * Implements the same C symbols as `macos_view.m` so `bridge.c`, `queue.c`,
 * `session.rs` and `native_view.rs` stay platform-agnostic:
 *
 *   jr_view_create/destroy/live_count/set_frame/set_fill/
 *   add_to_window(_inset)/remove_from_window/attach_input/present_buffer
 *   jr_window_content_size / jr_window_safe_area_top
 *   jr_clip_is_owner / jr_clip_generation / jr_mac_clip_set /
 *   jr_clipboard_sync_start / jr_clipboard_sync_stop
 *
 * plus the Linux-only bootstrap `jr_linux_init(GtkWindow*, GtkWidget* vbox)`.
 *
 * Threading: GTK is main-thread-only. Every entry point below may be called
 * from the Rust command threads or the FreeRDP worker, so all widget work is
 * marshalled with `jr_run_on_main_sync` (GCond + g_main_context_invoke_full)
 * or posted with `jr_run_on_main_async`. `present_buffer` copies the BGRX
 * framebuffer *before* returning because FreeRDP reuses it as soon as the
 * paint callback returns.
 *
 * Embedding: `jr_linux_init` reparents the WebKitWebView into a GtkOverlay
 * inside Tauri's `default_vbox`, then each session mounts a GtkDrawingArea as
 * an overlay child with `margin_top` = tab-bar inset. The webview keeps its
 * full-window size; the drawing area covers y >= inset only.
 *
 * Byte order: bridge.c uses PIXEL_FORMAT_BGRX32 (B,G,R,X in memory); cairo's
 * CAIRO_FORMAT_RGB24 reads the same layout (X ignored), so the framebuffer can
 * be wrapped without conversion.
 */

#include "bridge.h"

#include <gtk/gtk.h>

#include <stdatomic.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

/* ------------------------------------------------------------------ */
/* Main-thread marshalling                                             */
/* ------------------------------------------------------------------ */

static gboolean jr_on_main_thread(void)
{
	return g_main_context_is_owner(g_main_context_default());
}

typedef struct
{
	GSourceFunc fn;
	gpointer data;
	gboolean done;
	gboolean result;
	GMutex lock;
	GCond cond;
} JrSync;

static gboolean jr_sync_trampoline(gpointer p)
{
	JrSync* s = (JrSync*)p;
	s->result = s->fn(s->data) ? TRUE : FALSE;
	g_mutex_lock(&s->lock);
	s->done = TRUE;
	g_cond_signal(&s->cond);
	g_mutex_unlock(&s->lock);
	return G_SOURCE_REMOVE;
}

/* Runs `fn` on the GTK main thread and waits for it. Called from the main
 * thread this executes inline (same contract as AppKit dispatch_sync). */
static gboolean jr_run_on_main_sync(GSourceFunc fn, gpointer data)
{
	JrSync s;

	if (jr_on_main_thread())
		return fn(data);

	memset(&s, 0, sizeof(s));
	s.fn = fn;
	s.data = data;
	g_mutex_init(&s.lock);
	g_cond_init(&s.cond);
	g_main_context_invoke_full(g_main_context_default(), G_PRIORITY_DEFAULT, jr_sync_trampoline,
	                           &s, NULL);
	g_mutex_lock(&s.lock);
	while (!s.done)
		g_cond_wait(&s.cond, &s.lock);
	g_mutex_unlock(&s.lock);
	g_mutex_clear(&s.lock);
	g_cond_clear(&s.cond);
	return s.result;
}

static void jr_run_on_main_async(GSourceFunc fn, gpointer data, GDestroyNotify notify)
{
	g_main_context_invoke_full(g_main_context_default(), G_PRIORITY_DEFAULT, fn, data, notify);
}

/* ------------------------------------------------------------------ */
/* Globals (all GTK-touching state is main-thread confined)            */
/* ------------------------------------------------------------------ */

static GtkWidget* g_window = NULL;  /* top-level GtkApplicationWindow */
static GtkWidget* g_vbox = NULL;    /* Tauri default_vbox               */
static GtkWidget* g_webview = NULL; /* WebKitWebView reparented below   */
static GtkWidget* g_overlay = NULL; /* GtkOverlay inside g_vbox         */

static _Atomic int g_content_w = 0;
static _Atomic int g_content_h = 0;
static _Atomic(int) g_view_live = 0;

/* Live views; main-thread only. Used by the window-activation sweep. */
static GPtrArray* g_views = NULL;

typedef struct JrView
{
	GtkWidget* da; /* owned ref (g_object_ref_sink) */
	_Atomic int alive;
	_Atomic int mounted;
	_Atomic(jr_session_t*) input;
	_Atomic int fill_r;
	_Atomic int fill_g;
	_Atomic int fill_b;

	GMutex frame_lock;
	uint8_t* frame;
	int frame_w;
	int frame_h;
	int frame_stride;

	GtkIMContext* ime;
	gboolean preedit_active;
	gboolean committed_this_key;

	/* per-hardware-keycode route of the last press: 0 none, 1 scancode,
	 * 2 input method. The release must follow the same route (KI-034). */
	guint8 key_route[256];
	int pressed_buttons; /* bitmask 1=left 2=right 4=middle (local mirror) */
	int margin_top;
} JrView;

/* ------------------------------------------------------------------ */
/* Row 0: window bootstrap                                             */
/* ------------------------------------------------------------------ */

static void jr_overlay_size_allocate(GtkWidget* widget, GdkRectangle* alloc, gpointer data)
{
	(void)widget;
	(void)data;
	if (alloc)
	{
		g_atomic_int_set(&g_content_w, alloc->width);
		g_atomic_int_set(&g_content_h, alloc->height);
	}
}

static void jr_keyboard_sweep(JrView* v);

static void jr_window_active_changed(GObject* obj, GParamSpec* pspec, gpointer data)
{
	(void)obj;
	(void)pspec;
	(void)data;
	if (!g_window)
		return;
	if (!gtk_window_is_active(GTK_WINDOW(g_window)) && g_views)
	{
		for (guint i = 0; i < g_views->len; i++)
			jr_keyboard_sweep((JrView*)g_ptr_array_index(g_views, i));
	}
}

/* Called from Tauri `.setup()` on the GTK main thread, before the event loop
 * starts. Returns 0 on success, negative on failure. */
int jr_linux_init(void* window, void* vbox)
{
	GList* kids = NULL;
	GList* l = NULL;

	if (!jr_on_main_thread())
	{
		fprintf(stderr, "[jr-linux] init called off the GTK main thread\n");
		return -1;
	}
	if (!window || !vbox)
		return -2;
	if (!GTK_IS_CONTAINER((GtkWidget*)vbox))
	{
		fprintf(stderr, "[jr-linux] default_vbox is not a GTK container\n");
		return -3;
	}
	g_window = GTK_WIDGET(window);
	g_vbox = GTK_WIDGET(vbox);

	kids = gtk_container_get_children(GTK_CONTAINER(g_vbox));
	for (l = kids; l; l = l->next)
	{
		const char* type_name = G_OBJECT_TYPE_NAME(l->data);
		if (g_strcmp0(type_name, "WebKitWebView") == 0 ||
		    g_str_has_suffix(type_name, "WebKitWebView"))
		{
			g_webview = GTK_WIDGET(l->data);
			break;
		}
	}
	g_list_free(kids);
	if (!g_webview)
	{
		fprintf(stderr, "[jr-linux] WebKitWebView not found in default_vbox\n");
		return -4;
	}

	/* Reparent: vbox -> overlay -> webview. Keep the webview's visible state;
	 * only its parent changes (Tauri still owns show/hide and event routing). */
	g_overlay = gtk_overlay_new();
	g_object_ref(g_webview);
	gtk_container_remove(GTK_CONTAINER(g_vbox), g_webview);
	gtk_container_add(GTK_CONTAINER(g_overlay), g_webview);
	gtk_box_pack_start(GTK_BOX(g_vbox), g_overlay, TRUE, TRUE, 0);
	g_object_unref(g_webview);
	gtk_widget_show(g_overlay);

	g_signal_connect(g_overlay, "size-allocate", G_CALLBACK(jr_overlay_size_allocate), NULL);
	g_signal_connect(g_window, "notify::is-active", G_CALLBACK(jr_window_active_changed), NULL);
	g_views = g_ptr_array_new();

	fprintf(stderr, "[jr-linux] overlay installed (webview reparented)\n");
	return 0;
}

void* jr_linux_surface_handle(void)
{
	return g_overlay;
}

/* ------------------------------------------------------------------ */
/* View lifecycle                                                      */
/* ------------------------------------------------------------------ */

static void jr_ime_commit(GtkIMContext* imc, const gchar* text, gpointer data);
static void jr_ime_preedit_changed(GtkIMContext* imc, gpointer data);
static gboolean jr_ime_retrieve_surrounding(GtkIMContext* imc, gpointer data);
static gboolean jr_ime_delete_surrounding(GtkIMContext* imc, gint offset, gint n_chars,
                                          gpointer data);
static gboolean jr_da_draw(GtkWidget* widget, cairo_t* cr, gpointer data);
static gboolean jr_da_motion(GtkWidget* widget, GdkEventMotion* event, gpointer data);
static gboolean jr_da_button(GtkWidget* widget, GdkEventButton* event, gpointer data);
static gboolean jr_da_scroll(GtkWidget* widget, GdkEventScroll* event, gpointer data);
static gboolean jr_da_key_press(GtkWidget* widget, GdkEventKey* event, gpointer data);
static gboolean jr_da_key_release(GtkWidget* widget, GdkEventKey* event, gpointer data);
static gboolean jr_da_focus_in(GtkWidget* widget, GdkEventFocus* event, gpointer data);
static gboolean jr_da_focus_out(GtkWidget* widget, GdkEventFocus* event, gpointer data);
static void jr_da_realize(GtkWidget* widget, gpointer data);
static void jr_da_unrealize(GtkWidget* widget, gpointer data);

static gboolean jr_view_create_on_main(gpointer data)
{
	JrView* v = (JrView*)data;
	GtkWidget* da = gtk_drawing_area_new();

	v->da = da;
	gtk_widget_set_can_focus(da, TRUE);
	gtk_widget_add_events(da, GDK_POINTER_MOTION_MASK | GDK_BUTTON_PRESS_MASK |
	                              GDK_BUTTON_RELEASE_MASK | GDK_SCROLL_MASK |
	                              GDK_SMOOTH_SCROLL_MASK | GDK_KEY_PRESS_MASK |
	                              GDK_KEY_RELEASE_MASK | GDK_FOCUS_CHANGE_MASK |
	                              GDK_ENTER_NOTIFY_MASK | GDK_LEAVE_NOTIFY_MASK);
	gtk_widget_set_hexpand(da, TRUE);
	gtk_widget_set_vexpand(da, TRUE);
	gtk_widget_set_halign(da, GTK_ALIGN_FILL);
	gtk_widget_set_valign(da, GTK_ALIGN_FILL);

	g_signal_connect(da, "draw", G_CALLBACK(jr_da_draw), v);
	g_signal_connect(da, "motion-notify-event", G_CALLBACK(jr_da_motion), v);
	g_signal_connect(da, "button-press-event", G_CALLBACK(jr_da_button), v);
	g_signal_connect(da, "button-release-event", G_CALLBACK(jr_da_button), v);
	g_signal_connect(da, "scroll-event", G_CALLBACK(jr_da_scroll), v);
	g_signal_connect(da, "key-press-event", G_CALLBACK(jr_da_key_press), v);
	g_signal_connect(da, "key-release-event", G_CALLBACK(jr_da_key_release), v);
	g_signal_connect(da, "focus-in-event", G_CALLBACK(jr_da_focus_in), v);
	g_signal_connect(da, "focus-out-event", G_CALLBACK(jr_da_focus_out), v);
	g_signal_connect(da, "realize", G_CALLBACK(jr_da_realize), v);
	g_signal_connect(da, "unrealize", G_CALLBACK(jr_da_unrealize), v);

	v->ime = gtk_im_multicontext_new();
	gtk_im_context_set_use_preedit(v->ime, TRUE);
	g_signal_connect(v->ime, "commit", G_CALLBACK(jr_ime_commit), v);
	g_signal_connect(v->ime, "preedit-changed", G_CALLBACK(jr_ime_preedit_changed), v);
	g_signal_connect(v->ime, "retrieve-surrounding", G_CALLBACK(jr_ime_retrieve_surrounding), v);
	g_signal_connect(v->ime, "delete-surrounding", G_CALLBACK(jr_ime_delete_surrounding), v);

	/* Own one reference; the overlay adds its own while mounted. */
	g_object_ref_sink(da);
	if (g_views)
		g_ptr_array_add(g_views, v);
	return TRUE;
}

void* jr_view_create(void)
{
	JrView* v = (JrView*)calloc(1, sizeof(JrView));
	if (!v)
		return NULL;
	atomic_init(&v->alive, 1);
	atomic_init(&v->mounted, 0);
	atomic_init(&v->input, NULL);
	atomic_init(&v->fill_r, 0x1e);
	atomic_init(&v->fill_g, 0x20);
	atomic_init(&v->fill_b, 0x26);
	g_mutex_init(&v->frame_lock);
	if (!jr_run_on_main_sync(jr_view_create_on_main, v) || !v->da)
	{
		g_mutex_clear(&v->frame_lock);
		free(v);
		return NULL;
	}
	fprintf(stderr, "[jr-view] create live=%d\n", atomic_fetch_add(&g_view_live, 1) + 1);
	return v;
}

int jr_view_live_count(void)
{
	return atomic_load(&g_view_live);
}

static gboolean jr_view_destroy_on_main(gpointer data)
{
	JrView* v = (JrView*)data;
	GtkWidget* da = v->da;

	if (g_views)
		g_ptr_array_remove(g_views, v);
	if (!da)
		return TRUE;
	if (v->ime)
	{
		gtk_im_context_focus_out(v->ime);
		gtk_im_context_reset(v->ime);
		g_object_unref(v->ime);
		v->ime = NULL;
	}
	if (gtk_widget_get_parent(da))
	{
		GtkWidget* parent = gtk_widget_get_parent(da);
		gtk_container_remove(GTK_CONTAINER(parent), da);
	}
	if (gtk_widget_has_focus(da) && g_webview)
		gtk_widget_grab_focus(g_webview);
	gtk_widget_destroy(da);
	g_object_unref(da);
	v->da = NULL;
	return TRUE;
}

void jr_view_destroy(void* handle)
{
	JrView* v = (JrView*)handle;
	if (!v)
		return;
	atomic_store(&v->alive, 0);
	jr_run_on_main_sync(jr_view_destroy_on_main, v);
	g_mutex_lock(&v->frame_lock);
	free(v->frame);
	v->frame = NULL;
	g_mutex_unlock(&v->frame_lock);
	g_mutex_clear(&v->frame_lock);
	fprintf(stderr, "[jr-view] destroy live=%d\n", atomic_fetch_sub(&g_view_live, 1) - 1);
	free(v);
}

static gboolean jr_view_mount_on_main(gpointer data)
{
	JrView* v = (JrView*)data;
	if (!g_overlay || !v->da)
		return FALSE;
	if (!gtk_widget_get_parent(v->da))
		gtk_overlay_add_overlay(GTK_OVERLAY(g_overlay), v->da);
	gtk_widget_set_margin_top(v->da, MAX(v->margin_top, 0));
	gtk_widget_set_margin_bottom(v->da, 0);
	gtk_widget_set_margin_start(v->da, 0);
	gtk_widget_set_margin_end(v->da, 0);
	gtk_widget_show(v->da);
	gtk_widget_grab_focus(v->da);
	atomic_store(&v->mounted, 1);
	fprintf(stderr, "[jr-view] mount margin_top=%d\n", v->margin_top);
	return TRUE;
}

void jr_view_add_to_window(void* handle, void* window)
{
	(void)window;
	if (!handle)
		return;
	((JrView*)handle)->margin_top = 0;
	jr_run_on_main_sync(jr_view_mount_on_main, handle);
}

void jr_view_add_to_window_inset(void* handle, void* window, double top_inset)
{
	JrView* v = (JrView*)handle;
	(void)window;
	if (!v)
		return;
	v->margin_top = (int)(top_inset > 0 ? top_inset + 0.5 : 0);
	jr_run_on_main_sync(jr_view_mount_on_main, v);
}

static gboolean jr_view_unmount_on_main(gpointer data)
{
	JrView* v = (JrView*)data;
	if (!v->da)
		return FALSE;
	if (gtk_widget_get_parent(v->da))
	{
		GtkWidget* parent = gtk_widget_get_parent(v->da);
		gtk_container_remove(GTK_CONTAINER(parent), v->da);
	}
	gtk_widget_hide(v->da);
	atomic_store(&v->mounted, 0);
	if (gtk_widget_has_focus(v->da) && g_webview)
		gtk_widget_grab_focus(g_webview);
	fprintf(stderr, "[jr-view] unmount\n");
	return TRUE;
}

void jr_view_remove_from_window(void* handle)
{
	if (!handle)
		return;
	jr_run_on_main_sync(jr_view_unmount_on_main, handle);
}

void jr_view_set_frame(void* handle, double x, double y_top, double w, double h)
{
	/* Geometry is owned by the GtkOverlay + margins; nothing to do. */
	(void)handle;
	(void)x;
	(void)y_top;
	(void)w;
	(void)h;
}

static gboolean jr_view_set_fill_on_main(gpointer data)
{
	JrView* v = (JrView*)data;
	if (v->da)
		gtk_widget_queue_draw(v->da);
	return TRUE;
}

void jr_view_set_fill(void* handle, uint8_t r, uint8_t g, uint8_t b)
{
	JrView* v = (JrView*)handle;
	if (!v)
		return;
	atomic_store(&v->fill_r, r);
	atomic_store(&v->fill_g, g);
	atomic_store(&v->fill_b, b);
	jr_run_on_main_sync(jr_view_set_fill_on_main, v);
}

/* ------------------------------------------------------------------ */
/* Input attach / focus                                                */
/* ------------------------------------------------------------------ */

static void jr_keyboard_sweep(JrView* v)
{
	jr_session_t* s;

	if (!v)
		return;
	s = atomic_load(&v->input);
	if (!s)
		return;
	memset(v->key_route, 0, sizeof(v->key_route));
	v->pressed_buttons = 0;
	if (jr_session_enqueue_reset_modifiers(s) == 0)
		fprintf(stderr, "[jr-input] keyboard modifier sweep sent\n");
}

static gboolean jr_view_attach_on_main(gpointer data)
{
	JrView* v = (JrView*)data;
	jr_session_t* s = atomic_load(&v->input);

	v->pressed_buttons = 0;
	memset(v->key_route, 0, sizeof(v->key_route));
	v->preedit_active = FALSE;
	if (v->ime)
		gtk_im_context_reset(v->ime);
	if (s)
	{
		if (v->da)
			gtk_widget_grab_focus(v->da);
		if (jr_session_enqueue_reset_modifiers(s) == 0)
			fprintf(stderr, "[jr-input] attach: keyboard modifier reset sent\n");
	}
	else if (v->da && gtk_widget_has_focus(v->da) && g_webview)
	{
		gtk_widget_grab_focus(g_webview);
	}
	return TRUE;
}

void jr_view_attach_input(void* handle, void* session)
{
	JrView* v = (JrView*)handle;
	if (!v)
		return;
	atomic_store(&v->input, (jr_session_t*)session);
	/* Synchronous: detach must complete before the session is destroyed. */
	jr_run_on_main_sync(jr_view_attach_on_main, v);
}

/* ------------------------------------------------------------------ */
/* Rendering                                                           */
/* ------------------------------------------------------------------ */

static gboolean jr_da_draw(GtkWidget* widget, cairo_t* cr, gpointer data)
{
	JrView* v = (JrView*)data;
	int cw = gtk_widget_get_allocated_width(widget);
	int ch = gtk_widget_get_allocated_height(widget);

	cairo_set_source_rgb(cr, atomic_load(&v->fill_r) / 255.0, atomic_load(&v->fill_g) / 255.0,
	                     atomic_load(&v->fill_b) / 255.0);
	cairo_paint(cr);

	g_mutex_lock(&v->frame_lock);
	if (v->frame && v->frame_w > 0 && v->frame_h > 0 && v->frame_stride > 0 && cw > 0 &&
	    ch > 0)
	{
		double scale = MIN((double)cw / (double)v->frame_w, (double)ch / (double)v->frame_h);
		if (scale > 0)
		{
			double dw = (double)v->frame_w * scale;
			double dh = (double)v->frame_h * scale;
			double ox = ((double)cw - dw) / 2.0;
			double oy = ((double)ch - dh) / 2.0;
			/* RGB24 wraps the FreeRDP BGRX32 framebuffer directly (X ignored). */
			cairo_surface_t* surface = cairo_image_surface_create_for_data(
			    v->frame, CAIRO_FORMAT_RGB24, v->frame_w, v->frame_h, v->frame_stride);
			if (cairo_surface_status(surface) == CAIRO_STATUS_SUCCESS)
			{
				cairo_save(cr);
				cairo_translate(cr, ox, oy);
				cairo_scale(cr, scale, scale);
				cairo_set_source_surface(cr, surface, 0, 0);
				cairo_pattern_set_filter(cairo_get_source(cr), CAIRO_FILTER_BILINEAR);
				cairo_paint(cr);
				cairo_restore(cr);
			}
			cairo_surface_destroy(surface);
		}
	}
	g_mutex_unlock(&v->frame_lock);
	return FALSE;
}

static gboolean jr_present_queue_draw(gpointer data)
{
	gtk_widget_queue_draw(GTK_WIDGET(data));
	return G_SOURCE_REMOVE;
}

void jr_view_present_buffer(void* handle, const uint8_t* buffer, int width, int height,
                            int stride, int dirty_x, int dirty_y, int dirty_w, int dirty_h)
{
	JrView* v = (JrView*)handle;
	size_t bytes;
	uint8_t* copy;

	(void)dirty_x;
	(void)dirty_y;
	(void)dirty_w;
	(void)dirty_h; /* full-frame present (same policy as macos_view.m) */

	if (!v || atomic_load(&v->alive) == 0)
		return;
	if (!buffer || width <= 0 || height <= 0 || stride <= 0)
		return;

	/* Copy out of the FreeRDP-owned buffer before it is reused. */
	bytes = (size_t)stride * (size_t)height;
	copy = (uint8_t*)malloc(bytes);
	if (!copy)
		return;
	memcpy(copy, buffer, bytes);

	g_mutex_lock(&v->frame_lock);
	free(v->frame);
	v->frame = copy;
	v->frame_w = width;
	v->frame_h = height;
	v->frame_stride = stride;
	g_mutex_unlock(&v->frame_lock);

	if (v->da)
	{
		/* The callback only needs the widget; its ref keeps it alive across a
		 * concurrent destroy. Coalesced by gtk_widget_queue_draw. */
		g_object_ref(v->da);
		jr_run_on_main_async(jr_present_queue_draw, v->da, g_object_unref);
	}
}

/* ------------------------------------------------------------------ */
/* Coordinate mapping                                                  */
/* ------------------------------------------------------------------ */

static gboolean jr_map_point(JrView* v, gdouble lx, gdouble ly, int* out_x, int* out_y)
{
	jr_session_t* s = atomic_load(&v->input);
	int dw = 0;
	int dh = 0;
	int cw;
	int ch;
	double scale;
	double ox;
	double oy;
	double x;
	double y;

	if (!s || !v->da)
		return FALSE;
	if (jr_session_get_size(s, &dw, &dh) != 0 || dw <= 0 || dh <= 0)
		return FALSE;
	cw = gtk_widget_get_allocated_width(v->da);
	ch = gtk_widget_get_allocated_height(v->da);
	if (cw <= 0 || ch <= 0)
		return FALSE;
	scale = MIN((double)cw / (double)dw, (double)ch / (double)dh);
	if (scale <= 0)
		return FALSE;
	ox = ((double)cw - (double)dw * scale) / 2.0;
	oy = ((double)ch - (double)dh * scale) / 2.0;
	/* GTK and RDP both use top-left origin: no Y flip here. */
	x = (lx - ox) / scale;
	y = (ly - oy) / scale;
	if (x < 0)
		x = 0;
	if (x > dw - 1)
		x = dw - 1;
	if (y < 0)
		y = 0;
	if (y > dh - 1)
		y = dh - 1;
	*out_x = (int)x;
	*out_y = (int)y;
	return TRUE;
}

/* ------------------------------------------------------------------ */
/* Mouse                                                               */
/* ------------------------------------------------------------------ */

static gboolean jr_da_motion(GtkWidget* widget, GdkEventMotion* event, gpointer data)
{
	JrView* v = (JrView*)data;
	jr_session_t* s = atomic_load(&v->input);
	int x = 0;
	int y = 0;

	(void)widget;
	if (!s)
		return FALSE;
	if (jr_map_point(v, event->x, event->y, &x, &y))
		jr_session_enqueue_mouse_move(s, x, y);
	return TRUE;
}

static gboolean jr_da_button(GtkWidget* widget, GdkEventButton* event, gpointer data)
{
	JrView* v = (JrView*)data;
	jr_session_t* s = atomic_load(&v->input);
	int x = 0;
	int y = 0;
	int down;
	int button;
	int bit;

	if (!s)
		return FALSE;
	if (event->type == GDK_BUTTON_PRESS)
		gtk_widget_grab_focus(widget);
	if (!jr_map_point(v, event->x, event->y, &x, &y))
		return TRUE;

	down = (event->type == GDK_BUTTON_PRESS) ? 1 : 0;
	/* GDK: 1=left, 2=middle, 3=right. bridge.c expects 1=left, 2=right,
	 * 3=middle (the macOS NSEvent numbering) — swap 2 and 3. */
	switch (event->button)
	{
		case 1:
			button = 1;
			bit = 1;
			break;
		case 2:
			button = 3;
			bit = 4;
			break;
		case 3:
			button = 2;
			bit = 2;
			break;
		default:
			return FALSE;
	}
	if (down)
		v->pressed_buttons |= bit;
	else
		v->pressed_buttons &= ~bit;
	jr_session_enqueue_mouse_button(s, button, down, x, y);
	return TRUE;
}

static gboolean jr_da_scroll(GtkWidget* widget, GdkEventScroll* event, gpointer data)
{
	JrView* v = (JrView*)data;
	jr_session_t* s = atomic_load(&v->input);
	int x = 0;
	int y = 0;
	gdouble dx = 0.0;
	gdouble dy = 0.0;
	int dxi;
	int dyi;
	int hnegative = 0;
	int vnegative = 0;

	(void)widget;
	if (!s)
		return FALSE;
	if (!jr_map_point(v, event->x, event->y, &x, &y))
		return TRUE;

	if (!gdk_event_get_scroll_deltas((GdkEvent*)event, &dx, &dy))
	{
		switch (event->direction)
		{
			case GDK_SCROLL_UP:
				dy = 1.0;
				break;
			case GDK_SCROLL_DOWN:
				dy = -1.0;
				break;
			case GDK_SCROLL_LEFT:
				dx = -1.0;
				break;
			case GDK_SCROLL_RIGHT:
				dx = 1.0;
				break;
			default:
				return TRUE;
		}
		dx *= 120.0; /* discrete notch -> RDP WHEEL_DELTA */
		dy *= 120.0;
	}
	else
	{
		dx *= 3.0; /* smooth/trackpad deltas -> wheel units */
		dy *= 3.0;
	}
	dyi = (int)CLAMP(dy, -511.0, 511.0);
	dxi = (int)CLAMP(dx, -511.0, 511.0);
	if (dyi == 0 && dxi == 0)
		return TRUE;
	if (dyi < 0)
		vnegative = 1;
	if (dxi < 0)
		hnegative = 1;
	jr_session_enqueue_mouse_wheel(s, abs(dyi), vnegative, abs(dxi), hnegative, x, y);
	return TRUE;
}

/* ------------------------------------------------------------------ */
/* Keyboard: evdev -> XT (PS/2 set 1)                                  */
/* ------------------------------------------------------------------ */

typedef struct
{
	unsigned char sc;
	unsigned char ext;
} JrXtCode;

/* GDK hardware_keycode == evdev keycode + 8 on both X11 and Wayland (xkb
 * convention). `evdev` here is already hardware_keycode - 8. */
static JrXtCode jr_evdev_to_xt(unsigned evdev)
{
	JrXtCode k = {0, 0};
	switch (evdev)
	{
		case 1: k.sc = 0x01; break; /* Esc */
		case 2: k.sc = 0x02; break;
		case 3: k.sc = 0x03; break;
		case 4: k.sc = 0x04; break;
		case 5: k.sc = 0x05; break;
		case 6: k.sc = 0x06; break;
		case 7: k.sc = 0x07; break;
		case 8: k.sc = 0x08; break;
		case 9: k.sc = 0x09; break;
		case 10: k.sc = 0x0A; break;
		case 11: k.sc = 0x0B; break;
		case 12: k.sc = 0x0C; break; /* - */
		case 13: k.sc = 0x0D; break; /* = */
		case 14: k.sc = 0x0E; break; /* Backspace */
		case 15: k.sc = 0x0F; break; /* Tab */
		case 16: k.sc = 0x10; break;
		case 17: k.sc = 0x11; break;
		case 18: k.sc = 0x12; break;
		case 19: k.sc = 0x13; break;
		case 20: k.sc = 0x14; break;
		case 21: k.sc = 0x15; break;
		case 22: k.sc = 0x16; break;
		case 23: k.sc = 0x17; break;
		case 24: k.sc = 0x18; break;
		case 25: k.sc = 0x19; break;
		case 26: k.sc = 0x1A; break; /* [ */
		case 27: k.sc = 0x1B; break; /* ] */
		case 28: k.sc = 0x1C; break; /* Enter */
		case 29: k.sc = 0x1D; break; /* LCtrl */
		case 30: k.sc = 0x1E; break;
		case 31: k.sc = 0x1F; break;
		case 32: k.sc = 0x20; break;
		case 33: k.sc = 0x21; break;
		case 34: k.sc = 0x22; break;
		case 35: k.sc = 0x23; break;
		case 36: k.sc = 0x24; break;
		case 37: k.sc = 0x25; break;
		case 38: k.sc = 0x26; break;
		case 39: k.sc = 0x27; break; /* ; */
		case 40: k.sc = 0x28; break; /* ' */
		case 41: k.sc = 0x29; break; /* ` */
		case 42: k.sc = 0x2A; break; /* LShift */
		case 43: k.sc = 0x2B; break; /* \ */
		case 44: k.sc = 0x2C; break;
		case 45: k.sc = 0x2D; break;
		case 46: k.sc = 0x2E; break;
		case 47: k.sc = 0x2F; break;
		case 48: k.sc = 0x30; break;
		case 49: k.sc = 0x31; break;
		case 50: k.sc = 0x32; break;
		case 51: k.sc = 0x33; break; /* , */
		case 52: k.sc = 0x34; break; /* . */
		case 53: k.sc = 0x35; break; /* / */
		case 54: k.sc = 0x36; break; /* RShift */
		case 55: k.sc = 0x37; break; /* KP * */
		case 56: k.sc = 0x38; break; /* LAlt */
		case 57: k.sc = 0x39; break; /* Space */
		case 58: k.sc = 0x3A; break; /* CapsLock */
		case 59: k.sc = 0x3B; break; /* F1 */
		case 60: k.sc = 0x3C; break; /* F2 */
		case 61: k.sc = 0x3D; break; /* F3 */
		case 62: k.sc = 0x3E; break; /* F4 */
		case 63: k.sc = 0x3F; break; /* F5 */
		case 64: k.sc = 0x40; break; /* F6 */
		case 65: k.sc = 0x41; break; /* F7 */
		case 66: k.sc = 0x42; break; /* F8 */
		case 67: k.sc = 0x43; break; /* F9 */
		case 68: k.sc = 0x44; break; /* F10 */
		case 69: k.sc = 0x45; break; /* NumLock */
		case 70: k.sc = 0x46; break; /* ScrollLock */
		case 71: k.sc = 0x47; break; /* KP 7 */
		case 72: k.sc = 0x48; break; /* KP 8 */
		case 73: k.sc = 0x49; break; /* KP 9 */
		case 74: k.sc = 0x4A; break; /* KP - */
		case 75: k.sc = 0x4B; break; /* KP 4 */
		case 76: k.sc = 0x4C; break; /* KP 5 */
		case 77: k.sc = 0x4D; break; /* KP 6 */
		case 78: k.sc = 0x4E; break; /* KP + */
		case 79: k.sc = 0x4F; break; /* KP 1 */
		case 80: k.sc = 0x50; break; /* KP 2 */
		case 81: k.sc = 0x51; break; /* KP 3 */
		case 82: k.sc = 0x52; break; /* KP 0 */
		case 83: k.sc = 0x53; break; /* KP . */
		case 87: k.sc = 0x57; break; /* F11 */
		case 88: k.sc = 0x58; break; /* F12 */
		case 96: k.sc = 0x1C; k.ext = 1; break; /* KP Enter */
		case 97: k.sc = 0x1D; k.ext = 1; break; /* RCtrl */
		case 98: k.sc = 0x35; k.ext = 1; break; /* KP / */
		case 99: k.sc = 0x37; k.ext = 1; break; /* PrintScreen */
		case 100: k.sc = 0x38; k.ext = 1; break; /* RAlt / AltGr */
		case 102: k.sc = 0x47; k.ext = 1; break; /* Home */
		case 103: k.sc = 0x48; k.ext = 1; break; /* Up */
		case 104: k.sc = 0x49; k.ext = 1; break; /* PageUp */
		case 105: k.sc = 0x4B; k.ext = 1; break; /* Left */
		case 106: k.sc = 0x4D; k.ext = 1; break; /* Right */
		case 107: k.sc = 0x4F; k.ext = 1; break; /* End */
		case 108: k.sc = 0x50; k.ext = 1; break; /* Down */
		case 109: k.sc = 0x51; k.ext = 1; break; /* PageDown */
		case 110: k.sc = 0x52; k.ext = 1; break; /* Insert */
		case 111: k.sc = 0x53; k.ext = 1; break; /* Delete */
		case 125: k.sc = 0x5B; k.ext = 1; break; /* LSuper / Win */
		case 126: k.sc = 0x5C; k.ext = 1; break; /* RSuper / Win */
		case 127: k.sc = 0x5D; k.ext = 1; break; /* Menu / Compose */
		default: break;
	}
	return k;
}

/* Text keys go through the input method (layout-aware, IME/dead keys);
 * navigation/function/modifier keys go straight to the scancode path. */
static gboolean jr_evdev_is_text(unsigned evdev)
{
	if (evdev >= 2 && evdev <= 13)
		return TRUE; /* digits 1..0 and - = */
	if (evdev >= 16 && evdev <= 27)
		return TRUE; /* QWERTY row and [ ] */
	if (evdev >= 30 && evdev <= 41)
		return TRUE; /* ASDF row, ; ' ` */
	if (evdev >= 43 && evdev <= 53)
		return TRUE; /* \ ZXCV row, , . / */
	if (evdev == 57)
		return TRUE; /* Space */
	return FALSE;
}

/* Keys the IME may swallow while composing; once the composition committed we
 * replay the key as a self-contained press+release (macOS doCommandBySelector). */
static gboolean jr_evdev_is_command_key(unsigned evdev)
{
	switch (evdev)
	{
		case 1:   /* Esc */
		case 14:  /* Backspace */
		case 15:  /* Tab */
		case 28:  /* Enter */
		case 96:  /* KP Enter */
		case 102: /* Home */
		case 103: /* Up */
		case 104: /* PageUp */
		case 105: /* Left */
		case 106: /* Right */
		case 107: /* End */
		case 108: /* Down */
		case 109: /* PageDown */
		case 110: /* Insert */
		case 111: /* Delete */
			return TRUE;
		default:
			return FALSE;
	}
}

static void jr_send_scancode(JrView* v, jr_session_t* s, unsigned evdev, int down, int repeat)
{
	JrXtCode k = jr_evdev_to_xt(evdev);
	if (!k.sc)
		return;
	jr_session_enqueue_key_scancode(s, down ? 1 : 0, down && repeat ? 1 : 0, k.sc, k.ext);
}

static gboolean jr_da_key_press(GtkWidget* widget, GdkEventKey* event, gpointer data)
{
	JrView* v = (JrView*)data;
	jr_session_t* s = atomic_load(&v->input);
	unsigned hw = event->hardware_keycode;
	unsigned evdev;
	JrXtCode xt;
	gboolean use_ime;
	guint8 previous_route;
	gboolean consumed;
	gboolean shortcut;

	(void)widget;
	if (!s)
		return FALSE;
	if (hw < 8 || hw > 255)
		return FALSE;
	evdev = hw - 8;
	xt = jr_evdev_to_xt(evdev);
	if (!xt.sc)
		return FALSE;

	shortcut = (event->state & (GDK_CONTROL_MASK | GDK_SUPER_MASK | GDK_MOD1_MASK)) != 0 &&
	           (event->state & GDK_MOD5_MASK) == 0;
	if (v->preedit_active)
		use_ime = TRUE; /* composing: every key belongs to the IME */
	else if (evdev == 100)
		use_ime = TRUE; /* Right Alt / AltGr produces text */
	else
		use_ime = jr_evdev_is_text(evdev) && !shortcut;

	previous_route = v->key_route[hw];

	if (use_ime && v->ime)
	{
		v->committed_this_key = FALSE;
		consumed = gtk_im_context_filter_keypress(v->ime, event);
		if (consumed)
		{
			v->key_route[hw] = 2;
			/* When the IME consumed a command key *and* committed, replay the
			 * key so the remote still sees e.g. the Enter that confirmed a
			 * Pinyin candidate. While composition is still active (candidate
			 * navigation) the key stays local. */
			if (!v->preedit_active && v->committed_this_key &&
			    jr_evdev_is_command_key(evdev))
			{
				jr_send_scancode(v, s, evdev, 1, 0);
				jr_send_scancode(v, s, evdev, 0, 0);
			}
			return TRUE;
		}
		/* No input method handled it (e.g. no IBus): fall back to scancode. */
	}

	v->key_route[hw] = 1;
	jr_send_scancode(v, s, evdev, 1, previous_route == 1);
	return TRUE;
}

static gboolean jr_da_key_release(GtkWidget* widget, GdkEventKey* event, gpointer data)
{
	JrView* v = (JrView*)data;
	jr_session_t* s = atomic_load(&v->input);
	unsigned hw = event->hardware_keycode;
	guint8 route;

	(void)widget;
	if (!s || hw < 8 || hw > 255)
		return FALSE;
	route = v->key_route[hw];
	v->key_route[hw] = 0;
	/* Symmetry (KI-034): a press routed to the IME must not emit a lone
	 * scancode release. IME commits carry text, command replay carries its
	 * own press+release pair. */
	if (route != 1)
		return TRUE;
	jr_send_scancode(v, s, (unsigned)hw - 8, 0, 0);
	return TRUE;
}

/* ------------------------------------------------------------------ */
/* IME                                                                 */
/* ------------------------------------------------------------------ */

static void jr_ime_commit(GtkIMContext* imc, const gchar* text, gpointer data)
{
	JrView* v = (JrView*)data;
	jr_session_t* s = atomic_load(&v->input);

	(void)imc;
	v->committed_this_key = TRUE;
	v->preedit_active = FALSE;
	if (s && text && *text)
	{
		fprintf(stderr, "[jr-input] IME commit len=%zu\n", strlen(text));
		jr_session_enqueue_unicode_text(s, text);
	}
}

static void jr_ime_preedit_changed(GtkIMContext* imc, gpointer data)
{
	JrView* v = (JrView*)data;
	gchar* preedit = NULL;

	gtk_im_context_get_preedit_string(imc, &preedit, NULL, NULL);
	v->preedit_active = (preedit && *preedit);
	fprintf(stderr, "[jr-input] preedit len=%zu (never forwarded)\n",
	        preedit ? strlen(preedit) : (size_t)0);
	g_free(preedit);
}

static gboolean jr_ime_retrieve_surrounding(GtkIMContext* imc, gpointer data)
{
	(void)data;
	/* Valid (empty) surrounding text: CJK input methods stall when the client
	 * reports "no usable client" (mirrors macOS selectedRange {0,0}). */
	gtk_im_context_set_surrounding(imc, "", -1, 0);
	return TRUE;
}

static gboolean jr_ime_delete_surrounding(GtkIMContext* imc, gint offset, gint n_chars,
                                          gpointer data)
{
	(void)imc;
	(void)offset;
	(void)n_chars;
	(void)data;
	return TRUE;
}

static gboolean jr_da_focus_in(GtkWidget* widget, GdkEventFocus* event, gpointer data)
{
	JrView* v = (JrView*)data;
	(void)widget;
	(void)event;
	if (v->ime)
		gtk_im_context_focus_in(v->ime);
	return FALSE;
}

static gboolean jr_da_focus_out(GtkWidget* widget, GdkEventFocus* event, gpointer data)
{
	JrView* v = (JrView*)data;
	(void)widget;
	(void)event;
	if (v->ime)
	{
		gtk_im_context_focus_out(v->ime);
		gtk_im_context_reset(v->ime);
	}
	v->preedit_active = FALSE;
	jr_keyboard_sweep(v);
	return FALSE;
}

static void jr_da_realize(GtkWidget* widget, gpointer data)
{
	JrView* v = (JrView*)data;
	GdkWindow* gw = gtk_widget_get_window(widget);
	if (v->ime && gw)
		gtk_im_context_set_client_window(v->ime, gw);
}

static void jr_da_unrealize(GtkWidget* widget, gpointer data)
{
	JrView* v = (JrView*)data;
	(void)widget;
	if (v->ime)
		gtk_im_context_set_client_window(v->ime, NULL);
}

/* ------------------------------------------------------------------ */
/* Window geometry                                                     */
/* ------------------------------------------------------------------ */

void jr_window_content_size(void* window, double* w, double* h)
{
	int cw;
	int ch;

	(void)window;
	if (!w || !h)
		return;
	cw = g_atomic_int_get(&g_content_w);
	ch = g_atomic_int_get(&g_content_h);
	/* Before the first size-allocate (window not mapped yet) report the
	 * configured content size; Rust subtracts the 44px tab inset itself. */
	if (cw <= 0)
		cw = 1280;
	if (ch <= 0)
		ch = 800;
	*w = (double)cw;
	*h = (double)ch;
}

double jr_window_safe_area_top(void* window)
{
	(void)window;
	return 0.0; /* no macOS-style safe-area inset on Linux; CSD is outside vbox */
}

/* ------------------------------------------------------------------ */
/* Clipboard sync: GtkClipboard <-> CLIPRDR (text only)                */
/* ------------------------------------------------------------------ */

/* Cross-thread owner view (the FreeRDP worker reads these). */
static _Atomic(uintptr_t) g_clip_owner = 0;
static _Atomic(uint64_t) g_clip_generation = 0;

/* Main-thread-confined state. */
static jr_session_t* g_clip_session = NULL;
static guint g_clip_timer = 0;
static gchar* g_last_text = NULL;

int jr_clip_is_owner(void* session)
{
	return atomic_load(&g_clip_owner) == (uintptr_t)session;
}

uint64_t jr_clip_generation(void)
{
	return atomic_load(&g_clip_generation);
}

typedef struct
{
	jr_session_t* session;
	uint64_t generation;
	gchar* text;
} JrClipSet;

static gboolean jr_clip_set_on_main(gpointer data)
{
	JrClipSet* c = (JrClipSet*)data;
	GtkClipboard* cb;

	/* Re-validate on the main thread: a delayed callback from a session that
	 * lost focus must never overwrite the clipboard (multi-session rule). */
	if (atomic_load(&g_clip_owner) != (uintptr_t)c->session ||
	    atomic_load(&g_clip_generation) != c->generation)
	{
		fprintf(stderr, "[jr-clip] remote->local dropped (stale owner/generation)\n");
		return G_SOURCE_REMOVE;
	}
	cb = gtk_clipboard_get(GDK_SELECTION_CLIPBOARD);
	if (cb)
	{
		/* Update the echo baseline BEFORE writing: the poll tick compares
		 * against g_last_text, so our own write is never sent back. */
		g_free(g_last_text);
		g_last_text = g_strdup(c->text ? c->text : "");
		gtk_clipboard_set_text(cb, g_last_text, -1);
		fprintf(stderr, "[jr-clip] remote->local len=%zu\n", strlen(g_last_text));
	}
	return G_SOURCE_REMOVE;
}

static void jr_clip_set_free(gpointer data)
{
	JrClipSet* c = (JrClipSet*)data;
	g_free(c->text);
	g_free(c);
}

void jr_mac_clip_set(void* session, uint64_t generation, const char* utf8)
{
	JrClipSet* c = g_new0(JrClipSet, 1);
	c->session = (jr_session_t*)session;
	c->generation = generation;
	c->text = g_strdup(utf8 ? utf8 : "");
	/* Async: the RDP worker must never block on the GTK main loop. */
	jr_run_on_main_async(jr_clip_set_on_main, c, jr_clip_set_free);
}

typedef struct
{
	uint64_t generation;
	gboolean announce;
} JrClipRequest;

static void jr_clip_text_received(GtkClipboard* clipboard, const gchar* text, gpointer data)
{
	JrClipRequest* r = (JrClipRequest*)data;
	jr_session_t* s;

	(void)clipboard;
	if (!text || !*text)
	{
		g_free(r);
		return;
	}
	if (atomic_load(&g_clip_generation) != r->generation || !g_clip_session ||
	    atomic_load(&g_clip_owner) != (uintptr_t)g_clip_session)
	{
		g_free(r);
		return;
	}
	s = g_clip_session;
	if (r->announce)
	{
		/* Initial snapshot (KI-040): store it for the CLIPRDR handshake and
		 * enqueue it so this session announces ClientFormatList even if it
		 * started in the background. */
		jr_clip_store_text(s, text);
		jr_session_enqueue_local_clipboard_text(s, text);
		g_free(g_last_text);
		g_last_text = g_strdup(text);
		fprintf(stderr, "[jr-clip] initial snapshot len=%zu\n", strlen(text));
	}
	else if (!g_last_text || g_strcmp0(g_last_text, text) != 0)
	{
		g_free(g_last_text);
		g_last_text = g_strdup(text);
		fprintf(stderr, "[jr-clip] local-change len=%zu\n", strlen(text));
		jr_session_enqueue_local_clipboard_text(s, text);
	}
	g_free(r);
}

static void jr_clip_request_text(uint64_t generation, gboolean announce)
{
	GtkClipboard* cb = gtk_clipboard_get(GDK_SELECTION_CLIPBOARD);
	JrClipRequest* r;

	if (!cb)
		return;
	r = g_new0(JrClipRequest, 1);
	r->generation = generation;
	r->announce = announce;
	/* Async only: gtk_clipboard_wait_for_text would spin a nested main loop
	 * on the RDP/command thread and deadlock. */
	gtk_clipboard_request_text(cb, jr_clip_text_received, r);
}

static gboolean jr_clip_tick(gpointer data)
{
	(void)data;
	if (!g_clip_session || atomic_load(&g_clip_owner) != (uintptr_t)g_clip_session)
		return TRUE;
	jr_clip_request_text(atomic_load(&g_clip_generation), FALSE);
	return TRUE;
}

typedef struct
{
	jr_session_t* session;
	uint64_t generation;
} JrClipStart;

static gboolean jr_clip_start_on_main(gpointer data)
{
	JrClipStart* st = (JrClipStart*)data;

	/* A newer start/stop already superseded this one. */
	if (atomic_load(&g_clip_generation) != st->generation)
		return G_SOURCE_REMOVE;
	g_clip_session = st->session;
	if (g_clip_timer)
	{
		g_source_remove(g_clip_timer);
		g_clip_timer = 0;
	}
	g_free(g_last_text);
	g_last_text = NULL;
	/* Snapshot now (async read) so MonitorReady can offer it; then poll. */
	jr_clip_request_text(st->generation, TRUE);
	g_clip_timer = g_timeout_add(500, jr_clip_tick, NULL);
	fprintf(stderr, "[jr-clip] sync started\n");
	return G_SOURCE_REMOVE;
}

void jr_clipboard_sync_start(void* session)
{
	JrClipStart* st;

	/* Owner/generation are bumped synchronously in the caller: a stale
	 * callback that already left the worker is invalidated before we return. */
	atomic_store(&g_clip_owner, (uintptr_t)session);
	atomic_fetch_add(&g_clip_generation, 1);
	st = g_new0(JrClipStart, 1);
	st->session = (jr_session_t*)session;
	st->generation = atomic_load(&g_clip_generation);
	jr_run_on_main_async(jr_clip_start_on_main, st, g_free);
}

static gboolean jr_clip_stop_on_main(gpointer data)
{
	(void)data;
	if (g_clip_timer)
	{
		g_source_remove(g_clip_timer);
		g_clip_timer = 0;
	}
	g_clip_session = NULL;
	g_free(g_last_text);
	g_last_text = NULL;
	fprintf(stderr, "[jr-clip] sync stopped\n");
	return TRUE;
}

void jr_clipboard_sync_stop(void)
{
	/* Synchronous invalidation (unlike the macOS async stop): no in-flight
	 * write may land after the session lost focus. */
	atomic_store(&g_clip_owner, 0);
	atomic_fetch_add(&g_clip_generation, 1);
	jr_run_on_main_sync(jr_clip_stop_on_main, NULL);
}