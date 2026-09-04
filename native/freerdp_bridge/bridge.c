/* Embedded FreeRDP bridge — C side.
 *
 * Owns the FreeRDP/WinPR types; Rust talks only through bridge.h. Based on the
 * canonical FreeRDP 3.31.0 `client/Sample` (Apache-2.0): client entry points +
 * freerdp_connect + event loop + gdi framebuffer. The session runs a *blocking*
 * event loop on a worker thread owned by the Rust `RdpSession`.
 */

#include "bridge.h"
#include "queue.h"

#include <freerdp/freerdp.h>
#include <freerdp/client.h>
#include <freerdp/settings.h>
#include <freerdp/gdi/gdi.h>
#include <freerdp/codec/color.h>
#include <freerdp/event.h>
#include <freerdp/channels/channels.h>
#include <freerdp/client/cliprdr.h>
#include <winpr/collections.h>

#include <winpr/wtypes.h>
#include <winpr/crt.h>
#include <winpr/synch.h>

#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define JR_MAX_HANDLES 64

#ifndef CB_RESPONSE_OK
#define CB_RESPONSE_OK 0x0002
#endif
#ifndef CB_RESPONSE_FAIL
#define CB_RESPONSE_FAIL 0x0004
#endif
#ifndef CB_CAPSTYPE_GENERAL
#define CB_CAPSTYPE_GENERAL 0x0001
#endif
#ifndef CB_CAPS_VERSION_2
#define CB_CAPS_VERSION_2 0x0002
#endif

static void jr_clip_wire(jr_session_t* s);
static void jr_clip_announce(jr_session_t* s);
static char* jr_clip_snapshot(jr_session_t* s);
static void jr_on_channel_connected(void* context, const ChannelConnectedEventArgs* e);

/* Single-owner input dispatch: the worker thread is the ONLY thread allowed
 * to touch the FreeRDP input/CLIPRDR APIs (KI-018 / single-owner rule). The
 * AppKit main thread only enqueues through `jr_session_enqueue_*`. */
static void jr_send_mouse_move(jr_session_t* s, int x, int y);
static void jr_send_mouse_button(jr_session_t* s, int button, int down, int x, int y);
static void jr_send_mouse_wheel(jr_session_t* s, int delta, int negative, int hdelta, int hnegative,
                                int x, int y);
static void jr_send_key_scancode(jr_session_t* s, int down, int repeat, int scancode, int extended);
static void jr_send_unicode_text(jr_session_t* s, const char* utf8);
static void jr_send_reset_modifiers(jr_session_t* s);
static void jr_dispatch_cmd(jr_cmd* c, void* user);

/* Custom context: the rdpContext is embedded at offset 0 via rdpClientContext,
 * so a `jr_context_t*` is address-identical to the `rdpContext*`. */
typedef struct
{
	rdpClientContext common;
	jr_session_t* session; /* back-pointer set by jr_session_create */
} jr_context_t;

struct jr_session
{
	jr_session_callbacks_t cb;
	jr_cert_callbacks_t cert;

	char* host;
	char* certificate_name;
	uint16_t port;
	char* username;
	char* password;
	int width;
	int height;
	int color_depth;

	volatile int disconnecting;
	int started;
	int frame_count;

	/* Single-owner command queue (KI-018 / §12): AppKit main thread only
	 * ENQUEUES; the RDP worker thread drains and calls the FreeRDP APIs. */
	jr_cmd_queue* cmdq;
	HANDLE cmd_wake; /* WinPR auto-reset event; SetEvent after enqueue */

	freerdp* instance;
	rdpContext* context;
	rdpGdi* gdi;

	/* Clipboard (CLIPRDR, text-only). */
	CliprdrClientContext* cliprdr;
	int clip_ready;
	char* clip_text;
	CRITICAL_SECTION clip_lock;

	char last_error[256];
};

static void jr_set_error(jr_session_t* s, const char* fmt, ...)
{
	va_list ap;
	if (!s)
		return;
	va_start(ap, fmt);
	(void)vsnprintf(s->last_error, sizeof(s->last_error), fmt, ap);
	va_end(ap);
}

const char* jr_freerdp_version(void)
{
	return freerdp_get_version_string();
}

const char* jr_last_error(jr_session_t* s)
{
	return s ? s->last_error : "";
}

/* ------------------------------------------------------------------ */
/* Certificate TOFU — Rust decides. Never auto-accept a changed cert.  */
/* ------------------------------------------------------------------ */

static DWORD jr_verify_certificate_ex(freerdp* instance, const char* host, UINT16 port,
                                      const char* common_name, const char* subject,
                                      const char* issuer, const char* fingerprint, DWORD flags)
{
	jr_session_t* s = ((jr_context_t*)instance->context)->session;
	jr_cert_info_t info;

	(void)port;
	(void)flags;

	if (!s || !s->cert.verify_certificate)
		return 0; /* reject by default */
	info.host = host;
	info.common_name = common_name;
	info.subject = subject;
	info.issuer = issuer;
	info.fingerprint = fingerprint;
	return (DWORD)s->cert.verify_certificate(s->cert.user, &info);
}

static DWORD jr_verify_changed_certificate_ex(freerdp* instance, const char* host, UINT16 port,
                                              const char* common_name, const char* subject,
                                              const char* issuer, const char* new_fingerprint,
                                              const char* old_subject, const char* old_issuer,
                                              const char* old_fingerprint, DWORD flags)
{
	jr_session_t* s = ((jr_context_t*)instance->context)->session;
	jr_cert_info_t ninfo;
	jr_cert_info_t oinfo;

	(void)port;
	(void)flags;

	if (!s || !s->cert.verify_changed_certificate)
		return 0; /* changed certificate → reject */
	ninfo.host = host;
	ninfo.common_name = common_name;
	ninfo.subject = subject;
	ninfo.issuer = issuer;
	ninfo.fingerprint = new_fingerprint;
	oinfo.host = host;
	oinfo.common_name = old_subject;
	oinfo.subject = old_subject;
	oinfo.issuer = old_issuer;
	oinfo.fingerprint = old_fingerprint;
	return (DWORD)s->cert.verify_changed_certificate(s->cert.user, &ninfo, &oinfo);
}

/* ------------------------------------------------------------------ */
/* DEV-only pixel diagnostics. No-op unless JR_RDP_DIAG /              */
/* JR_RDP_DUMP_FRAME is set in the environment. Proves the framebuffer */
/* actually contains composited desktop pixels (vs zero-filled         */
/* geometry). Never on in production.                                  */
/* ------------------------------------------------------------------ */

static int jr_diag_on = -1; /* lazily read from env on first use */
static int jr_dump_on = -1;
static uint32_t jr_best_nnz = 0;

static int jr_env_flag(const char* name)
{
	const char* v = getenv(name);
	if (!v || !*v)
		return 0;
	return strcmp(v, "1") == 0 || strcmp(v, "true") == 0 || strcmp(v, "TRUE") == 0 ||
	       strcmp(v, "yes") == 0 || strcmp(v, "on") == 0;
}

static const char* jr_format_name(UINT32 fmt)
{
	switch (fmt)
	{
		case PIXEL_FORMAT_BGRA32:
			return "BGRA32";
		case PIXEL_FORMAT_RGBA32:
			return "RGBA32";
		case PIXEL_FORMAT_BGRX32:
			return "BGRX32";
		case PIXEL_FORMAT_RGBX32:
			return "RGBX32";
		case PIXEL_FORMAT_XRGB32:
			return "XRGB32";
		case PIXEL_FORMAT_XBGR32:
			return "XBGR32";
		case PIXEL_FORMAT_ARGB32:
			return "ARGB32";
		case PIXEL_FORMAT_ABGR32:
			return "ABGR32";
		default:
			return "?";
	}
}

static void jr_write_raw(const char* path, rdpGdi* gdi)
{
	FILE* f = fopen(path, "wb");
	if (!f)
		return;
	(void)fwrite(gdi->primary_buffer, 1, (size_t)gdi->stride * (size_t)gdi->height, f);
	fclose(f);
}

static void jr_write_ppm(const char* path, rdpGdi* gdi)
{
	FILE* f = fopen(path, "wb");
	const BYTE* buf = (const BYTE*)gdi->primary_buffer;
	const UINT32 fmt = gdi->dstFormat;
	const int w = gdi->width;
	const int h = gdi->height;
	int x, y;

	if (!f)
		return;
	fprintf(f, "P6\n%d %d\n255\n", w, h);
	for (y = 0; y < h; y++)
	{
		const BYTE* row = buf + (size_t)y * (size_t)gdi->stride;
		for (x = 0; x < w; x++)
		{
			const BYTE* px = row + (size_t)x * 4;
			UINT32 color = FreeRDPReadColor(px, fmt);
			BYTE r, g, b, a;
			FreeRDPSplitColor(color, fmt, &r, &g, &b, &a, NULL);
			fputc(r, f);
			fputc(g, f);
			fputc(b, f);
		}
	}
	fclose(f);
}

static void jr_diag_frame(jr_session_t* s, rdpGdi* gdi)
{
	const BYTE* buf;
	int w, h, stride;
	UINT32 fmt;
	uint32_t total, nsamples, step, k;
	uint32_t nnz = 0, nnz_rgb = 0, minb = 255, maxb = 0, hash = 2166136261u;
	const BYTE *first, *center, *last;

	if (jr_diag_on < 0)
	{
		jr_diag_on = jr_env_flag("JR_RDP_DIAG");
		jr_dump_on = jr_env_flag("JR_RDP_DUMP_FRAME");
	}
	if (!jr_diag_on && !jr_dump_on)
		return;

	buf = (const BYTE*)gdi->primary_buffer;
	w = gdi->width;
	h = gdi->height;
	stride = (int)gdi->stride;
	fmt = gdi->dstFormat;
	total = (uint32_t)w * (uint32_t)h;
	nsamples = total < 2048 ? total : 2048;
	step = total / nsamples;
	if (step < 1)
		step = 1;

	first = buf;
	center = buf + (size_t)(h / 2) * (size_t)stride + (size_t)(w / 2) * 4;
	last = buf + (size_t)(h - 1) * (size_t)stride + (size_t)(w - 1) * 4;

	for (k = 0; k < nsamples; k++)
	{
		uint32_t idx = k * step;
		uint32_t x = idx % (uint32_t)w;
		uint32_t y = idx / (uint32_t)w;
		const BYTE* px = buf + (size_t)y * (size_t)stride + (size_t)x * 4;
		BYTE b0 = px[0], b1 = px[1], b2 = px[2], b3 = px[3];
		BYTE lum = b0 > b1 ? b0 : b1;
		lum = lum > b2 ? lum : b2;

		if (b0 || b1 || b2 || b3)
			nnz++;
		if (b0 || b1 || b2)
			nnz_rgb++;
		if (lum < minb)
			minb = lum;
		if (lum > maxb)
			maxb = lum;

		hash ^= b0;
		hash *= 16777619u;
		hash ^= b1;
		hash *= 16777619u;
		hash ^= b2;
		hash *= 16777619u;
		hash ^= b3;
		hash *= 16777619u;
	}

	if (jr_diag_on)
	{
		fprintf(stderr,
		        "[jr-diag] frame#%d %dx%d stride=%d dstFormat=%s nnz=%u/%u rgb_nonzero=%u "
		        "min=%u max=%u hash=%08X\n"
		        "[jr-diag]   first=%02X%02X%02X%02X center=%02X%02X%02X%02X "
		        "last=%02X%02X%02X%02X\n",
		        s->frame_count, w, h, stride, jr_format_name(fmt), nnz, nsamples, nnz_rgb, minb,
		        maxb, hash, first[0], first[1], first[2], first[3], center[0], center[1],
		        center[2], center[3], last[0], last[1], last[2], last[3]);
	}

	if (!jr_dump_on)
		return;

	if (s->frame_count == 1)
	{
		jr_write_ppm("/tmp/jetson-remote-frame.ppm", gdi);
		jr_write_raw("/tmp/jetson-remote-frame.raw", gdi);
		fprintf(stderr, "[jr-diag] dumped first frame -> /tmp/jetson-remote-frame.{ppm,raw}\n");
	}
	if (nnz_rgb > jr_best_nnz)
	{
		jr_best_nnz = nnz_rgb;
		jr_write_ppm("/tmp/jetson-remote-frame-best.ppm", gdi);
		fprintf(stderr,
		        "[jr-diag] best frame#%d (rgb_nonzero=%u) -> /tmp/jetson-remote-frame-best.ppm\n",
		        s->frame_count, nnz_rgb);
	}
}

/* ------------------------------------------------------------------ */
/* GDI paint / resize                                                 */
/* ------------------------------------------------------------------ */

static BOOL jr_begin_paint(rdpContext* context)
{
	rdpGdi* gdi = context->gdi;
	if (gdi && gdi->primary && gdi->primary->hdc && gdi->primary->hdc->hwnd &&
	    gdi->primary->hdc->hwnd->invalid)
		gdi->primary->hdc->hwnd->invalid->null = TRUE;
	return TRUE;
}

static void jr_emit_frame(jr_session_t* s, rdpGdi* gdi);

static BOOL jr_end_paint(rdpContext* context)
{
	jr_session_t* s = ((jr_context_t*)context)->session;
	rdpGdi* gdi = context->gdi;

	jr_emit_frame(s, gdi);
	return TRUE;
}

/* RDPGFX writes directly into gdi->primary_buffer and does not reliably call
 * Update.EndPaint. Keep one presentation path for legacy GDI and the GFX
 * pipeline; the event loop also invokes this at a modest cadence so a GFX-only
 * desktop cannot remain on the native view's placeholder colour. */
static void jr_emit_frame(jr_session_t* s, rdpGdi* gdi)
{
	if (!s || !gdi || !gdi->primary)
		return;

	s->frame_count++;
	jr_diag_frame(s, gdi); /* DEV-only; no-op unless env-flagged */

	/* Full-frame signal for the SPIKE; dirty-rect refinement is Phase 4B-2/3. */
	if (s->cb.on_frame_updated)
		s->cb.on_frame_updated(s->cb.user, 0, 0, gdi->width, gdi->height);
}

static BOOL jr_desktop_resize(rdpContext* context)
{
	jr_session_t* s = ((jr_context_t*)context)->session;
	rdpGdi* gdi = context->gdi;
	rdpSettings* settings = context->settings;
	UINT32 w;
	UINT32 h;
	BOOL ok;

	if (!gdi)
		return FALSE;
	w = freerdp_settings_get_uint32(settings, FreeRDP_DesktopWidth);
	h = freerdp_settings_get_uint32(settings, FreeRDP_DesktopHeight);
	ok = gdi_resize(gdi, w, h);
	if (ok && s && s->cb.on_desktop_resized)
		s->cb.on_desktop_resized(s->cb.user, (int32_t)w, (int32_t)h);
	return ok;
}

/* ------------------------------------------------------------------ */
/* Lifecycle callbacks                                               */
/* ------------------------------------------------------------------ */

static BOOL jr_pre_connect(freerdp* instance)
{
	rdpSettings* settings = instance->context->settings;
	fprintf(stderr, "[jr-clip] jr_pre_connect running\n");
	/* We only use fingerprints (not PEM) for now — no Preferences needed. */
	freerdp_settings_set_bool(settings, FreeRDP_CertificateCallbackPreferPEM, FALSE);

	/* Wire the common channel handlers. This is what makes RDPGFX (the graphics
	 * pipeline) decode into gdi->primary_buffer: the default handler calls
	 * gdi_graphics_pipeline_init() when the RDPGFX dynamic channel connects.
	 * Without it, XRDP streams the desktop over GFX and every surface frame is
	 * silently discarded -> black screen, no EndPaint. */
	int sub_rc = PubSub_SubscribeChannelConnected(instance->context->pubSub,
	                                              freerdp_client_OnChannelConnectedEventHandler);
	int sub_rc2 = PubSub_SubscribeChannelConnected(instance->context->pubSub, jr_on_channel_connected);
	int sub_rc3 = PubSub_SubscribeChannelDisconnected(instance->context->pubSub,
	                                                  freerdp_client_OnChannelDisconnectedEventHandler);
	fprintf(stderr, "[jr-clip] subscribe rc: %d %d %d\n", sub_rc, sub_rc2, sub_rc3);
	return TRUE;
}

static BOOL jr_post_connect(freerdp* instance)
{
	jr_session_t* s = ((jr_context_t*)instance->context)->session;
	rdpContext* context = instance->context;
	fprintf(stderr, "[jr-clip] jr_post_connect running\n");

	if (!gdi_init(instance, PIXEL_FORMAT_BGRX32))
		return FALSE;
	s->gdi = context->gdi;

	context->update->BeginPaint = jr_begin_paint;
	context->update->EndPaint = jr_end_paint;
	context->update->DesktopResize = jr_desktop_resize;

	/* Static channels may not be wired at PostConnect; the ChannelConnected
	 * PubSub event is the authoritative hook (matches FreeRDP's own X11/SDL
	 * clients). */
	if (!s->cliprdr)
		s->cliprdr = (CliprdrClientContext*)freerdp_channels_get_static_channel_interface(
		    context->channels, CLIPRDR_SVC_CHANNEL_NAME);
	jr_clip_wire(s);

	/* Clear any stale held-modifier state the (possibly reused) remote X
	 * session retained from earlier sessions whose key-ups were lost
	 * (KI-023: macOS swallows the flagsChanged of a Cmd released while the
	 * app was inactive — Cmd+Tab / Cmd+Q / Spotlight). See the regression
	 * guide §2.4 before touching this. */
	jr_send_reset_modifiers(s);

	if (s->cb.on_connected)
		s->cb.on_connected(s->cb.user);
	return TRUE;
}

static void jr_post_disconnect(freerdp* instance)
{
	jr_session_t* s = ((jr_context_t*)instance->context)->session;

	PubSub_UnsubscribeChannelConnected(instance->context->pubSub,
	                                   freerdp_client_OnChannelConnectedEventHandler);
	PubSub_UnsubscribeChannelConnected(instance->context->pubSub, jr_on_channel_connected);
	PubSub_UnsubscribeChannelDisconnected(instance->context->pubSub,
	                                      freerdp_client_OnChannelDisconnectedEventHandler);

	if (s)
	{
		s->gdi = NULL;
		s->cliprdr = NULL;
		s->clip_ready = 0;
	}
	gdi_free(instance);
}

static BOOL jr_client_new(freerdp* instance, rdpContext* context)
{
	jr_context_t* jr = (jr_context_t*)context;
	(void)jr;

	instance->VerifyCertificateEx = jr_verify_certificate_ex;
	instance->VerifyChangedCertificateEx = jr_verify_changed_certificate_ex;
	instance->PreConnect = jr_pre_connect;
	instance->PostConnect = jr_post_connect;
	instance->PostDisconnect = jr_post_disconnect;
	return TRUE;
}

static void jr_client_free(freerdp* instance, rdpContext* context)
{
	(void)instance;
	(void)context;
}

/* ------------------------------------------------------------------ */
/* Public session API                                                */
/* ------------------------------------------------------------------ */

jr_session_t* jr_session_create(const jr_connect_params_t* params,
                                const jr_session_callbacks_t* cb,
                                const jr_cert_callbacks_t* cert)
{
	jr_session_t* s;
	RDP_CLIENT_ENTRY_POINTS ep;
	rdpContext* ctx;

	if (!params || !params->certificate_name || !params->host || !params->username || !params->password)
		return NULL;

	s = (jr_session_t*)calloc(1, sizeof(jr_session_t));
	if (!s)
		return NULL;

	/* Init the lock up-front so the `fail` path can always Delete it. */
	InitializeCriticalSection(&s->clip_lock);

	s->certificate_name = strdup(params->certificate_name);
	s->host = strdup(params->host);
	s->username = strdup(params->username);
	s->password = strdup(params->password);
	s->port = params->port ? params->port : 3389;
	s->width = params->width ? params->width : 1024;
	s->height = params->height ? params->height : 768;
	s->color_depth = params->color_depth ? params->color_depth : 32;
	if (cb)
		s->cb = *cb;
	if (cert)
		s->cert = *cert;

	if (!s->certificate_name || !s->host || !s->username || !s->password)
		goto fail;

	ZeroMemory(&ep, sizeof(ep));
	ep.Size = sizeof(ep);
	ep.Version = RDP_CLIENT_INTERFACE_VERSION;
	ep.ContextSize = sizeof(jr_context_t);
	ep.ClientNew = jr_client_new;
	ep.ClientFree = jr_client_free;

	ctx = freerdp_client_context_new(&ep);
	if (!ctx)
		goto fail;

	s->context = ctx;
	s->instance = ctx->instance;
	((jr_context_t*)ctx)->session = s;

	/* Per-session command queue + wake event. Must exist before the view
	 * attaches input (attachInput enqueues a modifier reset immediately).
	 * MANUAL-RESET event: WinPR's CreateEventA does not implement auto-reset
	 * events (observed at runtime), so the worker ResetEvent()s it each loop
	 * iteration before draining. */
	s->cmdq = jr_cmdq_create();
	s->cmd_wake = CreateEvent(NULL, TRUE, FALSE, NULL);
	if (!s->cmdq || !s->cmd_wake)
		goto fail;
	return s;

fail:
	jr_session_destroy(s);
	return NULL;
}

void jr_session_destroy(jr_session_t* s)
{
	if (!s)
		return;
	if (s->context)
	{
		if (s->started)
			freerdp_client_stop(s->context);
		freerdp_client_context_free(s->context);
		s->context = NULL;
		s->instance = NULL;
	}
	free(s->host);
	free(s->certificate_name);
	free(s->username);
	free(s->password);
	free(s->clip_text);
	jr_cmdq_destroy(s->cmdq);
	s->cmdq = NULL;
	if (s->cmd_wake)
	{
		CloseHandle(s->cmd_wake);
		s->cmd_wake = NULL;
	}
	DeleteCriticalSection(&s->clip_lock);
	free(s);
}

static int jr_run_loop(jr_session_t* s)
{
	freerdp* instance = s->instance;
	rdpContext* context = s->context;
	BOOL rc;
	HANDLE handles[JR_MAX_HANDLES + 1]; /* +1 for the command wake event */
	DWORD nCount;
	DWORD status;

	rc = freerdp_connect(instance);
	if (!rc)
	{
		jr_set_error(s, "connection failure (0x%08X)",
		             (unsigned int)freerdp_get_last_error(context));
		freerdp_disconnect(instance);
		return -1;
	}

	while (!freerdp_shall_disconnect_context(context) && !s->disconnecting)
	{
		/* Manual-reset wake: clear BEFORE draining so an enqueue that lands
		 * during the drain re-signals the event and the next iteration wakes
		 * immediately (no lost wake, no busy-spin). */
		ResetEvent(s->cmd_wake);

		/* Drain pending input/clipboard/resize commands FIRST. This is the
		 * only place FreeRDP input APIs are called (single-owner rule). */
		jr_cmdq_drain(s->cmdq, jr_dispatch_cmd, s);

		ZeroMemory(handles, sizeof(handles));
		nCount = freerdp_get_event_handles(context, handles, JR_MAX_HANDLES);
		if (nCount == 0)
		{
			jr_set_error(s, "freerdp_get_event_handles failed");
			break;
		}
		/* Append the command wake event so an enqueue from the main thread
		 * wakes the loop immediately instead of waiting out the 33ms tick. */
		handles[nCount] = s->cmd_wake;
		/* RDPGFX can update the GDI primary buffer without Update.EndPaint.
		 * A short event-loop timeout gives the native surface a bounded-latency
		 * presentation tick for that path without a second reader thread. */
		status = WaitForMultipleObjects(nCount + 1, handles, FALSE, 33);
		if (status == WAIT_TIMEOUT)
		{
			jr_emit_frame(s, context->gdi);
			continue;
		}
		if (status == WAIT_FAILED)
		{
			jr_set_error(s, "WaitForMultipleObjects failed");
			break;
		}
		/* Our wake event fired (status == WAIT_OBJECT_0 + nCount): nothing to
		 * do here — the drained queue is emptied at the top of the next
		 * iteration. FreeRDP handles are checked the same either way, which is
		 * a no-op (returns TRUE) when only the command event was signaled. */
		if (!freerdp_check_event_handles(context))
		{
			if (freerdp_get_last_error(context) == FREERDP_ERROR_SUCCESS)
				jr_set_error(s, "failed to check event handles");
			break;
		}
		jr_emit_frame(s, context->gdi);
	}

	freerdp_disconnect(instance);
	return 0;
}

int jr_session_connect(jr_session_t* s)
{
	rdpSettings* settings;
	int rc;

	if (!s || !s->context)
		return -1;

	settings = s->context->settings;
	freerdp_settings_set_bool(settings, FreeRDP_AutoReconnectionEnabled, FALSE);
	freerdp_settings_set_bool(settings, FreeRDP_RedirectClipboard, TRUE);
	freerdp_settings_set_string(settings, FreeRDP_CertificateName, s->certificate_name);
	freerdp_settings_set_string(settings, FreeRDP_ServerHostname, s->host);
	freerdp_settings_set_uint32(settings, FreeRDP_ServerPort, s->port);
	freerdp_settings_set_string(settings, FreeRDP_Username, s->username);
	freerdp_settings_set_string(settings, FreeRDP_Password, s->password);
	freerdp_settings_set_uint32(settings, FreeRDP_DesktopWidth, (UINT32)s->width);
	freerdp_settings_set_uint32(settings, FreeRDP_DesktopHeight, (UINT32)s->height);
	freerdp_settings_set_uint32(settings, FreeRDP_ColorDepth, (UINT32)s->color_depth);

	if (freerdp_client_start(s->context) < 0)
	{
		jr_set_error(s, "client start failed");
		return -1;
	}
	s->started = 1;

	rc = jr_run_loop(s);

	if (s->cb.on_disconnected)
		s->cb.on_disconnected(s->cb.user);
	return rc;
}

int jr_session_disconnect(jr_session_t* s)
{
	if (!s)
		return -1;
	s->disconnecting = 1;
	if (s->context)
		freerdp_abort_connect_context(s->context);
	return 0;
}

/* ------------------------------------------------------------------ */
/* Framebuffer / input                                               */
/* ------------------------------------------------------------------ */

int jr_session_get_size(jr_session_t* s, int* width, int* height)
{
	if (!s || !s->gdi)
		return -1;
	if (width)
		*width = (int)s->gdi->width;
	if (height)
		*height = (int)s->gdi->height;
	return 0;
}

int jr_session_get_framebuffer(jr_session_t* s, const uint8_t** buffer, int* width, int* height,
                               int* stride)
{
	if (!s || !s->gdi || !s->gdi->primary_buffer)
		return -1;
	*buffer = s->gdi->primary_buffer;
	*width = (int)s->gdi->width;
	*height = (int)s->gdi->height;
	*stride = (int)s->gdi->stride;
	return 0;
}

/* ---- worker-only send helpers (single-owner rule) ------------------- */

static void jr_send_mouse_move(jr_session_t* s, int x, int y)
{
	UINT16 flags = PTR_FLAGS_MOVE;
	/* CONTRACT (KI-018, regression guide §2.1): a drag is PURE PTR_FLAGS_MOVE
	 * — motion events must NEVER carry held-button bits (a BUTTONn without
	 * DOWN is parsed by xrdp as a button RELEASE, not a move; see xrdp_wm.c
	 * xrdp_wm_process_input_mouse). Held state lives on the server side.
	 * DO NOT change this to "BUTTON1|MOVE": doing so breaks window dragging. */
	if (!s || !s->context || !s->gdi)
		return;
	(void)freerdp_input_send_mouse_event(s->context->input, flags, (UINT16)x, (UINT16)y);
}

static void jr_send_mouse_button(jr_session_t* s, int button, int down, int x, int y)
{
	UINT16 flags = 0;
	if (!s || !s->context || !s->gdi)
		return;
	switch (button)
	{
		case 2:
			flags = PTR_FLAGS_BUTTON2;
			break;
		case 3:
			flags = PTR_FLAGS_BUTTON3;
			break;
		default:
			flags = PTR_FLAGS_BUTTON1;
			break;
	}
	if (down)
	{
		flags |= PTR_FLAGS_DOWN;
		flags |= PTR_FLAGS_MOVE; /* update position before the press lands */
	}
	/* release = BUTTONn WITHOUT DOWN (and without MOVE): xrdp keeps the held
	 * state itself and treats a button-bit drop as the release. */
	(void)freerdp_input_send_mouse_event(s->context->input, flags, (UINT16)x, (UINT16)y);
}

static void jr_send_mouse_wheel(jr_session_t* s, int delta, int negative, int hdelta, int hnegative,
                                int x, int y)
{
	UINT16 flags;
	if (!s || !s->context || !s->gdi)
		return;
	if (delta > 0)
	{
		flags = PTR_FLAGS_WHEEL | (UINT16)(delta & WheelRotationMask);
		if (negative)
			flags |= PTR_FLAGS_WHEEL_NEGATIVE;
		if (!freerdp_input_send_mouse_event(s->context->input, flags, (UINT16)x, (UINT16)y))
			return;
	}
	if (hdelta > 0)
	{
		flags = PTR_FLAGS_HWHEEL | (UINT16)(hdelta & WheelRotationMask);
		if (hnegative)
			flags |= PTR_FLAGS_WHEEL_NEGATIVE;
		if (!freerdp_input_send_mouse_event(s->context->input, flags, (UINT16)x, (UINT16)y))
			return;
	}
}

static void jr_send_key_scancode(jr_session_t* s, int down, int repeat, int scancode, int extended)
{
	UINT16 flags = 0;
	if (!s || !s->context || !s->gdi)
		return;
	/* FreeRDP 3: absence of RELEASE = press; DOWN marks an autorepeat. */
	if (!down)
		flags |= KBD_FLAGS_RELEASE;
	else if (repeat)
		flags |= KBD_FLAGS_DOWN;
	if (extended)
		flags |= KBD_FLAGS_EXTENDED;
	(void)freerdp_input_send_keyboard_event(s->context->input, flags, (UINT8)scancode);
}

/* Send committed IME text as RDP unicode keyboard input (Mac IME commit path,
 * §22). Reuses the tested UTF-8→UTF-16LE codec: a supplementary code point
 * (> U+FFFF) is already split into its surrogate pair, and every UTF-16 code
 * unit is sent as press + release so XRDP/X recompose the run correctly. */
static void jr_send_unicode_text(jr_session_t* s, const char* utf8)
{
	uint32_t len = 0;
	uint8_t* buf;
	uint32_t i;

	if (!s || !s->context || !s->gdi || !utf8)
		return;
	buf = jr_utf8_to_utf16le(utf8, &len);
	if (!buf)
		return;
	/* `len` includes the UTF-16 null terminator; iterate code units (2 bytes). */
	for (i = 0; i + 1 < len; i += 2)
	{
		uint16_t unit = (uint16_t)(buf[i] | (uint16_t)(buf[i + 1] << 8));
		if (unit == 0)
			break;
		(void)freerdp_input_send_unicode_keyboard_event(s->context->input, 0, unit);
		(void)freerdp_input_send_unicode_keyboard_event(s->context->input, KBD_FLAGS_RELEASE, unit);
	}
	free(buf);
}

/* Release every modifier key so the remote keyboard state can never stay
 * stuck (KI-023). macOS swallows the flagsChanged of a Command key released
 * while the app is inactive (Cmd+Tab / Cmd+Q / Spotlight), and xrdp retains
 * the stale "Super held" state in its persistent X session across reconnects
 * — the user then sees Super+E open the file manager and Super+Space eaten by
 * the input-method shortcut. Releasing a non-held key is a no-op on the
 * server, so this runs unconditionally at connect/focus/attach. */
static void jr_send_reset_modifiers(jr_session_t* s)
{
	static const struct
	{
		int sc;
		int ext;
	} kMods[] = {
	    {0x1D, 0}, /* LCtrl          */
	    {0x1D, 1}, /* RCtrl   (E0)   */
	    {0x2A, 0}, /* LShift         */
	    {0x36, 0}, /* RShift         */
	    {0x38, 0}, /* LAlt           */
	    {0x38, 1}, /* RAlt    (E0)   */
	    {0x5B, 1}, /* LMeta   (E0)   */
	    {0x5C, 1}, /* RMeta   (E0)   */
	};
	size_t i;

	if (!s || !s->context || !s->gdi)
		return;
	for (i = 0; i < sizeof(kMods) / sizeof(kMods[0]); i++)
		jr_send_key_scancode(s, 0, 0, kMods[i].sc, kMods[i].ext);
	fprintf(stderr, "[jr-input] keyboard modifier reset: 8 releases sent\n");
}

/* Dispatch a queue command to the FreeRDP API. Runs ONLY on the worker thread
 * (called from jr_cmdq_drain inside jr_run_loop). */
static void jr_dispatch_cmd(jr_cmd* c, void* user)
{
	jr_session_t* s = (jr_session_t*)user;
	if (!s || !c)
		return;
	switch (c->kind)
	{
		case JR_CMD_MOUSE_MOVE:
			jr_send_mouse_move(s, c->a, c->b);
			break;
		case JR_CMD_MOUSE_BUTTON:
			jr_send_mouse_button(s, c->a, c->b, c->c, c->d);
			break;
		case JR_CMD_MOUSE_WHEEL:
			jr_send_mouse_wheel(s, c->a, c->b, c->c, c->d, c->e, c->f);
			break;
		case JR_CMD_KEY_SCANCODE:
			jr_send_key_scancode(s, c->a, c->b, c->c, c->d);
			break;
		case JR_CMD_UNICODE_TEXT:
			jr_send_unicode_text(s, c->owned_utf8);
			break;
		case JR_CMD_LOCAL_CLIPBOARD_TEXT:
			jr_clip_store_text(s, c->owned_utf8);
			jr_clip_announce(s);
			break;
		case JR_CMD_RESIZE:
			jr_session_set_size(s, c->a, c->b);
			break;
		case JR_CMD_RESET_MODIFIERS:
			jr_send_reset_modifiers(s);
			break;
		default:
			break;
	}
}

/* ---- public enqueue entry points (AppKit main thread) --------------- */

int jr_session_enqueue_mouse_move(jr_session_t* s, int x, int y)
{
	if (!s || !s->cmdq)
		return -1;
	jr_cmdq_enqueue_move(s->cmdq, x, y);
	if (s->cmd_wake)
		SetEvent(s->cmd_wake);
	return 0;
}

int jr_session_enqueue_mouse_button(jr_session_t* s, int button, int down, int x, int y)
{
	if (!s || !s->cmdq)
		return -1;
	jr_cmdq_enqueue_button(s->cmdq, button, down, x, y);
	if (s->cmd_wake)
		SetEvent(s->cmd_wake);
	return 0;
}

int jr_session_enqueue_mouse_wheel(jr_session_t* s, int delta, int negative, int hdelta,
                                   int hnegative, int x, int y)
{
	if (!s || !s->cmdq)
		return -1;
	jr_cmdq_enqueue_wheel(s->cmdq, delta, negative, hdelta, hnegative, x, y);
	if (s->cmd_wake)
		SetEvent(s->cmd_wake);
	return 0;
}

int jr_session_enqueue_key_scancode(jr_session_t* s, int down, int repeat, int scancode,
                                    int extended)
{
	if (!s || !s->cmdq)
		return -1;
	jr_cmdq_enqueue_scancode(s->cmdq, down, repeat, scancode, extended);
	if (s->cmd_wake)
		SetEvent(s->cmd_wake);
	return 0;
}

int jr_session_enqueue_unicode_text(jr_session_t* s, const char* utf8)
{
	if (!s || !s->cmdq)
		return -1;
	jr_cmdq_enqueue_unicode(s->cmdq, utf8);
	if (s->cmd_wake)
		SetEvent(s->cmd_wake);
	return 0;
}

int jr_session_enqueue_local_clipboard_text(jr_session_t* s, const char* utf8)
{
	if (!s || !s->cmdq)
		return -1;
	jr_cmdq_enqueue_clipboard(s->cmdq, utf8);
	if (s->cmd_wake)
		SetEvent(s->cmd_wake);
	return 0;
}

int jr_session_enqueue_resize(jr_session_t* s, int w, int h)
{
	if (!s || !s->cmdq)
		return -1;
	jr_cmdq_enqueue_resize(s->cmdq, w, h);
	if (s->cmd_wake)
		SetEvent(s->cmd_wake);
	return 0;
}

int jr_session_enqueue_reset_modifiers(jr_session_t* s)
{
	if (!s || !s->cmdq)
		return -1;
	jr_cmdq_enqueue_reset_modifiers(s->cmdq);
	if (s->cmd_wake)
		SetEvent(s->cmd_wake);
	return 0;
}

/* ------------------------------------------------------------------ */
/* Clipboard (CLIPRDR) — text only, both directions.                  */
/* Mac side implements jr_mac_clip_set/get in macos_view.m.            */
/* ------------------------------------------------------------------ */

void jr_mac_clip_set(void* session, uint64_t generation, const char* utf8); /* macos_view.m */
int jr_clip_is_owner(void* session);                                        /* macos_view.m */
uint64_t jr_clip_generation(void);                                          /* macos_view.m */

#define JR_CF_TEXT 1
#define JR_CF_UNICODETEXT 13

char* jr_utf16le_to_utf8(const uint8_t* data, uint32_t len)
{
	size_t cap = (size_t)len + 8;
	char* out = (char*)malloc(cap);
	size_t o = 0;
	size_t i = 0;
	if (!out)
		return NULL;
	while (i + 1 < len)
	{
		UINT32 cp = (UINT32)data[i] | ((UINT32)data[i + 1] << 8);
		i += 2;
		if (cp == 0)
			break;
		if (cp >= 0xD800 && cp <= 0xDBFF && i + 1 < len)
		{
			UINT32 lo = (UINT32)data[i] | ((UINT32)data[i + 1] << 8);
			if (lo >= 0xDC00 && lo <= 0xDFFF)
		{
				cp = 0x10000 + ((cp - 0xD800) << 10) + (lo - 0xDC00);
				i += 2;
			}
		}
		if (o + 5 >= cap)
		{
			cap *= 2;
			char* n = (char*)realloc(out, cap);
			if (!n) { free(out); return NULL; }
			out = n;
		}
		if (cp < 0x80)
			out[o++] = (char)cp;
		else if (cp < 0x800)
		{
			out[o++] = (char)(0xC0 | (cp >> 6));
			out[o++] = (char)(0x80 | (cp & 0x3F));
		}
		else if (cp < 0x10000)
		{
			out[o++] = (char)(0xE0 | (cp >> 12));
			out[o++] = (char)(0x80 | ((cp >> 6) & 0x3F));
			out[o++] = (char)(0x80 | (cp & 0x3F));
		}
		else
		{
			out[o++] = (char)(0xF0 | (cp >> 18));
			out[o++] = (char)(0x80 | ((cp >> 12) & 0x3F));
			out[o++] = (char)(0x80 | ((cp >> 6) & 0x3F));
			out[o++] = (char)(0x80 | (cp & 0x3F));
		}
	}
	out[o] = '\0';
	return out;
}

uint8_t* jr_utf8_to_utf16le(const char* s, uint32_t* out_len)
{
	size_t n = strlen(s);
	BYTE* out = (BYTE*)malloc(n * 2 + 4);
	size_t o = 0;
	size_t i = 0;
	if (!out)
		return NULL;
	while (i < n)
	{
		unsigned char c = (unsigned char)s[i];
		UINT32 cp = 0;
		if (c < 0x80) { cp = c; i += 1; }
		else if ((c & 0xE0) == 0xC0 && i + 1 < n)
		{ cp = ((c & 0x1F) << 6) | (s[i+1] & 0x3F); i += 2; }
		else if ((c & 0xF0) == 0xE0 && i + 2 < n)
		{ cp = ((c & 0x0F) << 12) | ((s[i+1] & 0x3F) << 6) | (s[i+2] & 0x3F); i += 3; }
		else if ((c & 0xF8) == 0xF0 && i + 3 < n)
		{ cp = ((c & 0x07) << 18) | ((s[i+1] & 0x3F) << 12) | ((s[i+2] & 0x3F) << 6) | (s[i+3] & 0x3F); i += 4; }
		else { i += 1; continue; }
		if (cp >= 0x10000)
		{
			UINT32 v = cp - 0x10000;
			UINT32 hi = 0xD800 + (v >> 10), lo = 0xDC00 + (v & 0x3FF);
			out[o++] = hi & 0xFF; out[o++] = (hi >> 8) & 0xFF;
			out[o++] = lo & 0xFF; out[o++] = (lo >> 8) & 0xFF;
		}
		else
		{
			out[o++] = cp & 0xFF; out[o++] = (cp >> 8) & 0xFF;
		}
	}
	out[o++] = 0; out[o++] = 0; /* null terminator */
	*out_len = (UINT32)o;
	return out;
}

static UINT jr_clip_server_capabilities(CliprdrClientContext* ctx, const CLIPRDR_CAPABILITIES* caps)
{
	(void)ctx;
	(void)caps;
	/* The server responded to our capabilities. Do NOT reply (avoids a
	 * capabilities ping-pong); the handshake is complete once MonitorReady
	 * arrives. */
	fprintf(stderr, "[jr-clip] server capabilities received\n");
	return 0;
}

static UINT jr_clip_monitor_ready(CliprdrClientContext* ctx, const CLIPRDR_MONITOR_READY* mon)
{
	jr_session_t* s = (jr_session_t*)ctx->custom;

	(void)mon;
	if (!s)
		return 0;
	fprintf(stderr, "[jr-clip][session=%p] monitor-ready\n", (void*)s);
	s->clip_ready = 1;
	/* Offer the session-owned snapshot if the initial Mac sync already stored
	 * one (jr_clipboard_sync_start stores it thread-safely on the main thread
	 * before the worker connects). No NSPasteboard access here. */
	if (s->clip_text)
		jr_clip_announce(s);
	return 0;
}

static UINT jr_clip_server_format_list(CliprdrClientContext* ctx, const CLIPRDR_FORMAT_LIST* list)
{
	CLIPRDR_FORMAT_LIST_RESPONSE ack;
	UINT32 want = 0;

	fprintf(stderr, "[jr-clip] server format list: %u formats\n", list->numFormats);
	ZeroMemory(&ack, sizeof(ack));
	ack.common.msgFlags = CB_RESPONSE_OK;
	ctx->ClientFormatListResponse(ctx, &ack);

	for (UINT32 i = 0; i < list->numFormats; i++)
	{
		UINT32 id = list->formats[i].formatId;
		fprintf(stderr, "[jr-clip]   format id=%u\n", id);
		if (id == JR_CF_UNICODETEXT) { want = id; break; }
		if (id == JR_CF_TEXT) want = id;
	}
	if (want)
	{
		CLIPRDR_FORMAT_DATA_REQUEST req;
		fprintf(stderr, "[jr-clip] requesting data for format %u\n", want);
		ZeroMemory(&req, sizeof(req));
		req.requestedFormatId = want;
		ctx->lastRequestedFormatId = want;
		ctx->ClientFormatDataRequest(ctx, &req);
	}
	return 0;
}

static UINT jr_clip_server_format_data_response(CliprdrClientContext* ctx,
                                                const CLIPRDR_FORMAT_DATA_RESPONSE* resp)
{
	jr_session_t* s = (jr_session_t*)ctx->custom;
	char* utf8 = NULL;

	fprintf(stderr, "[jr-clip][session=%p] server data response (fmt %u)\n", (void*)s,
	        ctx->lastRequestedFormatId);
	if (!resp->requestedFormatData)
		return 0;
	if (ctx->lastRequestedFormatId == JR_CF_UNICODETEXT)
		utf8 = jr_utf16le_to_utf8(resp->requestedFormatData, resp->common.dataLen);
	else
		utf8 = strdup((const char*)resp->requestedFormatData);
	if (utf8 && s)
	{
		/* Log direction + length + session only — never the text body. */
		fprintf(stderr, "[jr-clip][session=%p] remote-text len=%zu\n", (void*)s, strlen(utf8));
		/* Only the currently-focused session may write the Mac pasteboard.
		 * `jr_mac_clip_set` re-validates owner/generation on the main thread,
		 * guarding against a delayed A callback overwriting after a tab switch. */
		if (jr_clip_is_owner(s))
			jr_mac_clip_set(s, jr_clip_generation(), utf8);
	}
	free(utf8);
	return 0;
}

static UINT jr_clip_server_format_data_request(CliprdrClientContext* ctx,
                                               const CLIPRDR_FORMAT_DATA_REQUEST* req)
{
	CLIPRDR_FORMAT_DATA_RESPONSE resp;
	char* text = jr_clip_snapshot((jr_session_t*)ctx->custom);
	BYTE* data = NULL;
	UINT32 len = 0;

	fprintf(stderr, "[jr-clip][session=%p] server-request format %u (%s)\n", (void*)ctx->custom,
	        req->requestedFormatId, text ? "has-snapshot" : "empty");

	ZeroMemory(&resp, sizeof(resp));
	if (text && req->requestedFormatId == JR_CF_UNICODETEXT)
		data = jr_utf8_to_utf16le(text, &len);
	else if (text && req->requestedFormatId == JR_CF_TEXT)
	{
		len = (UINT32)strlen(text) + 1;
		data = (BYTE*)malloc(len);
		if (data)
		{
			for (UINT32 i = 0; i < len - 1; i++)
				data[i] = ((unsigned char)text[i] < 0x80) ? (BYTE)text[i] : (BYTE)'?';
			data[len - 1] = 0;
		}
	}
	if (data)
	{
		resp.requestedFormatData = data;
		resp.common.dataLen = len;
		resp.common.msgFlags = CB_RESPONSE_OK;
	}
	else
	{
		resp.common.msgFlags = CB_RESPONSE_FAIL;
	}
	ctx->ClientFormatDataResponse(ctx, &resp);
	free(data);
	free(text);
	return 0;
}

/* Get/wire the cliprdr client context if the channel OPEN has completed.
 * Deterministic fallback: the ChannelConnected event and the interface fetch
 * both race against the connection state machine, so callers (Mac timer,
 * 0.5s cadence) retry through here until it succeeds. */
static int jr_clip_ensure(jr_session_t* s)
{
	if (!s || !s->context)
		return -1;
	if (!s->cliprdr)
	{
		s->cliprdr = (CliprdrClientContext*)freerdp_channels_get_static_channel_interface(
		    s->context->channels, CLIPRDR_SVC_CHANNEL_NAME);
		if (!s->cliprdr)
			return -1;
		jr_clip_wire(s);
	}
	return 0;
}

static void jr_on_channel_connected(void* context, const ChannelConnectedEventArgs* e)
{
	/* The PubSub callback context is the rdpContext (published as
	 * instance->context), NOT the freerdp instance. */
	rdpContext* ctx = (rdpContext*)context;
	jr_session_t* s;

	if (!ctx)
		return;
	s = ((jr_context_t*)ctx)->session;
	if (!s || !e)
		return;
	fprintf(stderr, "[jr-clip] channel connected: %s (iface=%p s=%p sclip=%p)\n",
	        e->name ? e->name : "(null)", e->pInterface, (void*)s, (void*)s->cliprdr);
	if (!e->name || strcmp(e->name, CLIPRDR_SVC_CHANNEL_NAME) != 0)
		return;
	if (s->cliprdr)
	{
		fprintf(stderr, "[jr-clip] ALREADY WIRED, return\n");
		return;
	}
	fprintf(stderr, "[jr-clip] wiring with e->pInterface\n");
	s->cliprdr = (CliprdrClientContext*)e->pInterface;
	jr_clip_wire(s);
}

static void jr_clip_wire(jr_session_t* s)
{
	CLIPRDR_GENERAL_CAPABILITY_SET gset;
	CLIPRDR_CAPABILITIES caps;

	fprintf(stderr, "[jr-clip] jr_clip_wire enter (s=%p clip=%p)\n", (void*)s,
	        s ? (void*)s->cliprdr : NULL);
	if (!s || !s->cliprdr)
	{
		fprintf(stderr, "[jr-clip] wire: no cliprdr interface (yet)\n");
		return;
	}
	s->cliprdr->custom = s;
	s->cliprdr->ServerCapabilities = jr_clip_server_capabilities;
	s->cliprdr->MonitorReady = jr_clip_monitor_ready;
	s->cliprdr->ServerFormatList = jr_clip_server_format_list;
	s->cliprdr->ServerFormatDataResponse = jr_clip_server_format_data_response;
	s->cliprdr->ServerFormatDataRequest = jr_clip_server_format_data_request;

	/* MS-RDPECLIP: the CLIENT initiates the capabilities exchange. (This
	 * mirrors FreeRDP's own X11/SDL clients; waiting for the server first
	 * deadlocks the handshake.) */
	fprintf(stderr, "[jr-clip] sending client capabilities\n");
	gset.capabilitySetType = CB_CAPSTYPE_GENERAL;
	gset.capabilitySetLength = 12;
	gset.version = CB_CAPS_VERSION_2;
	gset.generalFlags = 0;
	caps.cCapabilitiesSets = 1;
	caps.capabilitySets = (CLIPRDR_CAPABILITY_SET*)&gset;
	if (s->cliprdr->ClientCapabilities(s->cliprdr, &caps) != 0)
		fprintf(stderr, "[jr-clip] client capabilities SEND FAILED\n");
}

/* Send a ClientFormatList announcing CF_TEXT + CF_UNICODETEXT. Worker-only
 * (single-owner rule): never called from the AppKit main thread. As a KI-019
 * safety net, `jr_clip_ensure` lazily (re)wires the cliprdr interface if the
 * ChannelConnected event was missed. */
static void jr_clip_announce(jr_session_t* s)
{
	CLIPRDR_FORMAT formats[2];
	CLIPRDR_FORMAT_LIST list;

	if (!s || !s->clip_ready)
		return;
	if (jr_clip_ensure(s) != 0)
		return; /* channel not OPEN yet; retry on the next dispatch */

	fprintf(stderr, "[jr-clip][session=%p] format-list sent\n", (void*)s);
	ZeroMemory(&formats, sizeof(formats));
	formats[0].formatId = JR_CF_TEXT;
	formats[0].formatName = NULL;
	formats[1].formatId = JR_CF_UNICODETEXT;
	formats[1].formatName = NULL;
	ZeroMemory(&list, sizeof(list));
	list.numFormats = 2;
	list.formats = formats;
	if (s->cliprdr->ClientFormatList(s->cliprdr, &list) != 0)
		fprintf(stderr, "[jr-clip][session=%p] ClientFormatList send failed\n", (void*)s);
}

/* Thread-safe snapshot store (no FreeRDP handshake). Safe from the AppKit main
 * thread (initial sync in jr_clipboard_sync_start) AND from the worker
 * (dispatch of JR_CMD_LOCAL_CLIPBOARD_TEXT). */
int jr_clip_store_text(jr_session_t* s, const char* utf8)
{
	if (!s)
		return -1;
	EnterCriticalSection(&s->clip_lock);
	free(s->clip_text);
	s->clip_text = strdup(utf8 ? utf8 : "");
	LeaveCriticalSection(&s->clip_lock);
	return 0;
}

/* Read the session-owned clipboard snapshot (worker only; never NSPasteboard). */
static char* jr_clip_snapshot(jr_session_t* s)
{
	char* out = NULL;
	if (!s)
		return NULL;
	EnterCriticalSection(&s->clip_lock);
	if (s->clip_text)
		out = strdup(s->clip_text);
	LeaveCriticalSection(&s->clip_lock);
	return out;
}

int jr_session_set_size(jr_session_t* s, int width, int height)
{
	rdpSettings* settings;
	if (!s || !s->context)
		return -1;
	settings = s->context->settings;
	if (!freerdp_settings_set_uint32(settings, FreeRDP_DesktopWidth, (UINT32)width))
		return -1;
	if (!freerdp_settings_set_uint32(settings, FreeRDP_DesktopHeight, (UINT32)height))
		return -1;
	return 0;
}
