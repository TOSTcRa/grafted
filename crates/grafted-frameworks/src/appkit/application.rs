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

    // Apps create windows via NSWindow API → our X11 bridge.
    // SwiftUI apps: WindowGroup not yet implemented, so show a diagnostic window.
    // AppKit apps: their [NSWindow init] calls go through our implementation.
    if MAIN_WINDOW_ID.load(std::sync::atomic::Ordering::Acquire) == 0 {
        if let Some(wid) = display::create_window(200, 200, 640, 200, "Grafted") {
            let ctx = unsafe {
                crate::cg::context::CGBitmapContextCreate(
                    std::ptr::null_mut(), 640, 200, 8, 640 * 4,
                    std::ptr::null(), 0,
                )
            };
            if !ctx.is_null() {
                use crate::cg::context::*;
                use crate::cg::geometry::*;
                use crate::ct::font::draw_text_bitmap;
                unsafe {
                    CGContextSetRGBFillColor(ctx, 0.15, 0.15, 0.18, 1.0);
                    CGContextFillRect(ctx, CGRect {
                        origin: CGPoint { x: 0.0, y: 0.0 },
                        size: CGSize { width: 640.0, height: 200.0 },
                    });
                }
                draw_text_bitmap(ctx, "App loaded — SwiftUI render not yet wired", 20.0, 20.0, [1.0, 0.8, 0.2, 1.0], 1.5);
                draw_text_bitmap(ctx, "AppKit [NSWindow] calls translate to X11 properly", 20.0, 60.0, [0.7, 0.7, 0.7, 1.0], 1.0);
                draw_text_bitmap(ctx, "Need SwiftUI WindowGroup -> NSWindow bridge", 20.0, 85.0, [0.7, 0.7, 0.7, 1.0], 1.0);
                draw_text_bitmap(ctx, "Press ESC to quit", 20.0, 130.0, [0.4, 0.8, 1.0, 1.0], 1.0);
                display::show_window(wid);
                display::flush_window(wid, ctx);
            }
            MAIN_WINDOW_ID.store(wid, std::sync::atomic::Ordering::Release);
            MAIN_WINDOW_CTX.store(ctx as *mut u8, std::sync::atomic::Ordering::Release);
        }
    }

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
                    // Redraw: blit the CGContext to the X11 window
                    let ctx = MAIN_WINDOW_CTX.load(std::sync::atomic::Ordering::Acquire);
                    if !ctx.is_null() {
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

static MAIN_WINDOW_ID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
static MAIN_WINDOW_CTX: AtomicPtr<u8> = AtomicPtr::new(std::ptr::null_mut());

/// Called from our compiled SwiftUI.swift when WindowGroup/MenuBarExtra creates a window.
/// This is the REAL bridge: Swift code → grafted C function → X11 window.
#[unsafe(no_mangle)]
pub extern "C" fn grafted_swiftui_create_window(title: *const i8, w: i32, h: i32) -> i32 {
    let title_str = if title.is_null() {
        "App".to_string()
    } else {
        unsafe { std::ffi::CStr::from_ptr(title) }.to_string_lossy().into_owned()
    };

    display::init_display();

    if let Some(wid) = display::create_window(100, 100, w as u32, h as u32, &title_str) {
        let ctx = unsafe {
            crate::cg::context::CGBitmapContextCreate(
                std::ptr::null_mut(), w as usize, h as usize, 8, w as usize * 4,
                std::ptr::null(), 0,
            )
        };

        if !ctx.is_null() {
            use crate::cg::context::*;
            use crate::cg::geometry::*;
            use crate::ct::font::draw_text_bitmap;

            // Draw the app's window background
            unsafe {
                // macOS-style window background
                CGContextSetRGBFillColor(ctx, 0.93, 0.93, 0.93, 1.0);
                CGContextFillRect(ctx, CGRect {
                    origin: CGPoint { x: 0.0, y: 0.0 },
                    size: CGSize { width: w as f64, height: h as f64 },
                });
                // Title bar
                CGContextSetRGBFillColor(ctx, 0.78, 0.78, 0.78, 1.0);
                CGContextFillRect(ctx, CGRect {
                    origin: CGPoint { x: 0.0, y: 0.0 },
                    size: CGSize { width: w as f64, height: 28.0 },
                });
                // Traffic lights
                for (i, color) in [(1.0,0.38,0.34), (1.0,0.74,0.17), (0.21,0.78,0.35)].iter().enumerate() {
                    CGContextSetRGBFillColor(ctx, color.0, color.1, color.2, 1.0);
                    CGContextFillRect(ctx, CGRect {
                        origin: CGPoint { x: 8.0 + i as f64 * 20.0, y: 8.0 },
                        size: CGSize { width: 12.0, height: 12.0 },
                    });
                }
            }
            // Title text
            draw_text_bitmap(ctx, &title_str, (w as f64 / 2.0) - (title_str.len() as f64 * 4.0), 7.0, [0.2, 0.2, 0.2, 1.0], 1.0);
            // App content message
            draw_text_bitmap(ctx, "SwiftUI App.body executed successfully!", 20.0, 50.0, [0.1, 0.1, 0.1, 1.0], 1.5);
            draw_text_bitmap(ctx, &format!("Window: {} ({}x{})", title_str, w, h), 20.0, 90.0, [0.3, 0.3, 0.3, 1.0], 1.0);
            draw_text_bitmap(ctx, "Swift -> SwiftUI.App.main() -> body -> Scene -> X11", 20.0, 115.0, [0.3, 0.3, 0.3, 1.0], 1.0);

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

/// Called from SwiftUI.swift App.main() after body is evaluated — enters the run loop.
#[unsafe(no_mangle)]
pub extern "C" fn grafted_swiftui_run_loop() {
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
    let is_swift = arg0 > 0x1000; // Swift metadata is a pointer, C argc is small

    if is_swift {
        log::info!("NSApplicationMain: Swift App.main() — metadata={:#x} conformance={:#x}", arg0, arg1);

        // Parse the conformance descriptor (relative pointers)
        // Layout: protocol(+0), type(+4), witness_table(+8), flags(+12)
        let conf = arg1 as *const i32;
        let wt_rel = unsafe { *conf.add(2) }; // witness table relative pointer
        let wt_addr = (arg1 as i64 + 8 + wt_rel as i64) as *const i32;
        log::info!("  witness table at {:p}", wt_addr);

        // Witness table entries are relative pointers.
        // For App protocol: wt[4] is likely the body getter.
        // Let's find the first entry that points into __TEXT (executable code).
        for i in 0..8 {
            let rel = unsafe { *wt_addr.add(i) };
            let abs_addr = wt_addr as u64 + (i as u64 * 4) + rel as u64;
            let in_text = abs_addr >= 0x100001000 && abs_addr < 0x100100000;
            log::info!("  wt[{i}]: rel={rel:+} → {abs_addr:#x} {}", if in_text { "← CODE" } else { "" });
        }
    }

    let app = unsafe { ns_application_shared(std::ptr::null_mut(), std::ptr::null_mut()) };
    unsafe { ns_application_run(app, std::ptr::null_mut()) };
    0
}
