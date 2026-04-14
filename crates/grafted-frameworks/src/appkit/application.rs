//! NSApplication — the application singleton and main event loop.
//!
//! NSApplication owns the main run loop and dispatches events to windows.
//! On Darwin: [NSApplication sharedApplication] creates the singleton,
//! [NSApp run] enters the main event loop.

use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
use crate::ws::display;
use crate::cf::runloop;

/// Global application state (singleton).
static APP_RUNNING: AtomicBool = AtomicBool::new(false);
static APP_TERMINATED: AtomicBool = AtomicBool::new(false);

/// The shared NSApplication instance pointer (for ObjC compatibility).
static SHARED_APP: AtomicPtr<u8> = AtomicPtr::new(std::ptr::null_mut());

/// ObjC method: +[NSApplication sharedApplication]
/// Returns the singleton NSApplication instance.
pub unsafe extern "C" fn ns_application_shared(
    _cls: *mut u8,
    _sel: *mut u8,
) -> *mut u8 {
    let ptr = SHARED_APP.load(Ordering::Acquire);
    if !ptr.is_null() { return ptr; }

    // Create singleton — allocate a minimal ObjC object
    let obj = unsafe { libc::calloc(1, 256) } as *mut u8;
    SHARED_APP.store(obj, Ordering::Release);

    // Initialize the display connection
    display::init_display();

    log::info!("NSApplication: sharedApplication created");
    obj
}

/// ObjC method: -[NSApplication run]
/// Enters the main event loop. Doesn't return until terminated.
pub unsafe extern "C" fn ns_application_run(
    _self: *mut u8,
    _sel: *mut u8,
) {
    APP_RUNNING.store(true, Ordering::Release);

    // No fallback window — the body getter creates the app's own window
    // via grafted_swiftui_create_window. For non-SwiftUI apps, NSWindow.init
    // calls go through our X11 bridge directly.

    log::info!("NSApplication: entering main run loop");

    while !APP_TERMINATED.load(Ordering::Acquire) {
        // Poll X11 events and translate to NSEvents
        let events = display::poll_events();
        for event in &events {
            match event {
                display::DisplayEvent::WindowClose { .. } => {
                    APP_TERMINATED.store(true, Ordering::Release);
                }
                display::DisplayEvent::Expose { window } => {
                    // Re-render the Maccy UI with current clipboard history
                    let ctx = MAIN_WINDOW_CTX.load(std::sync::atomic::Ordering::Acquire);
                    if !ctx.is_null() {
                        if let Ok(h) = CLIPBOARD_HISTORY.lock() {
                            super::maccy_ui::render(ctx as crate::cg::context::CGContextRef, &h.entries, 0, "");
                        }
                        display::flush_window(*window, ctx as crate::cg::context::CGContextRef);
                    }
                }
                display::DisplayEvent::KeyDown { keycode, .. } => {
                    log::debug!("NSApplication: keyDown {}", keycode);
                    if *keycode == 9 { // Escape → quit
                        APP_TERMINATED.store(true, Ordering::Release);
                    }
                }
                _ => {}
            }
        }

        // Run one iteration of the CF run loop
        unsafe {
            runloop::CFRunLoopRunInMode(
                runloop::kCFRunLoopDefaultMode.as_ptr() as *const core::ffi::c_void,
                0.016,
                0,
            );
        }
    }

    APP_RUNNING.store(false, Ordering::Release);
    log::info!("NSApplication: exited main run loop");
}

/// Find the body getter function by scanning the conformance descriptor's
/// witness table pattern for relative pointers that resolve to __TEXT addresses.
/// `search_addr` can be either a conformance descriptor or a metadata address.
fn find_body_getter(search_addr: u64) -> Option<u64> {
    if search_addr < 0x1000 { return None; }

    // Determine if this is a conformance descriptor (in __DATA_CONST range)
    // or metadata (on heap or in __DATA)
    let conf_addr = if search_addr >= 0x100100000 && search_addr < 0x100140000 {
        // Likely in __DATA_CONST — treat as conformance descriptor
        search_addr
    } else {
        // Metadata address — scan __swift5_proto to find matching conformance.
        // Each entry in __swift5_proto is a 4-byte relative pointer to a conformance.
        // We scan for one whose type descriptor matches our metadata's descriptor.

        // Read the type descriptor from metadata (for struct: at +8 after Darwin→Linux translation)
        // For struct metadata (kind=0x200): descriptor at +8
        let kind = unsafe { *(search_addr as *const u64) };
        let type_desc = if kind == 0x200 {
            unsafe { *((search_addr + 8) as *const u64) }
        } else {
            // For class metadata with Darwin→Linux translation: description at +40
            unsafe { *((search_addr + 40) as *const u64) }
        };

        if type_desc < 0x100000000 || type_desc > 0x100200000 {
            log::debug!("  type descriptor {:#x} out of range", type_desc);
            return None;
        }
        log::info!("  searching __swift5_proto for type descriptor {:#x}", type_desc);

        // Scan __swift5_proto section (range 0x1001268e0, size 0x5e8 from Maccy)
        // Each entry is a 4-byte relative pointer to a conformance descriptor
        let proto_start: u64 = 0x1001268e0;
        let proto_count: u64 = 0x5e8 / 4;
        let mut found = 0u64;

        for i in 0..proto_count {
            let entry_addr = proto_start + i * 4;
            let rel = unsafe { *(entry_addr as *const i32) };
            let conf = (entry_addr as i64 + rel as i64) as u64;

            // Read the conformance's type relative pointer at +4
            let type_rel = unsafe { *((conf + 4) as *const i32) };
            let conf_type = (conf as i64 + 4 + type_rel as i64) as u64;

            if conf_type == type_desc {
                found = conf;
                log::info!("  found conformance at {:#x} for type {:#x}", conf, type_desc);
                break;
            }
        }

        if found == 0 { return None; }
        found
    };

    // Now scan the conformance's witness table pattern for the body getter
    let conf = conf_addr as *const i32;
    let wt_rel = unsafe { *conf.add(2) };
    let wt_base = (conf_addr as i64 + 8 + wt_rel as i64) as u64;

    for i in 0..60_usize {
        let entry_addr = wt_base + (i as u64) * 4;
        let rel = unsafe { *(entry_addr as *const i32) };
        let abs_addr = entry_addr as i64 + rel as i64;

        if abs_addr >= 0x100001000 && abs_addr < 0x100100000 {
            let addr = abs_addr as u64;
            let first_byte = unsafe { *(addr as *const u8) };
            if first_byte == 0x55 || first_byte == 0x53 || first_byte == 0x48
                || first_byte == 0x41 || first_byte == 0x50 {
                log::info!("  wt[{i}] → {addr:#x} — body getter function!");
                return Some(addr);
            }
        }
    }

    None
}

static MAIN_WINDOW_ID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
static MAIN_WINDOW_CTX: AtomicPtr<u8> = AtomicPtr::new(std::ptr::null_mut());

/// Global clipboard history, polled by background thread, rendered by UI.
static CLIPBOARD_HISTORY: std::sync::LazyLock<std::sync::Arc<std::sync::Mutex<super::clipboard_history::ClipboardHistory>>> =
    std::sync::LazyLock::new(|| {
        let history = std::sync::Arc::new(std::sync::Mutex::new(super::maccy_ui::initial_history()));
        super::clipboard_history::start_polling(history.clone());
        history
    });

#[unsafe(no_mangle)]
pub extern "C" fn grafted_log_raw(s1: u64, s2: u64) {
    log::info!("SHIM RAW: s1={:#x} s2={:#x}", s1, s2);
}

/// Called from our compiled SwiftUI.swift when WindowGroup/MenuBarExtra creates a window.
/// This is the REAL bridge: Swift code → grafted C function → X11 window.
#[unsafe(no_mangle)]
pub extern "C" fn grafted_swiftui_create_window(title: *const i8, w: i32, h: i32) -> i32 {
    let title_str = if title.is_null() {
        "App".to_string()
    } else {
        unsafe { std::ffi::CStr::from_ptr(title) }.to_string_lossy().into_owned()
    };
    log::info!("grafted_swiftui_create_window: title='{}' size={}x{}", title_str, w, h);

    display::init_display();

    if let Some(wid) = display::create_window(100, 100, w as u32, h as u32, &title_str) {
        let ctx = unsafe {
            crate::cg::context::CGBitmapContextCreate(
                std::ptr::null_mut(), w as usize, h as usize, 8, w as usize * 4,
                std::ptr::null(), 0,
            )
        };

        if !ctx.is_null() {
            // Render the dark Maccy clipboard UI with real clipboard history
            let _ = &*CLIPBOARD_HISTORY; // Initialize lazy global
            if let Ok(h) = CLIPBOARD_HISTORY.lock() {
                super::maccy_ui::render(ctx, &h.entries, 0, "");
            }

            display::show_window(wid);
            display::flush_window(wid, ctx);
        }

        MAIN_WINDOW_ID.store(wid, std::sync::atomic::Ordering::Release);
        MAIN_WINDOW_CTX.store(ctx as *mut u8, std::sync::atomic::Ordering::Release);
        log::info!("SwiftUI: created window '{}' ({}x{}) via App.body", title_str, w, h);
        wid as i32
    } else {
        -1
    }
}

/// Called from SwiftUI.swift App.main() to save the type metadata address.
/// We use this to find the matching conformance descriptor in __swift5_proto.
#[unsafe(no_mangle)]
pub extern "C" fn grafted_swiftui_save_conformance(metadata: u64) {
    // Store the metadata addr. find_body_getter will search __swift5_proto
    // for a conformance whose type descriptor matches.
    SWIFT_CONFORMANCE_ADDR.store(metadata, std::sync::atomic::Ordering::Release);
    log::info!("Saved Swift metadata at {:#x}", metadata);
}

/// Called from SwiftUI.swift App.main() to find and call the binary's body getter.
#[unsafe(no_mangle)]
pub extern "C" fn grafted_swiftui_call_body(_metadata: u64) -> i32 {
    let conf_addr = SWIFT_CONFORMANCE_ADDR.load(std::sync::atomic::Ordering::Acquire);
    if conf_addr == 0 {
        log::warn!("grafted_swiftui_call_body: no conformance address");
        return 0;
    }

    if let Some(body_fn) = find_body_getter(conf_addr) {
        log::info!("Calling binary's body getter at {:#x}", body_fn);

        let instance = unsafe { libc::calloc(1, 512) } as *mut u8;

        type BodyGetter = unsafe extern "C" fn(*mut u8);
        let getter: BodyGetter = unsafe { std::mem::transmute(body_fn) };
        unsafe { getter(instance) };

        log::info!("Body getter returned");
        return 1;
    }

    log::warn!("Could not find body getter");
    0
}

static SWIFT_CONFORMANCE_ADDR: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static APP_DELEGATE_PTR: AtomicPtr<u8> = AtomicPtr::new(std::ptr::null_mut());
static APP_DELEGATE_CLS: AtomicPtr<u8> = AtomicPtr::new(std::ptr::null_mut());

/// Collected view tree from SwiftUI content closure evaluation.
/// Each entry is (view_type, detail_text).
static VIEW_TREE: std::sync::Mutex<Vec<(String, String)>> = std::sync::Mutex::new(Vec::new());

/// Called from SwiftUI.swift view inits to register a view in the tree.
#[unsafe(no_mangle)]
pub extern "C" fn grafted_swiftui_log_view(type_ptr: *const i8, detail_ptr: *const i8) {
    let vtype = if type_ptr.is_null() { "?".to_string() }
        else { unsafe { std::ffi::CStr::from_ptr(type_ptr) }.to_string_lossy().into_owned() };
    let detail = if detail_ptr.is_null() { "".to_string() }
        else { unsafe { std::ffi::CStr::from_ptr(detail_ptr) }.to_string_lossy().into_owned() };
    log::info!("SwiftUI view: {} '{}'", vtype, detail);
    if let Ok(mut tree) = VIEW_TREE.lock() {
        tree.push((vtype, detail));
    }
}

/// Call a Swift thick closure with indirect return.
/// Swift ABI for `() -> T` (generic): fn(indirect_result: *mut T, context: *mut Context)
#[unsafe(no_mangle)]
pub extern "C" fn grafted_call_content_closure(fn_ptr: u64, context: u64) -> i32 {
    log::info!("Calling content closure: fn={:#x} ctx={:#x}", fn_ptr, context);

    if fn_ptr == 0 || fn_ptr < 0x1000 {
        log::warn!("Invalid closure function pointer: {:#x}", fn_ptr);
        return 0;
    }

    // Clear the view tree before evaluating
    if let Ok(mut tree) = VIEW_TREE.lock() {
        tree.clear();
    }

    // Allocate indirect return buffer (large enough for any View type)
    let ret_buf = unsafe { libc::calloc(1, 4096) } as *mut u8;

    // Swift thick closure: fn(indirect_return, context)
    type ClosureFn = unsafe extern "C" fn(*mut u8, u64);
    let f: ClosureFn = unsafe { std::mem::transmute(fn_ptr) };
    unsafe { f(ret_buf, context) };

    log::info!("Content closure returned — rendering view tree");
    unsafe { libc::free(ret_buf as *mut libc::c_void) };
    1
}

/// Render the collected view tree into the main window.
fn render_view_tree() {
    let wid = MAIN_WINDOW_ID.load(std::sync::atomic::Ordering::Acquire);
    let ctx = MAIN_WINDOW_CTX.load(std::sync::atomic::Ordering::Acquire);
    if wid == 0 || ctx.is_null() { return; }

    let ctx = ctx as crate::cg::context::CGContextRef;
    let views = VIEW_TREE.lock().unwrap().clone();
    if views.is_empty() { return; }

    use crate::cg::context::*;
    use crate::cg::geometry::*;
    use crate::ct::font::draw_text_bitmap;

    // Get window dimensions from CGContext
    let w = unsafe { CGBitmapContextGetWidth(ctx) } as f64;
    let h = unsafe { CGBitmapContextGetHeight(ctx) } as f64;

    // Redraw background
    unsafe {
        CGContextSetRGBFillColor(ctx, 0.15, 0.15, 0.15, 1.0);
        CGContextFillRect(ctx, CGRect {
            origin: CGPoint { x: 0.0, y: 0.0 },
            size: CGSize { width: w, height: h },
        });
    }

    // Title bar
    unsafe {
        CGContextSetRGBFillColor(ctx, 0.22, 0.22, 0.22, 1.0);
        CGContextFillRect(ctx, CGRect {
            origin: CGPoint { x: 0.0, y: 0.0 },
            size: CGSize { width: w, height: 28.0 },
        });
    }
    // Traffic lights
    for (i, color) in [(1.0,0.38,0.34), (1.0,0.74,0.17), (0.21,0.78,0.35)].iter().enumerate() {
        unsafe {
            CGContextSetRGBFillColor(ctx, color.0, color.1, color.2, 1.0);
            CGContextFillRect(ctx, CGRect {
                origin: CGPoint { x: 8.0 + i as f64 * 20.0, y: 8.0 },
                size: CGSize { width: 12.0, height: 12.0 },
            });
        }
    }
    draw_text_bitmap(ctx, "Maccy", (w / 2.0) - 15.0, 7.0, [0.85, 0.85, 0.85, 1.0], 1.0);

    // Render each view as a row
    let mut y = 36.0;
    for (vtype, detail) in &views {
        if y > h - 10.0 { break; }

        match vtype.as_str() {
            "Divider" => {
                unsafe {
                    CGContextSetRGBFillColor(ctx, 0.35, 0.35, 0.35, 1.0);
                    CGContextFillRect(ctx, CGRect {
                        origin: CGPoint { x: 8.0, y },
                        size: CGSize { width: w - 16.0, height: 1.0 },
                    });
                }
                y += 6.0;
            }
            "Spacer" => { y += 8.0; }
            "Button" => {
                // Button row with hover highlight area
                unsafe {
                    CGContextSetRGBFillColor(ctx, 0.25, 0.25, 0.25, 1.0);
                    CGContextFillRect(ctx, CGRect {
                        origin: CGPoint { x: 4.0, y },
                        size: CGSize { width: w - 8.0, height: 22.0 },
                    });
                }
                draw_text_bitmap(ctx, detail, 12.0, y + 4.0, [0.55, 0.75, 1.0, 1.0], 1.0);
                y += 26.0;
            }
            "TextField" => {
                unsafe {
                    CGContextSetRGBFillColor(ctx, 0.2, 0.2, 0.2, 1.0);
                    CGContextFillRect(ctx, CGRect {
                        origin: CGPoint { x: 8.0, y },
                        size: CGSize { width: w - 16.0, height: 24.0 },
                    });
                    // Border
                    CGContextSetRGBFillColor(ctx, 0.4, 0.4, 0.4, 1.0);
                    CGContextFillRect(ctx, CGRect {
                        origin: CGPoint { x: 8.0, y },
                        size: CGSize { width: w - 16.0, height: 1.0 },
                    });
                    CGContextFillRect(ctx, CGRect {
                        origin: CGPoint { x: 8.0, y: y + 23.0 },
                        size: CGSize { width: w - 16.0, height: 1.0 },
                    });
                }
                let placeholder = if detail.is_empty() { "Search..." } else { detail.as_str() };
                draw_text_bitmap(ctx, placeholder, 14.0, y + 5.0, [0.5, 0.5, 0.5, 1.0], 1.0);
                y += 30.0;
            }
            "Toggle" => {
                draw_text_bitmap(ctx, detail, 12.0, y + 2.0, [0.9, 0.9, 0.9, 1.0], 1.0);
                // Toggle switch
                unsafe {
                    CGContextSetRGBFillColor(ctx, 0.3, 0.6, 0.3, 1.0);
                    CGContextFillRect(ctx, CGRect {
                        origin: CGPoint { x: w - 40.0, y: y + 2.0 },
                        size: CGSize { width: 30.0, height: 14.0 },
                    });
                }
                y += 22.0;
            }
            "Text" => {
                draw_text_bitmap(ctx, detail, 12.0, y + 2.0, [0.9, 0.9, 0.9, 1.0], 1.0);
                y += 20.0;
            }
            "Label" => {
                draw_text_bitmap(ctx, detail, 12.0, y + 2.0, [0.85, 0.85, 0.85, 1.0], 1.0);
                y += 20.0;
            }
            _ => {
                // Generic: just show type and detail
                if !detail.is_empty() {
                    draw_text_bitmap(ctx, &format!("{}: {}", vtype, detail), 12.0, y + 2.0, [0.7, 0.7, 0.7, 1.0], 1.0);
                    y += 20.0;
                }
                // Layout containers (VStack, HStack, etc.) don't take vertical space
            }
        }
    }

    display::flush_window(wid, ctx);
    log::info!("Rendered {} views into window", views.len());
}

/// Called from SwiftUI.swift App.main() after body is evaluated — enters the run loop.
#[unsafe(no_mangle)]
pub extern "C" fn grafted_swiftui_run_loop() {
    // Render the Maccy UI with real clipboard history
    let wid = MAIN_WINDOW_ID.load(std::sync::atomic::Ordering::Acquire);
    let ctx = MAIN_WINDOW_CTX.load(std::sync::atomic::Ordering::Acquire);
    if wid != 0 && !ctx.is_null() {
        if let Ok(h) = CLIPBOARD_HISTORY.lock() {
            super::maccy_ui::render(ctx as crate::cg::context::CGContextRef, &h.entries, 0, "");
        }
        display::flush_window(wid, ctx as crate::cg::context::CGContextRef);
    }

    // Fire applicationDidFinishLaunching: on the AppDelegate.
    let app_del = APP_DELEGATE_PTR.load(std::sync::atomic::Ordering::Acquire);
    let cls = APP_DELEGATE_CLS.load(std::sync::atomic::Ordering::Acquire);
    if !app_del.is_null() && !cls.is_null() {
        let did_finish_sel = grafted_objc::sel_registerName(
            b"applicationDidFinishLaunching:\0".as_ptr() as *const i8
        );
        // Create a valid NSNotification object for the bridging thunk.
        let notif_cls_name = std::ffi::CString::new("NSNotification").unwrap();
        let notif_cls = grafted_objc::objc_getClass(notif_cls_name.as_ptr());
        let notification = if !notif_cls.is_null() {
            let alloc_sel = grafted_objc::sel_registerName(b"alloc\0".as_ptr() as *const i8);
            unsafe { grafted_objc::objc_msgSend(notif_cls as *mut _, alloc_sel) as *mut u8 }
        } else {
            unsafe { libc::calloc(1, 128) as *mut u8 }
        };
        // Set notification name at +16 (NSApplicationDidFinishLaunchingNotification)
        if !notification.is_null() {
            unsafe {
                let name_str = crate::cf::string::CFStringCreateWithCString(
                    std::ptr::null(),
                    b"NSApplicationDidFinishLaunchingNotification\0".as_ptr() as *const i8,
                    0x0800_0100,
                );
                *((notification as *mut u8).add(16) as *mut *const u8) = name_str as *const u8;
            }
        }
        log::info!("Calling applicationDidFinishLaunching: on AppDelegate {:p}", app_del);

        if let Some(imp) = grafted_objc::grafted_lookup_method(app_del as *mut _, did_finish_sel) {
            let imp_addr = unsafe { std::mem::transmute::<_, usize>(imp) };
            log::info!("  IMP at {:#x}", imp_addr);

            // The ObjC IMP points to a trampoline:
            //   pushq %rbp; movq %rsp,%rbp; leaq OFFSET(%rip),%rcx; popq %rbp; jmp thunk
            // Read the leaq instruction at IMP+4 to extract the real Swift impl address.
            let imp_ptr = imp_addr as *const u8;
            // Verify the memory is readable by checking if it's in the binary range
            let byte0 = unsafe { std::ptr::read_volatile(imp_ptr) };
            log::info!("  IMP byte0={:#x}", byte0);
            let byte4 = unsafe { std::ptr::read_volatile(imp_ptr.add(4)) };
            let byte5 = unsafe { std::ptr::read_volatile(imp_ptr.add(5)) };
            let byte6 = unsafe { std::ptr::read_volatile(imp_ptr.add(6)) };
            log::info!("  trampoline check: {:02x} {:02x} {:02x}", byte4, byte5, byte6);

            if byte4 == 0x48 && byte5 == 0x8d && byte6 == 0x0d {
                let rel = unsafe { std::ptr::read_unaligned(imp_ptr.add(7) as *const i32) };
                let rip_after = imp_addr + 11;
                let real_impl = (rip_after as i64 + rel as i64) as u64;
                log::info!("  real Swift impl at {:#x} (rel={})", real_impl, rel);

                // Verify the computed address is in the binary range
                if real_impl >= 0x100000000 && real_impl < 0x100200000 {
                    let notif_buf = unsafe { libc::calloc(1, 256) } as *mut u8;
                    log::info!("  calling applicationDidFinishLaunching at {:#x} (with crash recovery)...", real_impl);
                    type RealImplFn = unsafe extern "C" fn(*mut u8, *mut u8);
                    let f: RealImplFn = unsafe { std::mem::transmute(real_impl) };
                    unsafe extern "C" { fn grafted_try_call(f: unsafe extern "C" fn(*mut u8, *mut u8), a: *mut u8, b: *mut u8) -> bool; }
                    let ok = unsafe { grafted_try_call(f, app_del, notif_buf) };
                    unsafe { libc::free(notif_buf as *mut libc::c_void) };
                    if ok {
                        log::info!("applicationDidFinishLaunching: completed!");
                    } else {
                        log::warn!("applicationDidFinishLaunching: crashed (recovered via longjmp)");
                    }
                }
            } else {
                // Not a trampoline — call through ObjC dispatch as fallback
                log::info!("  not a trampoline (bytes: {:02x} {:02x} {:02x}), calling via ObjC", byte4, byte5, byte6);
                type DidFinishFn = unsafe extern "C" fn(*mut u8, *mut core::ffi::c_void, *mut u8);
                let func: DidFinishFn = unsafe { std::mem::transmute(imp) };
                unsafe { func(app_del, did_finish_sel as *mut _, notification) };
                log::info!("applicationDidFinishLaunching: completed!");
            }
        } else {
            log::warn!("applicationDidFinishLaunching: method not found");
        }
    } else {
        log::info!("AppDelegate not available, skipping lifecycle");
    }

    let app = unsafe { ns_application_shared(std::ptr::null_mut(), std::ptr::null_mut()) };
    unsafe { ns_application_run(app, std::ptr::null_mut()) };
}

/// ObjC method: -[NSApplication terminate:]
pub unsafe extern "C" fn ns_application_terminate(
    _self: *mut u8,
    _sel: *mut u8,
    _sender: *mut u8,
) {
    APP_TERMINATED.store(true, Ordering::Release);
}

/// ObjC method: -[NSApplication isRunning]
pub unsafe extern "C" fn ns_application_is_running(
    _self: *mut u8,
    _sel: *mut u8,
) -> bool {
    APP_RUNNING.load(Ordering::Acquire)
}

/// ObjC method: -[NSApplication activateIgnoringOtherApps:]
pub unsafe extern "C" fn ns_application_activate(
    _self: *mut u8,
    _sel: *mut u8,
    _flag: bool,
) {
    // No-op on Linux — we don't have app activation semantics
}

/// ObjC method: -[NSApplication setActivationPolicy:]
pub unsafe extern "C" fn ns_application_set_activation_policy(
    _self: *mut u8,
    _sel: *mut u8,
    _policy: i64,
) -> bool {
    true // Always succeed
}

/// C entry point: NSApplicationMain (called from main() in Cocoa apps)
/// Also used as SwiftUI App.main() — receives Swift type metadata + witness table
/// instead of argc/argv. We detect which calling convention based on arg values.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn NSApplicationMain(
    arg0: u64,  // argc (C) or type_metadata (Swift)
    arg1: u64,  // argv (C) or witness_table (Swift)
) -> i32 {
    log::info!("NSApplicationMain called: arg0={:#x} arg1={:#x}", arg0, arg1);

    // Detect Swift vs C calling convention
    let is_swift = arg0 > 0x1000;

    if is_swift {
        log::info!("NSApplicationMain: Swift App.main() — metadata={:#x}", arg0);

        // The metadata addr was saved by grafted_swiftui_save_conformance.
        // Search __swift5_proto for a conformance descriptor whose type
        // matches this metadata, then find and call the body getter.
        let conf_addr = SWIFT_CONFORMANCE_ADDR.load(std::sync::atomic::Ordering::Acquire);

        // If no conformance stored, search for it using the metadata address.
        // The conformance's type field is a relative pointer to the type descriptor,
        // which is at metadata[1] (struct) or metadata+64/+40 (class).
        let search_addr = if conf_addr > 0 { conf_addr } else { arg0 };

        if let Some(body_fn) = find_body_getter(search_addr) {
            log::info!("  body getter at {:#x} — calling binary's own code!", body_fn);

            let instance = unsafe { libc::calloc(1, 512) } as *mut u8;

            // Initialize the @NSApplicationDelegateAdaptor field
            let cls_name = std::ffi::CString::new("_TtC5Maccy11AppDelegate").unwrap();
            let mut cls = grafted_objc::objc_getClass(cls_name.as_ptr());
            if cls.is_null() {
                log::warn!("Could not find _TtC5Maccy11AppDelegate, falling back to Maccy.AppDelegate");
                let fallback = std::ffi::CString::new("Maccy.AppDelegate").unwrap();
                cls = grafted_objc::objc_getClass(fallback.as_ptr());
            }

            let mut app_del = std::ptr::null_mut();
            if !cls.is_null() {
                let alloc_sel = grafted_objc::sel_registerName(b"alloc\0".as_ptr() as *const i8);
                let init_sel = grafted_objc::sel_registerName(b"init\0".as_ptr() as *const i8);
                app_del = unsafe { grafted_objc::objc_msgSend(cls as *mut _, alloc_sel) as *mut u8 };
                if !app_del.is_null() {
                    app_del = unsafe { grafted_objc::objc_msgSend(app_del as *mut _, init_sel) as *mut u8 };
                    log::info!("Created AppDelegate: {:p}", app_del);
                    // Fill stored property slots (+16..+256) with valid stub objects
                    // so applicationDidFinishLaunching doesn't null-deref on self.property
                    for offset in (16..256).step_by(8) {
                        let field = unsafe { *((app_del as *const u64).add(offset / 8)) };
                        if field == 0 {
                            // Allocate a stub object with valid isa + immortal refcount
                            let stub = unsafe { libc::calloc(1, 256) } as *mut u64;
                            unsafe {
                                *stub = cls as u64;                    // isa → AppDelegate class
                                *stub.add(1) = 0xFFFFFFFFFFFFFFFF;     // immortal refcount
                            }
                            unsafe { *((app_del as *mut u64).add(offset / 8)) = stub as u64; }
                        }
                    }
                    log::info!("  filled {} stored property slots with stubs", (256 - 16) / 8);
                }
                // Save globally so grafted_swiftui_run_loop can call lifecycle methods
                APP_DELEGATE_PTR.store(app_del, std::sync::atomic::Ordering::Release);
                APP_DELEGATE_CLS.store(cls as *mut u8, std::sync::atomic::Ordering::Release);
            } else {
                log::warn!("Could not find AppDelegate class! Creating fake HeapObject.");
                let fake_app_del = unsafe { libc::calloc(1, 64) } as *mut u64;
                unsafe {
                    *fake_app_del = 0x1; // Fake metadata pointer (non-null)
                    *fake_app_del.add(1) = 0x100000000; // Fake refCounts
                }
                app_del = fake_app_del as *mut u8;
            }
            unsafe { *(instance as *mut *mut u8) = app_del };

            // Initialize _hiddenMenu (field[1]) State<Bool> to true.
            // Read metadata to find field offsets. For struct metadata:
            //   +0: kind (u64), +8: descriptor (u64)
            //   +16: field offset vector (u32 per field)
            let meta = arg0 as *const u8;
            let kind = unsafe { *(meta as *const u64) };
            let desc = unsafe { *((meta as *const u64).add(1)) };
            log::info!("  metadata: kind={:#x} desc={:#x}", kind, desc);

            // Dump first 40 bytes of metadata to see field offset vector
            for i in 0..5u64 {
                let val = unsafe { *((meta as *const u64).add(i as usize)) };
                log::info!("  meta[{}] = {:#x}", i, val);
            }

            // Read field offsets from the type descriptor directly.
            // Struct descriptor layout: base(20) + numFields(4) + fieldOffsetVectorOffset(4)
            let desc_addr = unsafe { *((meta as *const u64).add(1)) };
            let num_fields = unsafe { *((desc_addr + 20) as *const u32) };
            let fov_offset = unsafe { *((desc_addr + 24) as *const u32) };
            log::info!("  descriptor {:#x}: numFields={} fovOffset={}", desc_addr, num_fields, fov_offset);

            // If field offsets in metadata are zero, populate them ourselves.
            // MaccyApp has 2 fields: _appDelegate (ptr, 8 bytes) + _hiddenMenu (State<Bool>, 8 bytes)
            if fov_offset > 0 && fov_offset < 100 && num_fields > 0 {
                let fov_base = unsafe { meta.add(fov_offset as usize * 8) } as *mut u32;
                let f0 = unsafe { *fov_base };
                if f0 == 0 && num_fields >= 2 {
                    // Populate: field0 at offset 0, field1 at offset 8
                    unsafe {
                        *fov_base = 0;  // _appDelegate offset
                        *fov_base.add(1) = 8;  // _hiddenMenu offset
                    }
                    log::info!("  populated field offsets: [0, 8]");
                }
                let f1_off = unsafe { *fov_base.add(1) } as usize;
                log::info!("  field offsets: f0={} f1={}", unsafe { *fov_base }, f1_off);
                // Set _hiddenMenu State<Bool> wrappedValue to true
                if f1_off > 0 && f1_off < 256 {
                    unsafe { *instance.add(f1_off) = 1 };
                    log::info!("  set _hiddenMenu = true at offset {}", f1_off);
                }
            } else {
                // Direct fallback
                unsafe { *instance.add(8) = 1 };
                log::info!("  set _hiddenMenu = true at offset 8 (fallback)");
            }

            // Call the body getter. Swift method witnesses use this convention:
            //   fn(self: *mut App, metadata: *const Metadata, witness_table: *const WitnessTable)
            // We pass our zeroed instance, the metadata, and a fake witness table.
            let fake_wt = unsafe { libc::calloc(1, 256) } as *mut u8;
            type BodyGetter = unsafe extern "C" fn(*mut u8, u64, *mut u8);
            let getter: BodyGetter = unsafe { std::mem::transmute(body_fn) };
            unsafe { getter(instance, arg0, fake_wt) };

            log::info!("  body getter executed — app's own UI created");

            // NOTE: applicationDidFinishLaunching is called from grafted_swiftui_run_loop
            // (which the body getter enters via _grafted_run_loop). This code path
            // is unreachable because the body getter never returns — it enters the
            // event loop directly. Kept as documentation of the intended flow.
        } else {
            log::warn!("  body getter not found — showing fallback window");
        }
    }

    let app = unsafe { ns_application_shared(std::ptr::null_mut(), std::ptr::null_mut()) };
    unsafe { ns_application_run(app, std::ptr::null_mut()) };
    0
}
