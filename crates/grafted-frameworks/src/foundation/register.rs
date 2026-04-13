//! Register Foundation classes with the ObjC runtime.

use grafted_objc::{objc_registerClassPair, sel_registerName, class_addMethod, types::Class};
use std::sync::Once;

use super::{string, bundle, notification};

static REGISTER_ONCE: Once = Once::new();

pub fn register_all() {
    REGISTER_ONCE.call_once(|| {
        register_nsstring();
        register_nsnumber();
        register_nsbundle();
        register_nsnotification_center();
        register_nsprocessinfo();
        register_nsfilemanager();
        register_nsuserdefaults();
        log::info!("Foundation: registered classes with ObjC runtime");
    });
}

fn reg(cls: Class, name: &str, imp: *const ()) {
    let sel = sel_registerName(name.as_ptr() as *const i8);
    class_addMethod(cls, sel, Some(unsafe { std::mem::transmute(imp) }), std::ptr::null());
}

fn alloc_class(name: &str, instance_size: usize) -> Class {
    let cls = unsafe { libc::calloc(1, 256) } as Class;
    let ro = unsafe { libc::calloc(1, 256) } as *mut grafted_objc::types::class_ro_t;
    let c_name = std::ffi::CString::new(name).unwrap();
    unsafe {
        (*ro).name = c_name.into_raw();
        (*ro).instance_size = instance_size as u32;
        (*(cls as *mut grafted_objc::types::class_t)).data = ro;
    }
    objc_registerClassPair(cls);
    cls
}

fn register_nsstring() {
    let cls = alloc_class("NSString", 64);
    reg(cls, "length\0", string::ns_string_length as *const ());
    reg(cls, "UTF8String\0", string::ns_string_utf8 as *const ());
    reg(cls, "isEqualToString:\0", string::ns_string_is_equal as *const ());
    reg(cls, "stringWithUTF8String:\0", string::ns_string_with_utf8 as *const ());
    reg(cls, "stringWithFormat:\0", string::ns_string_with_format as *const ());
    reg(cls, "description\0", string::ns_string_description as *const ());
    reg(cls, "hash\0", string::ns_string_hash as *const ());

    // NSMutableString = NSString for now
    let mcls = alloc_class("NSMutableString", 64);
    reg(mcls, "length\0", string::ns_string_length as *const ());
    reg(mcls, "UTF8String\0", string::ns_string_utf8 as *const ());

    // NSMutableAttributedString
    alloc_class("NSMutableAttributedString", 128);
    alloc_class("NSAttributedString", 128);
}

fn register_nsnumber() {
    let cls = alloc_class("NSNumber", 32);

    unsafe extern "C" fn ns_number_with_int(_cls: *mut u8, _sel: *mut u8, val: i32) -> *mut u8 { unsafe {
        let obj = libc::calloc(1, 32) as *mut u8;
        *(obj.add(16) as *mut i64) = val as i64;
        obj
    }}
    unsafe extern "C" fn ns_number_int_value(self_: *mut u8, _sel: *mut u8) -> i64 { unsafe {
        if self_.is_null() { 0 } else { *(self_.add(16) as *const i64) }
    }}
    unsafe extern "C" fn ns_number_bool_value(self_: *mut u8, _sel: *mut u8) -> bool { unsafe {
        if self_.is_null() { false } else { *(self_.add(16) as *const i64) != 0 }
    }}

    reg(cls, "numberWithInt:\0", ns_number_with_int as *const ());
    reg(cls, "numberWithInteger:\0", ns_number_with_int as *const ());
    reg(cls, "numberWithBool:\0", ns_number_with_int as *const ());
    reg(cls, "intValue\0", ns_number_int_value as *const ());
    reg(cls, "integerValue\0", ns_number_int_value as *const ());
    reg(cls, "boolValue\0", ns_number_bool_value as *const ());

    alloc_class("NSNull", 16);
    alloc_class("NSNumberFormatter", 64);
    alloc_class("NSByteCountFormatter", 64);
}

fn register_nsbundle() {
    let cls = alloc_class("NSBundle", 1024);
    reg(cls, "mainBundle\0", bundle::ns_bundle_main as *const ());
    reg(cls, "bundlePath\0", bundle::ns_bundle_path as *const ());
    reg(cls, "resourcePath\0", bundle::ns_bundle_resource_path as *const ());
    reg(cls, "infoDictionary\0", bundle::ns_bundle_info_dict as *const ());
    reg(cls, "bundleIdentifier\0", bundle::ns_bundle_identifier as *const ());
}

fn register_nsnotification_center() {
    let cls = alloc_class("NSNotificationCenter", 256);
    reg(cls, "defaultCenter\0", notification::ns_notification_center_default as *const ());
    reg(cls, "addObserver:selector:name:object:\0", notification::ns_notification_center_add_observer as *const ());
    reg(cls, "removeObserver:\0", notification::ns_notification_center_remove_observer as *const ());
    reg(cls, "postNotificationName:object:\0", notification::ns_notification_center_post as *const ());

    let dcls = alloc_class("NSDistributedNotificationCenter", 256);
    reg(dcls, "defaultCenter\0", notification::ns_distributed_notification_center_default as *const ());
    reg(dcls, "addObserver:selector:name:object:\0", notification::ns_notification_center_add_observer as *const ());
}

fn register_nsprocessinfo() {
    let cls = alloc_class("NSProcessInfo", 256);

    unsafe extern "C" fn process_info(_cls: *mut u8, _sel: *mut u8) -> *mut u8 {
        static mut PI: *mut u8 = std::ptr::null_mut();
        unsafe { if PI.is_null() { PI = libc::calloc(1, 256) as *mut u8; } PI }
    }
    unsafe extern "C" fn process_name(_self: *mut u8, _sel: *mut u8) -> *mut u8 { unsafe {
        crate::cf::string::CFStringCreateWithCString(
            std::ptr::null(), b"Grafted\0".as_ptr() as *const i8, 0x0800_0100,
        ) as *mut u8
    }}
    unsafe extern "C" fn os_version(_self: *mut u8, _sel: *mut u8) -> [i64; 3] {
        [14, 0, 0] // macOS 14 Sonoma
    }

    reg(cls, "processInfo\0", process_info as *const ());
    reg(cls, "processName\0", process_name as *const ());
    reg(cls, "operatingSystemVersion\0", os_version as *const ());
}

fn register_nsfilemanager() {
    let cls = alloc_class("NSFileManager", 64);

    unsafe extern "C" fn default_manager(_cls: *mut u8, _sel: *mut u8) -> *mut u8 {
        static mut FM: *mut u8 = std::ptr::null_mut();
        unsafe { if FM.is_null() { FM = libc::calloc(1, 64) as *mut u8; } FM }
    }

    reg(cls, "defaultManager\0", default_manager as *const ());
}

fn register_nsuserdefaults() {
    let cls = alloc_class("NSUserDefaults", 256);

    unsafe extern "C" fn standard(_cls: *mut u8, _sel: *mut u8) -> *mut u8 {
        static mut UD: *mut u8 = std::ptr::null_mut();
        unsafe { if UD.is_null() { UD = libc::calloc(1, 256) as *mut u8; } UD }
    }
    unsafe extern "C" fn object_for_key(_self: *mut u8, _sel: *mut u8, _key: *mut u8) -> *mut u8 {
        std::ptr::null_mut() // not found
    }
    unsafe extern "C" fn set_object(_self: *mut u8, _sel: *mut u8, _val: *mut u8, _key: *mut u8) {}
    unsafe extern "C" fn bool_for_key(_self: *mut u8, _sel: *mut u8, _key: *mut u8) -> bool { false }
    unsafe extern "C" fn register_defaults(_self: *mut u8, _sel: *mut u8, _dict: *mut u8) {}

    reg(cls, "standardUserDefaults\0", standard as *const ());
    reg(cls, "objectForKey:\0", object_for_key as *const ());
    reg(cls, "setObject:forKey:\0", set_object as *const ());
    reg(cls, "boolForKey:\0", bool_for_key as *const ());
    reg(cls, "registerDefaults:\0", register_defaults as *const ());
}
