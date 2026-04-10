//! Register AppKit classes with the ObjC runtime.
//!
//! Each class gets its methods registered so objc_msgSend dispatches correctly.
//! Darwin binaries call [NSApplication sharedApplication], [NSWindow alloc], etc.

use grafted_objc::{objc_registerClassPair, sel_registerName, class_addMethod, types::Class};
use std::sync::Once;

use super::{application, window, view};

static REGISTER_ONCE: Once = Once::new();

pub fn register_all() {
    REGISTER_ONCE.call_once(|| {
        register_nsapplication();
        register_nswindow();
        register_nsview();
        register_nsscreen();
        register_nsobject();
        log::info!("AppKit: registered all classes with ObjC runtime");
    });
}

fn reg(cls: Class, name: &str, imp: *const ()) {
    let sel = sel_registerName(name.as_ptr() as *const i8);
    class_addMethod(cls, sel, Some(unsafe { std::mem::transmute(imp) }), std::ptr::null());
}

fn alloc_class(name: &str, instance_size: usize) -> Class {
    // Allocate a class_t struct
    let cls = unsafe { libc::calloc(1, 256) } as Class;
    // Set instance size via class_ro_t
    let ro = unsafe { libc::calloc(1, 256) } as *mut grafted_objc::types::class_ro_t;
    let c_name = std::ffi::CString::new(name).unwrap();
    let name_ptr = c_name.into_raw();
    unsafe {
        (*ro).name = name_ptr;
        (*ro).instance_size = instance_size as u32;
        (*(cls as *mut grafted_objc::types::class_t)).data = ro;
    }
    objc_registerClassPair(cls);
    cls
}

fn register_nsapplication() {
    let cls = alloc_class("NSApplication", 256);

    reg(cls, "sharedApplication\0", application::ns_application_shared as *const ());
    reg(cls, "run\0", application::ns_application_run as *const ());
    reg(cls, "terminate:\0", application::ns_application_terminate as *const ());
    reg(cls, "isRunning\0", application::ns_application_is_running as *const ());
    reg(cls, "activateIgnoringOtherApps:\0", application::ns_application_activate as *const ());
    reg(cls, "setActivationPolicy:\0", application::ns_application_set_activation_policy as *const ());
}

fn register_nswindow() {
    let cls = alloc_class("NSWindow", 1024); // large enough for NSWindowData

    reg(cls, "initWithContentRect:styleMask:backing:defer:\0", window::ns_window_init as *const ());
    reg(cls, "setTitle:\0", window::ns_window_set_title as *const ());
    reg(cls, "makeKeyAndOrderFront:\0", window::ns_window_make_key_and_order_front as *const ());
    reg(cls, "orderOut:\0", window::ns_window_order_out as *const ());
    reg(cls, "close\0", window::ns_window_close as *const ());
    reg(cls, "display\0", window::ns_window_display as *const ());
    reg(cls, "frame\0", window::ns_window_frame as *const ());
    reg(cls, "setContentView:\0", window::ns_window_set_content_view as *const ());
    reg(cls, "contentView\0", window::ns_window_content_view as *const ());
    reg(cls, "graphicsContext\0", window::ns_window_graphics_context as *const ());
}

fn register_nsview() {
    let cls = alloc_class("NSView", 1024);

    reg(cls, "initWithFrame:\0", view::ns_view_init_with_frame as *const ());
    reg(cls, "frame\0", view::ns_view_frame as *const ());
    reg(cls, "setFrame:\0", view::ns_view_set_frame as *const ());
    reg(cls, "bounds\0", view::ns_view_bounds as *const ());
    reg(cls, "addSubview:\0", view::ns_view_add_subview as *const ());
    reg(cls, "superview\0", view::ns_view_superview as *const ());
    reg(cls, "setNeedsDisplay:\0", view::ns_view_set_needs_display as *const ());
    reg(cls, "isHidden\0", view::ns_view_is_hidden as *const ());
    reg(cls, "setHidden:\0", view::ns_view_set_hidden as *const ());
    reg(cls, "drawRect:\0", view::ns_view_draw_rect as *const ());
}

fn register_nsscreen() {
    let cls = alloc_class("NSScreen", 64);

    // +[NSScreen mainScreen] — returns a singleton with screen dimensions
    unsafe extern "C" fn ns_screen_main(_cls: *mut u8, _sel: *mut u8) -> *mut u8 {
        static mut SCREEN: *mut u8 = std::ptr::null_mut();
        unsafe {
            if SCREEN.is_null() {
                SCREEN = libc::calloc(1, 128) as *mut u8;
            }
            SCREEN
        }
    }

    unsafe extern "C" fn ns_screen_frame(_self: *mut u8, _sel: *mut u8) -> crate::cg::geometry::CGRect {
        // Default to 1920x1080
        crate::cg::geometry::CGRect {
            origin: crate::cg::geometry::CGPoint { x: 0.0, y: 0.0 },
            size: crate::cg::geometry::CGSize { width: 1920.0, height: 1080.0 },
        }
    }

    reg(cls, "mainScreen\0", ns_screen_main as *const ());
    reg(cls, "frame\0", ns_screen_frame as *const ());
    reg(cls, "visibleFrame\0", ns_screen_frame as *const ());
}

fn register_nsobject() {
    let cls = alloc_class("NSObject", 16);

    unsafe extern "C" fn ns_object_init(self_: *mut u8, _sel: *mut u8) -> *mut u8 { self_ }
    unsafe extern "C" fn ns_object_class(_self: *mut u8, _sel: *mut u8) -> *mut u8 { std::ptr::null_mut() }
    unsafe extern "C" fn ns_object_responds(_self: *mut u8, _sel: *mut u8, _s: *mut u8) -> bool { false }
    unsafe extern "C" fn ns_object_description(_self: *mut u8, _sel: *mut u8) -> *mut u8 { std::ptr::null_mut() }

    reg(cls, "init\0", ns_object_init as *const ());
    reg(cls, "class\0", ns_object_class as *const ());
    reg(cls, "respondsToSelector:\0", ns_object_responds as *const ());
    reg(cls, "description\0", ns_object_description as *const ());
}
