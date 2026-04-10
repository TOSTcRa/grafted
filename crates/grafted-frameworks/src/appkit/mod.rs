//! AppKit emulation — NSApplication, NSWindow, NSView, NSEvent.
//!
//! All classes are registered with the ObjC runtime so that Darwin binaries
//! can use objc_msgSend to interact with them. Methods are implemented as
//! extern "C" functions registered via class_addMethod.

pub mod application;
pub mod window;
pub mod view;
pub mod event;
pub mod register;

/// Register all AppKit classes with the ObjC runtime.
/// Call this before running any Darwin binary that uses AppKit.
pub fn register_classes() {
    register::register_all();
}
