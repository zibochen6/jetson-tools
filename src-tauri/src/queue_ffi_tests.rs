//! Rust FFI tests for the C command queue (native/freerdp_bridge/queue.c).
//!
//! The queue is compiled into `freerdp_bridge_c` by `build.rs`, so these unit
//! tests link and exercise the real C implementation — no re-implementation,
//! no mock. This is the §30 "Command queue" gate.

use std::ffi::{c_char, c_void, CStr, CString};

#[repr(C)]
struct JrCmd {
    kind: i32,
    a: i32,
    b: i32,
    c: i32,
    d: i32,
    e: i32,
    f: i32,
    owned_utf8: *mut c_char,
}

const K_MOVE: i32 = 0;
const K_BUTTON: i32 = 1;
const K_WHEEL: i32 = 2;
const K_KEY: i32 = 3;
const K_UNICODE: i32 = 4;
const K_CLIPBOARD: i32 = 5;
const K_RESIZE: i32 = 6;
const K_RESET: i32 = 7;

extern "C" {
    fn jr_cmdq_create() -> *mut c_void;
    fn jr_cmdq_destroy(q: *mut c_void);
    fn jr_cmdq_enqueue_move(q: *mut c_void, x: i32, y: i32);
    fn jr_cmdq_enqueue_button(q: *mut c_void, button: i32, down: i32, x: i32, y: i32);
    fn jr_cmdq_enqueue_wheel(
        q: *mut c_void,
        delta: i32,
        negative: i32,
        hdelta: i32,
        hnegative: i32,
        x: i32,
        y: i32,
    );
    fn jr_cmdq_enqueue_scancode(
        q: *mut c_void,
        down: i32,
        repeat: i32,
        scancode: i32,
        extended: i32,
    );
    fn jr_cmdq_enqueue_unicode(q: *mut c_void, utf8: *const c_char);
    fn jr_cmdq_enqueue_clipboard(q: *mut c_void, utf8: *const c_char);
    fn jr_cmdq_enqueue_resize(q: *mut c_void, w: i32, h: i32);
    fn jr_cmdq_enqueue_reset_modifiers(q: *mut c_void);
    fn jr_cmdq_drain(
        q: *mut c_void,
        cb: Option<unsafe extern "C" fn(*mut JrCmd, *mut c_void)>,
        user: *mut c_void,
    ) -> usize;
}

#[derive(Default)]
struct Recorder {
    kinds: Vec<i32>,
    moves: Vec<(i32, i32)>,
    buttons: Vec<(i32, i32)>,
    texts: Vec<String>,
    key_codes: Vec<i32>,
}

unsafe extern "C" fn record(cmd: *mut JrCmd, user: *mut c_void) {
    let c = unsafe { &*cmd };
    let rec = unsafe { &mut *(user as *mut Recorder) };
    rec.kinds.push(c.kind);
    match c.kind {
        K_MOVE => rec.moves.push((c.a, c.b)),
        K_BUTTON => rec.buttons.push((c.a, c.b)),
        K_KEY => rec.key_codes.push(c.c),
        K_UNICODE | K_CLIPBOARD => {
            let s = if c.owned_utf8.is_null() {
                String::new()
            } else {
                unsafe { CStr::from_ptr(c.owned_utf8) }
                    .to_string_lossy()
                    .into_owned()
            };
            rec.texts.push(s);
        }
        _ => {}
    }
}

fn new_q() -> *mut c_void {
    let q = unsafe { jr_cmdq_create() };
    assert!(!q.is_null(), "queue allocation failed");
    q
}

fn drain(q: *mut c_void) -> Recorder {
    let mut rec = Recorder::default();
    unsafe {
        jr_cmdq_drain(q, Some(record), &mut rec as *mut Recorder as *mut c_void);
    }
    rec
}

fn drain_count(q: *mut c_void) -> usize {
    let mut rec = Recorder::default();
    unsafe { jr_cmdq_drain(q, Some(record), &mut rec as *mut Recorder as *mut c_void) }
}

#[test]
fn mouse_button_ordering_is_fifo() {
    let q = new_q();
    unsafe {
        jr_cmdq_enqueue_button(q, 1, 1, 10, 20); // left DOWN
        jr_cmdq_enqueue_button(q, 1, 0, 10, 20); // left UP
        jr_cmdq_enqueue_button(q, 2, 1, 30, 40); // right DOWN
        jr_cmdq_enqueue_button(q, 2, 0, 30, 40); // right UP
    }
    let rec = drain(q);
    assert_eq!(rec.kinds, vec![K_BUTTON, K_BUTTON, K_BUTTON, K_BUTTON]);
    assert_eq!(rec.buttons, vec![(1, 1), (1, 0), (2, 1), (2, 0)]);
    unsafe { jr_cmdq_destroy(q) };
}

#[test]
fn wheel_scancode_resize_roundtrip() {
    let q = new_q();
    unsafe {
        jr_cmdq_enqueue_wheel(q, 120, 0, 0, 0, 40, 50);
        jr_cmdq_enqueue_scancode(q, 1, 0, 0x1d, 0); // LCtrl down
        jr_cmdq_enqueue_resize(q, 1920, 1080);
    }
    let rec = drain(q);
    assert_eq!(rec.kinds, vec![K_WHEEL, K_KEY, K_RESIZE]);
    assert_eq!(rec.key_codes, vec![0x1d]);
    unsafe { jr_cmdq_destroy(q) };
}

#[test]
fn consecutive_moves_coalesce_to_latest() {
    let q = new_q();
    unsafe {
        jr_cmdq_enqueue_button(q, 1, 1, 0, 0); // DOWN
        jr_cmdq_enqueue_move(q, 1, 1);
        jr_cmdq_enqueue_move(q, 2, 2);
        jr_cmdq_enqueue_move(q, 3, 3);
        jr_cmdq_enqueue_button(q, 1, 0, 3, 3); // UP
    }
    let rec = drain(q);
    // DOWN, single coalesced MOVE(3,3), UP.
    assert_eq!(rec.kinds, vec![K_BUTTON, K_MOVE, K_BUTTON]);
    assert_eq!(rec.moves, vec![(3, 3)]);
    unsafe { jr_cmdq_destroy(q) };
}

#[test]
fn no_coalescing_across_button_transition() {
    let q = new_q();
    unsafe {
        jr_cmdq_enqueue_button(q, 1, 1, 0, 0); // DOWN
        jr_cmdq_enqueue_move(q, 1, 1);
        jr_cmdq_enqueue_button(q, 1, 0, 1, 1); // UP
        jr_cmdq_enqueue_move(q, 2, 2); // new move AFTER up must not merge
    }
    let rec = drain(q);
    assert_eq!(rec.kinds, vec![K_BUTTON, K_MOVE, K_BUTTON, K_MOVE]);
    assert_eq!(rec.moves, vec![(1, 1), (2, 2)]);
    unsafe { jr_cmdq_destroy(q) };
}

#[test]
fn clipboard_command_between_moves_does_not_alter_button_lifecycle() {
    let q = new_q();
    unsafe {
        let mac = CString::new("MAC").unwrap();
        jr_cmdq_enqueue_button(q, 1, 1, 5, 5); // DOWN
        jr_cmdq_enqueue_move(q, 6, 6);
        jr_cmdq_enqueue_clipboard(q, mac.as_ptr()); // clipboard in the middle
        jr_cmdq_enqueue_move(q, 7, 7);
        jr_cmdq_enqueue_button(q, 1, 0, 7, 7); // UP
    }
    let rec = drain(q);
    // Button lifecycle must be intact: DOWN ... UP, moves only coalesce with
    // adjacent moves (the clipboard command is a boundary).
    assert_eq!(
        rec.kinds,
        vec![K_BUTTON, K_MOVE, K_CLIPBOARD, K_MOVE, K_BUTTON]
    );
    assert_eq!(rec.buttons, vec![(1, 1), (1, 0)]);
    assert_eq!(rec.moves, vec![(6, 6), (7, 7)]); // NOT merged across clipboard
    unsafe { jr_cmdq_destroy(q) };
}

#[test]
fn text_commands_carry_owned_utf8() {
    let q = new_q();
    unsafe {
        let mac = CString::new("MAC_123中文").unwrap();
        let remote = CString::new("REMOTE_456").unwrap();
        jr_cmdq_enqueue_unicode(q, mac.as_ptr());
        jr_cmdq_enqueue_clipboard(q, remote.as_ptr());
    }
    let rec = drain(q);
    assert_eq!(rec.kinds, vec![K_UNICODE, K_CLIPBOARD]);
    assert_eq!(rec.texts, vec!["MAC_123中文", "REMOTE_456"]);
    unsafe { jr_cmdq_destroy(q) };
}

#[test]
fn no_command_after_destroy() {
    // destroy() frees pending owned strings, marks the queue destroyed, and
    // releases memory; every enqueue entry point also NULL-guards so a freed /
    // never-created queue is safe.
    let q = new_q();
    unsafe {
        let pending = CString::new("pending").unwrap();
        jr_cmdq_enqueue_unicode(q, pending.as_ptr());
        jr_cmdq_enqueue_button(q, 1, 1, 0, 0);
        jr_cmdq_destroy(q); // frees pending text, frees the allocation
    }
    // Create/destroy round-trip stays clean afterwards.
    let q2 = new_q();
    unsafe { jr_cmdq_enqueue_resize(q2, 1280, 800) };
    assert_eq!(drain_count(q2), 1);
    unsafe { jr_cmdq_destroy(q2) };
    // NULL queue is a hard no-op for every path.
    unsafe {
        jr_cmdq_enqueue_move(std::ptr::null_mut(), 1, 1);
        jr_cmdq_enqueue_button(std::ptr::null_mut(), 1, 1, 0, 0);
        jr_cmdq_enqueue_unicode(std::ptr::null_mut(), std::ptr::null());
        jr_cmdq_destroy(std::ptr::null_mut());
    }
}

#[test]
fn overflow_evicts_oldest_move_never_a_button() {
    let q = new_q();
    unsafe {
        // One move, then fill the rest with buttons (buttons never coalesce).
        jr_cmdq_enqueue_move(q, 1, 1);
        for i in 0..4095 {
            jr_cmdq_enqueue_button(q, 1, i % 2, i, 0);
        }
        // Queue is now full: [MOVE, 4095 buttons]. Enqueue one more button —
        // the oldest MOVE must be evicted, never a button.
        jr_cmdq_enqueue_button(q, 1, 1, 9999, 0);
    }
    let rec = drain(q);
    assert_eq!(rec.kinds.len(), 4096);
    assert!(
        !rec.kinds.contains(&K_MOVE),
        "oldest move should have been evicted"
    );
    assert_eq!(rec.kinds.iter().filter(|k| **k == K_BUTTON).count(), 4096);
    // The newest button survived at the tail.
    assert_eq!(rec.buttons.last(), Some(&(1, 1)));
    unsafe { jr_cmdq_destroy(q) };
}

#[test]
fn reset_modifiers_enqueues_as_distinct_command() {
    let q = new_q();
    unsafe {
        jr_cmdq_enqueue_reset_modifiers(q);
        jr_cmdq_enqueue_reset_modifiers(q);
    }
    let rec = drain(q);
    assert_eq!(rec.kinds, vec![K_RESET, K_RESET]);
    unsafe { jr_cmdq_destroy(q) };
}

#[test]
fn drain_on_empty_returns_zero() {
    let q = new_q();
    assert_eq!(drain_count(q), 0);
    unsafe { jr_cmdq_destroy(q) };
}
