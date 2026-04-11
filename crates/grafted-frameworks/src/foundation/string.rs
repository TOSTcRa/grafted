//! NSString — toll-free bridged to CFString.
//!
//! On Darwin, NSString and CFString are the same object. We implement NSString
//! methods as ObjC methods on the NSString class that operate on CFStringInner.

use crate::cf::string::CFStringInner;
use crate::cf::types::*;

/// ObjC: -[NSString length]
pub unsafe extern "C" fn ns_string_length(self_: *mut u8, _sel: *mut u8) -> u64 {
    if self_.is_null() { return 0; }
    let cf = self_ as *const CFStringInner;
    unsafe { (*cf).bytes.len() as u64 }
}

/// ObjC: -[NSString UTF8String]
pub unsafe extern "C" fn ns_string_utf8(self_: *mut u8, _sel: *mut u8) -> *const i8 {
    if self_.is_null() { return std::ptr::null(); }
    let cf = self_ as *const CFStringInner;
    // Safety: CFStringInner.bytes is UTF-8 without embedded NULs (usually)
    unsafe { (*cf).bytes.as_ptr() as *const i8 }
}

/// ObjC: -[NSString isEqualToString:]
pub unsafe extern "C" fn ns_string_is_equal(self_: *mut u8, _sel: *mut u8, other: *mut u8) -> bool {
    if self_.is_null() || other.is_null() { return self_ == other; }
    let s1 = self_ as *const CFStringInner;
    let s2 = other as *const CFStringInner;
    unsafe { (*s1).bytes == (*s2).bytes }
}

/// ObjC: +[NSString stringWithUTF8String:]
pub unsafe extern "C" fn ns_string_with_utf8(
    _cls: *mut u8, _sel: *mut u8, cstr: *const i8,
) -> *mut u8 {
    unsafe {
        crate::cf::string::CFStringCreateWithCString(
            std::ptr::null(), cstr, 0x0800_0100,
        ) as *mut u8
    }
}

/// ObjC: +[NSString stringWithFormat:] (simplified — just returns the format string)
pub unsafe extern "C" fn ns_string_with_format(
    _cls: *mut u8, _sel: *mut u8, format: *mut u8,
) -> *mut u8 {
    if format.is_null() { return std::ptr::null_mut(); }
    unsafe { CFRetain(format as CFTypeRef) };
    format
}

/// ObjC: -[NSString description] — returns self
pub unsafe extern "C" fn ns_string_description(self_: *mut u8, _sel: *mut u8) -> *mut u8 {
    self_
}

/// ObjC: -[NSString hash]
pub unsafe extern "C" fn ns_string_hash(self_: *mut u8, _sel: *mut u8) -> u64 {
    if self_.is_null() { return 0; }
    let cf = self_ as *const CFStringInner;
    // Simple FNV-like hash
    let mut h: u64 = 14695981039346656037;
    for &b in unsafe { &(*cf).bytes } {
        h ^= b as u64;
        h = h.wrapping_mul(1099511628211);
    }
    h
}
