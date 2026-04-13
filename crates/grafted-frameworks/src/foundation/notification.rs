//! NSNotificationCenter — basic notification dispatch.


/// ObjC: +[NSNotificationCenter defaultCenter]
pub unsafe extern "C" fn ns_notification_center_default(
    _cls: *mut u8, _sel: *mut u8,
) -> *mut u8 {
    static mut CENTER: *mut u8 = std::ptr::null_mut();
    unsafe {
        if CENTER.is_null() {
            CENTER = libc::calloc(1, 256) as *mut u8;
        }
        CENTER
    }
}

/// ObjC: +[NSDistributedNotificationCenter defaultCenter]
pub unsafe extern "C" fn ns_distributed_notification_center_default(
    cls: *mut u8, sel: *mut u8,
) -> *mut u8 {
    unsafe { ns_notification_center_default(cls, sel) }
}

/// ObjC: -[NSNotificationCenter addObserver:selector:name:object:]
pub unsafe extern "C" fn ns_notification_center_add_observer(
    _self: *mut u8, _sel: *mut u8,
    _observer: *mut u8, _selector: *mut u8, _name: *mut u8, _object: *mut u8,
) {
    // Stub — notifications are silently ignored for now
}

/// ObjC: -[NSNotificationCenter removeObserver:]
pub unsafe extern "C" fn ns_notification_center_remove_observer(
    _self: *mut u8, _sel: *mut u8, _observer: *mut u8,
) {}

/// ObjC: -[NSNotificationCenter postNotificationName:object:]
pub unsafe extern "C" fn ns_notification_center_post(
    _self: *mut u8, _sel: *mut u8, _name: *mut u8, _object: *mut u8,
) {}
