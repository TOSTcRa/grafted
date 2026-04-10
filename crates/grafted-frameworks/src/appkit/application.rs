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
    log::info!("NSApplication: entering main run loop");

    while !APP_TERMINATED.load(Ordering::Acquire) {
        // Poll X11 events and dispatch them
        let events = display::poll_events();
        for event in &events {
            match event {
                display::DisplayEvent::WindowClose { window: _ } => {
                    // Default behavior: terminate on window close
                    APP_TERMINATED.store(true, Ordering::Release);
                }
                display::DisplayEvent::Expose { window } => {
                    // TODO: dispatch to NSWindow's display method
                    log::trace!("NSApplication: expose event for window {}", window);
                }
                display::DisplayEvent::KeyDown { window, keycode } => {
                    log::trace!("NSApplication: keyDown {} in window {}", keycode, window);
                }
                _ => {}
            }
        }

        // Run one iteration of the CF run loop (handles timers, sources)
        unsafe {
            runloop::CFRunLoopRunInMode(
                runloop::kCFRunLoopDefaultMode.as_ptr() as *const core::ffi::c_void,
                0.016, // ~60fps
                0,
            );
        }
    }

    APP_RUNNING.store(false, Ordering::Release);
    log::info!("NSApplication: exited main run loop");
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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn NSApplicationMain(
    _argc: i32,
    _argv: *const *const i8,
) -> i32 {
    let app = unsafe { ns_application_shared(std::ptr::null_mut(), std::ptr::null_mut()) };
    unsafe { ns_application_run(app, std::ptr::null_mut()) };
    0
}
