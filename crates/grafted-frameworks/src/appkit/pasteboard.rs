//! NSPasteboard — system clipboard access.
//!
//! Bridges Darwin clipboard to X11 selections (CLIPBOARD/PRIMARY).
//! Apps use [NSPasteboard generalPasteboard] to read/write clipboard data.

/// ObjC: +[NSPasteboard generalPasteboard]
pub unsafe extern "C" fn ns_pasteboard_general(_cls: *mut u8, _sel: *mut u8) -> *mut u8 {
    static mut PB: *mut u8 = std::ptr::null_mut();
    unsafe { if PB.is_null() { PB = libc::calloc(1, 256) as *mut u8; } PB }
}

/// ObjC: -[NSPasteboard stringForType:]
pub unsafe extern "C" fn ns_pasteboard_string_for_type(
    _self: *mut u8, _sel: *mut u8, _type: *mut u8,
) -> *mut u8 {
    // TODO: read from X11 CLIPBOARD selection
    std::ptr::null_mut()
}

/// ObjC: -[NSPasteboard setString:forType:]
pub unsafe extern "C" fn ns_pasteboard_set_string(
    _self: *mut u8, _sel: *mut u8, _string: *mut u8, _type: *mut u8,
) -> bool {
    // TODO: write to X11 CLIPBOARD selection
    true
}

/// ObjC: -[NSPasteboard clearContents]
pub unsafe extern "C" fn ns_pasteboard_clear(_self: *mut u8, _sel: *mut u8) -> i64 {
    1 // return change count
}

/// ObjC: -[NSPasteboard changeCount]
pub unsafe extern "C" fn ns_pasteboard_change_count(_self: *mut u8, _sel: *mut u8) -> i64 {
    1
}

/// ObjC: -[NSPasteboard types]
pub unsafe extern "C" fn ns_pasteboard_types(_self: *mut u8, _sel: *mut u8) -> *mut u8 {
    // Return empty array
    unsafe {
        crate::cf::array::CFArrayCreate(std::ptr::null(), std::ptr::null(), 0, std::ptr::null()) as *mut u8
    }
}

/// ObjC: -[NSPasteboard pasteboardItems]
pub unsafe extern "C" fn ns_pasteboard_items(_self: *mut u8, _sel: *mut u8) -> *mut u8 {
    unsafe {
        crate::cf::array::CFArrayCreate(std::ptr::null(), std::ptr::null(), 0, std::ptr::null()) as *mut u8
    }
}
