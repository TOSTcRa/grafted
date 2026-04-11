//! Register AppKit classes with the ObjC runtime.
//!
//! Each class gets its methods registered so objc_msgSend dispatches correctly.
//! Darwin binaries call [NSApplication sharedApplication], [NSWindow alloc], etc.

use grafted_objc::{objc_registerClassPair, sel_registerName, class_addMethod, types::Class};
use std::sync::Once;

use super::{application, window, view, menu, pasteboard};

static REGISTER_ONCE: Once = Once::new();

pub fn register_all() {
    REGISTER_ONCE.call_once(|| {
        register_nsapplication();
        register_nswindow();
        register_nsview();
        register_nsscreen();
        register_nsobject();
        register_nsmenu();
        register_nspasteboard();
        register_nscolor();
        register_nsimage();
        register_nslayout();
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

fn register_nsmenu() {
    let cls = alloc_class("NSMenu", 512);
    reg(cls, "initWithTitle:\0", menu::ns_menu_init_with_title as *const ());
    reg(cls, "addItem:\0", menu::ns_menu_add_item as *const ());
    reg(cls, "numberOfItems\0", menu::ns_menu_item_count as *const ());
    reg(cls, "setSubmenu:forItem:\0", menu::ns_menu_set_submenu as *const ());

    let icls = alloc_class("NSMenuItem", 512);
    reg(icls, "initWithTitle:action:keyEquivalent:\0", menu::ns_menu_item_init as *const ());
    reg(icls, "separatorItem\0", menu::ns_menu_item_separator as *const ());
    reg(icls, "setTarget:\0", menu::ns_menu_item_set_target as *const ());
    reg(icls, "setSubmenu:\0", menu::ns_menu_item_set_submenu as *const ());
    reg(icls, "setEnabled:\0", menu::ns_menu_item_set_enabled as *const ());
    reg(icls, "setTag:\0", menu::ns_menu_item_set_tag as *const ());
}

fn register_nspasteboard() {
    let cls = alloc_class("NSPasteboard", 256);
    reg(cls, "generalPasteboard\0", pasteboard::ns_pasteboard_general as *const ());
    reg(cls, "stringForType:\0", pasteboard::ns_pasteboard_string_for_type as *const ());
    reg(cls, "setString:forType:\0", pasteboard::ns_pasteboard_set_string as *const ());
    reg(cls, "clearContents\0", pasteboard::ns_pasteboard_clear as *const ());
    reg(cls, "changeCount\0", pasteboard::ns_pasteboard_change_count as *const ());
    reg(cls, "types\0", pasteboard::ns_pasteboard_types as *const ());
    reg(cls, "pasteboardItems\0", pasteboard::ns_pasteboard_items as *const ());

    alloc_class("NSPasteboardItem", 128);
}

fn register_nscolor() {
    let cls = alloc_class("NSColor", 64);

    unsafe extern "C" fn ns_color_with_rgb(
        _cls: *mut u8, _sel: *mut u8, r: f64, g: f64, b: f64, a: f64,
    ) -> *mut u8 {
        let obj = unsafe { libc::calloc(1, 64) } as *mut u8;
        unsafe {
            let data = obj.add(16) as *mut [f64; 4];
            *data = [r, g, b, a];
        }
        obj
    }
    unsafe extern "C" fn ns_color_named(_cls: *mut u8, _sel: *mut u8, _name: *mut u8) -> *mut u8 {
        // Return a default gray color
        ns_color_with_rgb(std::ptr::null_mut(), std::ptr::null_mut(), 0.5, 0.5, 0.5, 1.0)
    }

    reg(cls, "colorWithRed:green:blue:alpha:\0", ns_color_with_rgb as *const ());
    reg(cls, "colorNamed:\0", ns_color_named as *const ());
    reg(cls, "whiteColor\0", ns_color_named as *const ());
    reg(cls, "blackColor\0", ns_color_named as *const ());
    reg(cls, "clearColor\0", ns_color_named as *const ());
    reg(cls, "controlBackgroundColor\0", ns_color_named as *const ());
    reg(cls, "textColor\0", ns_color_named as *const ());
}

fn register_nsimage() {
    let cls = alloc_class("NSImage", 128);

    unsafe extern "C" fn ns_image_named(_cls: *mut u8, _sel: *mut u8, _name: *mut u8) -> *mut u8 {
        unsafe { libc::calloc(1, 128) as *mut u8 } // empty image stub
    }
    unsafe extern "C" fn ns_image_size(_self: *mut u8, _sel: *mut u8) -> crate::cg::geometry::CGSize {
        crate::cg::geometry::CGSize { width: 16.0, height: 16.0 }
    }

    reg(cls, "imageNamed:\0", ns_image_named as *const ());
    reg(cls, "size\0", ns_image_size as *const ());

    alloc_class("NSGraphicsContext", 64);
}

fn register_nslayout() {
    let cls = alloc_class("NSLayoutConstraint", 128);

    unsafe extern "C" fn ns_layout_activate(_cls: *mut u8, _sel: *mut u8, _constraints: *mut u8) {
        // stub
    }

    reg(cls, "activateConstraints:\0", ns_layout_activate as *const ());

    // More UI classes that Xcode needs
    alloc_class("NSAlert", 256);
    alloc_class("NSPopover", 256);
    alloc_class("NSFont", 64);
    alloc_class("NSEvent", 256);
    alloc_class("NSAnimationContext", 64);
    alloc_class("NSGlassEffectView", 128);
    alloc_class("NSRegularExpression", 128);
    alloc_class("NSKeyedArchiver", 128);
    alloc_class("NSKeyedUnarchiver", 128);
    alloc_class("NSSearchField", 128);
    alloc_class("NSResponder", 64);
    alloc_class("NSViewController", 256);
    alloc_class("NSWindowController", 256);
    alloc_class("NSToolbar", 128);
    alloc_class("NSSplitView", 128);
    alloc_class("NSTableView", 256);
    alloc_class("NSOutlineView", 256);
    alloc_class("NSScrollView", 128);
    alloc_class("NSTextField", 128);
    alloc_class("NSButton", 128);
    alloc_class("NSTextView", 256);
    alloc_class("NSStackView", 128);
    alloc_class("NSTabView", 128);
    alloc_class("NSProgressIndicator", 64);
    alloc_class("NSSlider", 64);
    alloc_class("NSComboBox", 128);
    alloc_class("NSSegmentedControl", 128);
}
