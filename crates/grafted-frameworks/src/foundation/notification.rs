//! NSNotificationCenter - real observer dispatch.

use std::sync::Mutex;

struct Observer {
    observer: *mut u8,
    selector: *mut u8,
    /// Registered name (None = wildcard / nil).
    name: Option<String>,
    /// Registered object (None = wildcard / nil, otherwise a pointer to
    /// the CFString/ObjC object passed at registration time).
    object: *mut u8,
}

unsafe impl Send for Observer {}

static OBSERVERS: Mutex<Vec<Observer>> = Mutex::new(Vec::new());

fn read_cfstring(ptr: *mut u8) -> Option<String> {
    if ptr.is_null() { return None; }
    // Try CFStringInner
    unsafe {
        let s = ptr as *const crate::cf::string::CFStringInner;
        let rb = &(*s).base;
        if rb.type_id() == crate::cf::types::CF_STRING_TYPE_ID {
            return std::str::from_utf8(&(*s).bytes).ok().map(String::from);
        }
    }
    // Fall back: raw C string (for Darwin-emitted name constants that are byte arrays)
    unsafe {
        let c = std::ffi::CStr::from_ptr(ptr as *const i8);
        c.to_str().ok().map(String::from)
    }
}

/// Fire `selector` on `observer` with a single argument `notification_ptr`.
unsafe fn dispatch_one(observer: *mut u8, selector: *mut u8, notification: *mut u8) {
    if observer.is_null() || selector.is_null() { return; }
    unsafe {
        let Some(imp) = grafted_objc::grafted_lookup_method(observer as *mut _, selector as *const _)
            else { return; };
        type Callback = unsafe extern "C" fn(*mut u8, *mut u8, *mut u8);
        let f: Callback = std::mem::transmute(imp);
        f(observer, selector, notification);
    }
}


/// +[NSNotificationCenter defaultCenter]
pub unsafe extern "C" fn ns_notification_center_default(
    _cls: *mut u8, _sel: *mut u8,
) -> *mut u8 {
    static CENTER: std::sync::atomic::AtomicPtr<u8> =
        std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());
    let p = CENTER.load(std::sync::atomic::Ordering::Acquire);
    if !p.is_null() { return p; }
    let obj = unsafe { libc::calloc(1, 256) } as *mut u8;
    if obj.is_null() { return CENTER.load(std::sync::atomic::Ordering::Acquire); }
    match CENTER.compare_exchange(
        std::ptr::null_mut(), obj,
        std::sync::atomic::Ordering::AcqRel,
        std::sync::atomic::Ordering::Acquire,
    ) {
        Ok(_) => obj,
        Err(winner) => { unsafe { libc::free(obj as *mut libc::c_void) }; winner }
    }
}

/// +[NSDistributedNotificationCenter defaultCenter]
pub unsafe extern "C" fn ns_distributed_notification_center_default(
    cls: *mut u8, sel: *mut u8,
) -> *mut u8 {
    // Distributed = process-local fallback; the cross-process semantics
    // aren't implemented because we're single-process inside grafted.
    unsafe { ns_notification_center_default(cls, sel) }
}

/// -[NSNotificationCenter addObserver:selector:name:object:]
pub unsafe extern "C" fn ns_notification_center_add_observer(
    _self: *mut u8, _sel: *mut u8,
    observer: *mut u8, selector: *mut u8, name: *mut u8, object: *mut u8,
) {
    let name_str = read_cfstring(name);
    let mut g = OBSERVERS.lock().unwrap();
    g.push(Observer {
        observer,
        selector,
        name: name_str.clone(),
        object,
    });
    log::debug!("NSNotificationCenter: observer {:p} registered for name={:?} object={:p}",
        observer, name_str, object);
}

/// -[NSNotificationCenter removeObserver:]
pub unsafe extern "C" fn ns_notification_center_remove_observer(
    _self: *mut u8, _sel: *mut u8, observer: *mut u8,
) {
    let mut g = OBSERVERS.lock().unwrap();
    g.retain(|o| o.observer != observer);
}

/// -[NSNotificationCenter removeObserver:name:object:]
pub unsafe extern "C" fn ns_notification_center_remove_observer_name_object(
    _self: *mut u8, _sel: *mut u8,
    observer: *mut u8, name: *mut u8, object: *mut u8,
) {
    let name_str = read_cfstring(name);
    let mut g = OBSERVERS.lock().unwrap();
    g.retain(|o| {
        let observer_match = o.observer == observer;
        let name_match = name_str.is_none() || o.name == name_str;
        let object_match = object.is_null() || o.object == object;
        !(observer_match && name_match && object_match)
    });
}

/// -[NSNotificationCenter postNotificationName:object:]
pub unsafe extern "C" fn ns_notification_center_post(
    _self: *mut u8, _sel: *mut u8, name: *mut u8, object: *mut u8,
) {
    post_by_name(name, object);
}

/// -[NSNotificationCenter postNotificationName:object:userInfo:]
pub unsafe extern "C" fn ns_notification_center_post_user_info(
    _self: *mut u8, _sel: *mut u8, name: *mut u8, object: *mut u8, _user_info: *mut u8,
) {
    // userInfo dict is part of NSNotification; we don't build a full
    // NSNotification object yet - we pass `name` as the argument.
    post_by_name(name, object);
}

fn post_by_name(name: *mut u8, object: *mut u8) {
    let name_str = read_cfstring(name);
    log::debug!("NSNotificationCenter: post name={:?}", name_str);
    // Collect matching observers first (clone the pointers) so we release the
    // lock before dispatch - observers may call back into the center.
    let matches: Vec<(*mut u8, *mut u8)> = {
        let g = OBSERVERS.lock().unwrap();
        g.iter().filter_map(|o| {
            let name_match = o.name.is_none() || o.name == name_str;
            let object_match = o.object.is_null() || object.is_null() || o.object == object;
            if name_match && object_match {
                Some((o.observer, o.selector))
            } else {
                None
            }
        }).collect()
    };
    for (observer, selector) in matches {
        // Pass `name` as the notification argument - good enough for
        unsafe { dispatch_one(observer, selector, name) };
    }
}

/// Public helper so internal event sources (clipboard change poller etc.)
pub fn post_notification_name_static(name_cstr: &[u8]) {
    assert!(name_cstr.last() == Some(&0), "name_cstr must be NUL-terminated");
    // Wrap in a CFString so read_cfstring produces the right name string.
    let cf = unsafe {
        crate::cf::string::CFStringCreateWithCString(
            std::ptr::null(), name_cstr.as_ptr() as *const i8, 0x0800_0100,
        ) as *mut u8
    };
    post_by_name(cf, std::ptr::null_mut());
}


#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    static CB_COUNT: AtomicU32 = AtomicU32::new(0);
    static CB_NAME_BYTES: std::sync::Mutex<Vec<u8>> = std::sync::Mutex::new(Vec::new());

    // Test observer callback - records that it fired and saves the name arg.
    unsafe extern "C" fn test_callback(_self: *mut u8, _sel: *mut u8, notification: *mut u8) {
        CB_COUNT.fetch_add(1, Ordering::SeqCst);
        if !notification.is_null() {
            let cf = notification as *const crate::cf::string::CFStringInner;
            let bytes = unsafe { &(*cf).bytes };
            *CB_NAME_BYTES.lock().unwrap() = bytes.clone();
        }
    }

    fn reset() {
        OBSERVERS.lock().unwrap().clear();
        CB_COUNT.store(0, Ordering::SeqCst);
        CB_NAME_BYTES.lock().unwrap().clear();
    }

    fn cfstr(s: &str) -> *mut u8 {
        let c = std::ffi::CString::new(s).unwrap();
        unsafe {
            crate::cf::string::CFStringCreateWithCString(
                std::ptr::null(), c.as_ptr(), 0x0800_0100,
            ) as *mut u8
        }
    }

    /// A fake "observer" object - we register it as a class with our ObjC
    /// runtime so grafted_lookup_method finds our callback when dispatched.
    fn make_observer_with_selector(sel_name: &str) -> (*mut u8, *mut u8) {
        use grafted_objc::{types::Class, types::class_t, types::class_ro_t,
            objc_registerClassPair, sel_registerName, class_addMethod};
        // Register a tiny class
        let cls = unsafe { libc::calloc(1, 256) } as Class;
        let ro = unsafe { libc::calloc(1, 256) } as *mut class_ro_t;
        let cls_name = std::ffi::CString::new(
            format!("NotifTestObserver_{}", std::ptr::addr_of!(CB_COUNT) as usize)
        ).unwrap();
        unsafe {
            (*ro).name = cls_name.into_raw();
            (*ro).instance_size = 64;
            (*(cls as *mut class_t)).data = ro;
        }
        objc_registerClassPair(cls);
        let sel_c = std::ffi::CString::new(sel_name).unwrap();
        let sel = sel_registerName(sel_c.as_ptr());
        class_addMethod(cls, sel, Some(unsafe { std::mem::transmute(test_callback as *const ()) }), std::ptr::null());
        // Allocate an instance
        let obj = unsafe { libc::calloc(1, 64) } as *mut u8;
        unsafe { *(obj as *mut *const core::ffi::c_void) = cls as *const _; }
        (obj, sel as *mut u8)
    }

    #[test]
    fn observer_fires_on_matching_name() {
        reset();
        let (obs, sel) = make_observer_with_selector("onClipboardChange:");
        unsafe {
            ns_notification_center_add_observer(
                std::ptr::null_mut(), std::ptr::null_mut(),
                obs, sel,
                cfstr("NSPasteboardDidChangeNotification"),
                std::ptr::null_mut(),
            );
            ns_notification_center_post(
                std::ptr::null_mut(), std::ptr::null_mut(),
                cfstr("NSPasteboardDidChangeNotification"),
                std::ptr::null_mut(),
            );
        }
        assert_eq!(CB_COUNT.load(Ordering::SeqCst), 1, "observer should fire exactly once");
    }

    #[test]
    fn observer_does_not_fire_on_different_name() {
        reset();
        let (obs, sel) = make_observer_with_selector("onFocusChange:");
        unsafe {
            ns_notification_center_add_observer(
                std::ptr::null_mut(), std::ptr::null_mut(),
                obs, sel,
                cfstr("NSApplicationDidBecomeActiveNotification"),
                std::ptr::null_mut(),
            );
            ns_notification_center_post(
                std::ptr::null_mut(), std::ptr::null_mut(),
                cfstr("SomeOtherNotification"),
                std::ptr::null_mut(),
            );
        }
        assert_eq!(CB_COUNT.load(Ordering::SeqCst), 0, "mismatched name must not fire");
    }

    #[test]
    fn nil_name_is_wildcard() {
        reset();
        let (obs, sel) = make_observer_with_selector("onAny:");
        unsafe {
            ns_notification_center_add_observer(
                std::ptr::null_mut(), std::ptr::null_mut(),
                obs, sel,
                std::ptr::null_mut(),  // nil name = wildcard
                std::ptr::null_mut(),
            );
            ns_notification_center_post(
                std::ptr::null_mut(), std::ptr::null_mut(),
                cfstr("RandomEvent"),
                std::ptr::null_mut(),
            );
            ns_notification_center_post(
                std::ptr::null_mut(), std::ptr::null_mut(),
                cfstr("AnotherEvent"),
                std::ptr::null_mut(),
            );
        }
        assert_eq!(CB_COUNT.load(Ordering::SeqCst), 2, "nil name must match every post");
    }

    #[test]
    fn remove_observer_stops_firing() {
        reset();
        let (obs, sel) = make_observer_with_selector("onEvent:");
        unsafe {
            ns_notification_center_add_observer(
                std::ptr::null_mut(), std::ptr::null_mut(),
                obs, sel,
                cfstr("Event"), std::ptr::null_mut(),
            );
            ns_notification_center_post(
                std::ptr::null_mut(), std::ptr::null_mut(),
                cfstr("Event"), std::ptr::null_mut(),
            );
            ns_notification_center_remove_observer(
                std::ptr::null_mut(), std::ptr::null_mut(), obs,
            );
            ns_notification_center_post(
                std::ptr::null_mut(), std::ptr::null_mut(),
                cfstr("Event"), std::ptr::null_mut(),
            );
        }
        assert_eq!(CB_COUNT.load(Ordering::SeqCst), 1, "removed observer must not fire on subsequent posts");
    }
}
