//! NSBundle — access to the .app bundle's resources.
//!
//! Apps use NSBundle to find Info.plist, nib files, images, and localizations.
//! We read from the actual filesystem .app bundle structure.

use std::path::{Path, PathBuf};

/// Internal bundle state stored at the ObjC instance.
#[repr(C)]
pub struct NSBundleData {
    pub bundle_path: [u8; 512],
    pub bundle_path_len: usize,
}

const BUNDLE_DATA_OFFSET: usize = 16;

unsafe fn bundle_data(obj: *mut u8) -> *mut NSBundleData {
    obj.add(BUNDLE_DATA_OFFSET) as *mut NSBundleData
}

/// ObjC: +[NSBundle mainBundle]
pub unsafe extern "C" fn ns_bundle_main(_cls: *mut u8, _sel: *mut u8) -> *mut u8 {
    static mut MAIN_BUNDLE: *mut u8 = std::ptr::null_mut();
    unsafe {
        if !MAIN_BUNDLE.is_null() { return MAIN_BUNDLE; }
        let obj = libc::calloc(1, 1024) as *mut u8;
        // Try to find .app bundle from executable path
        // Convention: /path/to/Foo.app/Contents/MacOS/Foo → bundle = /path/to/Foo.app
        let exe = std::env::args().next().unwrap_or_default();
        let bundle_path = find_app_bundle(&exe).unwrap_or_else(|| PathBuf::from("."));
        let path_bytes = bundle_path.to_string_lossy();
        let data = &mut *bundle_data(obj);
        let len = path_bytes.len().min(511);
        data.bundle_path[..len].copy_from_slice(&path_bytes.as_bytes()[..len]);
        data.bundle_path_len = len;
        MAIN_BUNDLE = obj;
        obj
    }
}

/// ObjC: -[NSBundle bundlePath]
pub unsafe extern "C" fn ns_bundle_path(self_: *mut u8, _sel: *mut u8) -> *mut u8 {
    if self_.is_null() { return std::ptr::null_mut(); }
    let data = unsafe { &*bundle_data(self_) };
    let path_str = std::str::from_utf8(&data.bundle_path[..data.bundle_path_len]).unwrap_or("");
    unsafe {
        crate::cf::string::CFStringCreateWithCString(
            std::ptr::null(),
            path_str.as_ptr() as *const i8,
            0x0800_0100,
        ) as *mut u8
    }
}

/// ObjC: -[NSBundle resourcePath]
pub unsafe extern "C" fn ns_bundle_resource_path(self_: *mut u8, _sel: *mut u8) -> *mut u8 {
    if self_.is_null() { return std::ptr::null_mut(); }
    let data = unsafe { &*bundle_data(self_) };
    let base = std::str::from_utf8(&data.bundle_path[..data.bundle_path_len]).unwrap_or("");
    let resource = format!("{}/Contents/Resources\0", base);
    unsafe {
        crate::cf::string::CFStringCreateWithCString(
            std::ptr::null(),
            resource.as_ptr() as *const i8,
            0x0800_0100,
        ) as *mut u8
    }
}

/// ObjC: -[NSBundle infoDictionary] — returns empty dictionary for now
pub unsafe extern "C" fn ns_bundle_info_dict(_self: *mut u8, _sel: *mut u8) -> *mut u8 {
    unsafe {
        crate::cf::dictionary::CFDictionaryCreateMutable(
            std::ptr::null(), 0, std::ptr::null(), std::ptr::null(),
        ) as *mut u8
    }
}

/// ObjC: -[NSBundle bundleIdentifier]
pub unsafe extern "C" fn ns_bundle_identifier(_self: *mut u8, _sel: *mut u8) -> *mut u8 {
    let id = b"com.grafted.app\0";
    unsafe {
        crate::cf::string::CFStringCreateWithCString(
            std::ptr::null(), id.as_ptr() as *const i8, 0x0800_0100,
        ) as *mut u8
    }
}

fn find_app_bundle(exe_path: &str) -> Option<PathBuf> {
    let path = Path::new(exe_path);
    let mut current = path.parent()?;
    for _ in 0..5 {
        if current.extension().and_then(|e| e.to_str()) == Some("app") {
            return Some(current.to_path_buf());
        }
        current = current.parent()?;
    }
    None
}
