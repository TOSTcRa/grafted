//! WindowServer bridge — maps Darwin window operations to X11.
//!
//! On macOS, apps talk to WindowServer via Mach IPC to create windows,
//! display content, and receive events. We bridge this to X11 using
//! runtime-loaded libX11 (dlopen) so the binary works headless if X11
//! is unavailable.
//!
//! Architecture:
//!   NSWindow → CGWindow (ID) → WindowServer bridge → X11 Window
//!   CGContext pixel buffer → XImage → XPutImage → X11 Window
//!   X11 Event → CFRunLoopSource → NSEvent

pub mod x11;
pub mod display;

pub use display::*;
