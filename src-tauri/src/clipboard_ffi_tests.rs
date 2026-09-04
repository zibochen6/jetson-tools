//! Rust FFI tests for the text codecs (native/freerdp_bridge/bridge.c):
//! UTF-8 <-> UTF-16LE used by the clipboard and the IME commit path (§30).

use std::ffi::{c_char, c_void, CStr, CString};

extern "C" {
    fn jr_utf8_to_utf16le(utf8: *const c_char, out_len: *mut u32) -> *mut u8;
    fn jr_utf16le_to_utf8(data: *const u8, len: u32) -> *mut c_char;
}

/// Call the C codec and return the UTF-16 code units (little-endian decoded).
fn to_utf16_units(s: &str) -> Vec<u16> {
    let c = CString::new(s).unwrap();
    let mut len: u32 = 0;
    let p = unsafe { jr_utf8_to_utf16le(c.as_ptr(), &mut len) };
    assert!(!p.is_null(), "jr_utf8_to_utf16le returned NULL for {s:?}");
    let n_units = (len as usize) / 2 - 1; // `len` includes the UTF-16 NULL terminator
    let mut units = Vec::with_capacity(n_units);
    for i in 0..n_units {
        let lo = unsafe { *p.add(i * 2) } as u16;
        let hi = unsafe { *p.add(i * 2 + 1) } as u16;
        units.push(lo | (hi << 8));
    }
    unsafe { libc::free(p as *mut c_void) };
    units
}

fn from_utf16_units(units: &[u16]) -> String {
    let mut bytes = Vec::with_capacity(units.len() * 2 + 2);
    for u in units {
        bytes.push((u & 0xFF) as u8);
        bytes.push((u >> 8) as u8);
    }
    bytes.push(0);
    bytes.push(0);
    let p = unsafe { jr_utf16le_to_utf8(bytes.as_ptr(), bytes.len() as u32) };
    assert!(!p.is_null());
    let s = unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned();
    unsafe { libc::free(p as *mut c_void) };
    s
}

#[test]
fn utf8_to_utf16le_bmp_chinese() {
    // "中" = U+4E2D, "文" = U+6587.
    let units = to_utf16_units("中文");
    assert_eq!(units, vec![0x4E2D, 0x6587]);
    assert_eq!(from_utf16_units(&units), "中文");
}

#[test]
fn utf8_to_utf16le_mixed_ascii_and_chinese() {
    let s = "MAC_123中文";
    let units = to_utf16_units(s);
    // 'M','A','C','_','1','2','3','中','文'
    assert_eq!(
        units,
        vec![0x4D, 0x41, 0x43, 0x5F, 0x31, 0x32, 0x33, 0x4E2D, 0x6587]
    );
    assert_eq!(from_utf16_units(&units), s);
}

#[test]
fn utf8_to_utf16le_emoji_surrogate_pair() {
    // "🌍" = U+1F30D → surrogate pair 0xD83C 0xDF0D.
    let units = to_utf16_units("🌍");
    assert_eq!(units, vec![0xD83C, 0xDF0D]);
    // Supplementary round-trip through the surrogate pair.
    assert_eq!(from_utf16_units(&[0xD83C, 0xDF0D]), "🌍");
}

#[test]
fn utf16le_to_utf8_bmp_chinese() {
    assert_eq!(from_utf16_units(&[0x4F60, 0x597D]), "你好");
}

#[test]
fn utf8_utf16_roundtrip() {
    for s in ["", "abcDEF123", "你好 Jetson", "测试ABC你好", "🌍 emoji 🎉"] {
        let units = to_utf16_units(s);
        assert_eq!(from_utf16_units(&units), s, "roundtrip failed for {s:?}");
    }
}
