/* Native desktop surface — macOS NSView.
 *
 * CALayer-backed; the FreeRDP framebuffer is blitted here (never through JS).
 * Every mutating operation is dispatched to the AppKit main thread (GCD), so
 * the C functions may be called from any thread. The framebuffer passed to
 * `present_buffer` is copied *before* dispatch because FreeRDP reuses it as
 * soon as the paint callback returns.
 *
 * Byte order: gdi_init uses PIXEL_FORMAT_BGRX32 (B,G,R,X in memory), matched by
 * kCGBitmapByteOrder32Little | kCGImageAlphaNoneSkipFirst (the shipping FreeRDP
 * Mac client combo, client/Mac/MRDPView.m).
 */

#import <Cocoa/Cocoa.h>
#import <QuartzCore/QuartzCore.h>
#include "bridge.h"

static void jr_cg_buffer_release(void* info, const void* data, size_t size)
{
	(void)info;
	(void)size;
	free((void*)data);
}

/* Apple virtual key code -> XT (PS/2 set 1) make code; `ext` = E0-prefixed. */
static BOOL jr_scancode_for_vk(unsigned short vk, unsigned char* out, BOOL* ext);

/* Modifier keys forwarded via flagsChanged (KI-023 stuck-modifier guard):
 * vk -> RDP scancode + shared NSEventModifierFlag bit + E0 preview flag.
 * Note AppKit exposes only ONE combined bit per modifier KIND (no L/R
 * distinction in modifierFlags), so L/R pairs share a bit; flagsChanged
 * disambiguates them via the vk of the event itself. */
typedef struct
{
	unsigned short vk;
	unsigned char sc;
	BOOL ext;
	NSUInteger bit;
} jr_mod_entry;

static const jr_mod_entry kJRMods[] = {
    {0x39, 0x3A, NO, NSEventModifierFlagCapsLock}, /* CapsLock        */
    {0x38, 0x2A, NO, NSEventModifierFlagShift},    /* LShift          */
    {0x3C, 0x36, NO, NSEventModifierFlagShift},    /* RShift          */
    {0x3B, 0x1D, NO, NSEventModifierFlagControl},  /* LCtrl           */
    {0x3E, 0x1D, YES, NSEventModifierFlagControl}, /* RCtrl  (E0)     */
    {0x3A, 0x38, NO, NSEventModifierFlagOption},   /* LOpt            */
    {0x3D, 0x38, YES, NSEventModifierFlagOption},  /* ROpt   (E0)     */
    {0x37, 0x5B, YES, NSEventModifierFlagCommand}, /* LCmd   (E0, Win)*/
    {0x36, 0x5C, YES, NSEventModifierFlagCommand}, /* RCmd   (E0, Win)*/
};
#define JR_MOD_COUNT (sizeof(kJRMods) / sizeof(kJRMods[0]))

static int jr_mod_index(unsigned short vk)
{
	size_t i;
	for (i = 0; i < JR_MOD_COUNT; i++)
	{
		if (kJRMods[i].vk == vk)
			return (int)i;
	}
	return -1;
}

@interface JRView : NSView
- (void)presentBuffer:(uint8_t*)buffer width:(int)w height:(int)h stride:(int)s;
- (void)setFillRed:(CGFloat)r green:(CGFloat)g blue:(CGFloat)b;
- (void)attachInput:(jr_session_t*)session;
@end

@implementation JRView
{
	jr_session_t* _inputSession; /* non-owning; NULL while detached */
	int _pressedButtons;         /* bitmask 1=left 2=right 4=middle */
	NSUInteger _lastModifiers;   /* last modifierFlags we saw (delta base) */
	BOOL _modHeld[JR_MOD_COUNT]; /* per-vk physical states we sent (KI-023) */
	id _becomeKeyObs;            /* window did-become-key observer        */
	id _resignKeyObs;            /* window did-resign-key observer        */
	id _becomeActiveObs;         /* NSApp did-become-active observer      */
	id _resignActiveObs;         /* NSApp did-resign-active observer      */
	int _dragLog;
	int _moveLog;
}

- (instancetype)initWithFrame:(NSRect)frame
{
	self = [super initWithFrame:frame];
	if (self)
	{
		NSNotificationCenter* nc = [NSNotificationCenter defaultCenter];
		JRView* __weak ws = self;
		self.wantsLayer = YES;
		self.layer.magnificationFilter = kCAFilterLinear;
		self.layer.contentsGravity = kCAGravityResizeAspect;
		self.layer.backgroundColor =
		    [[NSColor colorWithCalibratedWhite:0.12 alpha:1.0] CGColor];
		/* App-level focus/activation sweeps (KI-023): when the app stops/gets
		 * activated, the remote modifier state must be reset — macOS delivers
		 * no flagsChanged for a Cmd released while the app is inactive, and a
		 * forgotten release means a permanently stuck Super on the remote. */
		_becomeActiveObs = [nc addObserverForName:NSApplicationDidBecomeActiveNotification
		                                 object:nil queue:nil usingBlock:^(NSNotification* note) {
		  (void)note;
		  JRView* s = ws;
		  if (s)
			  [s jrKeyboardSweepRelease];
		}];
		_resignActiveObs = [nc addObserverForName:NSApplicationDidResignActiveNotification
		                                 object:nil queue:nil usingBlock:^(NSNotification* note) {
		  (void)note;
		  JRView* s = ws;
		  if (s)
			  [s jrKeyboardSweepRelease];
		}];
	}
	return self;
}

- (void)dealloc
{
	[self jrUnsubscribeWindowNotifications];
	if (_becomeActiveObs)
		[[NSNotificationCenter defaultCenter] removeObserver:_becomeActiveObs];
	if (_resignActiveObs)
		[[NSNotificationCenter defaultCenter] removeObserver:_resignActiveObs];
}

- (BOOL)isOpaque
{
	return YES;
}

/* `buffer` ownership transfers here; it is freed by the CGDataProvider release
 * callback once the layer replaces or drops the image. */
- (void)presentBuffer:(uint8_t*)buffer width:(int)w height:(int)h stride:(int)s
{
	CGColorSpaceRef cs;
	CGDataProviderRef provider;
	CGImageRef image;

	if (!buffer || w <= 0 || h <= 0 || s <= 0)
	{
		free(buffer);
		return;
	}

	cs = CGColorSpaceCreateDeviceRGB();
	provider = CGDataProviderCreateWithData(NULL, buffer, (size_t)s * (size_t)h,
	                                        jr_cg_buffer_release);
	image = CGImageCreate((size_t)w, (size_t)h, 8, 32, (size_t)s, cs,
	                      kCGImageAlphaNoneSkipFirst | kCGBitmapByteOrder32Little, provider,
	                      NULL, NO, kCGRenderingIntentDefault);
	if (image)
	{
		self.layer.contents = (__bridge id)image;
		CGImageRelease(image);
	}
	CGDataProviderRelease(provider);
	CGColorSpaceRelease(cs);
}

- (void)setFillRed:(CGFloat)r green:(CGFloat)g blue:(CGFloat)b
{
	self.layer.contents = nil;
	self.layer.backgroundColor =
	    [[NSColor colorWithCalibratedRed:r green:g blue:b alpha:1.0] CGColor];
}

/* ------------------------------------------------------------------ */
/* Input forwarding (Phase 4B-2): AppKit events -> RDP input channel.  */
/* ------------------------------------------------------------------ */

- (void)attachInput:(jr_session_t*)session
{
	_inputSession = session;
	_pressedButtons = 0;
	_lastModifiers = 0;
	memset(_modHeld, 0, sizeof(_modHeld));
	if (session)
	{
		/* Clear any modifier state a reused xrdp X session still believes is
		 * held (KI-023). No-op while the session isn't connected yet; the
		 * bridge re-runs the same reset from PostConnect once it is. Also runs
		 * on every tab-switch refocus. */
		if (jr_session_reset_keyboard_modifiers(session) == 0)
			NSLog(@"[jr-input] attach: keyboard modifier reset sent");
		if (self.window)
			[self.window makeFirstResponder:self];
	}
	else if (self.window && self.window.firstResponder == self)
		[self.window makeFirstResponder:self.nextResponder];
}

/* Release every modifier the remote side might believe is held, and drop the
 * local per-vk model. Runs on: window resign/become key, app resign/become
 * active, input attach. Releasing a key that isn't down is a no-op remotely.
 * We deliberately do NOT re-assert currently-held modifiers afterwards: with
 * only the combined NSEventModifierFlag bits (no L/R distinction) re-sending
 * "held" states could itself create a new stuck key; the next real
 * flagsChanged resyncs naturally. */
- (void)jrKeyboardSweepRelease
{
	if (!_inputSession)
		return;
	if (jr_session_reset_keyboard_modifiers(_inputSession) == 0)
		NSLog(@"[jr-input] keyboard modifier sweep sent (focus/window change)");
	memset(_modHeld, 0, sizeof(_modHeld));
	_lastModifiers = 0;
}

- (void)jrUnsubscribeWindowNotifications
{
	NSNotificationCenter* nc = [NSNotificationCenter defaultCenter];
	if (_becomeKeyObs)
	{
		[nc removeObserver:_becomeKeyObs];
		_becomeKeyObs = nil;
	}
	if (_resignKeyObs)
	{
		[nc removeObserver:_resignKeyObs];
		_resignKeyObs = nil;
	}
}

- (BOOL)acceptsFirstResponder
{
	return YES;
}

- (BOOL)acceptsFirstMouse:(NSEvent*)event
{
	(void)event;
	return YES;
}

- (void)viewDidMoveToWindow
{
	[super viewDidMoveToWindow];
	[self jrUnsubscribeWindowNotifications];
	if (self.window)
	{
		NSNotificationCenter* nc = [NSNotificationCenter defaultCenter];
		JRView* __weak ws = self;
		NSWindow* win = self.window;
		_becomeKeyObs = [nc addObserverForName:NSWindowDidBecomeKeyNotification
		                               object:win queue:nil usingBlock:^(NSNotification* note) {
		  (void)note;
		  JRView* s = ws;
		  if (s)
			  [s jrKeyboardSweepRelease];
		}];
		_resignKeyObs = [nc addObserverForName:NSWindowDidResignKeyNotification
		                               object:win queue:nil usingBlock:^(NSNotification* note) {
		  (void)note;
		  JRView* s = ws;
		  if (s)
			  [s jrKeyboardSweepRelease];
		}];
		if (_inputSession)
			[self.window makeFirstResponder:self];
	}
}

- (void)updateTrackingAreas
{
	[super updateTrackingAreas];
	for (NSTrackingArea* t in [self.trackingAreas copy])
		[self removeTrackingArea:t];
	NSTrackingArea* t = [[NSTrackingArea alloc]
	    initWithRect:self.bounds
	           options:NSTrackingMouseMoved | NSTrackingActiveInKeyWindow |
	                   NSTrackingInVisibleRect
	             owner:self
	          userInfo:nil];
	[self addTrackingArea:t];
}

/* Map a window point to desktop pixels, compensating for the aspect-fit
 * (letterboxed) layer contents. Y is flipped (AppKit bottom-left -> RDP top-left). */
- (BOOL)mapPoint:(NSPoint)loc toX:(int*)outX y:(int*)outY
{
	int dw = 0, dh = 0;
	NSSize vb;
	CGFloat scale, cw, ch, ox, oy;
	NSPoint p;
	CGFloat x, y;

	if (!_inputSession)
		return NO;
	if (jr_session_get_size(_inputSession, &dw, &dh) != 0 || dw <= 0 || dh <= 0)
		return NO;
	vb = self.bounds.size;
	if (vb.width <= 0 || vb.height <= 0)
		return NO;
	scale = MIN(vb.width / (CGFloat)dw, vb.height / (CGFloat)dh);
	if (scale <= 0)
		return NO;
	cw = dw * scale;
	ch = dh * scale;
	ox = (vb.width - cw) / 2;
	oy = (vb.height - ch) / 2;
	p = [self convertPoint:loc fromView:nil];
	x = (p.x - ox) / scale;
	y = (vb.height - p.y - oy) / scale;
	if (x < 0)
		x = 0;
	if (x > dw - 1)
		x = dw - 1;
	if (y < 0)
		y = 0;
	if (y > dh - 1)
		y = dh - 1;
	*outX = (int)x;
	*outY = (int)y;
	return YES;
}

- (void)mouseDown:(NSEvent*)e
{
	int x, y;
	NSLog(@"[jr-input] mouseDown");
	if (self.window.firstResponder != self)
		[self.window makeFirstResponder:self];
	if ([self mapPoint:e.locationInWindow toX:&x y:&y])
	{
		_pressedButtons |= 1;
		jr_session_send_mouse_button(_inputSession, 1, 1, x, y);
	}
}

- (void)mouseUp:(NSEvent*)e
{
	int x, y;
	NSLog(@"[jr-input] mouseUp");
	_pressedButtons &= ~1;
	/* xrdp: a release is BUTTON1 without DOWN (a pure MOVE never releases). */
	if ([self mapPoint:e.locationInWindow toX:&x y:&y])
		jr_session_send_mouse_button(_inputSession, 1, 0, x, y);
}

- (void)rightMouseDown:(NSEvent*)e
{
	int x, y;
	if ([self mapPoint:e.locationInWindow toX:&x y:&y])
	{
		_pressedButtons |= 2;
		jr_session_send_mouse_button(_inputSession, 2, 1, x, y);
	}
}

- (void)rightMouseUp:(NSEvent*)e
{
	int x, y;
	_pressedButtons &= ~2;
	if ([self mapPoint:e.locationInWindow toX:&x y:&y])
		jr_session_send_mouse_button(_inputSession, 2, 0, x, y);
}

- (void)otherMouseDown:(NSEvent*)e
{
	int x, y;
	if (e.buttonNumber != 2)
		return;
	if ([self mapPoint:e.locationInWindow toX:&x y:&y])
	{
		_pressedButtons |= 4;
		jr_session_send_mouse_button(_inputSession, 3, 1, x, y);
	}
}

- (void)otherMouseUp:(NSEvent*)e
{
	int x, y;
	if (e.buttonNumber != 2)
		return;
	_pressedButtons &= ~4;
	if ([self mapPoint:e.locationInWindow toX:&x y:&y])
		jr_session_send_mouse_button(_inputSession, 3, 0, x, y);
}

- (void)mouseDragged:(NSEvent*)e
{
	int x, y;
	if ([self mapPoint:e.locationInWindow toX:&x y:&y])
	{
		_dragLog++;
		if (_dragLog % 10 == 1)
			NSLog(@"[jr-input] mouseDragged #%d x=%d y=%d buttons=%d", _dragLog, x, y,
			      _pressedButtons);
		jr_session_send_mouse_move(_inputSession, x, y, _pressedButtons);
	}
	else
		NSLog(@"[jr-input] mouseDragged MAPFAIL");
}

- (void)rightMouseDragged:(NSEvent*)e
{
	[self mouseDragged:e];
}

- (void)otherMouseDragged:(NSEvent*)e
{
	[self mouseDragged:e];
}

- (void)mouseMoved:(NSEvent*)e
{
	int x, y;
	_moveLog++;
	if (_moveLog % 50 == 1)
		NSLog(@"[jr-input] mouseMoved #%d", _moveLog);
	if ([self mapPoint:e.locationInWindow toX:&x y:&y])
		jr_session_send_mouse_move(_inputSession, x, y, _pressedButtons);
}

- (void)scrollWheel:(NSEvent*)e
{
	int x, y;
	CGFloat dy = e.scrollingDeltaY;
	CGFloat dx = e.scrollingDeltaX;
	int dyi, dxi, ny = 0, nx = 0;

	if (![self mapPoint:e.locationInWindow toX:&x y:&y])
		return;
	if (!e.hasPreciseScrollingDeltas)
	{
		dy *= 120; /* discrete notches -> RDP WHEEL_DELTA */
		dx *= 120;
	}
	else
	{
		dy *= 3; /* trackpad: scale pixel deltas into wheel units */
		dx *= 3;
	}
	dyi = (int)MAX(-511, MIN(511, dy));
	dxi = (int)MAX(-511, MIN(511, dx));
	if (dyi < 0)
		ny = 1;
	if (dxi < 0)
		nx = 1;
	jr_session_send_mouse_wheel(_inputSession, abs(dyi), ny, abs(dxi), nx, x, y);
}

- (void)keyDown:(NSEvent*)e
{
	unsigned char sc;
	BOOL ext;
	if (!_inputSession)
		return;
	if (!jr_scancode_for_vk(e.keyCode, &sc, &ext))
		return;
	jr_session_send_key_scancode(_inputSession, 1, e.isARepeat ? 1 : 0, sc, ext ? 1 : 0);
	NSLog(@"[jr-input] keyDown vk=0x%02X sc=0x%02X%s repeat=%d", e.keyCode, sc,
	      ext ? " ext" : "", e.isARepeat ? 1 : 0);
}

- (void)keyUp:(NSEvent*)e
{
	unsigned char sc;
	BOOL ext;
	if (!_inputSession)
		return;
	if (!jr_scancode_for_vk(e.keyCode, &sc, &ext))
		return;
	jr_session_send_key_scancode(_inputSession, 0, 0, sc, ext ? 1 : 0);
	NSLog(@"[jr-input] keyUp vk=0x%02X sc=0x%02X%s", e.keyCode, sc, ext ? " ext" : "");
}

- (void)flagsChanged:(NSEvent*)e
{
	int idx = jr_mod_index(e.keyCode);
	const jr_mod_entry* m;
	NSUInteger now;
	NSUInteger prev;
	int down;

	if (!_inputSession || idx < 0)
		return;
	m = &kJRMods[idx];
	now = e.modifierFlags;
	prev = _lastModifiers;

	if ((now ^ prev) & m->bit)
	{
		/* The OS flag bit transitioned: authoritative direction. */
		down = (now & m->bit) ? 1 : 0;
	}
	else if (m->bit != NSEventModifierFlagCapsLock && (now & m->bit))
	{
		/* A second physical key of an L/R pair (e.g. RShift pressed while
		 * LShift is held) does not change the shared flag bit. Each physical
		 * key keeps its own last state and toggles, so press/release stay
		 * balanced on the wire instead of being dropped or doubled. */
		down = _modHeld[idx] ? 0 : 1;
	}
	else
	{
		/* No observable change (stale event, or release of a key the remote
		 * never learned about after a sweep): sending it would unbalance the
		 * remote state, so skip. */
		_lastModifiers = now;
		return;
	}

	jr_session_send_key_scancode(_inputSession, down, 0, m->sc, m->ext ? 1 : 0);
	_modHeld[idx] = down;
	_lastModifiers = now;
	NSLog(@"[jr-input] flagsChanged vk=0x%02X sc=0x%02X%s -> %s", e.keyCode, m->sc,
	      m->ext ? " ext" : "", down ? "DOWN" : "UP");
}

@end

/* Apple virtual key code -> XT (PS/2 set 1) make code; `ext` = E0-prefixed. */
static BOOL jr_scancode_for_vk(unsigned short vk, unsigned char* out, BOOL* ext)
{
	*ext = NO;
	switch (vk)
	{
		case 0x00: *out = 0x1E; return YES; /* A */
		case 0x01: *out = 0x1F; return YES; /* S */
		case 0x02: *out = 0x20; return YES; /* D */
		case 0x03: *out = 0x21; return YES; /* F */
		case 0x04: *out = 0x23; return YES; /* H */
		case 0x05: *out = 0x22; return YES; /* G */
		case 0x06: *out = 0x2C; return YES; /* Z */
		case 0x07: *out = 0x2D; return YES; /* X */
		case 0x08: *out = 0x2E; return YES; /* C */
		case 0x09: *out = 0x2F; return YES; /* V */
		case 0x0B: *out = 0x30; return YES; /* B */
		case 0x0C: *out = 0x10; return YES; /* Q */
		case 0x0D: *out = 0x11; return YES; /* W */
		case 0x0E: *out = 0x12; return YES; /* E */
		case 0x0F: *out = 0x13; return YES; /* R */
		case 0x10: *out = 0x15; return YES; /* Y */
		case 0x11: *out = 0x14; return YES; /* T */
		case 0x12: *out = 0x02; return YES; /* 1 */
		case 0x13: *out = 0x03; return YES; /* 2 */
		case 0x14: *out = 0x04; return YES; /* 3 */
		case 0x15: *out = 0x05; return YES; /* 4 */
		case 0x16: *out = 0x07; return YES; /* 6 */
		case 0x17: *out = 0x06; return YES; /* 5 */
		case 0x18: *out = 0x0D; return YES; /* = */
		case 0x19: *out = 0x0A; return YES; /* 9 */
		case 0x1A: *out = 0x08; return YES; /* 7 */
		case 0x1B: *out = 0x0C; return YES; /* - */
		case 0x1C: *out = 0x09; return YES; /* 8 */
		case 0x1D: *out = 0x0B; return YES; /* 0 */
		case 0x1E: *out = 0x1B; return YES; /* ] */
		case 0x1F: *out = 0x18; return YES; /* O */
		case 0x20: *out = 0x16; return YES; /* U */
		case 0x21: *out = 0x1A; return YES; /* [ */
		case 0x22: *out = 0x17; return YES; /* I */
		case 0x23: *out = 0x19; return YES; /* P */
		case 0x24: *out = 0x1C; return YES; /* Return */
		case 0x25: *out = 0x26; return YES; /* L */
		case 0x26: *out = 0x24; return YES; /* J */
		case 0x27: *out = 0x28; return YES; /* ' */
		case 0x28: *out = 0x25; return YES; /* K */
		case 0x29: *out = 0x27; return YES; /* ; */
		case 0x2A: *out = 0x2B; return YES; /* \ */
		case 0x2B: *out = 0x33; return YES; /* , */
		case 0x2C: *out = 0x35; return YES; /* / */
		case 0x2D: *out = 0x31; return YES; /* N */
		case 0x2E: *out = 0x32; return YES; /* M */
		case 0x2F: *out = 0x34; return YES; /* . */
		case 0x30: *out = 0x0F; return YES; /* Tab */
		case 0x31: *out = 0x39; return YES; /* Space */
		case 0x32: *out = 0x29; return YES; /* ` */
		case 0x33: *out = 0x0E; return YES; /* Backspace */
		case 0x35: *out = 0x01; return YES; /* Escape */
		case 0x41: *out = 0x53; return YES; /* KP . */
		case 0x43: *out = 0x37; return YES; /* KP * */
		case 0x45: *out = 0x4E; return YES; /* KP + */
		case 0x4B: *out = 0x35; *ext = YES; return YES; /* KP / */
		case 0x4C: *out = 0x1C; *ext = YES; return YES; /* KP Enter */
		case 0x4E: *out = 0x4A; return YES; /* KP - */
		case 0x52: *out = 0x52; return YES; /* KP 0 */
		case 0x53: *out = 0x4F; return YES; /* KP 1 */
		case 0x54: *out = 0x50; return YES; /* KP 2 */
		case 0x55: *out = 0x51; return YES; /* KP 3 */
		case 0x56: *out = 0x4B; return YES; /* KP 4 */
		case 0x57: *out = 0x4C; return YES; /* KP 5 */
		case 0x58: *out = 0x4D; return YES; /* KP 6 */
		case 0x59: *out = 0x47; return YES; /* KP 7 */
		case 0x5B: *out = 0x48; return YES; /* KP 8 */
		case 0x5C: *out = 0x49; return YES; /* KP 9 */
		case 0x60: *out = 0x3F; return YES; /* F5 */
		case 0x61: *out = 0x40; return YES; /* F6 */
		case 0x62: *out = 0x41; return YES; /* F7 */
		case 0x63: *out = 0x3D; return YES; /* F3 */
		case 0x64: *out = 0x42; return YES; /* F8 */
		case 0x65: *out = 0x43; return YES; /* F9 */
		case 0x67: *out = 0x57; return YES; /* F11 */
		case 0x69: *out = 0x64; return YES; /* F13 */
		case 0x6B: *out = 0x65; return YES; /* F14 */
		case 0x6D: *out = 0x44; return YES; /* F10 */
		case 0x6F: *out = 0x58; return YES; /* F12 */
		case 0x71: *out = 0x66; return YES; /* F15 */
		case 0x72: *out = 0x52; *ext = YES; return YES; /* Help/Insert */
		case 0x73: *out = 0x47; *ext = YES; return YES; /* Home */
		case 0x74: *out = 0x49; *ext = YES; return YES; /* PageUp */
		case 0x75: *out = 0x53; *ext = YES; return YES; /* ForwardDelete */
		case 0x76: *out = 0x3E; return YES; /* F4 */
		case 0x77: *out = 0x4F; *ext = YES; return YES; /* End */
		case 0x78: *out = 0x3C; return YES; /* F2 */
		case 0x79: *out = 0x51; *ext = YES; return YES; /* PageDown */
		case 0x7A: *out = 0x3B; return YES; /* F1 */
		case 0x7B: *out = 0x4B; *ext = YES; return YES; /* Left */
		case 0x7C: *out = 0x4D; *ext = YES; return YES; /* Right */
		case 0x7D: *out = 0x50; *ext = YES; return YES; /* Down */
		case 0x7E: *out = 0x48; *ext = YES; return YES; /* Up */
		default:
			return NO;
	}
}

static void run_on_main(void (^block)(void))
{
	if ([NSThread isMainThread])
		block();
	else
		dispatch_async(dispatch_get_main_queue(), block);
}

static CGFloat jr_content_safe_top(NSView* content)
{
	if (@available(macOS 11.0, *))
		return content.safeAreaInsets.top;
	return 0.0;
}

void* jr_view_create(void)
{
	__block JRView* v = nil;
	if ([NSThread isMainThread])
	{
		v = [[JRView alloc] initWithFrame:NSMakeRect(0, 0, 640, 480)];
	}
	else
	{
		dispatch_sync(dispatch_get_main_queue(), ^{
		  v = [[JRView alloc] initWithFrame:NSMakeRect(0, 0, 640, 480)];
		});
	}
	return (__bridge_retained void*)v;
}

void jr_view_destroy(void* handle)
{
	if (!handle)
		return;
	run_on_main(^{
	  JRView* v = (__bridge_transfer JRView*)handle;
	  (void)v;
	});
}

void jr_view_set_frame(void* handle, double x, double y_top, double w, double h)
{
	JRView* v = (__bridge JRView*)handle;
	run_on_main(^{
	  NSView* superview = v.superview;
	  double sy = 0.0;
	  if (superview)
		  sy = (double)superview.bounds.size.height - (y_top + h); /* flip y */
	  v.frame = NSMakeRect((CGFloat)x, (CGFloat)sy, (CGFloat)w, (CGFloat)h);
	});
}

void jr_view_add_to_window(void* handle, void* ns_window)
{
	JRView* v = (__bridge JRView*)handle;
	NSWindow* win = (__bridge NSWindow*)ns_window;
	run_on_main(^{
	  NSView* content = win.contentView;
	  v.frame = content.bounds;
	  v.autoresizingMask = NSViewWidthSizable | NSViewHeightSizable;
	  [content addSubview:v positioned:NSWindowAbove relativeTo:nil];
	});
}

void jr_view_add_to_window_inset(void* handle, void* ns_window, double top_inset)
{
	JRView* v = (__bridge JRView*)handle;
	NSWindow* win = (__bridge NSWindow*)ns_window;
	run_on_main(^{
	  NSView* content = win.contentView;
	  CGRect b = content.bounds;
	  CGFloat tabInset = top_inset > 0 ? (CGFloat)top_inset : 0;
	  /* Tauri's transparent/full-size content view extends underneath the
	   * macOS title bar. The web tab row starts below that safe area, while a
	   * sibling NSView is positioned in the full content-view coordinates.
	   * Reserving only the 44pt tab row therefore covered most of the tabs.
	   * Keep BOTH regions clear. */
	  CGFloat safeTop = jr_content_safe_top(content);
	  CGFloat inset = safeTop + tabInset;
	  /* AppKit origin is bottom-left: pin the view bottom, leave `inset` free
	   * at the top. Width+height sizable with fixed margins preserves the
	   * inset across window resizes. */
	  v.frame = NSMakeRect(0, 0, b.size.width, MAX(b.size.height - inset, 100));
	  v.autoresizingMask = NSViewWidthSizable | NSViewHeightSizable;
	  [content addSubview:v positioned:NSWindowAbove relativeTo:nil];
	  NSLog(@"[jr-view] mount bounds=%.0fx%.0f safeTop=%.0f tabInset=%.0f frame=%.0fx%.0f",
	        b.size.width, b.size.height, safeTop, tabInset, v.frame.size.width,
	        v.frame.size.height);
	});
}

void jr_view_remove_from_window(void* handle)
{
	JRView* v = (__bridge JRView*)handle;
	run_on_main(^{
	  [v removeFromSuperview];
	});
}

void jr_view_set_fill(void* handle, uint8_t r, uint8_t g, uint8_t b)
{
	JRView* v = (__bridge JRView*)handle;
	run_on_main(^{
	  [v setFillRed:(CGFloat)r / 255.0 green:(CGFloat)g / 255.0 blue:(CGFloat)b / 255.0];
	});
}

void jr_view_attach_input(void* handle, void* session)
{
	JRView* v = (__bridge JRView*)handle;
	if (!v)
		return;
	/* Synchronous: detach must complete before the session is destroyed. */
	if ([NSThread isMainThread])
		[v attachInput:(jr_session_t*)session];
	else
		dispatch_sync(dispatch_get_main_queue(), ^{
		  [v attachInput:(jr_session_t*)session];
		});
}

void jr_window_content_size(void* ns_window, double* w, double* h)
{
	NSWindow* win = (__bridge NSWindow*)ns_window;
	if (!win || !w || !h)
		return;
	if ([NSThread isMainThread])
	{
		*w = (double)win.contentView.bounds.size.width;
		*h = (double)win.contentView.bounds.size.height;
	}
	else
		dispatch_sync(dispatch_get_main_queue(), ^{
		  *w = (double)win.contentView.bounds.size.width;
		  *h = (double)win.contentView.bounds.size.height;
		});
}

double jr_window_safe_area_top(void* ns_window)
{
	NSWindow* win = (__bridge NSWindow*)ns_window;
	if (!win)
		return 0.0;
	__block double top = 0.0;
	if ([NSThread isMainThread])
		top = (double)jr_content_safe_top(win.contentView);
	else
		dispatch_sync(dispatch_get_main_queue(), ^{
		  top = (double)jr_content_safe_top(win.contentView);
		});
	return top;
}

void jr_view_present_buffer(void* handle, const uint8_t* buffer, int width, int height,
                            int stride, int dirty_x, int dirty_y, int dirty_w, int dirty_h)
{
	JRView* v = (__bridge JRView*)handle;
	size_t bytes;
	uint8_t* copy;

	(void)dirty_x;
	(void)dirty_y;
	(void)dirty_w;
	(void)dirty_h; /* full-frame present for the SPIKE (dirty-rect refinement later) */

	if (!buffer || width <= 0 || height <= 0 || stride <= 0)
		return;

	/* Copy out of the FreeRDP-owned buffer before it is reused. */
	bytes = (size_t)stride * (size_t)height;
	copy = (uint8_t*)malloc(bytes);
	if (!copy)
		return;
	memcpy(copy, buffer, bytes);

	run_on_main(^{
	  [v presentBuffer:copy width:width height:height stride:stride];
	});
}
/* ------------------------------------------------------------------ */
/* Clipboard sync: NSPasteboard <-> CLIPRDR (text only).               */
/* ------------------------------------------------------------------ */

static jr_session_t* g_clipSession = NULL;
static NSTimer* g_clipTimer = nil;
static NSInteger g_lastCount = 0;
static BOOL g_suppressNext = NO;
static NSString* g_lastText = nil;

void jr_mac_clip_set(const char* utf8)
{
	NSString* s = [NSString stringWithUTF8String:(utf8 ? utf8 : "")];
	run_on_main(^{
	  NSPasteboard* pb = [NSPasteboard generalPasteboard];
	  g_suppressNext = YES;
	  [pb clearContents];
	  [pb setString:s forType:NSPasteboardTypeString];
	  g_lastCount = pb.changeCount;
	  g_lastText = s;
	});
}

char* jr_mac_clip_get(void)
{
	if ([NSThread isMainThread])
	{
		NSString* s = [[NSPasteboard generalPasteboard] stringForType:NSPasteboardTypeString];
		return s ? strdup([s UTF8String]) : NULL;
	}
	__block char* out = NULL;
	/* CLIPRDR callbacks run on the FreeRDP worker thread; NSPasteboard
	 * belongs on the AppKit main thread. */
	dispatch_sync(dispatch_get_main_queue(), ^{
	  NSString* s = [[NSPasteboard generalPasteboard] stringForType:NSPasteboardTypeString];
	  out = s ? strdup([s UTF8String]) : NULL;
	});
	return out;
}

static void jr_clip_timer_tick(NSTimer* t)
{
	NSPasteboard* pb;
	NSInteger c;
	NSString* s;

	(void)t;
	if (!g_clipSession)
		return;
	pb = [NSPasteboard generalPasteboard];
	c = pb.changeCount;
	if (c == g_lastCount)
		return;
	g_lastCount = c;
	if (g_suppressNext)
	{
		g_suppressNext = NO;
		return;
	}
	s = [pb stringForType:NSPasteboardTypeString];
	if (!s)
		return;
	if (g_lastText && [s isEqualToString:g_lastText])
		return;
	g_lastText = s;
	jr_session_set_clipboard_text(g_clipSession, [s UTF8String]);
}

void jr_clipboard_sync_start(void* session)
{
	run_on_main(^{
	  g_clipSession = (jr_session_t*)session;
	  g_lastCount = [[NSPasteboard generalPasteboard] changeCount];
	  g_lastText = nil;
	  g_suppressNext = NO;
	  if (g_clipTimer)
		  [g_clipTimer invalidate];
	  g_clipTimer = [NSTimer scheduledTimerWithTimeInterval:0.5
		                                            repeats:YES
		                                              block:^(NSTimer* t) {
		                                                jr_clip_timer_tick(t);
		                                              }];
	});
}

void jr_clipboard_sync_stop(void)
{
	run_on_main(^{
	  if (g_clipTimer)
	  {
		  [g_clipTimer invalidate];
		  g_clipTimer = nil;
	  }
	  g_clipSession = NULL;
	});
}
