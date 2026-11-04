//! macOS: FLTK's Cocoa backend. Window gestures use FLTK's portable
//! fallback, glass stays opaque, the theme watcher polls. The appearance
//! (Dark/Light) comes from `AppleInterfaceStyle` through CoreFoundation,
//! which every Cocoa app (FLTK included) already links.

pub(super) use super::generic::*;
use std::ffi::{c_char, c_void};

type CFStringRef = *const c_void;
type CFTypeRef = *const c_void;

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    static kCFPreferencesAnyApplication: CFStringRef;
    fn CFStringCreateWithCString(
        alloc: *const c_void,
        s: *const c_char,
        encoding: u32,
    ) -> CFStringRef;
    fn CFPreferencesAppSynchronize(app: CFStringRef) -> u8;
    fn CFPreferencesCopyAppValue(key: CFStringRef, app: CFStringRef) -> CFTypeRef;
    fn CFGetTypeID(cf: CFTypeRef) -> usize;
    fn CFStringGetTypeID() -> usize;
    fn CFStringGetCString(s: CFStringRef, buf: *mut c_char, size: isize, encoding: u32) -> u8;
    fn CFRelease(cf: CFTypeRef);
}

const UTF8: u32 = 0x0800_0100; // kCFStringEncodingUTF8

/// Dark when the global `AppleInterfaceStyle` is "Dark" (it is absent in
/// Light mode). Re-synchronized on every call so changes are seen live.
pub(super) fn prefers_dark() -> Option<bool> {
    // SAFETY: CoreFoundation calls with valid, owned or constant objects;
    // every object created or copied here is released.
    unsafe {
        let key =
            CFStringCreateWithCString(std::ptr::null(), c"AppleInterfaceStyle".as_ptr(), UTF8);
        if key.is_null() {
            return None;
        }
        CFPreferencesAppSynchronize(kCFPreferencesAnyApplication);
        let value = CFPreferencesCopyAppValue(key, kCFPreferencesAnyApplication);
        CFRelease(key);
        if value.is_null() {
            return Some(false); // Light mode: the key is not set
        }
        let mut buf = [0 as c_char; 16];
        let dark = CFGetTypeID(value) == CFStringGetTypeID()
            && CFStringGetCString(value, buf.as_mut_ptr(), buf.len() as isize, UTF8) != 0
            && std::ffi::CStr::from_ptr(buf.as_ptr())
                .to_bytes()
                .eq_ignore_ascii_case(b"dark");
        CFRelease(value);
        Some(dark)
    }
}
