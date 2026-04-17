//! AppKit emulation - NSApplication, NSWindow, NSView, NSEvent.

pub mod application;
pub mod window;
pub mod view;
pub mod event;
pub mod menu;
pub mod pasteboard;
pub mod register;

/// Register all AppKit classes with the ObjC runtime.
/// Call this before running any Darwin binary that uses AppKit.
pub fn register_classes() {
    register::register_all();
}
